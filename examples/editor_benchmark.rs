use std::{collections::HashSet, path::Path, time::{Duration, Instant}};
use bevy::prelude::*;
use usd_bevy::{UsdPlugin, editor::{EditorBridge, EditorCommand, EditorPlugin}, live::LiveStagePlugin};

#[derive(Default, Debug)]
struct Measurement {
    open: Duration,
    idle: Duration,
    seek_median: Duration,
    seek_p95: Duration,
    seek_max: Duration,
    gpu_morph_entities: usize,
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

fn measure(path: &Path, gpu_prepared: bool, seek: bool) -> Result<Measurement, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, LiveStagePlugin, EditorPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    if gpu_prepared { app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin); }
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
    if seek {
        let mut timings = Vec::with_capacity(100);
        for iteration in 0..104 {
            let time = [0.0, 5.0, 10.0, 5.0][iteration % 4];
            bridge.send(EditorCommand::Seek(time))?;
            let start = Instant::now();
            app.update();
            let elapsed = start.elapsed();
            let state = bridge.view()?;
            if state.status != "Ready" || state.timeline.current != time
                || state.document.document_id != view.document.document_id {
                return Err(format!("seek did not preserve document and clock: {}", state.status).into());
            }
            if iteration >= 4 { timings.push(elapsed); }
        }
        timings.sort_unstable();
        result.seek_median = timings[49];
        result.seek_p95 = timings[94];
        result.seek_max = timings[99];
    }
    result.gpu_morph_entities = app.world_mut().query::<&usd_bevy::route::gpu_morph::UsdGpuMorph>().iter(app.world()).count();
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
    if !(1..=4).contains(&args.len()) { return Err("usage: editor_benchmark ASSET [SAMPLES] [cpu|gpu-prepared] [seek]".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let samples = args.get(1).map(|value| value.parse::<usize>()).transpose()?.unwrap_or(3);
    if !(1..=10).contains(&samples) { return Err("samples must be between 1 and 10".into()); }
    let seek = match args.get(3).map(String::as_str) {
        None => false,
        Some("seek") => true,
        _ => return Err("fourth argument must be seek".into()),
    };
    let gpu_prepared = match args.get(2).map(String::as_str) {
        None | Some("cpu") => false,
        Some("gpu-prepared") => true,
        _ => return Err("deformation must be cpu or gpu-prepared".into()),
    };
    println!("asset={} profile={} renderer=headless deformation={} excludes=plugin-startup,ui,gpu assets=all-retained-not-visibility-filtered samples={samples}",
        path.display(), if cfg!(debug_assertions) { "debug" } else { "release" },
        if gpu_prepared { "gpu-prepared" } else { "cpu" });
    println!("payloads=retained-cpu-bytes excludes=gpu-allocation,asset-handles,allocator-overhead");
    println!("seek={seek} seek_clocks=0,5,10,5 seek_warmup=4 seek_samples=100 seek_percentiles=nearest-rank payload_phase={}", if seek { "after-seeks" } else { "after-idle" });
    println!("sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes,seek_median_us,seek_p95_us,seek_max_us,gpu_morph_entities");
    for sample in 0..samples {
        let m = measure(&path, gpu_prepared, seek)?;
        println!("{sample},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{}", m.open.as_secs_f64()*1000.0,
            m.idle.as_secs_f64()*1_000_000.0, m.mesh_entities, m.subset_entities, m.mesh_assets,
            m.vertices, m.unreferenced_vertices, m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes,
            m.seek_median.as_secs_f64()*1_000_000.0, m.seek_p95.as_secs_f64()*1_000_000.0, m.seek_max.as_secs_f64()*1_000_000.0, m.gpu_morph_entities);
    }
    Ok(())
}

#[test]
fn editor_benchmark_counts_subset_payload_and_rejects_failed_opens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let m = measure(&root.join("assets/material_subsets.usda"), false, false).unwrap();
    assert_eq!((m.mesh_entities, m.subset_entities, m.mesh_assets), (3, 2, 4));
    assert_eq!((m.vertices, m.unreferenced_vertices), (16, 0));
    assert_eq!((m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes), (512, 96, 0, 0));
    assert!(measure(&root.join("assets/missing-editor-benchmark.usda"), false, false).is_err());
}

#[test]
fn morph_payload_measurement_counts_inline_attributes() {
    use bevy::mesh::morph::MorphAttributes;
    let mut mesh = Mesh::from(Cuboid::default());
    assert_eq!(mesh.get_morph_targets().map_or(0, size_of_val), 0);
    mesh.set_morph_targets(vec![MorphAttributes::default(); 24]);
    assert_eq!(mesh.get_morph_targets().map_or(0, size_of_val), 24 * size_of::<MorphAttributes>());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_animation.usda");
    let cpu = measure(&path, false, false).unwrap();
    let prepared = measure(&path, true, false).unwrap();
    assert_eq!(cpu.morph_bytes, 0);
    assert!(prepared.morph_bytes > 0);
    assert_eq!(prepared.morph_bytes % size_of::<MorphAttributes>(), 0);
    assert_eq!(prepared.image_bytes, 0);
    assert_eq!(cpu.mesh_entities, prepared.mesh_entities);
}

#[test]
fn seek_measurement_tracks_clocks_and_prepared_morphs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_tangent_normals.usda");
    let cpu = measure(&path, false, true).unwrap();
    let gpu = measure(&path, true, true).unwrap();
    assert_eq!(cpu.gpu_morph_entities, 0);
    assert!(gpu.gpu_morph_entities > 0);
    assert_eq!(cpu.mesh_entities, gpu.mesh_entities);
    for result in [cpu, gpu] {
        assert!(result.seek_median > Duration::ZERO);
        assert!(result.seek_median <= result.seek_p95 && result.seek_p95 <= result.seek_max);
        assert!(result.image_bytes > 0);
    }
}
