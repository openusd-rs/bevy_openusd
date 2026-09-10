use std::path::Path;
use bevy::{mesh::{Mesh, VertexAttributeValues}, prelude::*};

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

fn write_probe(mesh: &Mesh, directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return Err("missing positions".into()); };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { return Err("missing normals".into()); };
    if points.is_empty() || points.len() != normals.len()
        || points.iter().chain(normals).flatten().any(|v| !v.is_finite()) { return Err("invalid vertex data".into()); }
    let indices = mesh.indices().map(|indices| indices.iter().collect::<Vec<_>>()).unwrap_or_else(|| (0..points.len()).collect());
    if indices.len() % 3 != 0 || indices.iter().any(|i| *i >= points.len()) { return Err("invalid triangle indices".into()); }
    println!("{}", triangle_quality(points, normals, &indices));
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
    for authored in [false, true] {
        let normal = if authored { format!("normal3f[] normals = [{normal_text}] (interpolation = \"vertex\")") } else { String::new() };
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
        std::fs::write(directory.join(if authored { "with_normals.usda" } else { "without_normals.usda" }), text)?;
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
