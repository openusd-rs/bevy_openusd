//! UsdGeom → `bevy::render::mesh::Mesh`.
//!
//! Two kinds of input:
//! - Full meshes (`UsdGeom.Mesh`) — converts points / face indices / normals
//!   / uvs, fan-triangulates faces > 3 verts, expands `faceVarying` primvars.
//! - Primitive shapes (`Cube`, `Sphere`, `Cylinder`, `Capsule`) — delegate
//!   to Bevy's built-in `Meshable` primitives with the right dimensions.
//!
//! Orientation (`"leftHanded"` flips winding) and missing-normal fallback
//! (flat for polygonal/bilinear meshes, smooth otherwise) are handled here.

use crate::read::geom::{Axis, Interpolation, MeshPrimvar, Orientation, ReadCylinder, ReadMesh};
use bevy::asset::RenderAssetUsages;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, Meshable, PrimitiveTopology, VertexAttributeValues};

pub(crate) mod compact;
pub mod bounds;
pub(crate) mod affine;

/// Convert a `crate::read::geom::ReadMesh` into a Bevy mesh.
///
/// Steps:
/// 1. Triangulate polygon faces.
/// 2. Expand vertices for face/corner primvars and generated flat normals.
/// 3. Generate flat or smooth normals according to the subdivision scheme.
/// 4. Flip index winding when `orientation == LeftHanded`.
pub fn mesh_from_usd(read: &ReadMesh) -> Mesh {
    mesh_from_usd_subset(read, None)
}

/// Whether per-face or per-corner primvars require duplicated render vertices.
fn expands_vertices(read: &ReadMesh) -> bool {
    let non_indexed = |interp: Interpolation| {
        matches!(interp, Interpolation::FaceVarying | Interpolation::Uniform)
    };
    uses_flat_normals(read) || read
        .normals
        .as_ref()
        .map(|p| non_indexed(p.interpolation))
        .unwrap_or(false)
        || read
            .uvs
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false)
        || read
            .display_color
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false)
        || read
            .display_opacity
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false)
}

pub(crate) fn uses_flat_normals(read: &ReadMesh) -> bool {
    read.normals.is_none() && matches!(read.subdivision_scheme,
        crate::read::geom::SubdivScheme::None | crate::read::geom::SubdivScheme::Bilinear)
}

fn corner_points(read: &ReadMesh) -> Vec<usize> {
    let count: usize = read.face_vertex_counts.iter().map(|count| (*count).max(0) as usize).sum();
    (0..count).map(|corner| {
        let raw = read.face_vertex_indices.get(corner).copied().unwrap_or(0);
        (raw.max(0) as usize).min(read.points.len().saturating_sub(1))
    }).collect()
}

fn flat_corner_indices(read: &ReadMesh, subset: Option<&[i32]>) -> Vec<u32> {
    let positions: Vec<_> = corner_points(read).into_iter()
        .map(|point| triangulation_points(read).get(point).copied().unwrap_or([0.0; 3])).collect();
    let corners: Vec<_> = (0..positions.len()).map(|corner| corner as i32).collect();
    triangulate_mesh(read, &positions, &corners, subset)
}

fn triangulate_mesh(read: &ReadMesh, positions: &[[f32; 3]], indices: &[i32], subset: Option<&[i32]>) -> Vec<u32> {
    if read.hole_indices.is_empty() {
        return triangulate_polygon(positions, &read.face_vertex_counts, indices, read.orientation, subset);
    }
    let holes: std::collections::HashSet<_> = read.hole_indices.iter().copied().collect();
    let faces: Vec<_> = match subset {
        Some(faces) => faces.iter().copied().filter(|face| !holes.contains(face)).collect(),
        None => (0..read.face_vertex_counts.len() as i32).filter(|face| !holes.contains(face)).collect(),
    };
    triangulate_polygon(positions, &read.face_vertex_counts, indices, read.orientation, Some(&faces))
}

fn triangulation_points(read: &ReadMesh) -> &[[f32; 3]] {
    read.triangulation_points.as_deref().filter(|points| points.len() == read.points.len())
        .unwrap_or(&read.points)
}

/// Source USD point index for each emitted render vertex, including seam copies.
pub fn vertex_point_indices(read: &ReadMesh) -> Vec<usize> {
    if !expands_vertices(read) { return (0..read.points.len()).collect(); }
    let points = corner_points(read);
    if uses_flat_normals(read) {
        flat_corner_indices(read, None).into_iter().map(|corner| points[corner as usize]).collect()
    } else { points }
}

/// Indices for selected faces in the full mesh's render-vertex layout.
pub(crate) fn mesh_indices_for_faces(read: &ReadMesh, faces: &[i32]) -> Indices {
    let indices = if uses_flat_normals(read) {
        select_flat_indices(read, &flat_corner_indices(read, None), Some(faces))
    } else if expands_vertices(read) {
        flat_corner_indices(read, Some(faces))
    } else {
        triangulate_mesh(read, triangulation_points(read), &read.face_vertex_indices, Some(faces))
    };
    Indices::U32(indices)
}

fn select_flat_indices(read: &ReadMesh, triangles: &[u32], subset: Option<&[i32]>) -> Vec<u32> {
    let selected = subset.map(|faces| {
        flat_corner_indices(read, Some(faces)).chunks_exact(3)
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect::<std::collections::HashSet<_>>()
    });
    triangles.chunks_exact(3).enumerate().filter(|(_, triangle)| {
        selected.as_ref().is_none_or(|selected| selected.contains(&[triangle[0], triangle[1], triangle[2]]))
    }).flat_map(|(index, _)| (index as u32 * 3)..(index as u32 * 3 + 3)).collect()
}

/// Builds all faces or the supplied face subset, retaining the vertex layout.
pub fn mesh_from_usd_subset(read: &ReadMesh, face_subset: Option<&[i32]>) -> Mesh {
    let expand = expands_vertices(read);

    let (positions, normals, uvs, colors, indices) = if uses_flat_normals(read) {
        build_flat(read, face_subset)
    } else if expand {
        build_expanded(read, face_subset)
    } else {
        build_indexed(read, face_subset)
    };

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    // USD's `primvars:st` convention puts (0,0) at the texture's
    // bottom-left corner. Bevy / glTF / wgpu use top-left, so V is
    // inverted between the two systems. Flip on the way in so the
    // authored texture lands right-side-up — without this, eyes paint
    // on tails, etc.
    let uvs: Vec<[f32; 2]> = uvs.into_iter().map(|[u, v]| [u, 1.0 - v]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(cs) = colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cs);
    }
    // Indices first so `compute_smooth_normals` has a topology to
    // average across — it requires an indexed mesh to find adjacent
    // faces.
    mesh.insert_indices(Indices::U32(indices));
    if let Some(ns) = normals {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, ns);
    } else {
        mesh.compute_smooth_normals();
    }
    if read.uvs.is_some() {
        if let Err(e) = mesh.generate_tangents() {
            bevy::log::debug!("mesh: generate_tangents failed: {e}");
        }
    }
    mesh
}

/// Assembled mesh attributes: `(positions, normals?, uvs, colors?, indices)`.
type BuiltMesh = (
    Vec<[f32; 3]>,
    Option<Vec<[f32; 3]>>,
    Vec<[f32; 2]>,
    Option<Vec<[f32; 4]>>,
    Vec<u32>,
);

/// Build the common case: indexed triangle list, one vertex per USD point.
/// Uses vertex-level or constant interpolation only.
fn build_indexed(read: &ReadMesh, face_subset: Option<&[i32]>) -> BuiltMesh {
    let positions = read.points.clone();

    // Preserve authored normals; generate angle-weighted normals otherwise.
    let normals = read.normals.as_ref().and_then(|p| match p.interpolation {
        Interpolation::Vertex | Interpolation::Varying => {
            Some(expand_vertex_primvar(p, positions.len(), [0.0, 1.0, 0.0]))
        }
        Interpolation::Constant if !p.values.is_empty() => Some(vec![corner_normal(read, 0, 0, 0); positions.len()]),
        _ => None,
    }).or_else(|| Some(compute_point_smooth_normals(read)));

    let uvs = read
        .uvs
        .as_ref()
        .and_then(|p| match p.interpolation {
            Interpolation::Vertex | Interpolation::Varying => {
                Some(expand_vertex_primvar(p, positions.len(), [0.0, 0.0]))
            }
            Interpolation::Constant => Some(vec![corner_uv(read, 0, 0, 0); positions.len()]),
            _ => None,
        })
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    let colors = build_vertex_colors(read.display_color.as_ref(), read.display_opacity.as_ref(), positions.len());

    let indices = triangulate_mesh(
        read,
        triangulation_points(read),
        &read.face_vertex_indices,
        face_subset,
    );
    (positions, normals, uvs, colors, indices)
}

/// Expand constant, vertex or varying display primvars into vertex RGBA.
pub(crate) fn build_vertex_colors(display_color: Option<&MeshPrimvar<[f32; 3]>>, display_opacity: Option<&MeshPrimvar<f32>>, vertex_count: usize) -> Option<Vec<[f32; 4]>> {
    if display_color.is_none() && display_opacity.is_none() {
        return None;
    }
    let mut colors = vec![[1.0f32, 1.0, 1.0, 1.0]; vertex_count];
    if let Some(dc) = display_color {
        let rgbs = match dc.interpolation {
            Interpolation::Constant if !dc.values.is_empty() => {
                vec![sample_primvar_3(dc, 0, 0, 0, [1.0;3]); vertex_count]
            }
            // Single-value primvar — broadcast regardless of which
            // interpolation token was authored. Pixar's Kitchen_set
            // authors `primvars:displayColor = [(0.5, 0.5, 0.4)]`
            // with no `interpolation` token; the schema reader's
            // default of `Vertex` then fails to expand a 1-element
            // array to vertex_count and falls through to white.
            _ if dc.values.len() == 1 && dc.indices.is_empty() => vec![dc.values[0]; vertex_count],
            // `Varying` is semantically per-vertex for polygonal meshes,
            // so it rides the same indexed path as `Vertex`.
            Interpolation::Vertex | Interpolation::Varying => {
                expand_vertex_primvar(dc, vertex_count, [1.0, 1.0, 1.0])
            }
            _ => vec![[1.0, 1.0, 1.0]; vertex_count],
        };
        for (i, rgb) in rgbs.iter().enumerate() {
            colors[i][0] = rgb[0];
            colors[i][1] = rgb[1];
            colors[i][2] = rgb[2];
        }
    }
    if let Some(dop) = display_opacity {
        let alphas = match dop.interpolation {
            Interpolation::Constant if !dop.values.is_empty() => {
                vec![sample_primvar_1(dop, 0, 0, 0, 1.0); vertex_count]
            }
            // Single-value primvar — broadcast regardless of declared
            // interpolation (see `display_color` arm above for the
            // Pixar Kitchen_set rationale).
            _ if dop.values.len() == 1 && dop.indices.is_empty() => vec![dop.values[0]; vertex_count],
            Interpolation::Vertex | Interpolation::Varying => {
                expand_vertex_primvar(dop, vertex_count, 1.0)
            }
            _ => vec![1.0; vertex_count],
        };
        for (i, a) in alphas.iter().enumerate() {
            colors[i][3] = *a;
        }
    }
    Some(colors)
}

/// Build the fully-expanded form: one vertex per face corner so `faceVarying`
/// primvars (cube uvs, seams) can be represented.
fn build_expanded(read: &ReadMesh, face_subset: Option<&[i32]>) -> BuiltMesh {
    let corner_count: usize = read.face_vertex_counts.iter().map(|c| (*c).max(0) as usize).sum();
    let mut positions = Vec::with_capacity(corner_count);
    let mut normals_out: Vec<[f32; 3]> = Vec::with_capacity(corner_count);
    let mut uvs_out: Vec<[f32; 2]> = Vec::with_capacity(corner_count);
    let mut colors_out: Vec<[f32; 4]> = Vec::with_capacity(corner_count);

    let want_normals = read.normals.is_some();
    let want_uvs = read.uvs.is_some();
    let want_colors = read.display_color.is_some() || read.display_opacity.is_some();

    // Smooth fallback normals are computed before corner expansion.
    let smooth_per_point: Option<Vec<[f32; 3]>> =
        (!want_normals && !uses_flat_normals(read)).then(|| compute_point_smooth_normals(read));

    let mut corner_ix: usize = 0;
    for (face_ix, face_verts) in read.face_vertex_counts.iter().enumerate() {
        for k in 0..((*face_verts).max(0) as usize) {
            // Tolerate malformed indices: a missing corner reads as 0, and an
            // index past the point buffer clamps to the last point (never OOB).
            let raw = read.face_vertex_indices.get(corner_ix + k).copied().unwrap_or(0);
            let point_ix = (raw.max(0) as usize).min(read.points.len().saturating_sub(1));
            positions.push(read.points.get(point_ix).copied().unwrap_or([0.0, 0.0, 0.0]));
            if want_normals {
                normals_out.push(corner_normal(read, face_ix, corner_ix + k, point_ix));
            } else if let Some(ref ns) = smooth_per_point {
                normals_out.push(*ns.get(point_ix).unwrap_or(&[0.0, 1.0, 0.0]));
            }
            if want_uvs {
                uvs_out.push(corner_uv(read, face_ix, corner_ix + k, point_ix));
            } else {
                // Pad UVs so `ATTRIBUTE_UV_0` always has the same
                // length as `ATTRIBUTE_POSITION` — mismatched lengths
                // make Bevy silently drop the mesh.
                uvs_out.push([0.0, 0.0]);
            }
            if want_colors {
                colors_out.push(corner_color(read, face_ix, corner_ix + k, point_ix));
            }
        }
        corner_ix += (*face_verts).max(0) as usize;
    }

    // After expansion, indices become sequential 0..N per face, then
    // fan-triangulated. Re-derive a pseudo `faceVertexIndices` of the form
    // [0,1,2,3, 4,5,6, …] so `triangulate_fan` can do its job.
    let mut sequential = Vec::with_capacity(corner_count);
    let mut running = 0u32;
    for face_verts in &read.face_vertex_counts {
        for _ in 0..*face_verts {
            sequential.push(running as i32);
            running += 1;
        }
    }
    let reference_positions: Vec<_> = corner_points(read).into_iter()
        .map(|point| triangulation_points(read).get(point).copied().unwrap_or([0.0; 3])).collect();
    let indices = triangulate_mesh(read, &reference_positions, &sequential, face_subset);

    let emit_normals = want_normals || smooth_per_point.is_some();
    (
        positions,
        emit_normals.then_some(normals_out),
        uvs_out,
        want_colors.then_some(colors_out),
        indices,
    )
}

/// Angle-weighted unit normals from visible triangles in the source point domain.
/// Unreferenced points receive zero normals.
fn compute_point_smooth_normals(read: &ReadMesh) -> Vec<[f32; 3]> {
    use bevy::math::DVec3;
    let mut accum = vec![DVec3::ZERO; read.points.len()];
    let triangles = triangulate_mesh(read, triangulation_points(read), &read.face_vertex_indices, None);
    for triangle in triangles.chunks_exact(3) {
        let indices = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
        let points = indices.map(|index| DVec3::from_array(read.points[index].map(f64::from)));
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        let Some(normal) = normal.try_normalize() else { continue };
        for corner in 0..3 {
            let a = points[(corner + 1) % 3] - points[corner];
            let b = points[(corner + 2) % 3] - points[corner];
            let angle = a.cross(b).length().atan2(a.dot(b));
            if angle.is_finite() { accum[indices[corner]] += normal * angle; }
        }
    }
    accum.into_iter().map(|normal| normal.normalize_or_zero().as_vec3().to_array()).collect()
}

/// Per-corner normals for validated topology, averaged only within smooth fans.
pub(crate) fn crease_corner_normals(
    read: &ReadMesh, hard_edges: &std::collections::BTreeSet<[usize; 2]>, hard_corners: &std::collections::BTreeSet<usize>,
) -> MeshPrimvar<[f32; 3]> {
    use bevy::math::DVec3;
    fn root(parents: &mut [usize], mut index: usize) -> usize {
        while parents[index] != index {
            parents[index] = parents[parents[index]];
            index = parents[index];
        }
        index
    }
    let points = corner_points(read);
    let mut parents: Vec<_> = (0..points.len()).collect();
    let mut edges = std::collections::BTreeMap::<[usize; 2], [usize; 2]>::new();
    let mut offset = 0;
    for &count in &read.face_vertex_counts {
        let count = count as usize;
        for i in 0..count {
            let corners = [offset + i, offset + (i + 1) % count];
            let [a,b] = corners.map(|corner| points[corner]);
            let key = [a.min(b), a.max(b)];
            if hard_edges.contains(&key) { continue; }
            if let Some(previous) = edges.insert(key, corners) {
                for corner in corners {
                    let point = points[corner];
                    if hard_corners.contains(&point) { continue; }
                    let other = if points[previous[0]] == point { previous[0] } else { previous[1] };
                    let a = root(&mut parents, corner);
                    let b = root(&mut parents, other);
                    parents[a] = b;
                }
            }
        }
        offset += count;
    }
    let mut sums = vec![DVec3::ZERO; points.len()];
    for triangle in flat_corner_indices(read, None).chunks_exact(3) {
        let corners = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
        let positions = corners.map(|corner| DVec3::from_array(read.points[points[corner]].map(f64::from)));
        let Some(normal) = (positions[1] - positions[0]).cross(positions[2] - positions[0]).try_normalize() else { continue };
        for i in 0..3 {
            let a = positions[(i + 1) % 3] - positions[i];
            let b = positions[(i + 2) % 3] - positions[i];
            let angle = a.cross(b).length().atan2(a.dot(b));
            if angle.is_finite() { sums[root(&mut parents, corners[i])] += normal * angle; }
        }
    }
    let values = (0..points.len()).map(|corner| sums[root(&mut parents, corner)].normalize_or_zero().as_vec3().to_array()).collect();
    MeshPrimvar { values, interpolation: Interpolation::FaceVarying, indices: Vec::new() }
}

fn build_flat(read: &ReadMesh, face_subset: Option<&[i32]>) -> BuiltMesh {
    let (positions, _, uvs, colors, triangles) = build_expanded(read, None);
    let output_indices = select_flat_indices(read, &triangles, face_subset);
    let mut output_positions = Vec::with_capacity(triangles.len());
    let mut output_normals = Vec::with_capacity(triangles.len());
    let mut output_uvs = Vec::with_capacity(triangles.len());
    let mut output_colors = colors.as_ref().map(|_| Vec::with_capacity(triangles.len()));
    for triangle in triangles.chunks_exact(3) {
        let [a,b,c] = [triangle[0], triangle[1], triangle[2]];
        let point = |index: u32| Vec3::from_array(positions[index as usize]).as_dvec3();
        let normal = (point(b) - point(a)).cross(point(c) - point(a))
            .try_normalize().map(|normal| normal.as_vec3()).unwrap_or(Vec3::Y).to_array();
        for &corner in triangle {
            output_positions.push(positions[corner as usize]);
            output_normals.push(normal);
            output_uvs.push(uvs[corner as usize]);
            if let (Some(input), Some(output)) = (&colors, &mut output_colors) {
                output.push(input[corner as usize]);
            }
        }
    }
    (output_positions, Some(output_normals), output_uvs, output_colors, output_indices)
}

fn corner_normal(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 3] {
    let p = read.normals.as_ref().unwrap();
    if p.interpolation == Interpolation::Constant {
        let index = p.indices.first().copied().unwrap_or(0);
        return usize::try_from(index).ok().and_then(|index| p.values.get(index)).copied().unwrap_or([0.0, 1.0, 0.0]);
    }
    sample_primvar_3(p, face, corner, point, [0.0, 1.0, 0.0])
}

fn corner_color(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 4] {
    let rgb_fallback = [1.0_f32, 1.0, 1.0];
    let rgb = read
        .display_color
        .as_ref()
        .map(|dc| sample_primvar_3(dc, face, corner, point, rgb_fallback))
        .unwrap_or(rgb_fallback);
    let a = read
        .display_opacity
        .as_ref()
        .map(|dop| sample_primvar_1(dop, face, corner, point, 1.0))
        .unwrap_or(1.0);
    [rgb[0], rgb[1], rgb[2], a]
}

/// Sample a vec3 primvar at a specific corner. Single-value primvars
/// broadcast regardless of declared interpolation — Pixar's
/// Kitchen_set authors `primvars:displayColor = [(0.5, 0.5, 0.4)]`
/// without an `interpolation` token; the schema reader's default of
/// `Vertex` then fails to expand the 1-element array to vertex_count
/// and falls back to white.
fn sample_primvar_3(
    p: &MeshPrimvar<[f32; 3]>,
    face: usize,
    corner: usize,
    point: usize,
    fallback: [f32; 3],
) -> [f32; 3] {
    if p.values.len() == 1 && p.indices.is_empty() {
        return p.values[0];
    }
    let lookup = |slot: usize| -> [f32; 3] {
        let ix = if !p.indices.is_empty() {
            *p.indices.get(slot).unwrap_or(&0) as usize
        } else {
            slot
        };
        p.values.get(ix).copied().unwrap_or(fallback)
    };
    match p.interpolation {
        Interpolation::Constant => lookup(0),
        Interpolation::Uniform => lookup(face),
        Interpolation::Vertex | Interpolation::Varying => lookup(point),
        Interpolation::FaceVarying => lookup(corner),
    }
}

fn sample_primvar_1(
    p: &MeshPrimvar<f32>,
    face: usize,
    corner: usize,
    point: usize,
    fallback: f32,
) -> f32 {
    if p.values.len() == 1 && p.indices.is_empty() {
        return p.values[0];
    }
    let lookup = |slot: usize| -> f32 {
        let ix = if !p.indices.is_empty() {
            *p.indices.get(slot).unwrap_or(&0) as usize
        } else {
            slot
        };
        p.values.get(ix).copied().unwrap_or(fallback)
    };
    match p.interpolation {
        Interpolation::Constant => lookup(0),
        Interpolation::Uniform => lookup(face),
        Interpolation::Vertex | Interpolation::Varying => lookup(point),
        Interpolation::FaceVarying => lookup(corner),
    }
}

fn corner_uv(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 2] {
    let p = read.uvs.as_ref().unwrap();
    let fallback = [0.0, 0.0];
    if p.values.len() == 1 && p.indices.is_empty() {
        return p.values[0];
    }
    match p.interpolation {
        Interpolation::Constant => {
            let index = p.indices.first().copied().unwrap_or(0) as usize;
            p.values.get(index).copied().unwrap_or(fallback)
        },
        Interpolation::Uniform => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(face).unwrap_or(&0) as usize
            } else {
                face
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
        Interpolation::Vertex | Interpolation::Varying => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(point).unwrap_or(&0) as usize
            } else {
                point
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
        Interpolation::FaceVarying => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(corner).unwrap_or(&0) as usize
            } else {
                corner
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
    }
}

fn expand_vertex_primvar<T: Copy>(
    primvar: &MeshPrimvar<T>,
    expected_len: usize,
    fallback: T,
) -> Vec<T> {
    // Always emit exactly `expected_len` entries — Bevy 0.18 silently
    // drops the mesh if attribute lengths don't match
    // `ATTRIBUTE_POSITION`. Pad with `fallback` if the authored data
    // is short, truncate if it's long.
    let mut out = vec![fallback; expected_len];
    if primvar.indices.is_empty() {
        for (i, v) in primvar.values.iter().take(expected_len).enumerate() {
            out[i] = *v;
        }
    } else {
        for (i, ix) in primvar.indices.iter().take(expected_len).enumerate() {
            if let Some(v) = primvar.values.get(*ix as usize) {
                out[i] = *v;
            }
        }
    }
    out
}

/// Triangulate each face into a triangle list. Smart enough for the three
/// cases real USD assets throw at us:
///
/// - **n = 3**: emit as-is.
/// - **n = 4** (the dominant case in production assets): pick the *shorter*
///   diagonal. Non-planar quads — almost universal in subdivided cages and
///   imported FBX — produce a visible crease along whichever diagonal a fan
///   triangulator picks. Choosing the diagonal that minimises the triangle
///   pair's perimeter aligns the crease with the surface curvature, which
///   is what every offline renderer (and Maya/Blender's default) does.
/// - **n ≥ 5**: ear-clip. Fan triangulation of a concave n-gon emits
///   triangles *outside* the polygon (showing through to the back) and
///   misses parts inside it — which is exactly the "spiky / missing
///   triangles" symptom on production-asset n-gons. Ear clipping handles
///   concave faces correctly. We compute the polygon normal via Newell's
///   method (works on non-planar polygons too) and pick ears in 2D after
///   projecting onto the plane perpendicular to that normal.
///
/// Falls back to fan triangulation if the polygon is degenerate (all colinear
/// points) — emitting *something* matches USD's permissive behaviour.
///
/// `LeftHanded` orientation flips the winding so Bevy's default back-face
/// culling shows the right side.
///
/// `face_subset = Some(&[face_ix])` emits only the listed faces — used by
/// the GeomSubset per-material split.
fn triangulate_polygon(
    positions: &[[f32; 3]],
    counts: &[i32],
    indices: &[i32],
    orientation: Orientation,
    face_subset: Option<&[i32]>,
) -> Vec<u32> {
    // No vertices → no triangles. Emitting indices into an empty buffer would
    // later panic Bevy's normal/tangent generation.
    if positions.is_empty() {
        return Vec::new();
    }
    // Precompute each face's starting corner so a subset by face index
    // jumps straight to the right slice without rewalking the counts. Negative
    // counts (malformed USD) contribute zero rather than wrapping to a huge
    // `usize` that would overflow the running sum.
    let mut face_starts = Vec::with_capacity(counts.len());
    let mut running = 0usize;
    for c in counts {
        face_starts.push(running);
        running += (*c).max(0) as usize;
    }

    let face_iter: Box<dyn Iterator<Item = usize>> = match face_subset {
        None => Box::new(0..counts.len()),
        Some(sub) => Box::new(
            sub.iter()
                .map(|i| *i as usize)
                .filter(|i| *i < counts.len()),
        ),
    };

    let mut out = Vec::new();
    let emit = |out: &mut Vec<u32>, a: u32, b: u32, c: u32| match orientation {
        Orientation::RightHanded => out.extend_from_slice(&[a, b, c]),
        Orientation::LeftHanded => out.extend_from_slice(&[a, c, b]),
    };
    let nv = positions.len();
    // Read a corner's vertex index, tolerating an index array shorter than the
    // counts imply (malformed USD) — a missing corner reads as index 0.
    let idx_at = |c: usize| -> i32 { indices.get(c).copied().unwrap_or(0) };
    // Clamp a raw (possibly out-of-range or negative) point index so an emitted
    // mesh index never points past the vertex buffer (which the GPU would read
    // out of bounds).
    let clamp_v = |i: i32| -> u32 {
        if nv == 0 {
            0
        } else {
            (i.max(0) as usize).min(nv - 1) as u32
        }
    };
    let pos_of = |idx: i32| -> Vec3 {
        match positions.get(idx.max(0) as usize) {
            Some(p) => Vec3::new(p[0], p[1], p[2]),
            None => Vec3::ZERO,
        }
    };

    for face_ix in face_iter {
        let face_start = face_starts[face_ix];
        let n = counts[face_ix].max(0) as usize;
        if n < 3 {
            continue;
        }
        if n == 3 {
            let a = clamp_v(idx_at(face_start));
            let b = clamp_v(idx_at(face_start + 1));
            let c = clamp_v(idx_at(face_start + 2));
            emit(&mut out, a, b, c);
            continue;
        }
        if n == 4 {
            let i0 = idx_at(face_start);
            let i1 = idx_at(face_start + 1);
            let i2 = idx_at(face_start + 2);
            let i3 = idx_at(face_start + 3);
            // Pick the shorter diagonal: 0–2 vs 1–3.
            let p0 = pos_of(i0);
            let p1 = pos_of(i1);
            let p2 = pos_of(i2);
            let p3 = pos_of(i3);
            let d02 = (p2 - p0).length_squared();
            let d13 = (p3 - p1).length_squared();
            if d02 <= d13 {
                emit(&mut out, clamp_v(i0), clamp_v(i1), clamp_v(i2));
                emit(&mut out, clamp_v(i0), clamp_v(i2), clamp_v(i3));
            } else {
                emit(&mut out, clamp_v(i1), clamp_v(i2), clamp_v(i3));
                emit(&mut out, clamp_v(i1), clamp_v(i3), clamp_v(i0));
            }
            continue;
        }
        // n >= 5: ear clip. Clamp the slice end so a counts/indices mismatch
        // can't panic; skip the face if fewer than a triangle survives. Corner
        // indices are pre-clamped so ear-clip's emitted mesh indices stay valid.
        let end = (face_start + n).min(indices.len());
        if end.saturating_sub(face_start) < 3 {
            continue;
        }
        let face_indices: Vec<i32> = indices[face_start..end]
            .iter()
            .map(|i| clamp_v(*i) as i32)
            .collect();
        let face_positions: Vec<Vec3> = face_indices.iter().map(|i| pos_of(*i)).collect();
        ear_clip_into(&face_positions, &face_indices, &mut out, orientation);
    }
    out
}

/// Ear-clip a polygon (n ≥ 4 in practice) into triangles, appending into
/// `out`. Robust against concave polygons; for non-planar polygons we
/// project onto the plane perpendicular to the Newell normal so the 2D
/// containment test is meaningful.
///
/// Falls back to fan triangulation if no ears can be found (e.g. fully
/// degenerate / self-intersecting input). That matches USD's "translate
/// what you can, drop nothing" expectation.
fn ear_clip_into(
    positions: &[Vec3],
    indices: &[i32],
    out: &mut Vec<u32>,
    orientation: Orientation,
) {
    let n = positions.len();
    let emit = |out: &mut Vec<u32>, a: u32, b: u32, c: u32| match orientation {
        Orientation::RightHanded => out.extend_from_slice(&[a, b, c]),
        Orientation::LeftHanded => out.extend_from_slice(&[a, c, b]),
    };

    // Newell's method: robust normal even for non-planar polygons. Sums
    // per-edge cross-products of the projected components.
    let mut normal = Vec3::ZERO;
    for i in 0..n {
        let cur = positions[i];
        let nxt = positions[(i + 1) % n];
        normal.x += (cur.y - nxt.y) * (cur.z + nxt.z);
        normal.y += (cur.z - nxt.z) * (cur.x + nxt.x);
        normal.z += (cur.x - nxt.x) * (cur.y + nxt.y);
    }
    if normal.length_squared() < 1e-20 {
        // Degenerate polygon — fall back to fan.
        for k in 1..(n - 1) {
            emit(
                out,
                indices[0] as u32,
                indices[k] as u32,
                indices[k + 1] as u32,
            );
        }
        return;
    }
    let normal = normal.normalize();

    // Build orthonormal basis (u, v) on the polygon plane to project into
    // 2D. Pick the smallest absolute component of the normal as the
    // helper axis to avoid degeneracy.
    let helper = if normal.x.abs() < normal.y.abs() && normal.x.abs() < normal.z.abs() {
        Vec3::X
    } else if normal.y.abs() < normal.z.abs() {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let u = normal.cross(helper).normalize();
    let v = normal.cross(u);
    let project = |p: Vec3| -> [f32; 2] { [p.dot(u), p.dot(v)] };

    let pts2: Vec<[f32; 2]> = positions.iter().map(|p| project(*p)).collect();

    // Determine polygon winding in 2D. Signed area > 0 → CCW.
    let mut signed_area = 0.0f32;
    for i in 0..n {
        let a = pts2[i];
        let b = pts2[(i + 1) % n];
        signed_area += a[0] * b[1] - b[0] * a[1];
    }
    let ccw = signed_area > 0.0;

    // Active vertex list (linked-list-style via Vec).
    let mut remaining: Vec<usize> = (0..n).collect();
    // Worst-case ear clipping is O(n²) but n is tiny (≤ ~10 in practice).
    let max_iters = n * n + 8;
    let mut iters = 0;
    while remaining.len() > 3 && iters < max_iters {
        iters += 1;
        let m = remaining.len();
        let mut clipped = false;
        for i in 0..m {
            let i_prev = remaining[(i + m - 1) % m];
            let i_cur = remaining[i];
            let i_next = remaining[(i + 1) % m];
            let a = pts2[i_prev];
            let b = pts2[i_cur];
            let c = pts2[i_next];
            // Convex test in chosen winding.
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            let convex = if ccw { cross > 0.0 } else { cross < 0.0 };
            if !convex {
                continue;
            }
            // Ear test: no other remaining vertex inside triangle (a,b,c).
            let mut contains_other = false;
            for &idx in &remaining {
                if idx == i_prev || idx == i_cur || idx == i_next {
                    continue;
                }
                let p = pts2[idx];
                if point_in_triangle_2d(p, a, b, c) {
                    contains_other = true;
                    break;
                }
            }
            if contains_other {
                continue;
            }
            // Emit and clip.
            emit(
                out,
                indices[i_prev] as u32,
                indices[i_cur] as u32,
                indices[i_next] as u32,
            );
            remaining.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // No ear found — bail out and fan-triangulate the rest.
            break;
        }
    }
    if remaining.len() == 3 {
        emit(
            out,
            indices[remaining[0]] as u32,
            indices[remaining[1]] as u32,
            indices[remaining[2]] as u32,
        );
    } else if remaining.len() > 3 {
        // Fallback: fan over what's left.
        let r0 = remaining[0];
        for k in 1..(remaining.len() - 1) {
            emit(
                out,
                indices[r0] as u32,
                indices[remaining[k]] as u32,
                indices[remaining[k + 1]] as u32,
            );
        }
    }
}

/// Standard barycentric inside-triangle test. Includes points exactly on
/// edges (we still emit ears even if a vertex sits on an edge — the
/// alternative is endless retries on collinear data).
fn point_in_triangle_2d(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d_x = p[0] - c[0];
    let d_y = p[1] - c[1];
    let denom = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if denom.abs() < 1e-20 {
        return false;
    }
    let s = ((b[1] - c[1]) * d_x + (c[0] - b[0]) * d_y) / denom;
    let t = ((c[1] - a[1]) * d_x + (a[0] - c[0]) * d_y) / denom;
    s > 0.0 && t > 0.0 && (s + t) < 1.0
}

// ── Primitive shapes ────────────────────────────────────────────────────

/// Build a Bevy mesh from a UsdGeom.Cube's `size`. The USD cube is
/// size × size × size centred at the prim origin.
pub fn mesh_cube(size: f64) -> Mesh {
    Mesh::from(bevy::math::primitives::Cuboid::new(
        size as f32,
        size as f32,
        size as f32,
    ))
}

/// UsdGeom.Sphere radius → Bevy's UV sphere.
pub fn mesh_sphere(radius: f64) -> Mesh {
    Mesh::from(bevy::math::primitives::Sphere::new(radius as f32))
}

/// UsdGeom.Cylinder dimensions + axis. Bevy's `Cylinder` points up the Y
/// axis by convention, so we apply an axis rotation for X/Z cases.
pub fn mesh_cylinder(params: ReadCylinder) -> Mesh {
    let mut mesh = Mesh::from(bevy::math::primitives::Cylinder::new(
        params.radius as f32,
        params.height as f32,
    ));
    apply_axis(&mut mesh, params.axis);
    mesh
}

/// UsdGeom.Plane `width` × `length`. Y-normal plane centred at the origin.
pub fn mesh_plane(width: f64, length: f64) -> Mesh {
    Mesh::from(
        bevy::math::primitives::Plane3d::default()
            .mesh()
            .size(width as f32, length as f32),
    )
}

/// UsdGeom.Capsule dimensions + axis. Bevy's `Capsule3d` is Y-axis aligned.
pub fn mesh_capsule(params: ReadCylinder) -> Mesh {
    // UsdGeom.Capsule's `height` is the cylinder portion length (hemispheres
    // add `2*radius` to the total). Bevy's Capsule3d takes `half_length` =
    // half the cylinder portion.
    let mut mesh = Mesh::from(bevy::math::primitives::Capsule3d::new(
        params.radius as f32,
        params.height as f32,
    ));
    apply_axis(&mut mesh, params.axis);
    mesh
}

/// Rotate vertices so a Y-up primitive faces the requested axis.
fn apply_axis(mesh: &mut Mesh, axis: Axis) {
    let rot = match axis {
        Axis::Y => return,
        Axis::X => bevy::math::Quat::from_rotation_z(-core::f32::consts::FRAC_PI_2),
        Axis::Z => bevy::math::Quat::from_rotation_x(core::f32::consts::FRAC_PI_2),
    };
    rotate_mesh(mesh, rot);
}

pub fn rotate_mesh(mesh: &mut Mesh, rot: bevy::math::Quat) {
    if let Some(VertexAttributeValues::Float32x3(ps)) = mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in ps.iter_mut() {
            let v = rot * Vec3::new(p[0], p[1], p[2]);
            *p = [v.x, v.y, v.z];
        }
    }
    if let Some(VertexAttributeValues::Float32x3(ns)) = mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL) {
        for n in ns.iter_mut() {
            let v = rot * Vec3::new(n[0], n[1], n[2]);
            *n = [v.x, v.y, v.z];
        }
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;
    use crate::read::geom::SubdivScheme;

    #[test]
    fn crease_normals_split_only_sharp_fans_and_preserve_subset_mapping() {
        use std::collections::BTreeSet;
        for scale in [1e-20_f32, 1.0, 1e20] {
            let mut read = mesh(vec![[0.0,0.0,0.0],[scale,0.0,0.0],[0.0,scale,0.0],[0.0,0.0,scale]], vec![3,3], vec![0,1,2,1,0,3]);
            let smooth = crease_corner_normals(&read, &BTreeSet::new(), &BTreeSet::new());
            assert_eq!(smooth.values[0], smooth.values[4]);
            assert!((smooth.values[0][1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
            let corner = crease_corner_normals(&read, &BTreeSet::new(), &BTreeSet::from([0]));
            assert_eq!(corner.values[0], [0.0,0.0,1.0]);
            assert_eq!(corner.values[4], [0.0,1.0,0.0]);
            assert_eq!(corner.values[1], corner.values[3]);
            let sharp = crease_corner_normals(&read, &BTreeSet::from([[0,1]]), &BTreeSet::new());
            assert_eq!(&sharp.values[..3], &[[0.0,0.0,1.0];3]);
            assert_eq!(&sharp.values[3..], &[[0.0,1.0,0.0];3]);
            read.normals = Some(sharp);
            let converted = mesh_from_usd_subset(&read, Some(&[1]));
            assert_eq!(vertex_point_indices(&read), [0,1,2,1,0,3]);
            let Some(VertexAttributeValues::Float32x3(normals)) = converted.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
            for index in converted.indices().unwrap().iter() { assert_eq!(normals[index], [0.0,1.0,0.0]); }
        }
    }

    #[test]
    fn sampled_holes_preserve_source_primvars_and_subset_vertex_layouts() {
        let source = crate::UsdSource::new("sampled-holes.usda", br#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0),(1,0,0),(0,1,0),(2,0,0),(3,0,0),(2,1,0)]
    int[] faceVertexCounts = [3,3]
    int[] faceVertexIndices = [0,1,2,3,4,5]
    int[] holeIndices.timeSamples = { 0: [0,0], 10: [1], 20: [0,1] }
    color3f[] primvars:displayColor = [(1,0,0),(0,0,1)] ( interpolation = "uniform" )
    texCoord2f[] primvars:st = [(0,0),(1,1)] ( interpolation = "faceVarying" )
    int[] primvars:st:indices = [1,1,1,0,0,0]
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/M").unwrap();
        for scheme in [SubdivScheme::None, SubdivScheme::Bilinear, SubdivScheme::CatmullClark] {
            for time in [0.0, 10.0, 20.0] {
                let mut read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
                read.subdivision_scheme = scheme;
                let full = mesh_from_usd(&read);
                assert_eq!(full.indices().unwrap().len(), if time == 20.0 { 0 } else { 3 });
                assert_eq!(vertex_point_indices(&read).len(), full.count_vertices());
                for face in [0,1] {
                    let subset = mesh_from_usd_subset(&read, Some(&[face]));
                    assert_eq!(subset.count_vertices(), full.count_vertices());
                    let hidden = read.hole_indices.contains(&face);
                    assert_eq!(subset.indices().unwrap().len(), if hidden { 0 } else { 3 });
                    if !hidden {
                        let VertexAttributeValues::Float32x4(colors) = subset.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() else { panic!("colors") };
                        let VertexAttributeValues::Float32x2(uvs) = subset.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else { panic!("uvs") };
                        for index in subset.indices().unwrap().iter() {
                            assert_eq!(colors[index], if face == 0 { [1.0,0.0,0.0,1.0] } else { [0.0,0.0,1.0,1.0] });
                            assert_eq!(uvs[index], if face == 0 { [1.0,0.0] } else { [0.0,1.0] });
                        }
                    }
                }
                read.display_color = None;
                read.uvs = None;
                assert_eq!(mesh_from_usd(&read).indices().unwrap().len(), if time == 20.0 { 0 } else { 3 });
            }
        }
    }

    #[test]
    fn generated_normals_are_scale_independent_across_vertex_layouts() {
        for scale in [1e-20_f32, 1e-5, 0.01, 1.0, 1e20] {
            let mut read = mesh(vec![[0.0,0.0,0.0], [scale,0.0,0.0], [0.0,scale,0.0]], vec![3], vec![0,1,2]);
            read.subdivision_scheme = SubdivScheme::CatmullClark;
            for orientation in [Orientation::RightHanded, Orientation::LeftHanded] {
                read.orientation = orientation;
                let expected = if orientation == Orientation::RightHanded { [0.0,0.0,1.0] } else { [0.0,0.0,-1.0] };
                for expanded in [false, true] {
                    read.display_color = expanded.then(|| MeshPrimvar {
                        values: vec![[1.0;3]], interpolation: Interpolation::Uniform, indices: vec![],
                    });
                    let output = mesh_from_usd(&read);
                    let Some(VertexAttributeValues::Float32x3(normals)) = output.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
                    assert_eq!(normals, &[expected;3], "scale={scale} expanded={expanded}");
                }
            }
        }
    }

    #[test]
    fn constant_uv_indices_apply_to_flat_and_indexed_meshes() {
        use bevy::math::Vec2;
        let mut read = mesh(vec![[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0]], vec![3], vec![0,1,2]);
        read.uvs = Some(MeshPrimvar { values: vec![[0.1,0.2], [0.7,0.8]], interpolation: Interpolation::Constant, indices: vec![1] });
        for scheme in [SubdivScheme::None, SubdivScheme::CatmullClark] {
            read.subdivision_scheme = scheme;
            for subset in [None, Some([0].as_slice())] {
                let mesh = mesh_from_usd_subset(&read, subset);
                let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { panic!("uvs") };
                assert!(uvs.iter().all(|uv| Vec2::from_array(*uv).abs_diff_eq(Vec2::new(0.7,0.2), 1e-5)));
            }
        }
    }

    #[test]
    fn constant_display_indices_apply_to_flat_and_indexed_meshes() {
        let mut read = mesh(vec![[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0]], vec![3], vec![0,1,2]);
        read.display_color = Some(MeshPrimvar { values: vec![[1.0,0.0,0.0], [0.0,1.0,0.0]], interpolation: Interpolation::Constant, indices: vec![1] });
        read.display_opacity = Some(MeshPrimvar { values: vec![1.0,0.25], interpolation: Interpolation::Constant, indices: vec![1] });
        for scheme in [SubdivScheme::None, SubdivScheme::CatmullClark] {
            read.subdivision_scheme = scheme;
            for subset in [None, Some([0].as_slice())] {
                let mesh = mesh_from_usd_subset(&read, subset);
                let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
                assert!(colors.iter().all(|color| *color == [0.0,1.0,0.0,0.25]));
            }
        }
    }

    #[test]
    fn flat_normals_are_scale_independent_for_subsets_and_winding() {
        for scale in [1e-30_f32, 1e-20, 1e-5, 1.0, 1e20, 1e30] {
            let mut read = mesh(vec![[0.0,0.0,0.0], [scale,0.0,0.0], [0.0,scale,0.0]], vec![3], vec![0,1,2]);
            for scheme in [SubdivScheme::None, SubdivScheme::Bilinear] {
                read.subdivision_scheme = scheme;
                for orientation in [Orientation::RightHanded, Orientation::LeftHanded] {
                    read.orientation = orientation;
                    let expected = if orientation == Orientation::RightHanded { [0.0,0.0,1.0] } else { [0.0,0.0,-1.0] };
                    for subset in [None, Some([0].as_slice())] {
                        let output = mesh_from_usd_subset(&read, subset);
                        let Some(VertexAttributeValues::Float32x3(normals)) = output.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
                        assert_eq!(normals, &[expected;3], "scale={scale} scheme={scheme:?} subset={subset:?}");
                        assert_eq!(output.indices().unwrap().len(), 3);
                        assert_eq!(vertex_point_indices(&read).len(), 3);
                    }
                }
            }
        }
    }

    #[test]
    fn tangents_require_authored_uvs() {
        let mut read = mesh(vec![[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0]], vec![3], vec![0,1,2]);
        assert!(mesh_from_usd(&read).attribute(Mesh::ATTRIBUTE_TANGENT).is_none());
        read.uvs = Some(MeshPrimvar { values: vec![[0.0,0.0], [1.0,0.0], [0.0,1.0]],
            indices: vec![], interpolation: Interpolation::Vertex });
        assert!(mesh_from_usd(&read).attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
    }

    #[test]
    fn face_indices_match_full_subset_builders_across_vertex_layouts() {
        let base = mesh(
            vec![[0.0,0.0,0.0], [2.0,0.0,0.0], [1.0,0.5,0.0], [2.0,2.0,0.0], [0.0,2.0,0.0], [0.0,0.0,1.0]],
            vec![5,3], vec![0,1,2,3,4,0,5,1],
        );
        for layout in 0..6 {
            let mut read = base.clone();
            read.subdivision_scheme = if layout == 0 { SubdivScheme::None } else { SubdivScheme::CatmullClark };
            match layout {
                2 => read.normals = Some(MeshPrimvar { values: vec![[0.0,0.0,1.0]; 6], indices: vec![], interpolation: Interpolation::Vertex }),
                3 => read.uvs = Some(MeshPrimvar { values: vec![[0.0,0.0]; 8], indices: vec![], interpolation: Interpolation::FaceVarying }),
                4 => read.display_color = Some(MeshPrimvar { values: vec![[1.0,0.0,0.0]; 2], indices: vec![], interpolation: Interpolation::Uniform }),
                5 => read.normals = Some(MeshPrimvar { values: vec![[0.0,0.0,1.0]; 8], indices: vec![], interpolation: Interpolation::FaceVarying }),
                _ => {}
            }
            for orientation in [Orientation::LeftHanded, Orientation::RightHanded] {
                read.orientation = orientation;
                for holes in [vec![], vec![0], vec![1]] {
                    read.hole_indices = holes;
                    for faces in [&[][..], &[0], &[1], &[1,0], &[0,0], &[-1,8]] {
                        for deformed in [false, true] {
                            let mut sampled = read.clone();
                            if deformed {
                                sampled.triangulation_points = Some(sampled.points.clone());
                                sampled.points[2] = [0.5,1.5,0.8];
                            }
                            let expected = mesh_from_usd_subset(&sampled, Some(faces));
                            let indices = mesh_indices_for_faces(&sampled, faces);
                            assert_eq!(Some(&indices), expected.indices(), "layout={layout} orientation={orientation:?} faces={faces:?} deformed={deformed}");
                            assert!(indices.iter().all(|index| index < expected.count_vertices()));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn deformed_flat_faces_keep_reference_diagonals_and_subset_layout() {
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_test_simple.usda")).unwrap();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let reference = crate::read::geom::read_mesh_at(&stage, &path, Some(30.0)).unwrap().unwrap();
        let mapping = vertex_point_indices(&reference);
        let mut deformed = crate::route::skel::deformed_mesh(&crate::route::RouteCtx::at(&stage, &path, Some(30.0)))
            .unwrap().unwrap();
        assert_eq!(vertex_point_indices(&deformed), mapping);
        for face in 0..reference.face_vertex_counts.len() {
            let rest_subset = mesh_from_usd_subset(&reference, Some(&[face as i32]));
            let subset = mesh_from_usd_subset(&deformed, Some(&[face as i32]));
            assert_eq!(subset.indices(), rest_subset.indices());
            let VertexAttributeValues::Float32x3(positions) = subset.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            for (vertex, &point) in mapping.iter().enumerate() {
                assert_eq!(positions[vertex], deformed.points[point]);
            }
        }
        deformed.triangulation_points = None;
        assert_ne!(vertex_point_indices(&deformed), mapping);
    }

    #[test]
    fn polygonal_missing_normals_are_flat_with_stable_subset_vertex_maps() {
        let mut read = mesh(
            vec![[0.0,0.0,0.0], [1.0,0.0,0.0], [1.0,1.0,0.0], [0.0,1.0,0.0], [0.0,0.0,1.0]],
            vec![4,3], vec![0,1,2,3,0,4,1],
        );
        read.uvs = Some(MeshPrimvar { values: vec![[0.0,0.25]; 7], indices: vec![], interpolation: Interpolation::FaceVarying });
        for scheme in [SubdivScheme::None, SubdivScheme::Bilinear] {
            read.subdivision_scheme = scheme;
            for orientation in [Orientation::RightHanded, Orientation::LeftHanded] {
                read.orientation = orientation;
                let mapping = vertex_point_indices(&read);
                assert_eq!(mapping.len(), 9);
                let full = mesh_from_usd(&read);
                let VertexAttributeValues::Float32x3(positions) = full.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
                let VertexAttributeValues::Float32x3(normals) = full.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!("normals") };
                for (vertex, &point) in mapping.iter().enumerate() { assert_eq!(positions[vertex], read.points[point]); }
                for (triangle, ns) in positions.chunks_exact(3).zip(normals.chunks_exact(3)) {
                    let normal = (Vec3::from_array(triangle[1])-Vec3::from_array(triangle[0]))
                        .cross(Vec3::from_array(triangle[2])-Vec3::from_array(triangle[0])).normalize();
                    for n in ns { assert!(normal.distance(Vec3::from_array(*n)) < 1e-6); }
                }
                assert_ne!(normals[0], normals[6]);
                let subset = mesh_from_usd_subset(&read, Some(&[1]));
                assert_eq!(subset.attribute(Mesh::ATTRIBUTE_POSITION), full.attribute(Mesh::ATTRIBUTE_POSITION));
                assert_eq!(subset.attribute(Mesh::ATTRIBUTE_NORMAL), full.attribute(Mesh::ATTRIBUTE_NORMAL));
                assert_eq!(subset.indices().unwrap().iter().collect::<Vec<_>>(), vec![6,7,8]);
                assert_consistent(&subset);
            }
        }
        read.subdivision_scheme = SubdivScheme::CatmullClark;
        assert_eq!(vertex_point_indices(&read).len(), 7);
        read.subdivision_scheme = SubdivScheme::None;
        read.normals = Some(MeshPrimvar { values: vec![[0.0,1.0,0.0]], indices: vec![], interpolation: Interpolation::Constant });
        assert_eq!(vertex_point_indices(&read).len(), 7);
        let authored = mesh_from_usd(&read);
        let VertexAttributeValues::Float32x3(normals) = authored.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!("normals") };
        assert!(normals.iter().all(|normal| *normal == [0.0,1.0,0.0]));
    }

    #[test]
    fn indexed_sampled_normal_primvar_overrides_normals() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0), (0,0,1)]
    int[] faceVertexCounts = [3,3]
    int[] faceVertexIndices = [0,1,2,0,3,1]
    uniform token subdivisionScheme = "none"
    normal3f[] normals = [(1,0,0)] (interpolation = "constant")
    normal3f[] primvars:normals (interpolation = "faceVarying")
    normal3f[] primvars:normals.timeSamples = {
        0: [(0,0,1), (0,1,0)],
        10: [(0,0,-1), (0,-1,0)],
    }
    int[] primvars:normals:indices.timeSamples = {
        0: [0,0,0,1,1,1],
        10: [1,1,1,0,0,0],
    }
}
"#).open_stage().unwrap();
        let path = openusd::sdf::Path::new("/M").unwrap();
        for (time, first, second) in [
            (0.0, [0.0,0.0,1.0], [0.0,1.0,0.0]),
            (2.5, [0.0,0.0,0.5], [0.0,0.5,0.0]),
            (10.0, [0.0,-1.0,0.0], [0.0,0.0,-1.0]),
        ] {
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            assert_eq!(vertex_point_indices(&read), [0,1,2,0,3,1]);
            for subset in [None, Some(&[1][..])] {
                let mesh = mesh_from_usd_subset(&read, subset);
                let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
                    panic!("normal attribute missing");
                };
                assert_eq!(normals, &[first, first, first, second, second, second]);
                assert_consistent(&mesh);
            }
        }
    }

    #[test]
    fn normal_primvar_defaults_to_constant_and_absence_uses_normals() {
        for (primvar, expected, interpolation) in [
            ("normal3f[] primvars:normals = [(0,1,0)]", [0.0,1.0,0.0], Interpolation::Constant),
            ("", [1.0,0.0,0.0], Interpolation::Vertex),
        ] {
            let stage = crate::snippet::UsdSnippet::new(format!(r#"#usda 1.0
def Mesh "M" {{
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    normal3f[] normals = [(1,0,0), (1,0,0), (1,0,0)]
    {primvar}
}}
"#)).open_stage().unwrap();
            let read = crate::read::geom::read_mesh(&stage, &openusd::sdf::Path::new("/M").unwrap()).unwrap().unwrap();
            assert_eq!(read.normals.as_ref().unwrap().interpolation, interpolation);
            let mesh = mesh_from_usd(&read);
            let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
                panic!("normal attribute missing");
            };
            assert_eq!(normals, &[expected; 3]);
        }
    }

    #[test]
    fn inherited_normals_resolve_owner_samples_and_indices() {
        for (middle, local, expected) in [
            ("", "", [0.0, 1.0, 0.0]),
            ("normal3f[] primvars:normals", "", [0.0, 1.0, 0.0]),
            ("normal3f[] primvars:normals = None", "", [0.0, 1.0, 0.0]),
            ("normal3f[] primvars:normals = [(1,0,0)] (interpolation = \"vertex\")", "", [0.0, 1.0, 0.0]),
            ("normal3f[] primvars:normals = [(1,0,0)]", "", [1.0, 0.0, 0.0]),
            ("", "normal3f[] primvars:normals = [(0,0,-1)]", [0.0, 0.0, -1.0]),
        ] {
            let stage = crate::snippet::UsdSnippet::new(format!(r#"#usda 1.0
def Xform "Root" {{
    normal3f[] primvars:normals.timeSamples = {{ 0: [(0,0,1),(0,1,0)], 10: [(0,0,-1),(0,-1,0)] }}
    int[] primvars:normals:indices.timeSamples = {{ 0: [1], 10: [0] }}
    def Xform "Group" {{
        {middle}
        def Mesh "M" {{
            point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0,1,2]
            uniform token subdivisionScheme = "none"
            normal3f[] normals = [(1,0,0)] (interpolation = "constant")
            {local}
        }}
    }}
}}
"#)).open_stage().unwrap();
            let path = openusd::sdf::Path::new("/Root/Group/M").unwrap();
            for time in [0.0, 10.0] {
                let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
                let mesh = mesh_from_usd(&read);
                let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
                let inherited = expected == [0.0, 1.0, 0.0];
                assert_eq!(normals, &[if inherited && time == 10.0 { [0.0,0.0,-1.0] } else { expected }; 3]);
                assert_eq!(crate::live::prim_is_animated(&stage, &path), inherited);
            }
        }
    }

    /// Minimal `ReadMesh` with the geometry under test and everything else empty.
    fn mesh(points: Vec<[f32; 3]>, counts: Vec<i32>, indices: Vec<i32>) -> ReadMesh {
        ReadMesh {
            triangulation_points: None,
            points,
            face_vertex_counts: counts,
            face_vertex_indices: indices,
            hole_indices: Vec::new(),
            normals: None,
            uvs: None,
            orientation: Orientation::RightHanded,
            display_color: None,
            display_opacity: None,
            subsets: Vec::new(),
            double_sided: false,
            extent: None,
            subdivision_scheme: SubdivScheme::None,
        }
    }

    /// Bevy silently drops a mesh whose attribute lengths disagree with
    /// `ATTRIBUTE_POSITION`; every build path must keep them equal.
    fn assert_consistent(m: &Mesh) {
        let pos = m.count_vertices();
        if let Some(VertexAttributeValues::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0) {
            assert_eq!(uv.len(), pos, "uv length matches positions");
        }
        if let Some(VertexAttributeValues::Float32x3(n)) = m.attribute(Mesh::ATTRIBUTE_NORMAL) {
            assert_eq!(n.len(), pos, "normal length matches positions");
        }
    }

    #[test]
    fn out_of_range_indices_do_not_panic() {
        // A triangle references point 99 with only 3 points authored.
        let m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            vec![3],
            vec![0, 1, 99],
        );
        assert_consistent(&mesh_from_usd(&m));
    }

    #[test]
    fn counts_exceeding_indices_do_not_panic() {
        // Counts claim a quad, but only three indices exist.
        let m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            vec![4],
            vec![0, 1, 2],
        );
        assert_consistent(&mesh_from_usd(&m));
    }

    #[test]
    fn ngon_with_truncated_indices_does_not_panic() {
        // A declared 6-gon whose index array is too short — exercises the
        // ear-clip slice guard.
        let m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.]],
            vec![6],
            vec![0, 1, 2],
        );
        let _ = mesh_from_usd(&m);
    }

    #[test]
    fn negative_counts_and_indices_do_not_panic() {
        let m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            vec![-1, 3],
            vec![-5, 1, 2],
        );
        assert_consistent(&mesh_from_usd(&m));
    }

    #[test]
    fn expanded_path_out_of_range_does_not_panic() {
        // A faceVarying uv forces the expanded build path; indices still point
        // past the buffer.
        let mut m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            vec![3],
            vec![0, 1, 50],
        );
        m.uvs = Some(MeshPrimvar {
            values: vec![[0., 0.], [1., 0.], [0., 1.]],
            interpolation: Interpolation::FaceVarying,
            indices: Vec::new(),
        });
        assert_consistent(&mesh_from_usd(&m));
    }

    #[test]
    fn empty_points_with_faces_do_not_panic() {
        let m = mesh(Vec::new(), vec![3], vec![0, 1, 2]);
        let _ = mesh_from_usd(&m);
    }

    #[test]
    fn valid_quad_still_triangulates() {
        // Regression: a well-formed quad must still fan into two triangles.
        let m = mesh(
            vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            vec![4],
            vec![0, 1, 2, 3],
        );
        let mesh = mesh_from_usd(&m);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(6), "quad → 2 tris");
    }
}
