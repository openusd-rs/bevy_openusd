use std::collections::{HashMap, HashSet};

use bevy::mesh::{Mesh, VertexAttributeValues};
use usd_bevy::read::geom::ReadMesh;

fn surface_diagnostics(mesh: &Mesh) -> String {
    let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return "surface=missing_positions".into(); };
    let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) { Some(VertexAttributeValues::Float32x3(values)) => Some(values), _ => None };
    let indices = mesh.indices().map(|indices| indices.iter().collect::<Vec<_>>()).unwrap_or_else(|| (0..points.len()).collect());
    let mut faces = HashSet::new();
    let mut geometric_edges = HashMap::<[[u32;3];2], usize>::new();
    let mut indexed_edges = HashMap::<[usize;2], usize>::new();
    let (mut duplicate, mut degenerate, mut reversed, mut grazing, mut corners) = (0,0,0,0,0);
    let mut minimum_dot = 1.0f64;
    for triangle in indices.chunks_exact(3) {
        let Some(vertices) = triangle.iter().map(|index| points.get(*index).copied()).collect::<Option<Vec<_>>>() else { continue; };
        let p = vertices.iter().map(|point| bevy::math::DVec3::from_array(point.map(f64::from))).collect::<Vec<_>>();
        let Some(face) = (p[1]-p[0]).cross(p[2]-p[0]).try_normalize() else { degenerate += 1; continue; };
        let mut key = vertices.iter().map(|point| point.map(|value| if value == 0. { 0 } else { value.to_bits() })).collect::<Vec<_>>();
        for edge in [[0,1], [1,2], [2,0]] {
            let mut geometric = [key[edge[0]], key[edge[1]]];
            geometric.sort_unstable();
            *geometric_edges.entry(geometric).or_default() += 1;
            let mut indexed = [triangle[edge[0]], triangle[edge[1]]];
            indexed.sort_unstable();
            *indexed_edges.entry(indexed).or_default() += 1;
        }
        key.sort_unstable();
        if !faces.insert(key) { duplicate += 1; }
        for index in triangle {
            let Some(normal) = normals.and_then(|normals| normals.get(*index)).and_then(|normal| bevy::math::DVec3::from_array(normal.map(f64::from)).try_normalize()) else { continue; };
            let dot = face.dot(normal);
            minimum_dot = minimum_dot.min(dot);
            reversed += usize::from(dot < -0.001);
            grazing += usize::from(dot.abs() < 0.1);
            corners += 1;
        }
    }
    format!("triangles={} degenerate_triangles={degenerate} exact_duplicate_triangles={duplicate} checked_normal_corners={corners} reversed_normal_corners={reversed} grazing_normal_corners={grazing} minimum_face_normal_dot={} indexed_boundary_edges={} position_welded_boundary_edges={} position_welded_nonmanifold_edges={}",
        indices.len()/3, if corners == 0 { "none".into() } else { format!("{minimum_dot:.6}") },
        indexed_edges.values().filter(|count| **count == 1).count(), geometric_edges.values().filter(|count| **count == 1).count(),
        geometric_edges.values().filter(|count| **count > 2).count())
}

fn describe(mesh: &ReadMesh) -> String {
    let projected = usd_bevy::mesh::mesh_from_usd(mesh);
    let normals = mesh.normals.as_ref().map_or_else(|| "absent".to_string(), |normals| {
        let invalid = normals.values.iter().filter(|normal| {
            !normal.iter().all(|v| v.is_finite()) || normal.iter().all(|v| *v == 0.0)
        }).count();
        format!("{:?}:values={},indices={},invalid={invalid}", normals.interpolation, normals.values.len(), normals.indices.len())
    });
    let unique_normals = match projected.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(normals)) => normals.iter()
            .map(|normal| normal.map(f32::to_bits)).collect::<HashSet<_>>().len(),
        _ => 0,
    };
    let invalid_normals = match projected.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(normals)) => normals.iter().filter(|normal|
            !normal.iter().all(|value| value.is_finite()) || normal.iter().all(|value| *value == 0.0)).count(),
        _ => projected.count_vertices(),
    };
    let preserved = mesh.normals.as_ref().is_some_and(|normals| normals.indices.is_empty()
        && normals.interpolation == usd_bevy::read::geom::Interpolation::Vertex
        && matches!(projected.attribute(Mesh::ATTRIBUTE_NORMAL), Some(VertexAttributeValues::Float32x3(values)) if values == &normals.values));
    format!("points={} faces={} corners={} subdivision={:?} authored_normals={} uv_values={} projected_vertices={} projected_unique_normals={} projected_invalid_normals={} subsets={} double_sided={} authored_vertex_normals_preserved={preserved}\n  {}",
        mesh.points.len(), mesh.face_vertex_counts.len(), mesh.face_vertex_indices.len(), mesh.subdivision_scheme,
        normals, mesh.uvs.as_ref().map_or(0, |uv| uv.values.len()), projected.count_vertices(), unique_normals,
        invalid_normals, mesh.subsets.len(), mesh.double_sided, surface_diagnostics(&projected))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 3 { return Err("usage: scene_report ASSET [TIME] [REFINEMENT_LEVELS]".into()); }
    let time = args.get(1).map(|value| value.parse::<f64>()).transpose()?;
    if time.is_some_and(|value| !value.is_finite()) { return Err("time must be finite".into()); }
    let levels = args.get(2).map(|value| value.parse::<u32>()).transpose()?;
    if levels.is_some_and(|value| !(1..=6).contains(&value)) { return Err("refinement levels must be 1..=6".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let source = usd_bevy::UsdSource::new(&path, std::fs::read(&path)?)?;
    let stage = source.open_stage()?;
    if let Some(output) = std::env::var_os("USD_REPORT_EXPORT_LAYER") {
        use std::io::Write;
        let output = std::path::PathBuf::from(output);
        if output.extension().and_then(|value| value.to_str()) != Some("usda") { return Err("USD_REPORT_EXPORT_LAYER must end in .usda".into()); }
        let text = stage.root_layer().export_to_string()?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&output)?;
        file.write_all(text.as_bytes())?;
        println!("root_layer_export={} relative_assets_are_not_rebased=true", output.display());
    }
    println!("asset={} time={time:?} geometry=source-sampled-without-deformation scope=all-traversed-meshes-not-visibility-filtered", path.display());
    let mut pending = stage.prim("/")?.children()?;
    let mut count = 0;
    let mut refinement_errors = 0;
    while let Some(prim) = pending.pop() {
        pending.extend(prim.children()?);
        if let Some(mesh) = usd_bevy::read::geom::read_mesh_at(&stage, prim.path(), time)? {
            let subdivision_authored = prim.attribute("subdivisionScheme").resolve_info()?.has_authored_value();
            println!("{} subdivision_authored={subdivision_authored} {}", prim.path(), describe(&mesh));
            if mesh.subdivision_scheme.is_subdivision() {
                let rules = usd_bevy::read::subdivision::read_subdivision_at(&stage, prim.path(), time)?;
                rules.validate(mesh.points.len(), mesh.face_vertex_counts.len())?;
                println!("  boundary={} face_varying={} triangle_rule={} creases={} corners={} holes={}",
                    rules.boundary, rules.face_varying, rules.triangle_rule,
                    rules.crease_lengths.len(), rules.corner_indices.len(), rules.holes.len());
                println!("WARNING {} subdivision surface is not evaluated; projected geometry is the control cage", prim.path());
                if let Some(levels) = levels {
                    match usd_bevy::subdivision::refine_mesh(&mesh, &rules, levels) {
                        Ok(refined) => println!("  finite_refinement_levels={levels} {}", describe(&refined)),
                        Err(error) => {
                            refinement_errors += 1;
                            println!("ERROR {} refinement: {error}", prim.path());
                        }
                    }
                }
            }
            count += 1;
        }
    }
    println!("meshes={count}");
    if refinement_errors > 0 { return Err(format!("{refinement_errors} meshes failed refinement").into()); }
    Ok(())
}

#[test]
fn surface_report_detects_reversed_normals_duplicates_and_degeneracy() {
    let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, Default::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.,0.,0.], [1.,0.,0.], [0.,1.,0.]]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.,0.,-1.];3]);
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0,1,2,2,1,0,0,0,0]));
    let report = surface_diagnostics(&mesh);
    assert!(report.contains("triangles=3 degenerate_triangles=1 exact_duplicate_triangles=1"));
    assert!(report.contains("checked_normal_corners=6 reversed_normal_corners=3 grazing_normal_corners=0"));
    assert!(report.contains("minimum_face_normal_dot=-1.000000"));
    assert!(report.contains("indexed_boundary_edges=0 position_welded_boundary_edges=0 position_welded_nonmanifold_edges=0"));
    let mut split = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, Default::default());
    split.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.,0.,0.], [1.,0.,0.], [1.,1.,0.], [0.,0.,0.], [1.,1.,0.], [0.,1.,0.]]);
    split.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.,0.,1.];6]);
    assert!(surface_diagnostics(&split).contains("indexed_boundary_edges=6 position_welded_boundary_edges=4 position_welded_nonmanifold_edges=0"));
}

#[test]
fn report_distinguishes_authored_and_generated_normals() {
    let source = usd_bevy::UsdSource::new("report.usda", br#"#usda 1.0
def Mesh "Mesh" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    uniform token subdivisionScheme = "none"
}
"#.as_slice()).unwrap();
    let stage = source.open_stage().unwrap();
    let path = openusd::sdf::path("/Mesh").unwrap();
    let mut mesh = usd_bevy::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
    let report = describe(&mesh);
    assert!(report.contains("authored_normals=absent"));
    assert!(report.contains("projected_vertices=3"));
    assert!(report.contains("projected_unique_normals=1"));
    mesh.normals = Some(usd_bevy::read::geom::MeshPrimvar {
        values: vec![[0.0; 3]], interpolation: usd_bevy::read::geom::Interpolation::Constant, indices: vec![],
    });
    assert!(describe(&mesh).contains("invalid=1"));
}
