//! Uniform Catmull–Clark and bilinear refinement with bounded interpolation stencils.

use std::collections::{BTreeMap, BTreeSet};
use anyhow::{Result, ensure};
use crate::read::geom::{Interpolation, MeshPrimvar, ReadMesh, ReadSubset, SubdivScheme};
use crate::read::subdivision::ReadSubdivision;
#[cfg(test)]
use glam::DVec3;

const MAX_POINTS: usize = 1_000_000;
const MAX_CORNERS: usize = 4_000_000;
const MAX_WEIGHTS: usize = 16_000_000;
type Stencil = Vec<(usize, f64)>;

#[derive(Default)]
struct Sharpness {
    edges: BTreeMap<[usize; 2], f64>,
    corners: BTreeMap<usize, f64>,
}

fn decay(value: f64) -> f64 { if value >= 10.0 { 10.0 } else { (value - 1.0).max(0.0) } }

fn blend(a: &Stencil, b: &Stencil, weight: f64) -> Stencil {
    combine(a.iter().map(|&(index, value)| (index, value * weight))
        .chain(b.iter().map(|&(index, value)| (index, value * (1.0 - weight)))))
}

impl Sharpness {
    fn read(rules: Option<&ReadSubdivision>, faces: &[Vec<usize>]) -> Result<Self> {
        let mut result = Self::default();
        let Some(rules) = rules else { return Ok(result) };
        if rules.crease_indices.is_empty() && rules.corner_indices.is_empty() { return Ok(result); }
        let topology: BTreeSet<_> = faces.iter().flat_map(|face| face.iter().enumerate()
            .map(|(i, &a)| edge_key(a, face[(i + 1) % face.len()]))).collect();
        let per_chain = rules.crease_sharpnesses.len() == rules.crease_lengths.len();
        let mut offset = 0;
        let mut edge_offset = 0;
        for (chain, &length) in rules.crease_lengths.iter().enumerate() {
            let end = offset + length as usize;
            for pair in rules.crease_indices[offset..end].windows(2) {
                let edge = edge_key(pair[0] as usize, pair[1] as usize);
                ensure!(topology.contains(&edge), "crease does not name a mesh edge");
                let sharpness = rules.crease_sharpnesses[if per_chain { chain } else { edge_offset }];
                ensure!(result.edges.insert(edge, f64::from(sharpness)).is_none(), "duplicate crease edge");
                edge_offset += 1;
            }
            offset = end;
        }
        for (&index, &sharpness) in rules.corner_indices.iter().zip(&rules.corner_sharpnesses) {
            ensure!(result.corners.insert(index as usize, f64::from(sharpness)).is_none(), "duplicate sharp corner");
        }
        result.edges.retain(|_, value| *value > 0.0);
        result.corners.retain(|_, value| *value > 0.0);
        Ok(result)
    }

    fn vertex_mask(vertex: usize, corner: f64, edges: &[(usize, f64)], smooth: &Stencil) -> Stencil {
        let sharp: Vec<_> = edges.iter().filter(|(_, value)| *value > 0.0).collect();
        if corner > 0.0 || sharp.len() > 2 { vec![(vertex, 1.0)] }
        else if sharp.len() == 2 { vec![(vertex, 0.75), (sharp[0].0, 0.125), (sharp[1].0, 0.125)] }
        else { smooth.clone() }
    }

    fn vertex(vertex: usize, corner: f64, edges: &[(usize, f64)], smooth: Stencil) -> Stencil {
        let parent = Self::vertex_mask(vertex, corner, edges, &smooth);
        let child_edges: Vec<_> = edges.iter().map(|&(index, value)| (index, decay(value))).collect();
        let child = Self::vertex_mask(vertex, decay(corner), &child_edges, &smooth);
        let transitions: Vec<_> = edges.iter().map(|(_, value)| *value).chain(std::iter::once(corner))
            .filter(|value| *value > 0.0 && *value <= 1.0).collect();
        let weight = if transitions.is_empty() { 1.0 } else { transitions.iter().sum::<f64>() / transitions.len() as f64 };
        blend(&parent, &child, weight)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RefinementStep {
    vertex: Vec<Stencil>,
    varying: Vec<Stencil>,
    face_sizes: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryInterpolation { EdgeOnly, EdgeAndCorner }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceVaryingInterpolation { Reject, AllLinear }

/// Refines a sampled Catmull–Clark or bilinear mesh with supported rules and primvars.
/// Authored normals are removed; downstream normals approximate the finite mesh.
pub fn refine_mesh(mesh: &ReadMesh, rules: &ReadSubdivision, levels: u32) -> Result<ReadMesh> {
    rules.validate(mesh.points.len(), mesh.face_vertex_counts.len())?;
    let linear = mesh.subdivision_scheme == SubdivScheme::Bilinear && rules.scheme == "bilinear";
    ensure!(linear || (mesh.subdivision_scheme == SubdivScheme::CatmullClark && rules.scheme == "catmullClark"), "unsupported or mismatched subdivision scheme");
    ensure!(linear || rules.triangle_rule == "catmullClark", "unsupported triangle subdivision rule");
    let boundary = match rules.boundary.as_str() {
        "none" => BoundaryInterpolation::EdgeOnly,
        "edgeOnly" => BoundaryInterpolation::EdgeOnly,
        "edgeAndCorner" => BoundaryInterpolation::EdgeAndCorner,
        _ => anyhow::bail!("unsupported boundary interpolation"),
    };
    let face_varying = match rules.face_varying.as_str() {
        "all" => FaceVaryingInterpolation::AllLinear,
        "none" | "cornersOnly" | "cornersPlus1" | "cornersPlus2" | "boundaries" => {
            if linear { FaceVaryingInterpolation::AllLinear } else { FaceVaryingInterpolation::Reject }
        }
        _ => anyhow::bail!("unknown face-varying interpolation"),
    };
    let surface = refine_control_mesh(&mesh.points, &mesh.face_vertex_counts, &mesh.face_vertex_indices, levels, boundary, linear, Some(rules))?;
    let uvs = mesh.uvs.as_ref().map(|values| surface.interpolate_primvar(values, face_varying)).transpose()?;
    let display_color = mesh.display_color.as_ref().map(|values| surface.interpolate_primvar(values, face_varying)).transpose()?;
    let display_opacity = mesh.display_opacity.as_ref().map(|values| {
        let expanded = MeshPrimvar { values: values.values.iter().map(|&value| [value]).collect(),
            interpolation: values.interpolation, indices: values.indices.clone() };
        surface.interpolate_primvar(&expanded, face_varying).map(|result| MeshPrimvar {
            values: result.values.into_iter().map(|value| value[0]).collect(),
            interpolation: result.interpolation, indices: result.indices,
        })
    }).transpose()?;
    let subsets = surface.remap_subsets(&mesh.subsets)?;
    let triangulation_points = mesh.triangulation_points.as_ref().map(|points| surface.interpolate_vertex(points)).transpose()?;
    let mut holes: BTreeSet<_> = rules.holes.iter().map(|&face| face as usize).collect();
    if !linear && rules.boundary == "none" { holes.extend(boundary_faces(mesh)); }
    let keep: Vec<_> = surface.source_faces.iter().map(|face| !holes.contains(face)).collect();
    let hard_edges = surface.hard_edges;
    let hard_corners = surface.hard_corners;
    let output = ReadMesh {
        points: surface.points,
        hole_indices: Vec::new(),
        face_vertex_counts: vec![4; surface.faces.len()],
        face_vertex_indices: surface.faces.into_iter().flatten().map(|index| index as i32).collect(),
        triangulation_points, normals: None, uvs, display_color, display_opacity, subsets,
        orientation: mesh.orientation, double_sided: mesh.double_sided,
        extent: None, subdivision_scheme: mesh.subdivision_scheme,
    };
    let mut output = omit_faces(output, &keep);
    if !linear && (!hard_edges.is_empty() || !hard_corners.is_empty()) {
        output.normals = Some(crate::mesh::crease_corner_normals(&output, &hard_edges, &hard_corners));
    }
    Ok(output)
}

fn boundary_faces(mesh: &ReadMesh) -> BTreeSet<usize> {
    let mut edges = BTreeMap::<[usize; 2], usize>::new();
    let mut offset = 0;
    let mut faces = Vec::with_capacity(mesh.face_vertex_counts.len());
    for &count in &mesh.face_vertex_counts {
        let face = &mesh.face_vertex_indices[offset..offset + count as usize];
        for (corner, &a) in face.iter().enumerate() {
            *edges.entry(edge_key(a as usize, face[(corner + 1) % face.len()] as usize)).or_default() += 1;
        }
        faces.push(face);
        offset += count as usize;
    }
    let boundary: BTreeSet<_> = edges.into_iter().filter(|(_, count)| *count == 1)
        .flat_map(|(edge, _)| edge).collect();
    faces.into_iter().enumerate().filter_map(|(index, face)|
        face.iter().any(|&vertex| boundary.contains(&(vertex as usize))).then_some(index)).collect()
}

fn omit_faces(mut mesh: ReadMesh, keep: &[bool]) -> ReadMesh {
    if keep.iter().all(|&keep| keep) { return mesh; }
    fn primvar<T>(mut data: MeshPrimvar<T>, keep: &[bool]) -> MeshPrimvar<T> {
        match data.interpolation {
            Interpolation::Uniform => data.values = data.values.into_iter().zip(keep)
                .filter_map(|(value, &keep)| keep.then_some(value)).collect(),
            Interpolation::FaceVarying => data.values = data.values.into_iter().enumerate()
                .filter_map(|(corner, value)| keep[corner / 4].then_some(value)).collect(),
            _ => (),
        }
        data
    }
    let mut next = 0i32;
    let remap: Vec<_> = keep.iter().map(|&keep| if keep {
        let index = next;
        next += 1;
        Some(index)
    } else { None }).collect();
    mesh.face_vertex_indices = mesh.face_vertex_indices.chunks_exact(4).zip(keep)
        .filter(|(_, keep)| **keep).flat_map(|(face, _)| face.iter().copied()).collect();
    mesh.face_vertex_counts = vec![4; next as usize];
    mesh.uvs = mesh.uvs.map(|data| primvar(data, keep));
    mesh.display_color = mesh.display_color.map(|data| primvar(data, keep));
    mesh.display_opacity = mesh.display_opacity.map(|data| primvar(data, keep));
    for subset in &mut mesh.subsets {
        subset.indices = subset.indices.iter().filter_map(|&face| remap[face as usize]).collect();
    }
    mesh
}

/// Finite-level geometry with source-face identity, not limit-surface evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct RefinedSurface {
    pub points: Vec<[f32; 3]>,
    pub faces: Vec<Vec<usize>>,
    pub source_faces: Vec<usize>,
    control_points: usize,
    control_faces: usize,
    steps: Vec<RefinementStep>,
    hard_edges: BTreeSet<[usize; 2]>,
    hard_corners: BTreeSet<usize>,
}

impl RefinedSurface {
    /// Expands indexed primvars and refines values according to their interpolation.
    pub fn interpolate_primvar<const N: usize>(
        &self, primvar: &MeshPrimvar<[f32; N]>, face_varying: FaceVaryingInterpolation,
    ) -> Result<MeshPrimvar<[f32; N]>> {
        let expected = match primvar.interpolation {
            Interpolation::Constant => 1,
            Interpolation::Uniform => self.control_faces,
            Interpolation::Vertex | Interpolation::Varying => self.control_points,
            Interpolation::FaceVarying => self.steps[0].face_sizes.iter().sum(),
        };
        ensure!(N > 0 && primvar.values.iter().flatten().all(|value| value.is_finite()), "nonfinite or empty-lane primvar");
        let expanded = if primvar.indices.is_empty() {
            ensure!(primvar.values.len() == expected, "primvar value count does not match interpolation");
            primvar.values.clone()
        } else {
            ensure!(primvar.indices.len() == expected, "primvar index count does not match interpolation");
            primvar.indices.iter().map(|&index| {
                ensure!(index >= 0, "negative primvar index");
                primvar.values.get(index as usize).copied().ok_or_else(|| anyhow::anyhow!("primvar index out of range"))
            }).collect::<Result<Vec<_>>>()?
        };
        let values = match primvar.interpolation {
            Interpolation::Constant => expanded,
            Interpolation::Uniform => self.interpolate_uniform(&expanded)?,
            Interpolation::Vertex => self.interpolate_vertex(&expanded)?,
            Interpolation::Varying => self.interpolate_varying(&expanded)?,
            Interpolation::FaceVarying => {
                ensure!(face_varying == FaceVaryingInterpolation::AllLinear, "face-varying refinement requires explicit all-linear mode");
                self.interpolate_face_varying_linear(&expanded)?
            }
        };
        Ok(MeshPrimvar { values, interpolation: primvar.interpolation, indices: Vec::new() })
    }

    /// Maps material subset membership from control faces to refined faces.
    pub fn remap_subsets(&self, subsets: &[ReadSubset]) -> Result<Vec<ReadSubset>> {
        let mut descendants = vec![Vec::new(); self.control_faces];
        for (refined, &source) in self.source_faces.iter().enumerate() {
            ensure!(source < self.control_faces, "source face index out of range");
            descendants[source].push(i32::try_from(refined)?);
        }
        subsets.iter().map(|subset| {
            let mut selected = BTreeSet::new();
            for &face in &subset.indices {
                ensure!(face >= 0 && (face as usize) < self.control_faces, "subset face index out of range");
                selected.extend(descendants[face as usize].iter().copied());
            }
            Ok(ReadSubset { name: subset.name.clone(), indices: selected.into_iter().collect(),
                material_binding: subset.material_binding.clone() })
        }).collect()
    }

    /// Applies the recorded vertex interpolation rules to finite control-point values.
    /// This does not implement varying or face-varying interpolation.
    pub fn interpolate_vertex<const N: usize>(&self, values: &[[f32; N]]) -> Result<Vec<[f32; N]>> {
        ensure!(N > 0 && values.len() == self.control_points, "vertex primvar count must match control points");
        ensure!(values.iter().flatten().all(|value| value.is_finite()), "nonfinite vertex primvar");
        let mut values = values.to_vec();
        for step in &self.steps { values = evaluate(&step.vertex, &values)?; }
        Ok(values)
    }

    /// Linearly interpolates varying data while retaining values at existing vertices.
    pub fn interpolate_varying<const N: usize>(&self, values: &[[f32; N]]) -> Result<Vec<[f32; N]>> {
        ensure!(N > 0 && values.len() == self.control_points, "varying primvar count must match control points");
        ensure!(values.iter().flatten().all(|value| value.is_finite()), "nonfinite varying primvar");
        let mut values = values.to_vec();
        for step in &self.steps { values = evaluate(&step.varying, &values)?; }
        Ok(values)
    }

    /// Copies each source face's uniform value to all of its refined faces.
    pub fn interpolate_uniform<T: Clone>(&self, values: &[T]) -> Result<Vec<T>> {
        ensure!(values.len() == self.control_faces, "uniform primvar count must match control faces");
        self.source_faces.iter().map(|&face| values.get(face).cloned()
            .ok_or_else(|| anyhow::anyhow!("source face index out of range"))).collect()
    }

    /// Refines expanded face-corner values using the explicit all-linear rule.
    /// Shared geometric vertices do not merge independent corner values.
    pub fn interpolate_face_varying_linear<const N: usize>(&self, values: &[[f32; N]]) -> Result<Vec<[f32; N]>> {
        let corners: usize = self.steps[0].face_sizes.iter().sum();
        ensure!(N > 0 && values.len() == corners, "face-varying count must match control corners");
        ensure!(values.iter().flatten().all(|value| value.is_finite()), "nonfinite face-varying primvar");
        let mut values = values.to_vec();
        for step in &self.steps {
            let mut next = Vec::with_capacity(values.len() * 4);
            let mut offset = 0;
            for &size in &step.face_sizes {
                let face = &values[offset..offset + size];
                let center = std::array::from_fn(|lane| face.iter().map(|value| f64::from(value[lane])).sum::<f64>() / size as f64);
                let midpoint = |a: usize, b: usize| std::array::from_fn(|lane| ((f64::from(face[a][lane]) + f64::from(face[b][lane])) * 0.5) as f32);
                for (i, value) in face.iter().enumerate() {
                    next.extend([*value, midpoint(i, (i + 1) % size), center.map(|value| value as f32), midpoint((i + size - 1) % size, i)]);
                }
                offset += size;
            }
            ensure!(next.iter().flatten().all(|value| value.is_finite()), "nonfinite refined face-varying value");
            values = next;
        }
        Ok(values)
    }
}

fn evaluate<const N: usize>(stencils: &[Stencil], values: &[[f32; N]]) -> Result<Vec<[f32; N]>> {
    let output: Vec<_> = stencils.iter().map(|stencil| std::array::from_fn(|lane|
        stencil.iter().map(|&(index, weight)| f64::from(values[index][lane]) * weight).sum::<f64>() as f32)).collect();
    ensure!(output.iter().flatten().all(|value| value.is_finite()), "nonfinite refined value");
    Ok(output)
}

fn combine(terms: impl IntoIterator<Item = (usize, f64)>) -> Stencil {
    let mut weights = BTreeMap::new();
    for (index, weight) in terms { *weights.entry(index).or_insert(0.0) += weight; }
    weights.into_iter().filter(|(_, weight)| *weight != 0.0).collect()
}

fn push_stencil(output: &mut Vec<Stencil>, stencil: Stencil, remaining: &mut usize) -> Result<()> {
    ensure!(stencil.len() <= *remaining, "subdivision interpolation weights exceed budget");
    *remaining -= stencil.len();
    output.push(stencil);
    Ok(())
}

/// Refines positions and topology for 1–6 levels without primvars, sharpness or deformation.
/// Keeps disconnected-fan vertices fixed; rejects inconsistent winding and non-manifold edges.
pub fn catmull_clark(
    points: &[[f32; 3]], counts: &[i32], indices: &[i32], levels: u32,
    boundary: BoundaryInterpolation,
) -> Result<RefinedSurface> {
    refine_control_mesh(points, counts, indices, levels, boundary, false, None)
}

/// Linearly refines control faces to quads while retaining existing vertex values.
pub fn bilinear(points: &[[f32; 3]], counts: &[i32], indices: &[i32], levels: u32) -> Result<RefinedSurface> {
    refine_control_mesh(points, counts, indices, levels, BoundaryInterpolation::EdgeOnly, true, None)
}

fn refine_control_mesh(
    points: &[[f32; 3]], counts: &[i32], indices: &[i32], levels: u32,
    boundary: BoundaryInterpolation, linear: bool, rules: Option<&ReadSubdivision>,
) -> Result<RefinedSurface> {
    ensure!((1..=6).contains(&levels), "subdivision level must be 1..=6");
    ensure!(points.len() <= MAX_POINTS && indices.len() <= MAX_CORNERS, "subdivision input exceeds budget");
    ensure!(points.iter().flatten().all(|value| value.is_finite()), "nonfinite control point");
    let mut offset = 0usize;
    let mut faces = Vec::new();
    for &count in counts {
        ensure!(count >= 3, "subdivision faces require at least three corners");
        let end = offset.checked_add(count as usize).ok_or_else(|| anyhow::anyhow!("corner count overflow"))?;
        ensure!(end <= indices.len(), "face counts exceed corner indices");
        let face = indices[offset..end].iter().map(|&index| {
            ensure!(index >= 0 && (index as usize) < points.len(), "control point index out of range");
            Ok(index as usize)
        }).collect::<Result<Vec<_>>>()?;
        ensure!(face.iter().copied().collect::<BTreeSet<_>>().len() == face.len(), "repeated vertex in face");
        faces.push(face);
        offset = end;
    }
    ensure!(offset == indices.len(), "unused corner indices");
    let mut sharpness = Sharpness::read(rules, &faces)?;
    let mut surface = RefinedSurface { points: points.to_vec(), source_faces: (0..faces.len()).collect(), faces,
        control_points: points.len(), control_faces: counts.len(), steps: Vec::new(),
        hard_edges: BTreeSet::new(), hard_corners: BTreeSet::new() };
    for _ in 0..levels { surface = refine(surface, boundary, linear, &mut sharpness)?; }
    Ok(surface)
}

fn edge_key(a: usize, b: usize) -> [usize; 2] { [a.min(b), a.max(b)] }

fn refine(surface: RefinedSurface, boundary: BoundaryInterpolation, linear: bool, sharpness: &mut Sharpness) -> Result<RefinedSurface> {
    let points = &surface.points;
    let mut edges: BTreeMap<[usize; 2], Vec<(usize, bool)>> = BTreeMap::new();
    let mut vertex_faces = vec![Vec::new(); points.len()];
    let mut corners = 0usize;
    for (face_id, face) in surface.faces.iter().enumerate() {
        corners += face.len();
        ensure!(corners <= MAX_CORNERS / 4, "refined corner count exceeds budget");
        for (i, &a) in face.iter().enumerate() {
            vertex_faces[a].push(face_id);
            let b = face[(i + 1) % face.len()];
            let incident = edges.entry(edge_key(a, b)).or_default();
            ensure!(incident.len() < 2, "non-manifold edge");
            ensure!(incident.first().is_none_or(|(_, forward)| *forward != (a < b)), "inconsistent face winding");
            incident.push((face_id, a < b));
        }
    }
    let output_count = points.len() + surface.faces.len() + edges.len();
    ensure!(output_count <= MAX_POINTS, "refined point count exceeds budget");
    let face_stencils: Vec<Stencil> = surface.faces.iter().map(|face|
        face.iter().map(|&point| (point, 1.0 / face.len() as f64)).collect()).collect();
    let mut vertex_edges = vec![Vec::new(); points.len()];
    for &edge in edges.keys() { for vertex in edge { vertex_edges[vertex].push(edge); } }
    let mut output = Vec::with_capacity(output_count);
    let mut remaining = MAX_WEIGHTS - surface.steps.iter().map(|step|
        step.vertex.iter().chain(&step.varying).map(Vec::len).sum::<usize>()).sum::<usize>();
    for vertex in 0..points.len() {
        let faces = &vertex_faces[vertex];
        let incident = &vertex_edges[vertex];
        if linear || faces.is_empty() { push_stencil(&mut output, vec![(vertex, 1.0)], &mut remaining)?; continue; }
        let mut links: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for edge in incident {
            if let [(a, _), (b, _)] = edges[edge].as_slice() {
                links.entry(*a).or_default().push(*b);
                links.entry(*b).or_default().push(*a);
            }
        }
        let mut visited = BTreeSet::new();
        let mut pending = vec![faces[0]];
        while let Some(face) = pending.pop() {
            if !visited.insert(face) { continue; }
            pending.extend(links.get(&face).into_iter().flatten().copied());
        }
        if visited.len() != faces.len() {
            push_stencil(&mut output, vec![(vertex, 1.0)], &mut remaining)?;
            continue;
        }
        let boundary_neighbours: Vec<_> = incident.iter().filter(|edge| edges[*edge].len() == 1)
            .map(|edge| if edge[0] == vertex { edge[1] } else { edge[0] }).collect();
        let refined = if boundary_neighbours.is_empty() {
            ensure!(faces.len() == incident.len(), "invalid interior vertex fan");
            let n = faces.len() as f64;
            combine(faces.iter().flat_map(|&face| face_stencils[face].iter().map(move |&(index, weight)| (index, weight / (n * n))))
                .chain(incident.iter().flat_map(|edge| edge.iter().map(move |&index| (index, 1.0 / (n * n)))))
                .chain(std::iter::once((vertex, (n - 3.0) / n))))
        } else {
            ensure!(boundary_neighbours.len() == 2, "non-manifold boundary vertex");
            if boundary == BoundaryInterpolation::EdgeAndCorner && faces.len() == 1 { vec![(vertex, 1.0)] }
            else { vec![(vertex, 0.75), (boundary_neighbours[0], 0.125), (boundary_neighbours[1], 0.125)] }
        };
        if sharpness.edges.is_empty() && sharpness.corners.is_empty() {
            push_stencil(&mut output, refined, &mut remaining)?;
            continue;
        }
        let edge_sharpness: Vec<_> = incident.iter().map(|edge| {
            let value = if edges[edge].len() == 1 { 10.0 } else { sharpness.edges.get(edge).copied().unwrap_or(0.0) };
            (if edge[0] == vertex { edge[1] } else { edge[0] }, value)
        }).collect();
        let corner = if boundary == BoundaryInterpolation::EdgeAndCorner && faces.len() == 1 { 10.0 }
            else { sharpness.corners.get(&vertex).copied().unwrap_or(0.0) };
        push_stencil(&mut output, Sharpness::vertex(vertex, corner, &edge_sharpness, refined), &mut remaining)?;
    }
    for stencil in &face_stencils { push_stencil(&mut output, stencil.clone(), &mut remaining)?; }
    let mut edge_points = BTreeMap::new();
    for (&edge, incident) in &edges {
        edge_points.insert(edge, output.len());
        let point = if linear || incident.len() == 1 { vec![(edge[0], 0.5), (edge[1], 0.5)] }
            else { combine([(edge[0], 0.25), (edge[1], 0.25)].into_iter().chain(
                incident.iter().flat_map(|&(face, _)| face_stencils[face].iter().map(|&(index, weight)| (index, weight * 0.25))))) };
        let point = if linear || sharpness.edges.is_empty() { point } else {
            blend(&vec![(edge[0], 0.5), (edge[1], 0.5)], &point, sharpness.edges.get(&edge).copied().unwrap_or(0.0).min(1.0))
        };
        push_stencil(&mut output, point, &mut remaining)?;
    }
    let mut faces = Vec::with_capacity(corners);
    let mut source_faces = Vec::with_capacity(corners);
    for (face_id, face) in surface.faces.iter().enumerate() {
        for (i, &vertex) in face.iter().enumerate() {
            faces.push(vec![vertex, edge_points[&edge_key(vertex, face[(i + 1) % face.len()])],
                points.len() + face_id, edge_points[&edge_key(face[(i + face.len() - 1) % face.len()], vertex)]]);
            source_faces.push(surface.source_faces[face_id]);
        }
    }
    let mut child_sharpness = Sharpness::default();
    for (&edge, &value) in &sharpness.edges {
        let value = decay(value);
        if value > 0.0 {
            for endpoint in edge { child_sharpness.edges.insert(edge_key(endpoint, edge_points[&edge]), value); }
        }
    }
    child_sharpness.corners = sharpness.corners.iter().filter_map(|(&index, &value)| {
        let value = decay(value);
        (value > 0.0).then_some((index, value))
    }).collect();
    *sharpness = child_sharpness;
    let points = evaluate(&output, points)?;
    let mut varying = Vec::with_capacity(output_count);
    for vertex in 0..surface.points.len() { push_stencil(&mut varying, vec![(vertex, 1.0)], &mut remaining)?; }
    for stencil in face_stencils { push_stencil(&mut varying, stencil, &mut remaining)?; }
    for edge in edges.keys() { push_stencil(&mut varying, vec![(edge[0], 0.5), (edge[1], 0.5)], &mut remaining)?; }
    let mut steps = surface.steps;
    steps.push(RefinementStep { vertex: output, varying, face_sizes: surface.faces.iter().map(Vec::len).collect() });
    let hard_edges = sharpness.edges.iter().filter_map(|(&edge, &value)| (value >= 10.0).then_some(edge)).collect();
    let hard_corners = sharpness.corners.iter().filter_map(|(&corner, &value)| (value >= 10.0).then_some(corner)).collect();
    Ok(RefinedSurface { points, faces, source_faces, control_points: surface.control_points, control_faces: surface.control_faces, steps, hard_edges, hard_corners })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_sharpness_refines_cube_positions_and_decays() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/subdivision_cube.usda");
        let source = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::Path::new("/RoundedCube").unwrap();
        let mesh = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mut rules = crate::read::subdivision::read_subdivision_at(&stage, &path, None).unwrap();
        let smooth = refine_mesh(&mesh, &rules, 1).unwrap();
        let mut edges = BTreeSet::new();
        for face in mesh.face_vertex_indices.chunks_exact(4) {
            for i in 0..4 { edges.insert(edge_key(face[i] as usize, face[(i+1)%4] as usize)); }
        }
        rules.crease_indices = edges.iter().flatten().map(|&index| index as i32).collect();
        rules.crease_lengths = vec![2; edges.len()];
        rules.crease_sharpnesses = vec![10.0; edges.len()];
        let hard = refine_mesh(&mesh, &rules, 2).unwrap();
        assert_eq!(&hard.points[..8], mesh.points);
        let normals = hard.normals.as_ref().unwrap();
        assert_eq!(normals.interpolation, Interpolation::FaceVarying);
        assert_eq!(normals.values.len(), hard.face_vertex_indices.len());
        for (face, normals) in hard.face_vertex_indices.chunks_exact(4).zip(normals.values.chunks_exact(4)) {
            let p = |i: usize| DVec3::from_array(hard.points[face[i] as usize].map(f64::from));
            let expected = (p(1) - p(0)).cross(p(2) - p(0)).normalize().as_vec3().to_array();
            assert!(normals.iter().all(|normal| *normal == expected));
        }
        let mut left = mesh.clone();
        left.orientation = crate::read::geom::Orientation::LeftHanded;
        let left = refine_mesh(&left, &rules, 2).unwrap();
        for (a,b) in hard.normals.as_ref().unwrap().values.iter().zip(left.normals.unwrap().values) {
            assert_eq!(a.map(|value| -value), b);
        }
        rules.crease_sharpnesses.fill(0.5);
        let fractional = refine_mesh(&mesh, &rules, 1).unwrap();
        assert!(fractional.normals.is_none());
        for i in 0..8 { for lane in 0..3 {
            assert!((fractional.points[i][lane] - (mesh.points[i][lane] + smooth.points[i][lane]) * 0.5).abs() < 1e-6);
        } }
        rules.crease_sharpnesses.fill(1.0);
        assert_eq!(&refine_mesh(&mesh, &rules, 1).unwrap().points[..8], mesh.points);
        assert_ne!(&refine_mesh(&mesh, &rules, 2).unwrap().points[..8], mesh.points);
        rules.crease_sharpnesses.fill(0.0);
        assert_eq!(refine_mesh(&mesh, &rules, 1).unwrap().points, smooth.points);
        rules.corner_indices = vec![0];
        rules.corner_sharpnesses = vec![0.5];
        let corner = refine_mesh(&mesh, &rules, 1).unwrap();
        for lane in 0..3 { assert!((corner.points[0][lane] - (mesh.points[0][lane] + smooth.points[0][lane]) * 0.5).abs() < 1e-6); }
        rules.crease_indices[1] = 6;
        assert!(refine_mesh(&mesh, &rules, 1).unwrap_err().to_string().contains("mesh edge"));
        rules.crease_indices = vec![0,1,2];
        rules.crease_lengths = vec![3];
        rules.crease_sharpnesses = vec![0.25,0.75];
        let faces = mesh.face_vertex_indices.chunks_exact(4).map(|face| face.iter().map(|&i| i as usize).collect()).collect::<Vec<_>>();
        let edges = Sharpness::read(Some(&rules), &faces).unwrap().edges;
        assert_eq!(edges[&[0,1]], 0.25);
        assert_eq!(edges[&[1,2]], 0.75);
        rules.crease_sharpnesses = vec![3.0];
        assert!(Sharpness::read(Some(&rules), &faces).unwrap().edges.values().all(|value| *value == 3.0));
    }

    #[test]
    fn transitional_vertex_masks_average_decaying_sharpness() {
        let smooth = vec![(0,0.5),(1,0.25),(2,0.25)];
        let result = Sharpness::vertex(0, 0.0, &[(1,0.25),(2,0.75)], smooth.clone());
        let expected = blend(&vec![(0,0.75),(1,0.125),(2,0.125)], &smooth, 0.5);
        assert_eq!(result, expected);
        assert_eq!(Sharpness::vertex(0, 0.0, &[(1,10.0)], smooth.clone()), smooth);
        assert_eq!(Sharpness::vertex(0, 10.0, &[], smooth), vec![(0,1.0)]);
        assert_eq!(decay(10.0), 10.0);
        assert_eq!(decay(2.5), 1.5);
    }

    #[test]
    fn boundary_none_omits_boundary_faces_but_retains_interior_support() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/subdivision_holes.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::Path::new("/Surface").unwrap();
        let mesh = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mut rules = crate::read::subdivision::read_subdivision_at(&stage, &path, None).unwrap();
        rules.holes.clear();
        rules.boundary = "edgeOnly".into();
        let full = refine_mesh(&mesh, &rules, 2).unwrap();
        rules.boundary = "none".into();
        let interior = refine_mesh(&mesh, &rules, 2).unwrap();
        assert_eq!(interior.points, full.points);
        assert_eq!(interior.face_vertex_counts, [4;16]);
        assert!(interior.face_vertex_indices.iter().all(|&index| {
            let point = interior.points[index as usize];
            point[0].abs() <= 1.0 && point[2].abs() <= 1.0
        }));
        assert_eq!(boundary_faces(&mesh), [0,1,2,3,5,6,7,8].into_iter().collect());
        rules.holes = vec![4];
        assert!(refine_mesh(&mesh, &rules, 2).unwrap().face_vertex_indices.is_empty());
        let mut linear = mesh;
        linear.subdivision_scheme = SubdivScheme::Bilinear;
        rules.scheme = "bilinear".into();
        rules.holes.clear();
        assert_eq!(refine_mesh(&linear, &rules, 2).unwrap().face_vertex_counts.len(), 144);
    }

    #[test]
    fn holes_preserve_refinement_support_and_filter_primvars_and_subsets() {
        let source = crate::UsdSource::new("holes.usda", br#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0),(1,0,0),(3,0,0),(0,1,0),(1,1,0),(3,2,1)]
    int[] faceVertexCounts = [4,4]
    int[] faceVertexIndices = [0,1,4,3,1,2,5,4]
    token interpolateBoundary = "edgeOnly"
    token faceVaryingLinearInterpolation = "all"
    int[] holeIndices.timeSamples = { 0: [1], 10: [0] }
    texCoord2f[] primvars:st = [(0,0),(1,0),(1,1),(0,1)] ( interpolation = "faceVarying" )
    int[] primvars:st:indices = [0,1,2,3,0,1,2,3]
    color3f[] primvars:displayColor = [(1,0,0),(0,0,1)] ( interpolation = "uniform" )
    int[] primvars:displayColor:indices = [0,1]
    float[] primvars:displayOpacity = [0.5] ( interpolation = "constant" )
    def GeomSubset "Both" {
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices = [0,1]
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::Path::new("/M").unwrap();
        let mesh = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mut rules = crate::read::subdivision::read_subdivision_at(&stage, &path, Some(0.0)).unwrap();
        rules.holes.clear();
        let reference = refine_mesh(&mesh, &rules, 2).unwrap();
        for (time, color) in [(0.0,[1.0,0.0,0.0]), (10.0,[0.0,0.0,1.0])] {
            let rules = crate::read::subdivision::read_subdivision_at(&stage, &path, Some(time)).unwrap();
            let result = refine_mesh(&mesh, &rules, 2).unwrap();
            assert_eq!(result.points, reference.points);
            assert_eq!(result.face_vertex_counts, [4;16]);
            assert_eq!(result.uvs.as_ref().unwrap().values.len(), 64);
            assert_eq!(result.display_color.as_ref().unwrap().values, [color;16]);
            assert_eq!(result.display_opacity.as_ref().unwrap().values, [0.5]);
            assert_eq!(result.subsets[0].indices, (0..16).collect::<Vec<_>>());
            assert_eq!(crate::mesh::mesh_from_usd(&result).indices().unwrap().len(), 96);
        }
        rules.holes = vec![0,1,1];
        let empty = refine_mesh(&mesh, &rules, 2).unwrap();
        assert_eq!(empty.points, reference.points);
        assert!(empty.face_vertex_indices.is_empty());
        assert!(empty.uvs.unwrap().values.is_empty());
        assert!(empty.display_color.unwrap().values.is_empty());
        assert!(empty.subsets[0].indices.is_empty());
    }

    #[test]
    fn bilinear_geometry_and_vertex_values_follow_a_warped_patch() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [1.0,1.0,1.0], [0.0,1.0,0.0]];
        for levels in [1,2,3] {
            let surface = bilinear(&points, &[4], &[0,1,2,3], levels).unwrap();
            assert_eq!(&surface.points[..4], &points);
            assert_eq!(surface.interpolate_vertex(&points).unwrap(), surface.points);
            assert_eq!(surface.interpolate_varying(&points).unwrap(), surface.points);
            assert!(surface.points.iter().all(|point| (point[2] - point[0] * point[1]).abs() < 1e-6));
            assert!(surface.source_faces.iter().all(|&face| face == 0));
        }
        assert!(bilinear(&points, &[4], &[0,1,2,3], 0).is_err());
        assert!(bilinear(&points, &[3], &[0,1,4], 1).is_err());
    }

    #[test]
    fn bilinear_mesh_refines_default_face_varying_data_linearly() {
        let source = crate::UsdSource::new("bilinear.usda", br#"#usda 1.0
def Mesh "M" {
    uniform token subdivisionScheme = "bilinear"
    point3f[] points = [(0,0,0),(1,0,0),(1,1,1),(0,1,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    texCoord2f[] primvars:st = [(0,0),(1,0),(1,1),(0,1)] ( interpolation = "faceVarying" )
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::Path::new("/M").unwrap();
        let mesh = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mut rules = crate::read::subdivision::read_subdivision_at(&stage, &path, None).unwrap();
        assert_eq!(rules.face_varying, "cornersPlus1");
        for boundary in ["none", "edgeOnly", "edgeAndCorner"] {
            rules.boundary = boundary.into();
            let output = refine_mesh(&mesh, &rules, 1).unwrap();
            assert_eq!(output.subdivision_scheme, SubdivScheme::Bilinear);
            assert_eq!(&output.points[..4], &mesh.points);
            assert_eq!(output.points[4], [0.5,0.5,0.25]);
            let uv = &output.uvs.as_ref().unwrap().values;
            for (corner, &index) in output.face_vertex_indices.iter().enumerate() {
                assert_eq!(uv[corner], [output.points[index as usize][0], output.points[index as usize][1]]);
            }
            assert_eq!(crate::mesh::mesh_from_usd(&output).count_vertices(), 24);
        }
    }

    #[test]
    fn quad_boundary_modes_and_face_lineage() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [1.0,1.0,0.0], [0.0,1.0,0.0]];
        let fixed = catmull_clark(&points, &[4], &[0,1,2,3], 1, BoundaryInterpolation::EdgeAndCorner).unwrap();
        assert_eq!(&fixed.points[..4], &points);
        assert_eq!(fixed.points[4], [0.5,0.5,0.0]);
        assert_eq!(fixed.points.len(), 9);
        assert_eq!(fixed.faces.len(), 4);
        assert_eq!(fixed.source_faces, vec![0; 4]);
        let smooth = catmull_clark(&points, &[4], &[0,1,2,3], 1, BoundaryInterpolation::EdgeOnly).unwrap();
        assert_eq!(smooth.points[0], [0.125,0.125,0.0]);
        let twice = catmull_clark(&points, &[4], &[0,1,2,3], 2, BoundaryInterpolation::EdgeAndCorner).unwrap();
        assert_eq!(twice.points.len(), 25);
        assert_eq!(twice.faces.len(), 16);
        assert_eq!(twice.source_faces, vec![0; 16]);
    }

    #[test]
    fn closed_cube_refines_interior_vertices_and_preserves_winding() {
        let points = [[-1.0,-1.0,-1.0], [1.0,-1.0,-1.0], [1.0,1.0,-1.0], [-1.0,1.0,-1.0],
            [-1.0,-1.0,1.0], [1.0,-1.0,1.0], [1.0,1.0,1.0], [-1.0,1.0,1.0]];
        let indices = [0,3,2,1, 4,5,6,7, 0,1,5,4, 3,7,6,2, 0,4,7,3, 1,2,6,5];
        let mesh = catmull_clark(&points, &[4; 6], &indices, 1, BoundaryInterpolation::EdgeAndCorner).unwrap();
        assert_eq!(mesh.points.len(), 26);
        assert_eq!(mesh.faces.len(), 24);
        for axis in mesh.points[0] { assert!((axis + 5.0/9.0).abs() < 1e-6); }
        for face in &mesh.faces {
            let p = |i: usize| DVec3::from_array(mesh.points[face[i]].map(f64::from));
            assert!((p(1) - p(0)).cross(p(2) - p(0)).dot(p(0)) > 0.0);
        }
        for source in 0..6 { assert_eq!(mesh.source_faces.iter().filter(|&&face| face == source).count(), 4); }
        let transform = |point: [f32; 3]| [point[0] * 2.0 + 7.0, point[1] * 3.0 - 4.0, point[2] * 0.5 + 1.0];
        let transformed = catmull_clark(&points.map(transform), &[4; 6], &indices, 1, BoundaryInterpolation::EdgeAndCorner).unwrap();
        assert_eq!(transformed.faces, mesh.faces);
        for (actual, expected) in transformed.points.iter().zip(mesh.points.iter().copied().map(transform)) {
            assert!(actual.iter().zip(expected).all(|(actual, expected)| (*actual - expected).abs() < 1e-5));
        }
    }

    #[test]
    fn malformed_topology_is_rejected() {
        let points = [[0.0; 3]; 5];
        for (counts, indices) in [(vec![3], vec![0,1,-1]), (vec![4], vec![0,1,2]),
            (vec![3], vec![0,1,1]),
            (vec![3,3], vec![0,1,2,0,1,3]), (vec![3], vec![0,1,2,3])] {
            assert!(catmull_clark(&points, &counts, &indices, 1, BoundaryInterpolation::EdgeOnly).is_err());
        }
        assert!(catmull_clark(&[[f32::NAN; 3]], &[], &[], 1, BoundaryInterpolation::EdgeOnly).is_err());
        assert!(catmull_clark(&[], &[], &[], 7, BoundaryInterpolation::EdgeOnly).is_err());
        assert!(catmull_clark(&[], &[], &[], 0, BoundaryInterpolation::EdgeOnly).is_err());
    }

    #[test]
    fn disconnected_fans_keep_the_shared_vertex_fixed_without_splitting() {
        let points = [[0.0,0.0,0.0], [2.0,0.0,0.0], [0.0,2.0,0.0],
            [-3.0,0.0,0.0], [0.0,-3.0,0.0]];
        for boundary in [BoundaryInterpolation::EdgeOnly, BoundaryInterpolation::EdgeAndCorner] {
            for levels in [1,2,3] {
                let mesh = catmull_clark(&points, &[3,3], &[0,1,2,0,3,4], levels, boundary).unwrap();
                assert_eq!(mesh.points[0], points[0]);
                assert_eq!(mesh.interpolate_vertex(&points).unwrap(), mesh.points);
                let values = mesh.interpolate_vertex(&[[7.0],[1.0],[2.0],[3.0],[4.0]]).unwrap();
                assert_eq!(values[0], [7.0]);
                let incident: Vec<_> = mesh.faces.iter().enumerate().filter(|(_, face)| face.contains(&0)).collect();
                assert_eq!(incident.len(), 2);
                assert_eq!(incident.iter().map(|(face, _)| mesh.source_faces[*face]).collect::<Vec<_>>(), [0,1]);
                assert_eq!(mesh.points.iter().filter(|point| **point == points[0]).count(), 1);
            }
        }
        assert!(catmull_clark(&points, &[3,3,3], &[0,1,2,1,0,3,0,1,4], 1, BoundaryInterpolation::EdgeOnly).is_err());
        let closed_points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0], [0.0,0.0,1.0],
            [-2.0,0.0,0.0], [0.0,-2.0,0.0], [0.0,0.0,-2.0]];
        let closed = catmull_clark(&closed_points, &[3;8],
            &[0,2,1,0,1,3,1,2,3,2,0,3,0,5,4,0,4,6,4,5,6,5,0,6], 2, BoundaryInterpolation::EdgeOnly).unwrap();
        assert_eq!(closed.points[0], closed_points[0]);
        assert_eq!(closed.faces.iter().filter(|face| face.contains(&0)).count(), 6);
    }

    #[test]
    fn recorded_stencils_reproduce_geometry_and_vertex_primvars() {
        let points = [[0.0,0.0,0.0], [2.0,0.0,0.0], [2.0,2.0,0.0], [0.0,2.0,0.0]];
        for boundary in [BoundaryInterpolation::EdgeOnly, BoundaryInterpolation::EdgeAndCorner] {
            let refined = catmull_clark(&points, &[4], &[0,1,2,3], 3, boundary).unwrap();
            assert_eq!(refined.interpolate_vertex(&points).unwrap(), refined.points);
            let uvs = points.map(|point| [point[0] * 0.5, point[1] * 0.5]);
            let refined_uvs = refined.interpolate_vertex(&uvs).unwrap();
            for (uv, point) in refined_uvs.iter().zip(&refined.points) {
                assert!((uv[0] - point[0] * 0.5).abs() < 1e-6);
                assert!((uv[1] - point[1] * 0.5).abs() < 1e-6);
            }
            assert!(refined.interpolate_vertex(&[[7.0]; 4]).unwrap().iter().all(|value| (value[0] - 7.0).abs() < 1e-6));
            assert!(refined.interpolate_vertex(&[[0.0]; 3]).is_err());
            assert!(refined.interpolate_vertex(&[[f32::NAN]; 4]).is_err());
            assert!(refined.interpolate_vertex(&[[f32::INFINITY]; 4]).is_err());
            assert!(refined.interpolate_vertex(&[[0.0; 0]; 4]).is_err());
        }
    }

    #[test]
    fn stencil_budget_rejection_preserves_accumulated_weights() {
        let mut output = Vec::new();
        let mut remaining = 2;
        push_stencil(&mut output, vec![(0, 1.0)], &mut remaining).unwrap();
        assert!(push_stencil(&mut output, vec![(0, 0.5), (1, 0.5)], &mut remaining).is_err());
        assert_eq!(remaining, 1);
        assert_eq!(output, vec![vec![(0, 1.0)]]);
    }

    #[test]
    fn varying_values_are_linear_instead_of_vertex_smoothed() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [1.0,1.0,0.0], [0.0,1.0,0.0]];
        let mesh = catmull_clark(&points, &[4], &[0,1,2,3], 2, BoundaryInterpolation::EdgeOnly).unwrap();
        let varying = mesh.interpolate_varying(&points).unwrap();
        assert_eq!(&varying[..4], &points);
        assert_ne!(varying[0], mesh.points[0]);
        assert_eq!(varying.len(), mesh.points.len());
        assert!(mesh.interpolate_varying(&[[1.0]; 4]).unwrap().iter().all(|value| *value == [1.0]));
        assert!(mesh.interpolate_varying(&[[0.0]; 3]).is_err());
        assert!(mesh.interpolate_varying(&[[f32::NAN]; 4]).is_err());
    }

    #[test]
    fn indexed_primvars_expand_before_refinement() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [1.0,1.0,0.0], [0.0,1.0,0.0]];
        let surface = catmull_clark(&points, &[4], &[0,1,2,3], 2, BoundaryInterpolation::EdgeOnly).unwrap();
        for interpolation in [Interpolation::Constant, Interpolation::Uniform, Interpolation::Vertex,
            Interpolation::Varying, Interpolation::FaceVarying] {
            let count = if matches!(interpolation, Interpolation::Constant | Interpolation::Uniform) { 1 } else { 4 };
            let input = MeshPrimvar { values: vec![[2.0], [6.0]], interpolation,
                indices: (0..count).map(|i| i % 2).collect() };
            let expanded = MeshPrimvar { values: input.indices.iter().map(|&i| input.values[i as usize]).collect(),
                interpolation, indices: Vec::new() };
            let output = surface.interpolate_primvar(&input, FaceVaryingInterpolation::AllLinear).unwrap();
            let reference = surface.interpolate_primvar(&expanded, FaceVaryingInterpolation::AllLinear).unwrap();
            assert_eq!(output.values, reference.values);
            assert!(output.indices.is_empty());
            assert_eq!(output.interpolation, interpolation);
            if interpolation == Interpolation::FaceVarying {
                assert!(surface.interpolate_primvar(&input, FaceVaryingInterpolation::Reject).is_err());
            }
            for bad in [-1, 2] {
                let mut malformed = input.clone();
                malformed.indices[0] = bad;
                assert!(surface.interpolate_primvar(&malformed, FaceVaryingInterpolation::AllLinear).is_err());
            }
            let mut malformed = input.clone();
            malformed.indices.push(0);
            assert!(surface.interpolate_primvar(&malformed, FaceVaryingInterpolation::AllLinear).is_err());
        }
    }

    #[test]
    fn sampled_mesh_refinement_preserves_primvars_and_material_membership() {
        let source = crate::UsdSource::new("refine.usda", br#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0), (1,0,0), (1,1,0), (0,1,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    token faceVaryingLinearInterpolation = "all"
    uniform token orientation = "leftHanded"
    uniform bool doubleSided = true
    normal3f[] normals = [(0,0,1)] ( interpolation = "constant" )
    texCoord2f[] primvars:st = [(0,0), (1,0), (1,1), (0,1)] ( interpolation = "faceVarying" )
    int[] primvars:st:indices = [3,2,1,0]
    color3f[] primvars:displayColor = [(1,0,0)] ( interpolation = "uniform" )
    float[] primvars:displayOpacity = [0.25] ( interpolation = "constant" )
    def GeomSubset "red" {
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices = [0]
        rel material:binding = </Material>
    }
}
def Material "Material" {}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::Path::new("/M").unwrap();
        let mut input = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        input.triangulation_points = Some(input.points.clone());
        let rules = crate::read::subdivision::read_subdivision_at(&stage, &path, None).unwrap();
        let output = refine_mesh(&input, &rules, 2).unwrap();
        assert_eq!(output.points.len(), 25);
        assert_eq!(output.face_vertex_counts, [4;16]);
        assert_eq!(output.triangulation_points.as_ref().unwrap(), &output.points);
        assert!(output.normals.is_none());
        assert!(input.normals.is_some());
        assert_eq!(output.orientation, input.orientation);
        assert!(output.double_sided);
        assert_eq!(output.uvs.as_ref().unwrap().values.len(), 64);
        assert_eq!(output.uvs.as_ref().unwrap().values[0], [0.0,1.0]);
        assert_eq!(output.display_color.as_ref().unwrap().values, [[1.0,0.0,0.0];16]);
        assert_eq!(output.display_opacity.as_ref().unwrap().values, [0.25]);
        assert_eq!(output.subsets[0].indices, (0..16).collect::<Vec<_>>());
        assert_eq!(output.subsets[0].material_binding, input.subsets[0].material_binding);
        let rendered = crate::mesh::mesh_from_usd(&output);
        assert_eq!(rendered.indices().unwrap().len(), 96);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = rendered.attribute(bevy::mesh::Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
        assert!(normals.iter().all(|normal| normal.iter().all(|value| value.is_finite()) && normal[2] < -0.99));
        for mode in ["none", "cornersOnly", "cornersPlus1", "cornersPlus2", "boundaries"] {
            let mut unsupported = rules.clone();
            unsupported.face_varying = mode.into();
            assert!(refine_mesh(&input, &unsupported, 1).is_err());
        }
        let mut unsupported = rules.clone();
        unsupported.holes = vec![1];
        assert!(refine_mesh(&input, &unsupported, 1).is_err());
        unsupported = rules;
        unsupported.corner_indices = vec![0,0];
        unsupported.corner_sharpnesses = vec![1.0,1.0];
        assert!(refine_mesh(&input, &unsupported, 1).is_err());
        assert_eq!(input.points.len(), 4);
        assert_eq!(input.subsets[0].indices, [0]);
    }

    #[test]
    fn material_subsets_follow_source_faces_without_mutating_input() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [2.0,0.0,0.0],
            [0.0,1.0,0.0], [1.0,1.0,0.0], [2.0,1.0,0.0]];
        let mut surface = catmull_clark(&points, &[4,4], &[0,1,4,3,1,2,5,4], 2, BoundaryInterpolation::EdgeOnly).unwrap();
        let binding = openusd::sdf::Path::new("/Materials/Red").unwrap();
        let subsets = vec![ReadSubset { name: "red".into(), indices: vec![1,1], material_binding: Some(binding.clone()) },
            ReadSubset { name: "empty".into(), indices: vec![], material_binding: None }];
        let output = surface.remap_subsets(&subsets).unwrap();
        assert_eq!(output[0].name, "red");
        assert_eq!(output[0].material_binding, Some(binding));
        assert_eq!(output[0].indices.len(), 16);
        assert!(output[0].indices.iter().all(|&i| surface.source_faces[i as usize] == 1));
        assert!(output[1].indices.is_empty());
        assert_eq!(subsets[0].indices, [1,1]);
        for bad in [-1, 2] {
            let mut malformed = subsets.clone();
            malformed[0].indices.push(bad);
            assert!(surface.remap_subsets(&malformed).is_err());
        }
        surface.source_faces[0] = 2;
        assert!(surface.remap_subsets(&subsets).is_err());
        assert!(surface.interpolate_uniform(&[1,2]).is_err());
    }

    #[test]
    fn face_uniform_values_and_linear_uv_seams_survive_refinement() {
        let points = [[0.0,0.0,0.0], [1.0,0.0,0.0], [2.0,0.0,0.0],
            [0.0,1.0,0.0], [1.0,1.0,0.0], [2.0,1.0,0.0]];
        let indices = [0,1,4,3, 1,2,5,4];
        let uv = [[0.0,0.0], [1.0,0.0], [1.0,1.0], [0.0,1.0],
            [10.0,0.0], [11.0,0.0], [11.0,1.0], [10.0,1.0]];
        for levels in [1, 2, 3] {
            let mesh = catmull_clark(&points, &[4,4], &indices, levels, BoundaryInterpolation::EdgeAndCorner).unwrap();
            let values = mesh.interpolate_face_varying_linear(&uv).unwrap();
            assert_eq!(values.len(), mesh.faces.len() * 4);
            for (face, corners) in mesh.source_faces.iter().zip(values.chunks_exact(4)) {
                let start = if *face == 0 { 0.0 } else { 10.0 };
                assert!(corners.iter().all(|value| (start..=start + 1.0).contains(&value[0]) && (0.0..=1.0).contains(&value[1])));
            }
            let uniform = mesh.interpolate_uniform(&["red", "blue"]).unwrap();
            assert_eq!(uniform.len(), mesh.faces.len());
            for (&face, value) in mesh.source_faces.iter().zip(uniform) { assert_eq!(value, ["red", "blue"][face]); }
            assert!(mesh.interpolate_uniform(&[7]).is_err());
            assert!(mesh.interpolate_face_varying_linear(&uv[..7]).is_err());
            assert!(mesh.interpolate_face_varying_linear(&[[f32::INFINITY; 2]; 8]).is_err());
            if levels == 1 { assert_eq!(&values[..4], &[[0.0,0.0], [0.5,0.0], [0.5,0.5], [0.0,0.5]]); }
        }
    }
}
