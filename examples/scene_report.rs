use std::collections::HashSet;

use bevy::mesh::{Mesh, VertexAttributeValues};
use usd_bevy::read::geom::ReadMesh;

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
    format!("points={} faces={} corners={} subdivision={:?} authored_normals={} uv_values={} projected_vertices={} projected_unique_normals={} projected_invalid_normals={} subsets={} double_sided={}",
        mesh.points.len(), mesh.face_vertex_counts.len(), mesh.face_vertex_indices.len(), mesh.subdivision_scheme,
        normals, mesh.uvs.as_ref().map_or(0, |uv| uv.values.len()), projected.count_vertices(), unique_normals,
        invalid_normals, mesh.subsets.len(), mesh.double_sided)
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
