use std::path::Path;
use bevy::{mesh::{Mesh, VertexAttributeValues}, prelude::*};

fn fan_report(normals: &[[f32;3]], indices: &[usize]) -> String {
    fn root(parents: &mut [usize], mut i: usize) -> usize {
        while parents[i] != i { parents[i] = parents[parents[i]]; i = parents[i]; }
        i
    }
    let mut parents = (0..indices.len()).collect::<Vec<_>>();
    let mut edges = std::collections::BTreeMap::<[usize;2], Vec<[usize;2]>>::new();
    for (face, triangle) in indices.chunks_exact(3).enumerate() {
        for i in 0..3 {
            let j = (i+1)%3;
            let a = triangle[i]; let b = triangle[j];
            if a != b { edges.entry([a.min(b), a.max(b)]).or_default().push([face*3+i, face*3+j]); }
        }
    }
    let nonmanifold = edges.values().filter(|edges| edges.len()>2).count();
    let mut inconsistent = 0;
    for edge in edges.values().filter(|edge| edge.len()==2) {
        inconsistent += usize::from(indices[edge[0][0]] == indices[edge[1][0]]);
        for corner in edge[0] {
            let other = if indices[edge[1][0]] == indices[corner] { edge[1][0] } else { edge[1][1] };
            let a = root(&mut parents, corner); let b = root(&mut parents, other);
            parents[a] = b;
        }
    }
    let mut fans = std::collections::BTreeMap::<usize, std::collections::BTreeSet<usize>>::new();
    for (corner, point) in indices.iter().enumerate() { fans.entry(*point).or_default().insert(root(&mut parents, corner)); }
    let disconnected = fans.values().filter(|fans| fans.len()>1).count();
    let zero = fans.keys().filter(|i| normals[**i] == [0.0;3]).count();
    let zero_disconnected = fans.iter().filter(|(i,fans)| normals[**i] == [0.0;3] && fans.len()>1).count();
    format!("indexed_triangle_topology nonmanifold_edges={nonmanifold} inconsistent_winding_edges={inconsistent} disconnected_vertices={disconnected} zero_normal_vertices={zero} zero_normal_disconnected_vertices={zero_disconnected}")
}

fn zero_normal_witnesses(points: &[[f32;3]], normals: &[[f32;3]], indices: &[usize]) -> String {
    let referenced = indices.iter().copied().collect::<std::collections::BTreeSet<_>>();
    let mut output = String::new();
    for vertex in referenced.into_iter().filter(|i| normals[*i] == [0.0;3]).take(8) {
        let incident = indices.chunks_exact(3).filter(|t| t.contains(&vertex)).collect::<Vec<_>>();
        output.push_str(&format!("zero_normal_vertex={vertex} position={:?} incident_triangles={}\n", points[vertex], incident.len()));
        for t in incident.into_iter().take(12) {
            let p = [t[0],t[1],t[2]].map(|i| Vec3::from(points[i]).as_dvec3());
            let normal = (p[1]-p[0]).cross(p[2]-p[0]).normalize_or_zero();
            let corner = t.iter().position(|i| *i==vertex).unwrap();
            let a = p[(corner+1)%3]-p[corner];
            let b = p[(corner+2)%3]-p[corner];
            output.push_str(&format!("  triangle={t:?} positions={p:?} face_normal={normal:?} corner_angle={}\n", a.cross(b).length().atan2(a.dot(b))));
        }
    }
    output
}

fn triangle_quality(points: &[[f32;3]], normals: &[[f32;3]], indices: &[usize]) -> String {
    let thresholds = [0.0, 1e-8, 1e-6, 1e-4, 1e-2];
    let mut counts = [0usize;5];
    let mut reversed = [0usize;5];
    let mut minimum = f64::INFINITY;
    for t in indices.chunks_exact(3) {
        let p = [t[0], t[1], t[2]].map(|i| Vec3::from(points[i]).as_dvec3());
        let edges = [p[1]-p[0], p[2]-p[1], p[0]-p[2]];
        let cross = edges[0].cross(-edges[2]);
        let longest = edges.iter().map(|e| e.length_squared()).fold(0.0, f64::max);
        let quality = if longest == 0.0 { 0.0 } else { cross.length() / longest };
        minimum = minimum.min(quality);
        let flipped = cross.try_normalize().map_or(0, |face| t.iter().filter(|i|
            face.dot(Vec3::from(normals[**i]).as_dvec3()) < -0.001).count());
        for (i, threshold) in thresholds.iter().enumerate() {
            if quality <= *threshold { counts[i] += 1; reversed[i] += flipped; }
        }
    }
    format!("triangle_quality=double_area_over_longest_edge_squared minimum={minimum:e} thresholds={thresholds:?} cumulative_triangles={counts:?} cumulative_reversed_normal_corners={reversed:?}")
}

fn filtered_normal_report(points: &[[f32;3]], normals: &[[f32;3]], indices: &[usize]) -> String {
    let faces = indices.chunks_exact(3).filter_map(|t| {
        let vertices = [t[0],t[1],t[2]];
        let p = vertices.map(|i| Vec3::from(points[i]).as_dvec3());
        let cross = (p[1]-p[0]).cross(p[2]-p[0]);
        let normal = cross.try_normalize()?;
        let angles = [0,1,2].map(|i| {
            let a = p[(i+1)%3]-p[i];
            let b = p[(i+2)%3]-p[i];
            a.cross(b).length().atan2(a.dot(b))
        });
        let longest = [0,1,2].map(|i| (p[(i+1)%3]-p[i]).length_squared()).into_iter().fold(0.0, f64::max);
        Some((vertices, normal, angles, cross.length()/longest))
    }).collect::<Vec<_>>();
    let mut output = String::new();
    for threshold in [0.0, 1e-6, 1e-4] {
        let mut sums = vec![bevy::math::DVec3::ZERO; points.len()];
        for (vertices, normal, angles, quality) in &faces {
            if *quality <= threshold { continue; }
            for i in 0..3 { sums[vertices[i]] += *normal * angles[i]; }
        }
        let calculated = sums.into_iter().map(|sum| sum.normalize_or_zero()).collect::<Vec<_>>();
        let changed = calculated.iter().zip(normals).filter(|(a,b)|
            !a.abs_diff_eq(Vec3::from(**b).as_dvec3(), 1e-4)).count();
        let reversed = faces.iter().map(|(vertices, normal, _, _)| vertices.iter()
            .filter(|i| normal.dot(calculated[**i]) < -0.001).count()).sum::<usize>();
        let referenced = indices.iter().copied().collect::<std::collections::HashSet<_>>();
        let zero = referenced.iter().filter(|i| calculated[**i] == bevy::math::DVec3::ZERO).count();
        output.push_str(&format!("diagnostic_normal_filter={threshold:e} changed_vertices={changed} remaining_reversed_normal_corners={reversed} zero_referenced_normals={zero}\n"));
    }
    output
}

fn area_weighted_normals(points: &[[f32; 3]], indices: &[usize]) -> Vec<[f32; 3]> {
    let mut sums = vec![bevy::math::DVec3::ZERO; points.len()];
    for triangle in indices.chunks_exact(3) {
        let p = [triangle[0], triangle[1], triangle[2]].map(|i| Vec3::from(points[i]).as_dvec3());
        let normal = (p[1] - p[0]).cross(p[2] - p[0]);
        for index in triangle { sums[*index] += normal; }
    }
    sums.into_iter().map(|normal| normal.normalize_or_zero().as_vec3().to_array()).collect()
}

fn write_probe(mesh: &Mesh, directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return Err("missing positions".into()); };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { return Err("missing normals".into()); };
    if points.is_empty() || points.len() != normals.len()
        || points.iter().chain(normals).flatten().any(|v| !v.is_finite()) { return Err("invalid vertex data".into()); }
    let indices = mesh.indices().map(|indices| indices.iter().collect::<Vec<_>>()).unwrap_or_else(|| (0..points.len()).collect());
    if indices.len() % 3 != 0 || indices.iter().any(|i| *i >= points.len()) { return Err("invalid triangle indices".into()); }
    println!("{}", triangle_quality(points, normals, &indices));
    println!("{}", fan_report(normals, &indices));
    print!("{}", zero_normal_witnesses(points, normals, &indices));
    print!("{}", filtered_normal_report(points, normals, &indices));
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for i in &indices { let p = Vec3::from(points[*i]); low = low.min(p); high = high.max(p); }
    let center = low * 0.5 + high * 0.5;
    let radius = (high - low).length() * 0.5;
    if !center.is_finite() || !radius.is_finite() || radius <= 0.0 { return Err("invalid bounds".into()); }
    let eye = center + Vec3::new(1.0, 0.7, 1.3).normalize() * radius * 3.5;
    let camera = Transform::from_translation(eye).looking_at(center, Vec3::Y).to_matrix();
    let rows = camera.to_cols_array_2d().map(|r| format!("({}, {}, {}, {})", r[0], r[1], r[2], r[3])).join(", ");
    let vectors = |values: &[[f32;3]]| values.iter().map(|p| format!("({},{},{})", p[0],p[1],p[2])).collect::<Vec<_>>().join(",");
    let point_text = vectors(points);
    let normal_text = vectors(normals);
    let index_text = indices.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
    let counts = vec!["3"; indices.len()/3].join(",");
    std::fs::create_dir(directory)?;
    let area_text = vectors(&area_weighted_normals(points, &indices));
    for (name, values) in [("without_normals", None), ("with_normals", Some(&normal_text)),
        ("area_weighted_normals", Some(&area_text))] {
        let normal = values.map(|values| format!("normal3f[] normals = [{values}] (interpolation = \"vertex\")")).unwrap_or_default();
        let text = format!(r#"#usda 1.0
( upAxis = "Y" )
def Mesh "Probe" {{
    uniform token subdivisionScheme = "none"
    point3f[] points = [{point_text}]
    int[] faceVertexCounts = [{counts}]
    int[] faceVertexIndices = [{index_text}]
    {normal}
}}
def Camera "Camera" {{
    float focalLength = 50
    float verticalAperture = 41.421356
    float horizontalAperture = 73.63797
    float2 clippingRange = ({near}, {far})
    matrix4d xformOp:transform = ({rows})
    uniform token[] xformOpOrder = ["xformOp:transform"]
}}
"#, near=(radius*0.001).max(0.00001), far=(radius*100.0).max(1000.0));
        std::fs::write(directory.join(format!("{name}.usda")), text)?;
    }
    println!("local_triangle_probe vertices={} triangles={} materials_subsets_transforms_deformation_omitted=true", points.len(), indices.len()/3);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(3..=4).contains(&args.len()) { return Err("usage: normal_isolation ASSET PRIM NEW_DIRECTORY [LEVELS]".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let source = usd_bevy::UsdSource::new(&path, std::fs::read(&path)?)?;
    let stage = source.open_stage()?;
    let prim = openusd::sdf::path(&args[1])?;
    let mut read = usd_bevy::read::geom::read_mesh_at(&stage, &prim, Some(0.0))?.ok_or("not a mesh")?;
    if let Some(levels) = args.get(3) {
        let levels = levels.parse::<u32>()?;
        if !(1..=6).contains(&levels) { return Err("levels must be 1..=6".into()); }
        let rules = usd_bevy::read::subdivision::read_subdivision_at(&stage, &prim, Some(0.0))?;
        read = usd_bevy::subdivision::refine_mesh(&read, &rules, levels)?;
    }
    write_probe(&usd_bevy::mesh::mesh_from_usd(&read), Path::new(&args[2]))
}

#[test]
fn area_weights_preserve_triangle_area_ratio_and_orientation() {
    for scale in [1e-12, 1.0, 1e12] {
        let points = [[0.,0.,0.], [1.,0.,0.], [0.,1.,0.], [0.,0.,4.]]
            .map(|point| point.map(|v| v * scale));
        let normals = area_weighted_normals(&points, &[0,1,2,0,3,1]);
        let expected = Vec3::new(0.,4.,1.).normalize();
        assert!(Vec3::from(normals[0]).abs_diff_eq(expected, 1e-6));
        let reversed = area_weighted_normals(&points, &[0,2,1,0,1,3]);
        assert!(Vec3::from(reversed[0]).abs_diff_eq(-expected, 1e-6));
        assert_eq!(area_weighted_normals(&points, &[0,0,0]), vec![[0.;3]; 4]);
    }
}

#[test]
fn fan_report_separates_vertex_only_contacts_and_winding_errors() {
    assert!(fan_report(&[[0.0;3];5], &[0,1,2,0,3,4]).contains("disconnected_vertices=1 zero_normal_vertices=5 zero_normal_disconnected_vertices=1"));
    let joined = fan_report(&[[0.,0.,1.];4], &[0,1,2,1,0,3]);
    assert!(joined.contains("inconsistent_winding_edges=0 disconnected_vertices=0"));
    assert!(fan_report(&[[0.,0.,1.];4], &[0,1,2,0,1,3]).contains("inconsistent_winding_edges=1"));
    assert!(fan_report(&[[0.,0.,1.];5], &[0,1,2,1,0,3,0,1,4]).contains("nonmanifold_edges=1"));
    let points = [[2.,0.,0.], [0.,-1.,0.], [1.,-1.,0.], [1.,1.,0.], [0.,1.,0.]];
    let indices = [0,1,2,0,2,3,0,3,4,0,4,1];
    let mut normals = [[0.,0.,1.];5];
    normals[0] = [0.0;3];
    assert!(fan_report(&normals, &indices).contains("inconsistent_winding_edges=0 disconnected_vertices=0 zero_normal_vertices=1"));
    let witness = zero_normal_witnesses(&points, &normals, &indices);
    assert!(witness.contains("zero_normal_vertex=0 position=[2.0, 0.0, 0.0] incident_triangles=4"));
    assert_eq!(witness.matches("face_normal=").count(), 4);
}

#[test]
fn triangle_quality_is_scale_invariant_and_counts_skinny_faces() {
    for scale in [1e-12_f32, 1.0, 1e12] {
        let points = [[0.,0.,0.], [1.,0.,0.], [0.,1e-7,0.], [0.,1.,0.]].map(|p| p.map(|v| v*scale));
        let report = triangle_quality(&points, &[[0.,0.,-1.];4], &[0,1,2,0,1,3,0,0,0]);
        assert!(report.contains("cumulative_triangles=[1, 1, 2, 2, 2]"));
        assert!(report.contains("cumulative_reversed_normal_corners=[0, 0, 3, 3, 3]"));
        let filtered = filtered_normal_report(&points, &[[0.,0.,1.];4], &[0,1,2,0,1,3]);
        assert!(filtered.contains("diagnostic_normal_filter=0e0 changed_vertices=0 remaining_reversed_normal_corners=0"));
        assert!(filtered.contains("diagnostic_normal_filter=1e-6 changed_vertices=1 remaining_reversed_normal_corners=0"));
        assert!(filtered.contains("diagnostic_normal_filter=1e-6 changed_vertices=1 remaining_reversed_normal_corners=0 zero_referenced_normals=1"));
    }
}

#[test]
fn probe_preserves_triangles_and_normals_without_reusing_directory() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("probe");
    let mesh = Mesh::from(Cuboid::new(1.0, 2.0, 3.0));
    write_probe(&mesh, &directory).unwrap();
    assert!(write_probe(&mesh, &directory).is_err());
    for authored in [false, true] {
        let path = directory.join(if authored { "with_normals.usda" } else { "without_normals.usda" });
        let source = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap();
        let stage = source.open_stage().unwrap();
        let read = usd_bevy::read::geom::read_mesh(&stage, &openusd::sdf::path("/Probe").unwrap()).unwrap().unwrap();
        assert_eq!(read.points.len(), mesh.count_vertices());
        assert!(matches!(mesh.attribute(Mesh::ATTRIBUTE_POSITION), Some(VertexAttributeValues::Float32x3(points)) if *points == read.points));
        assert_eq!(read.face_vertex_indices.len(), mesh.indices().unwrap().len());
        assert_eq!(read.face_vertex_indices, mesh.indices().unwrap().iter().map(|i| i as i32).collect::<Vec<_>>());
        assert_eq!(read.normals.is_some(), authored);
        if authored { assert!(matches!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL), Some(VertexAttributeValues::Float32x3(normals)) if *normals == read.normals.unwrap().values)); }
    }
}
