//! Catmull-Clark limit positions for the control points of a cage.
//!
//! A subdivision surface lies inside its control cage wherever the cage
//! bulges, so drawing the cage itself makes rounded shapes puffier than
//! authored. Moving each control point onto the limit surface keeps the
//! cage's topology and memory while putting its vertices where the smooth
//! surface passes.

use crate::read::geom::ReadMesh;
use crate::read::subdivision::ReadSubdivision;
use glam::DVec3;

/// Each control point of `mesh` moved onto the Catmull-Clark limit surface.
/// Points on creases, at sharp or boundary corners, and on non-manifold edges
/// stay where they are.
pub fn limit_points(mesh: &ReadMesh, rules: &ReadSubdivision) -> Vec<[f32; 3]> {
    let points: Vec<DVec3> = mesh
        .points
        .iter()
        .map(|point| DVec3::from_array(point.map(f64::from)))
        .collect();
    let count = points.len();
    let mut faces = Vec::with_capacity(mesh.face_vertex_counts.len());
    let mut start = 0usize;
    for &corners in &mesh.face_vertex_counts {
        let corners = corners.max(0) as usize;
        let Some(face) = mesh.face_vertex_indices.get(start..start + corners) else {
            break;
        };
        if corners >= 3
            && face
                .iter()
                .all(|&index| (index as usize) < count && index >= 0)
        {
            faces.push(face);
        }
        start += corners;
    }
    let centroids: Vec<DVec3> = faces
        .iter()
        .map(|face| {
            face.iter()
                .map(|&index| points[index as usize])
                .sum::<DVec3>()
                / face.len() as f64
        })
        .collect();

    // Faces around each vertex, and every edge with the faces beside it.
    let mut face_sum = vec![DVec3::ZERO; count];
    let mut face_count = vec![0u32; count];
    let mut edges = Vec::with_capacity(mesh.face_vertex_indices.len());
    for (face, corners) in faces.iter().enumerate() {
        for (corner, &index) in corners.iter().enumerate() {
            face_sum[index as usize] += centroids[face];
            face_count[index as usize] += 1;
            let next = corners[(corner + 1) % corners.len()];
            edges.push((index.min(next) as u32, index.max(next) as u32, face as u32));
        }
    }
    edges.sort_unstable();

    // The once-subdivided neighborhood: edge midpoints, edge points, and the
    // boundary neighbors of each vertex.
    let mut valence = vec![0u32; count];
    let mut midpoint_sum = vec![DVec3::ZERO; count];
    let mut edge_point_sum = vec![DVec3::ZERO; count];
    let mut boundary: Vec<Vec<u32>> = vec![Vec::new(); count];
    let mut fixed = vec![false; count];
    for group in edges.chunk_by(|a, b| (a.0, a.1) == (b.0, b.1)) {
        let (a, b) = (group[0].0 as usize, group[0].1 as usize);
        let midpoint = (points[a] + points[b]) * 0.5;
        let edge_point = match group {
            [(_, _, left), (_, _, right)] => {
                (points[a] + points[b] + centroids[*left as usize] + centroids[*right as usize])
                    * 0.25
            }
            [_] => {
                boundary[a].push(b as u32);
                boundary[b].push(a as u32);
                midpoint
            }
            _ => {
                fixed[a] = true;
                fixed[b] = true;
                midpoint
            }
        };
        for vertex in [a, b] {
            valence[vertex] += 1;
            midpoint_sum[vertex] += midpoint;
            edge_point_sum[vertex] += edge_point;
        }
    }
    for (&index, &sharpness) in rules.corner_indices.iter().zip(&rules.corner_sharpnesses) {
        if let Some(fixed) = fixed.get_mut(index as usize) {
            *fixed |= sharpness > 0.0;
        }
    }
    let mut crease = 0usize;
    for (&length, &sharpness) in rules.crease_lengths.iter().zip(
        rules
            .crease_sharpnesses
            .iter()
            .chain(std::iter::repeat(&0.0)),
    ) {
        let end = crease + length.max(0) as usize;
        for &index in rules.crease_indices.get(crease..end).unwrap_or_default() {
            if let Some(fixed) = fixed.get_mut(index as usize) {
                *fixed |= sharpness > 0.0;
            }
        }
        crease = end;
    }
    let smooth_corners = rules.boundary == "edgeOnly";

    (0..count)
        .map(|vertex| {
            let point = points[vertex];
            let valence = valence[vertex];
            let limit = if fixed[vertex] || valence == 0 {
                point
            } else if boundary[vertex].is_empty() && face_count[vertex] == valence {
                let n = f64::from(valence);
                let faces = face_sum[vertex] / n;
                let midpoints = midpoint_sum[vertex] / n;
                let moved = (faces + midpoints * 2.0 + point * (n - 3.0)) / n;
                (moved * (n * n) + edge_point_sum[vertex] * 4.0 + face_sum[vertex])
                    / (n * (n + 5.0))
            } else if let [previous, next] = boundary[vertex][..]
                && (smooth_corners || face_count[vertex] > 1)
            {
                let (previous, next) = (points[previous as usize], points[next as usize]);
                let moved = (previous + point * 6.0 + next) / 8.0;
                ((previous + point) * 0.5 + moved * 4.0 + (next + point) * 0.5) / 6.0
            } else {
                point
            };
            limit.as_vec3().to_array()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> ReadMesh {
        let snippet = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0
def Mesh "Cube" {
    point3f[] points = [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]
    int[] faceVertexCounts = [4,4,4,4,4,4]
    int[] faceVertexIndices = [0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7]
}
"#,
        );
        let stage = snippet.open_stage().unwrap();
        crate::read::geom::read_mesh_at(&stage, &openusd::sdf::path("/Cube").unwrap(), None)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn closed_cage_corners_move_onto_the_smaller_limit_surface() {
        let limit = limit_points(&cube(), &ReadSubdivision::default());
        // The limit surface of a cube cage is a rounded box inside it: every
        // corner is pulled in along its diagonal, by the same amount.
        for point in &limit {
            let point = glam::Vec3::from_array(*point);
            assert!((point.x.abs() - point.y.abs()).abs() < 1e-5);
            assert!((point.x.abs() - 0.5).abs() < 1e-5, "{point}");
        }
    }

    #[test]
    fn open_cage_boundaries_follow_the_spline_and_corners_stay() {
        let snippet = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0
def Mesh "Grid" {
    point3f[] points = [(0,0,0),(1,0,0),(2,0,0),(0,1,0),(1,1,1),(2,1,0),(0,2,0),(1,2,0),(2,2,0)]
    int[] faceVertexCounts = [4,4,4,4]
    int[] faceVertexIndices = [0,1,4,3, 1,2,5,4, 3,4,7,6, 4,5,8,7]
}
"#,
        );
        let stage = snippet.open_stage().unwrap();
        let grid =
            crate::read::geom::read_mesh_at(&stage, &openusd::sdf::path("/Grid").unwrap(), None)
                .unwrap()
                .unwrap();
        let limit = limit_points(&grid, &ReadSubdivision::default());
        assert_eq!(limit[0], [0.0, 0.0, 0.0], "corners stay");
        assert_eq!(
            limit[1],
            [1.0, 0.0, 0.0],
            "a straight boundary stays straight"
        );
        // The raised center sinks toward its flat neighbors.
        assert!(limit[4][2] > 0.0 && limit[4][2] < 1.0);
    }
}
