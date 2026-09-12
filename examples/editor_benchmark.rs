use std::{collections::HashSet, path::Path, time::{Duration, Instant}};
use bevy::prelude::*;
use usd_bevy::{UsdPlugin, editor::{EditorBridge, EditorCommand, EditorPlugin}, live::LiveStagePlugin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeekMode { Idle, Repeated, Unique, Timeline }

impl SeekMode {
    fn samples(self) -> usize { match self { Self::Idle => 0, Self::Repeated => 100, Self::Unique | Self::Timeline => 1000 } }
    fn clock(self, iteration: usize) -> f64 {
        if matches!(self, Self::Unique | Self::Timeline) && iteration >= 4 { (iteration-3) as f64*10.0/self.samples() as f64 }
        else { [0.0,5.0,10.0,5.0][iteration%4] }
    }
}

fn parse_rss(status: &str) -> Option<usize> {
    let mut value = status.lines().find(|line| line.starts_with("VmRSS:"))?.split_whitespace();
    value.next()?;
    let kib = value.next()?.parse::<usize>().ok()?;
    if value.next()? != "kB" { return None; }
    kib.checked_mul(1024)
}

fn resident_bytes() -> Option<usize> { parse_rss(&std::fs::read_to_string("/proc/self/status").ok()?) }

fn asset_counts(world: &World) -> [usize; 3] {
    [world.resource::<Assets<Mesh>>().len(), world.resource::<Assets<StandardMaterial>>().len(), world.resource::<Assets<Image>>().len()]
}

fn report_routes(world: &World, phase: &str, samples: usize) {
    if let Some(timings) = world.get_resource::<usd_bevy::route::ProjectionTimings>() {
        let mut routes: Vec<_> = timings.0.iter().collect();
        routes.sort_by_key(|(_, timing)| std::cmp::Reverse(timing.matching + timing.application));
        for (name, timing) in routes {
            eprintln!("route_profile phase={phase} samples={samples} route={name} attempts={} matches={} match_ms={:.3} apply_ms={:.3}",
                timing.attempts, timing.matches, timing.matching.as_secs_f64()*1000.0, timing.application.as_secs_f64()*1000.0);
        }
    }
}

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
    peak_asset_counts: [usize; 3],
    rss_before_seeks: Option<usize>,
    rss_after_seeks: Option<usize>,
    cached_meshes: usize,
    cached_mesh_payload_bytes: usize,
    cached_tangent_payload_bytes: usize,
}

fn measure(path: &Path, gpu_prepared: bool, seek: SeekMode) -> Result<Measurement, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, LiveStagePlugin, EditorPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    if gpu_prepared { app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin); }
    if std::env::var_os("USD_PROFILE_ROUTES").is_some() { app.init_resource::<usd_bevy::route::ProjectionTimings>(); }
    app.update();
    let bridge = app.world().resource::<EditorBridge>().clone();
    bridge.send(EditorCommand::Open(path.to_str().ok_or("asset path must be UTF-8")?.into()))?;
    let start = Instant::now();
    app.update();
    let open = start.elapsed();
    report_routes(app.world(), "initial-open", 1);
    let view = bridge.view()?;
    if view.status != "Ready" { return Err(format!("editor open failed: {}", view.status).into()); }
    let start = Instant::now();
    for _ in 0..100 { app.update(); }
    let idle = start.elapsed() / 100;
    let mut result = Measurement { open, idle, peak_asset_counts: asset_counts(app.world()), rss_before_seeks: resident_bytes(), ..default() };
    if seek != SeekMode::Idle {
        if seek == SeekMode::Timeline {
            if !view.timeline.start.is_finite() || !view.timeline.end.is_finite()
                || view.timeline.end <= view.timeline.start || !(view.timeline.end-view.timeline.start).is_finite() {
                return Err("timeline seek requires a finite increasing timeline".into());
            }
            println!("seek_timeline_start={} seek_timeline_end={}", view.timeline.start, view.timeline.end);
        }
        let mut timings = Vec::with_capacity(seek.samples());
        for iteration in 0..seek.samples()+4 {
            if iteration == 4 {
                if let Some(mut timings) = app.world_mut().get_resource_mut::<usd_bevy::route::ProjectionTimings>() { timings.0.clear(); }
            }
            let normalized = seek.clock(iteration);
            let time = if seek == SeekMode::Timeline {
                view.timeline.start + (view.timeline.end-view.timeline.start) * (normalized/10.0)
            } else { normalized };
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
            for (peak,current) in result.peak_asset_counts.iter_mut().zip(asset_counts(app.world())) { *peak = (*peak).max(current); }
        }
        timings.sort_unstable();
        result.seek_median = timings[timings.len().div_ceil(2)-1];
        result.seek_p95 = timings[(timings.len()*95).div_ceil(100)-1];
        result.seek_max = *timings.last().unwrap();
        report_routes(app.world(), "measured-seeks", seek.samples());
    }
    result.rss_after_seeks = resident_bytes();
    if let Some(cache) = app.world().get_resource::<usd_bevy::route::cache::ProjectionCache>() {
        result.cached_meshes = cache.len();
        result.cached_mesh_payload_bytes = cache.retained_payload_bytes();
    }
    if let Some(cache) = app.world().get_resource::<usd_bevy::route::cache::MeshTangentCache>() {
        result.cached_tangent_payload_bytes = cache.retained_payload_bytes();
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
    if std::env::var_os("USD_PROFILE_IMAGES").is_some() {
        use std::hash::{Hash, Hasher};
        let mut groups = std::collections::HashMap::new();
        for (_, image) in app.world().resource::<Assets<Image>>().iter() {
            let Some(data) = &image.data else { continue };
            let size = image.texture_descriptor.size;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut hash);
            let key = (image.texture_descriptor.format, size.width, size.height,
                size.depth_or_array_layers, data.len(), hash.finish());
            let group = groups.entry(key).or_insert((0_usize, data));
            if group.1 != data { return Err("image profiling hash collision".into()); }
            group.0 += 1;
        }
        let unique: usize = groups.keys().map(|key| key.4).sum();
        eprintln!("image_payload_groups={} hashed_unique_bytes={unique} retained_bytes={}", groups.len(), result.image_bytes);
        let mut repeated: Vec<_> = groups.into_iter().filter(|(_, (count, _))| *count > 1)
            .map(|(key, (count, _))| (key, count)).collect();
        repeated.sort_by_key(|(key, count)| std::cmp::Reverse(key.4 * (count - 1)));
        for (key, count) in repeated {
            eprintln!("image_payload_repeat format={:?} size={}x{}x{} bytes={} copies={count} hash={}",
                key.0, key.1, key.2, key.3, key.4, key.5);
        }
    }
    if bridge.view()?.document.document_id != view.document.document_id {
        return Err("idle updates replaced the editor document".into());
    }
    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(1..=4).contains(&args.len()) { return Err("usage: editor_benchmark ASSET [SAMPLES] [cpu|gpu-prepared] [seek|seek-unique|seek-timeline]".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let samples = args.get(1).map(|value| value.parse::<usize>()).transpose()?.unwrap_or(3);
    if !(1..=10).contains(&samples) { return Err("samples must be between 1 and 10".into()); }
    let seek = match args.get(3).map(String::as_str) {
        None => SeekMode::Idle,
        Some("seek") => SeekMode::Repeated,
        Some("seek-unique") => SeekMode::Unique,
        Some("seek-timeline") => SeekMode::Timeline,
        _ => return Err("fourth argument must be seek, seek-unique or seek-timeline".into()),
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
    println!("seek={seek:?} seek_clocks={} seek_warmup={} seek_samples={} seek_percentiles=nearest-rank payload_phase={}",
        if seek == SeekMode::Timeline { "1000-distinct-times-in-authored-range" }
        else if seek == SeekMode::Unique { "1000-distinct-times-in-(0,10]" } else { "0,5,10,5" },
        if seek == SeekMode::Idle { 0 } else { 4 }, seek.samples(), if seek != SeekMode::Idle { "after-seeks" } else { "after-idle" });
    println!("rss=whole-process-linux-VmRSS-or-NA rss_phase=after-idle,after-seeks peak_assets=sampled-after-updates-not-allocation-counts");
    println!("sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes,seek_median_us,seek_p95_us,seek_max_us,gpu_morph_entities,peak_mesh_assets,peak_material_assets,peak_image_assets,rss_before_seeks,rss_after_seeks,cached_meshes,cached_mesh_payload_bytes,cached_tangent_payload_bytes");
    for sample in 0..samples {
        let m = measure(&path, gpu_prepared, seek)?;
        println!("{sample},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{},{},{},{},{},{},{},{},{}", m.open.as_secs_f64()*1000.0,
            m.idle.as_secs_f64()*1_000_000.0, m.mesh_entities, m.subset_entities, m.mesh_assets,
            m.vertices, m.unreferenced_vertices, m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes,
            m.seek_median.as_secs_f64()*1_000_000.0, m.seek_p95.as_secs_f64()*1_000_000.0, m.seek_max.as_secs_f64()*1_000_000.0, m.gpu_morph_entities,
            m.peak_asset_counts[0], m.peak_asset_counts[1], m.peak_asset_counts[2],
            m.rss_before_seeks.map_or("NA".into(), |v| v.to_string()), m.rss_after_seeks.map_or("NA".into(), |v| v.to_string()),
            m.cached_meshes,m.cached_mesh_payload_bytes,m.cached_tangent_payload_bytes);
    }
    Ok(())
}

#[test]
fn timeline_benchmark_seeks_the_composed_showcase() {
    let mode = SeekMode::Timeline;
    assert_eq!(mode.samples(), 1000);
    assert_eq!(mode.clock(4), 0.01);
    assert_eq!(mode.clock(1003), 10.0);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/flagship_showcase.usda");
    let result = measure(&path, true, mode).unwrap();
    assert_eq!(result.gpu_morph_entities, 1);
    assert!(result.mesh_entities >= 8);
    assert!(result.seek_max >= result.seek_p95 && result.seek_p95 >= result.seek_median);
}

#[test]
fn unique_seek_schedule_and_resident_measurement_are_explicit() {
    let clocks = (4..1004).map(|i| SeekMode::Unique.clock(i)).collect::<Vec<_>>();
    assert_eq!(clocks.first(),Some(&0.01));
    assert_eq!(clocks.last(),Some(&10.0));
    assert!(clocks.windows(2).all(|pair| pair[0]<pair[1]));
    assert_eq!(clocks.iter().map(|v| v.to_bits()).collect::<HashSet<_>>().len(),1000);
    assert_eq!(parse_rss("Name: probe\nVmRSS:\t123 kB\n"),Some(123*1024));
    assert_eq!(parse_rss("VmRSS: 123 MB"),None);
    assert_eq!(parse_rss("VmRSS: missing kB"),None);
    assert_eq!(parse_rss("Name: probe"),None);
    assert_eq!(parse_rss(&format!("VmRSS: {} kB",usize::MAX)),None);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_tangent_normals.usda");
    for gpu_prepared in [false,true] {
        let result = measure(&path,gpu_prepared,SeekMode::Unique).unwrap();
        assert!(result.seek_median>Duration::ZERO);
        assert!(result.seek_median<=result.seek_p95 && result.seek_p95<=result.seek_max);
        assert!(result.peak_asset_counts[0]>=result.mesh_assets);
        assert!(result.peak_asset_counts[0]<=16,"{result:?}");
        assert!(result.cached_meshes<=4,"{result:?}");
    }
}

#[test]
fn animated_materials_do_not_retain_every_sample() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/animation_showcase.usda");
    for gpu_prepared in [false, true] {
        let result = measure(&path, gpu_prepared, SeekMode::Unique).unwrap();
        assert!(result.peak_asset_counts[1] <= 16, "{result:?}");
    }
}

#[test]
fn editor_benchmark_counts_subset_payload_and_rejects_failed_opens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let m = measure(&root.join("assets/material_subsets.usda"), false, SeekMode::Idle).unwrap();
    assert_eq!((m.mesh_entities, m.subset_entities, m.mesh_assets), (3, 2, 3));
    assert_eq!((m.vertices, m.unreferenced_vertices), (8, 0));
    assert_eq!((m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes), (256, 48, 0, 0));
    assert!(measure(&root.join("assets/missing-editor-benchmark.usda"), false, SeekMode::Idle).is_err());
}

#[test]
fn morph_payload_measurement_counts_inline_attributes() {
    use bevy::mesh::morph::MorphAttributes;
    let mut mesh = Mesh::from(Cuboid::default());
    assert_eq!(mesh.get_morph_targets().map_or(0, size_of_val), 0);
    mesh.set_morph_targets(vec![MorphAttributes::default(); 24]);
    assert_eq!(mesh.get_morph_targets().map_or(0, size_of_val), 24 * size_of::<MorphAttributes>());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_animation.usda");
    let cpu = measure(&path, false, SeekMode::Idle).unwrap();
    let prepared = measure(&path, true, SeekMode::Idle).unwrap();
    assert_eq!(cpu.morph_bytes, 0);
    assert!(prepared.morph_bytes > 0);
    assert_eq!(prepared.morph_bytes % size_of::<MorphAttributes>(), 0);
    assert_eq!(prepared.image_bytes, 0);
    assert_eq!(cpu.mesh_entities, prepared.mesh_entities);
}

#[test]
fn seek_measurement_tracks_clocks_and_prepared_morphs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_tangent_normals.usda");
    let cpu = measure(&path, false, SeekMode::Repeated).unwrap();
    let gpu = measure(&path, true, SeekMode::Repeated).unwrap();
    assert_eq!(cpu.gpu_morph_entities, 0);
    assert!(gpu.gpu_morph_entities > 0);
    assert_eq!(cpu.mesh_entities, gpu.mesh_entities);
    for result in [cpu, gpu] {
        assert!(result.seek_median > Duration::ZERO);
        assert!(result.seek_median <= result.seek_p95 && result.seek_p95 <= result.seek_max);
        assert!(result.image_bytes > 0);
    }
}
