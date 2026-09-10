use std::{collections::HashSet, path::Path, time::{Duration, Instant}};
use bevy::prelude::*;
use usd_bevy::{UsdPlugin, editor::{EditorBridge, EditorCommand, EditorPlugin}, live::LiveStagePlugin};

#[derive(Default, Debug)]
struct Measurement {
    open: Duration,
    idle: Duration,
    mesh_entities: usize,
    subset_entities: usize,
    mesh_assets: usize,
    vertices: usize,
    unreferenced_vertices: usize,
    vertex_bytes: usize,
    index_bytes: usize,
    morph_bytes: usize,
    image_bytes: usize,
}

fn measure(path: &Path) -> Result<Measurement, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, LiveStagePlugin, EditorPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    app.update();
    let bridge = app.world().resource::<EditorBridge>().clone();
    bridge.send(EditorCommand::Open(path.to_str().ok_or("asset path must be UTF-8")?.into()))?;
    let start = Instant::now();
    app.update();
    let open = start.elapsed();
    let view = bridge.view()?;
    if view.status != "Ready" { return Err(format!("editor open failed: {}", view.status).into()); }
    let start = Instant::now();
    for _ in 0..100 { app.update(); }
    let idle = start.elapsed() / 100;
    let mut result = Measurement { open, idle, ..default() };
    result.mesh_entities = app.world_mut().query::<&Mesh3d>().iter(app.world()).count();
    result.subset_entities = app.world_mut().query::<&usd_bevy::route::subset::UsdSubset>().iter(app.world()).count();
    for (_, mesh) in app.world().resource::<Assets<Mesh>>().iter() {
        result.mesh_assets += 1;
        let vertices = mesh.count_vertices();
        result.vertices += vertices;
        result.vertex_bytes += mesh.attributes().map(|(_, values)| values.get_bytes().len()).sum::<usize>();
        if let Some(indices) = mesh.indices() {
            let referenced: HashSet<_> = indices.iter().collect();
            if referenced.iter().any(|&index| index >= vertices) { return Err("projected mesh has out-of-range indices".into()); }
            result.unreferenced_vertices += vertices - referenced.len();
            result.index_bytes += match indices {
                bevy::mesh::Indices::U16(values) => values.len() * size_of::<u16>(),
                bevy::mesh::Indices::U32(values) => values.len() * size_of::<u32>(),
            };
        }
        result.morph_bytes += mesh.get_morph_targets().map_or(0, size_of_val);
    }
    result.image_bytes = app.world().resource::<Assets<Image>>().iter()
        .map(|(_, image)| image.data.as_ref().map_or(0, Vec::len)).sum();
    if bridge.view()?.document.document_id != view.document.document_id {
        return Err("idle updates replaced the editor document".into());
    }
    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(1..=2).contains(&args.len()) { return Err("usage: editor_benchmark ASSET [SAMPLES]".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let samples = args.get(1).map(|value| value.parse::<usize>()).transpose()?.unwrap_or(3);
    if !(1..=10).contains(&samples) { return Err("samples must be between 1 and 10".into()); }
    println!("asset={} profile={} renderer=headless deformation=cpu excludes=plugin-startup,ui,gpu assets=all-retained-not-visibility-filtered samples={samples}",
        path.display(), if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes");
    for sample in 0..samples {
        let m = measure(&path)?;
        println!("{sample},{:.3},{:.3},{},{},{},{},{},{},{},{},{}", m.open.as_secs_f64()*1000.0,
            m.idle.as_secs_f64()*1_000_000.0, m.mesh_entities, m.subset_entities, m.mesh_assets,
            m.vertices, m.unreferenced_vertices, m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes);
    }
    Ok(())
}

#[test]
fn editor_benchmark_counts_subset_payload_and_rejects_failed_opens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let m = measure(&root.join("assets/material_subsets.usda")).unwrap();
    assert_eq!((m.mesh_entities, m.subset_entities, m.mesh_assets), (3, 2, 4));
    assert_eq!((m.vertices, m.unreferenced_vertices), (32, 16));
    assert_eq!((m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes), (1024, 96, 0, 0));
    assert!(measure(&root.join("assets/missing-editor-benchmark.usda")).is_err());
}
