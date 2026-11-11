use bevy::prelude::*;
use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};
use usd_bevy::{
    UsdPlugin,
    editor::{EditorBridge, EditorCommand, EditorPlugin},
    live::LiveStagePlugin,
};

#[path = "editor_benchmark/paired.rs"]
mod paired;

#[path = "editor_benchmark/stepped.rs"]
mod stepped;

#[path = "editor_benchmark/residency.rs"]
mod residency_probe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeekMode {
    Idle,
    Repeated,
    Quick,
    Unique,
    Timeline,
}

const SEEK_WARMUP: usize = 4;

fn seek_sample_phase(iteration: usize) -> (&'static str, usize) {
    if iteration < SEEK_WARMUP {
        ("seek_warmup", iteration)
    } else {
        ("seek", iteration - SEEK_WARMUP)
    }
}

impl SeekMode {
    fn samples(self) -> usize {
        match self {
            Self::Idle => 0,
            Self::Repeated => 100,
            Self::Quick => 8,
            Self::Unique | Self::Timeline => 1000,
        }
    }
    fn clock(self, iteration: usize) -> f64 {
        if matches!(self, Self::Unique | Self::Timeline) && iteration >= SEEK_WARMUP {
            (iteration - SEEK_WARMUP + 1) as f64 * 10.0 / self.samples() as f64
        } else {
            [0.0, 5.0, 10.0, 5.0][iteration % 4]
        }
    }
}

fn parse_rss(status: &str) -> Option<usize> {
    parse_status_bytes(status, "VmRSS:")
}

fn parse_status_bytes(status: &str, field: &str) -> Option<usize> {
    let mut value = status
        .lines()
        .find(|line| line.split_whitespace().next() == Some(field))?
        .split_whitespace();
    value.next()?;
    let kib = value.next()?.parse::<usize>().ok()?;
    if value.next()? != "kB" {
        return None;
    }
    kib.checked_mul(1024)
}

fn resident_bytes() -> Option<usize> {
    parse_rss(&std::fs::read_to_string("/proc/self/status").ok()?)
}

fn peak_resident_bytes() -> Option<usize> {
    parse_status_bytes(
        &std::fs::read_to_string("/proc/self/status").ok()?,
        "VmHWM:",
    )
}

fn asset_counts(world: &World) -> [usize; 3] {
    [
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<StandardMaterial>>().len(),
        world.resource::<Assets<Image>>().len(),
    ]
}

fn image_buffer_report(images: &Assets<Image>) -> String {
    let (buffers, payload, capacity) = images
        .iter()
        .filter_map(|(_, image)| image.data.as_ref())
        .fold((0usize, 0, 0), |(buffers, payload, capacity), data| {
            (
                buffers + 1,
                payload + data.len(),
                capacity + data.capacity(),
            )
        });
    format!(
        "image_buffer_scope=main-world-Assets-Image-data image_assets={} data_buffers={buffers} payload_bytes={payload} capacity_bytes={capacity} spare_bytes={} excludes=source,mesh,handles,allocator-overhead,scratch,gpu",
        images.len(),
        capacity - payload
    )
}

fn promote_deferred(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    use usd_bevy::route::residency::{DeferHiddenMeshes, UsdDeferredMesh};
    if !app.world().contains_resource::<DeferHiddenMeshes>() {
        return Err("USD_BENCH_PREWARM_HIDDEN requires USD_DEFER_HIDDEN_MESHES".into());
    }
    let pending_before = app
        .world_mut()
        .query_filtered::<Entity, With<UsdDeferredMesh>>()
        .iter(app.world())
        .count();
    let meshes_before = app.world().resource::<Assets<Mesh>>().len();
    let rss_before = resident_bytes();
    if let Some(mut timings) = app
        .world_mut()
        .get_resource_mut::<usd_bevy::route::ProjectionTimings>()
    {
        timings.0.clear();
    }
    if let Some(mut timings) = app
        .world_mut()
        .get_resource_mut::<usd_bevy::route::ProjectionVisibilityTimings>()
    {
        timings.0.clear();
    }
    if let Some(mut metrics) = app
        .world_mut()
        .get_resource_mut::<usd_bevy::route::cache::MeshCacheMetrics>()
    {
        metrics.0.clear();
    }
    if let Some(mut reads) = app
        .world_mut()
        .get_resource_mut::<usd_bevy::route::MeshReadTiming>()
    {
        *reads = default();
    }
    let start = Instant::now();
    app.world_mut().remove_resource::<DeferHiddenMeshes>();
    let mut updates = Vec::new();
    loop {
        let preparation_before = app
            .world()
            .get_resource::<usd_bevy::route::residency::ResidencyPreparationTiming>()
            .map_or(Duration::ZERO, |timing| timing.elapsed);
        let palette_before = app
            .world()
            .get_resource::<usd_bevy::route::gpu_skin::GpuSkinUpdateTiming>()
            .map_or(Duration::ZERO, |timing| timing.elapsed);
        let update_started = Instant::now();
        app.update();
        let update_elapsed = update_started.elapsed();
        updates.push(update_elapsed);
        if update_elapsed > Duration::from_millis(100)
            && app
                .world()
                .contains_resource::<usd_bevy::route::residency::ResidencyPreparationTiming>()
        {
            let preparation = app
                .world()
                .get_resource::<usd_bevy::route::residency::ResidencyPreparationTiming>()
                .map_or(Duration::ZERO, |timing| timing.elapsed)
                - preparation_before;
            let palette = app
                .world()
                .get_resource::<usd_bevy::route::gpu_skin::GpuSkinUpdateTiming>()
                .map_or(Duration::ZERO, |timing| timing.elapsed)
                - palette_before;
            eprintln!(
                "deferred_slow_update index={} elapsed_ms={:.3} preparation_ms={:.3} palette_ms={:.3}",
                updates.len(),
                update_elapsed.as_secs_f64() * 1000.0,
                preparation.as_secs_f64() * 1000.0,
                palette.as_secs_f64() * 1000.0
            );
        }
        if app
            .world_mut()
            .query_filtered::<Entity, With<UsdDeferredMesh>>()
            .iter(app.world())
            .next()
            .is_none()
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(120) {
            return Err("deferred prewarm exceeded 120 seconds".into());
        }
    }
    let elapsed = start.elapsed();
    let pending_after = app
        .world_mut()
        .query_filtered::<Entity, With<UsdDeferredMesh>>()
        .iter(app.world())
        .count();
    eprintln!(
        "deferred_prewarm scope=all-deferred-cpu excludes=gpu,visibility-edit pending_before={pending_before} pending_after={pending_after} elapsed_ms={:.3} mesh_assets_before={meshes_before} mesh_assets_after={} rss_before={} rss_after={}",
        elapsed.as_secs_f64() * 1000.0,
        app.world().resource::<Assets<Mesh>>().len(),
        rss_before.map_or("NA".into(), |value| value.to_string()),
        resident_bytes().map_or("NA".into(), |value| value.to_string())
    );
    updates.sort_unstable();
    eprintln!(
        "deferred_prewarm_updates count={} p95_ms={:.3} max_ms={:.3} budget_ms={}",
        updates.len(),
        updates[(updates.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1000.0,
        updates.last().unwrap().as_secs_f64() * 1000.0,
        app.world()
            .get_resource::<usd_bevy::route::residency::UsdResidencyBudget>()
            .map_or("unlimited".into(), |budget| budget
                .0
                .as_millis()
                .to_string())
    );
    report_routes(app.world(), "deferred-prewarm", 1);
    if pending_after != 0 {
        return Err("prewarm left deferred meshes pending".into());
    }
    Ok(())
}

fn report_routes(world: &World, phase: &str, samples: usize) {
    if let Some(timing) = world.get_resource::<usd_bevy::route::TextureSamplingTiming>() {
        eprintln!(
            "texture_sampling_profile phase={phase} images={} input_bytes={} appended_bytes={} elapsed_ms={:.3} slowest_ms={:.3}",
            timing.images,
            timing.input_bytes,
            timing.appended_bytes,
            timing.elapsed.as_secs_f64() * 1000.0,
            timing.slowest.as_secs_f64() * 1000.0
        );
    }
    if let Some(timing) =
        world.get_resource::<usd_bevy::route::residency::ResidencyPreparationTiming>()
    {
        eprintln!(
            "residency_preparation_profile phase={phase} attempts={} elapsed_ms={:.3} slowest_prim={:?} slowest_ms={:.3}",
            timing.attempts,
            timing.elapsed.as_secs_f64() * 1000.0,
            timing.slowest.as_ref().map(|(path, _)| path),
            timing
                .slowest
                .as_ref()
                .map_or(0.0, |(_, elapsed)| elapsed.as_secs_f64() * 1000.0)
        );
    }
    if let Some(timing) = world.get_resource::<usd_bevy::route::gpu_skin::GpuSkinUpdateTiming>() {
        eprintln!(
            "gpu_skin_update_profile phase={phase} updates={} joints={} elapsed_ms={:.3}",
            timing.updates,
            timing.joints,
            timing.elapsed.as_secs_f64() * 1000.0
        );
    }
    if let Some(timing) =
        world.get_resource::<usd_bevy::route::preparation::MeshPreparationTiming>()
    {
        eprintln!(
            "mesh_preparation_profile phase={phase} batches={} parallel_batches={} inline_jobs={} jobs={} cache_hits={} peak_jobs={} peak_input_bytes={} elapsed_ms={:.3}",
            timing.batches,
            timing.parallel_batches,
            timing.inline_jobs,
            timing.jobs,
            timing.cache_hits,
            timing.peak_jobs,
            timing.peak_input_bytes,
            timing.elapsed.as_secs_f64() * 1000.0
        );
    }
    if let Some(timing) =
        world.get_resource::<usd_bevy::route::gpu_skin::GpuSkinPreparationTiming>()
    {
        for (step, elapsed) in &timing.phases {
            eprintln!(
                "gpu_skin_preparation_profile phase={phase} attempts={} failures={} step={step} elapsed_ms={:.3}",
                timing.attempts,
                timing.failures,
                elapsed.as_secs_f64() * 1000.0
            );
        }
    }
    if let Some(timings) = world.get_resource::<usd_bevy::route::ProjectionVisibilityTimings>() {
        for ((route, hidden), row) in &timings.0 {
            let visibility = match hidden {
                Some(true) => "hidden",
                Some(false) => "visible",
                None => "unknown",
            };
            eprintln!(
                "route_visibility_profile phase={phase} samples={samples} route={route} hierarchy={visibility} matches={} apply_ms={:.3}",
                row.matches,
                row.application.as_secs_f64() * 1000.0
            );
        }
    }
    if let Some(reads) = world.get_resource::<usd_bevy::route::MeshReadTiming>() {
        eprintln!(
            "mesh_read_profile phase={phase} samples={samples} requests={} decodes={} missing={} errors={} array_bytes={} elapsed_ms={:.3} scope=registry-contexts overlaps=route-matching-and-application",
            reads.requests,
            reads.decodes,
            reads.missing,
            reads.errors,
            reads.array_bytes,
            reads.elapsed.as_secs_f64() * 1000.0
        );
    }
    if let Some(metrics) = world.get_resource::<usd_bevy::route::cache::MeshCacheMetrics>() {
        for (name, value) in &metrics.0 {
            eprintln!(
                "mesh_cache_profile phase={phase} samples={samples} counter={name} value={value}"
            );
        }
    }
    if let Some(timings) = world.get_resource::<usd_bevy::route::ProjectionTimings>() {
        let mut routes: Vec<_> = timings.0.iter().collect();
        routes.sort_by_key(|(_, timing)| std::cmp::Reverse(timing.matching + timing.application));
        for (name, timing) in routes {
            eprintln!(
                "route_profile phase={phase} samples={samples} route={name} attempts={} matches={} match_ms={:.3} apply_ms={:.3}",
                timing.attempts,
                timing.matches,
                timing.matching.as_secs_f64() * 1000.0,
                timing.application.as_secs_f64() * 1000.0
            );
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
    rss_process_peak: Option<usize>,
    cached_meshes: usize,
    cached_mesh_payload_bytes: usize,
    cached_tangent_payload_bytes: usize,
    cached_assembly_payload_bytes: usize,
}

fn open_editor(
    path: &Path,
    gpu_prepared: bool,
    preparation: Option<usd_bevy::route::preparation::ParallelMeshPreparation>,
) -> Result<(App, EditorBridge, Measurement), Box<dyn std::error::Error>> {
    open_editor_with_deferral(
        path,
        gpu_prepared,
        preparation,
        std::env::var_os("USD_DEFER_HIDDEN_MESHES").is_some(),
    )
}

fn open_editor_with_deferral(
    path: &Path,
    gpu_prepared: bool,
    preparation: Option<usd_bevy::route::preparation::ParallelMeshPreparation>,
    defer: bool,
) -> Result<(App, EditorBridge, Measurement), Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        UsdPlugin,
        LiveStagePlugin,
        EditorPlugin,
    ));
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>();
    if defer {
        app.init_resource::<usd_bevy::route::residency::DeferHiddenMeshes>();
    }
    if let Ok(value) = std::env::var("USD_RESIDENCY_BUDGET_MS") {
        app.insert_resource(usd_bevy::route::residency::UsdResidencyBudget(
            Duration::from_millis(value.parse()?),
        ));
    }
    if gpu_prepared {
        app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin);
    }
    if std::env::var_os("USD_PROFILE_ROUTES").is_some() {
        app.init_resource::<usd_bevy::route::TextureSamplingTiming>();
        app.init_resource::<usd_bevy::route::material::MaterialResolveTimings>();
        app.init_resource::<usd_bevy::route::gpu_skin::GpuSkinUpdateTiming>();
        app.init_resource::<usd_bevy::route::gpu_skin::GpuSkinPreparationTiming>();
        app.init_resource::<usd_bevy::route::preparation::MeshPreparationTiming>();
        app.init_resource::<usd_bevy::route::residency::ResidencyPreparationTiming>();
        app.init_resource::<usd_bevy::route::ProjectionTimings>();
        if std::env::var_os("USD_PROFILE_VISIBILITY").is_some() {
            app.init_resource::<usd_bevy::route::ProjectionVisibilityTimings>();
        }
        app.init_resource::<usd_bevy::route::cache::MeshCacheMetrics>();
        app.init_resource::<usd_bevy::route::MeshReadTiming>();
    }
    if let Some(preparation) = preparation {
        app.insert_resource(preparation);
    }
    app.update();
    let bridge = app.world().resource::<EditorBridge>().clone();
    bridge.send(EditorCommand::Open(
        path.to_str().ok_or("asset path must be UTF-8")?.into(),
    ))?;
    let start = Instant::now();
    app.update();
    let open = start.elapsed();
    report_routes(app.world(), "initial-open", 1);
    let view = bridge.view()?;
    if view.status != "Ready" {
        return Err(format!("editor open failed: {}", view.status).into());
    }
    let start = Instant::now();
    for _ in 0..100 {
        app.update();
    }
    let idle = start.elapsed() / 100;
    report_routes(app.world(), "after-idle", 100);
    if std::env::var_os("USD_BENCH_PREWARM_HIDDEN").is_some() {
        promote_deferred(&mut app)?;
    }
    let result = Measurement {
        open,
        idle,
        peak_asset_counts: asset_counts(app.world()),
        rss_before_seeks: resident_bytes(),
        ..default()
    };
    if bridge.view()?.document.document_id != view.document.document_id {
        return Err("idle updates replaced the editor document".into());
    }
    Ok((app, bridge, result))
}

fn reset_profiles(world: &mut World) {
    fn reset<T: Resource<Mutability = bevy::ecs::component::Mutable> + Default>(world: &mut World) {
        if let Some(mut timing) = world.get_resource_mut::<T>() {
            *timing = default();
        }
    }
    reset::<usd_bevy::route::preparation::MeshPreparationTiming>(world);
    reset::<usd_bevy::route::gpu_skin::GpuSkinPreparationTiming>(world);
    reset::<usd_bevy::route::gpu_skin::GpuSkinUpdateTiming>(world);
    reset::<usd_bevy::route::residency::ResidencyPreparationTiming>(world);
    reset::<usd_bevy::route::material::MaterialResolveTimings>(world);
    reset::<usd_bevy::route::TextureSamplingTiming>(world);
    reset::<usd_bevy::route::ProjectionTimings>(world);
    reset::<usd_bevy::route::ProjectionVisibilityTimings>(world);
    reset::<usd_bevy::route::cache::MeshCacheMetrics>(world);
    reset::<usd_bevy::route::MeshReadTiming>(world);
}

fn measure(
    path: &Path,
    gpu_prepared: bool,
    seek: SeekMode,
) -> Result<Measurement, Box<dyn std::error::Error>> {
    measure_with_steps(path, gpu_prepared, seek, None)
}

fn measure_with_steps(
    path: &Path,
    gpu_prepared: bool,
    seek: SeekMode,
    mut steps: Option<&mut dyn std::io::BufRead>,
) -> Result<Measurement, Box<dyn std::error::Error>> {
    if steps.is_some() && seek != SeekMode::Timeline {
        return Err("stepped measurements require seek-timeline".into());
    }
    let (mut app, bridge, mut result) = open_editor(path, gpu_prepared, None)?;
    let view = bridge.view()?;
    if seek != SeekMode::Idle {
        let raw_samples = steps.is_some() || std::env::var_os("USD_BENCH_RAW_SAMPLES").is_some();
        if seek == SeekMode::Timeline {
            if !view.timeline.start.is_finite()
                || !view.timeline.end.is_finite()
                || view.timeline.end <= view.timeline.start
                || !(view.timeline.end - view.timeline.start).is_finite()
            {
                return Err("timeline seek requires a finite increasing timeline".into());
            }
            println!(
                "seek_timeline_start={} seek_timeline_end={}",
                view.timeline.start, view.timeline.end
            );
        }
        let mut timings = Vec::with_capacity(seek.samples());
        let mut warmup = [Duration::ZERO; SEEK_WARMUP];
        if steps.is_some() {
            println!(
                "seek_step_config samples={} warmup={SEEK_WARMUP} pool_threads={} excludes=stdin-wait,stdout,gpu scope=full-headless-editor-update",
                seek.samples(),
                bevy::tasks::ComputeTaskPool::get().thread_num()
            );
        }
        for iteration in 0..seek.samples() + SEEK_WARMUP {
            if iteration == SEEK_WARMUP {
                println!(
                    "seek_warmup_summary samples={} total_ns={} max_ns={}",
                    warmup.len(),
                    warmup.iter().sum::<Duration>().as_nanos(),
                    warmup.iter().max().unwrap().as_nanos()
                );
                report_routes(app.world(), "after-seek-warmup", SEEK_WARMUP);
                reset_profiles(app.world_mut());
            }
            let normalized = seek.clock(iteration);
            let time = if seek == SeekMode::Timeline {
                view.timeline.start
                    + (view.timeline.end - view.timeline.start) * (normalized / 10.0)
            } else {
                normalized
            };
            if let Some(reader) = steps.as_deref_mut() {
                stepped::await_step(reader, &mut std::io::stdout().lock(), iteration)?;
            }
            bridge.send(EditorCommand::Seek(time))?;
            let texture_before = app
                .world()
                .get_resource::<usd_bevy::route::TextureSamplingTiming>()
                .map(|timing| (timing.elapsed, timing.images, timing.appended_bytes));
            let start = Instant::now();
            app.update();
            let elapsed = start.elapsed();
            let state = bridge.view()?;
            if state.status != "Ready"
                || state.timeline.current != time
                || state.document.document_id != view.document.document_id
            {
                return Err(
                    format!("seek did not preserve document and clock: {}", state.status).into(),
                );
            }
            if iteration >= SEEK_WARMUP {
                timings.push(elapsed);
            } else {
                warmup[iteration] = elapsed;
            }
            if raw_samples {
                let (prefix, index) = seek_sample_phase(iteration);
                println!(
                    "{prefix}_sample index={index} clock={time:.17} update_ns={}",
                    elapsed.as_nanos()
                );
                if let Some((elapsed, images, appended)) = texture_before {
                    let timing = app
                        .world()
                        .resource::<usd_bevy::route::TextureSamplingTiming>();
                    println!(
                        "{prefix}_texture_sample index={index} finalize_ns={} images={} appended_bytes={}",
                        (timing.elapsed - elapsed).as_nanos(),
                        timing.images - images,
                        timing.appended_bytes - appended
                    );
                }
            }
            for (peak, current) in result
                .peak_asset_counts
                .iter_mut()
                .zip(asset_counts(app.world()))
            {
                *peak = (*peak).max(current);
            }
        }
        timings.sort_unstable();
        result.seek_median = timings[timings.len().div_ceil(2) - 1];
        result.seek_p95 = timings[(timings.len() * 95).div_ceil(100) - 1];
        result.seek_max = *timings.last().unwrap();
        report_routes(app.world(), "measured-seeks", seek.samples());
    }
    measure_payload(&mut app, &mut result)?;
    if bridge.view()?.document.document_id != view.document.document_id {
        return Err("updates replaced the editor document".into());
    }
    Ok(result)
}

fn measure_payload(
    app: &mut App,
    result: &mut Measurement,
) -> Result<(), Box<dyn std::error::Error>> {
    result.rss_after_seeks = resident_bytes();
    result.rss_process_peak = peak_resident_bytes();
    if let Some(cache) = app
        .world()
        .get_resource::<usd_bevy::route::cache::ProjectionCache>()
    {
        result.cached_meshes = cache.len();
        result.cached_mesh_payload_bytes = cache.retained_payload_bytes();
    }
    if let Some(cache) = app
        .world()
        .get_resource::<usd_bevy::route::cache::MeshTangentCache>()
    {
        result.cached_tangent_payload_bytes = cache.retained_payload_bytes();
    }
    if let Some(cache) = app
        .world()
        .get_resource::<usd_bevy::route::cache::MeshAssemblyCache>()
    {
        result.cached_assembly_payload_bytes = cache.retained_payload_bytes();
    }
    result.gpu_morph_entities = app
        .world_mut()
        .query::<&usd_bevy::route::gpu_morph::UsdGpuMorph>()
        .iter(app.world())
        .count();
    result.mesh_entities = app.world_mut().query::<&Mesh3d>().iter(app.world()).count();
    if std::env::var_os("USD_PROFILE_ROUTES").is_some() {
        let mut fallbacks = app.world_mut().query::<(
            &usd_bevy::UsdPrimRef,
            &usd_bevy::route::gpu_skin::UsdCpuSkinFallback,
            &Mesh3d,
            Option<&Visibility>,
        )>();
        let count = fallbacks.iter(app.world()).count();
        eprintln!(
            "cpu_skin_fallback_total count={count} shown={}",
            count.min(16)
        );
        for (prim, fallback, mesh, visibility) in fallbacks.iter(app.world()).take(16) {
            let vertices = app
                .world()
                .resource::<Assets<Mesh>>()
                .get(&mesh.0)
                .map_or(0, Mesh::count_vertices);
            eprintln!(
                "cpu_skin_fallback path={:?} own_visibility={visibility:?} vertices={vertices} reason={:?}",
                prim.path, fallback.0
            );
        }
    }
    result.subset_entities = app
        .world_mut()
        .query::<&usd_bevy::route::subset::UsdSubset>()
        .iter(app.world())
        .count();
    for (_, mesh) in app.world().resource::<Assets<Mesh>>().iter() {
        result.mesh_assets += 1;
        let vertices = mesh.count_vertices();
        result.vertices += vertices;
        result.vertex_bytes += mesh
            .attributes()
            .map(|(_, values)| values.get_bytes().len())
            .sum::<usize>();
        if let Some(indices) = mesh.indices() {
            let referenced: HashSet<_> = indices.iter().collect();
            if referenced.iter().any(|&index| index >= vertices) {
                return Err("projected mesh has out-of-range indices".into());
            }
            result.unreferenced_vertices += vertices - referenced.len();
            result.index_bytes += match indices {
                bevy::mesh::Indices::U16(values) => values.len() * size_of::<u16>(),
                bevy::mesh::Indices::U32(values) => values.len() * size_of::<u32>(),
            };
        }
        result.morph_bytes += mesh.get_morph_targets().map_or(0, size_of_val);
    }
    result.image_bytes = app
        .world()
        .resource::<Assets<Image>>()
        .iter()
        .map(|(_, image)| image.data.as_ref().map_or(0, Vec::len))
        .sum();
    if std::env::var_os("USD_PROFILE_IMAGES").is_some() {
        use std::hash::{Hash, Hasher};
        eprintln!(
            "{}",
            image_buffer_report(app.world().resource::<Assets<Image>>())
        );
        let mut groups = std::collections::HashMap::new();
        for (_, image) in app.world().resource::<Assets<Image>>().iter() {
            let Some(data) = &image.data else { continue };
            let size = image.texture_descriptor.size;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut hash);
            let key = (
                image.texture_descriptor.format,
                size.width,
                size.height,
                size.depth_or_array_layers,
                data.len(),
                hash.finish(),
            );
            let group = groups.entry(key).or_insert((0_usize, data));
            if group.1 != data {
                return Err("image profiling hash collision".into());
            }
            group.0 += 1;
        }
        let unique: usize = groups.keys().map(|key| key.4).sum();
        eprintln!(
            "image_payload_groups={} hashed_unique_bytes={unique} retained_bytes={}",
            groups.len(),
            result.image_bytes
        );
        let mut repeated: Vec<_> = groups
            .into_iter()
            .filter(|(_, (count, _))| *count > 1)
            .map(|(key, (count, _))| (key, count))
            .collect();
        repeated.sort_by_key(|(key, count)| std::cmp::Reverse(key.4 * (count - 1)));
        for (key, count) in repeated {
            eprintln!(
                "image_payload_repeat format={:?} size={}x{}x{} bytes={} copies={count} hash={}",
                key.0, key.1, key.2, key.3, key.4, key.5
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(1..=4).contains(&args.len()) {
        return Err("usage: editor_benchmark ASSET [SAMPLES] [cpu|gpu-prepared] [seek|seek-quick|seek-unique|seek-timeline|seek-timeline-paired|seek-residency-paired]".into());
    }
    let path = std::fs::canonicalize(&args[0])?;
    let samples = args
        .get(1)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(3);
    if !(1..=10).contains(&samples) {
        return Err("samples must be between 1 and 10".into());
    }
    let seek = match args.get(3).map(String::as_str) {
        None => SeekMode::Idle,
        Some("seek") => SeekMode::Repeated,
        Some("seek-quick") => SeekMode::Quick,
        Some("seek-unique") => SeekMode::Unique,
        Some("seek-timeline" | "seek-timeline-paired" | "seek-residency-paired") => {
            SeekMode::Timeline
        }
        _ => {
            return Err(
                "fourth argument must be seek, seek-quick, seek-unique, seek-timeline, seek-timeline-paired or seek-residency-paired".into(),
            );
        }
    };
    let gpu_prepared = match args.get(2).map(String::as_str) {
        None | Some("cpu") => false,
        Some("gpu-prepared") => true,
        _ => return Err("deformation must be cpu or gpu-prepared".into()),
    };
    let stepped = stepped::configured(
        std::env::var_os("USD_BENCH_STEPPED").as_deref(),
        samples,
        args.get(3).map(String::as_str) == Some("seek-timeline"),
    )?;
    println!(
        "asset={} profile={} renderer=headless deformation={} excludes=plugin-startup,ui,gpu assets=all-retained-not-visibility-filtered samples={samples}",
        path.display(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        if gpu_prepared { "gpu-prepared" } else { "cpu" }
    );
    println!(
        "payloads=retained-cpu-bytes excludes=gpu-allocation,asset-handles,allocator-overhead"
    );
    if args.get(3).is_some_and(|mode| {
        matches!(
            mode.as_str(),
            "seek-timeline-paired" | "seek-residency-paired"
        )
    }) {
        let reverse =
            residency_probe::reverse(std::env::var_os("USD_BENCH_PAIR_REVERSE").as_deref())?;
        for sample in 0..samples {
            println!("paired_sample_begin={sample}");
            if args[3] == "seek-residency-paired" {
                paired::run_residency(
                    &path,
                    gpu_prepared,
                    seek.samples(),
                    (sample % 2 != 0) ^ reverse,
                )?;
            } else {
                paired::run(
                    &path,
                    gpu_prepared,
                    seek.samples(),
                    (sample % 2 != 0) ^ reverse,
                )?;
            }
        }
        return Ok(());
    }
    if std::env::var_os("USD_BENCH_PREWARM_HIDDEN").is_some() {
        println!(
            "prewarm=all-deferred-after-idle payload_phase_override=after-prewarm-or-seeks idle_phase=before-prewarm"
        );
    }
    println!(
        "seek={seek:?} seek_clocks={} seek_warmup={} seek_samples={} seek_percentiles=nearest-rank payload_phase={}",
        if seek == SeekMode::Timeline {
            "1000-distinct-times-in-authored-range"
        } else if seek == SeekMode::Unique {
            "1000-distinct-times-in-(0,10]"
        } else {
            "0,5,10,5"
        },
        if seek == SeekMode::Idle {
            0
        } else {
            SEEK_WARMUP
        },
        seek.samples(),
        if seek != SeekMode::Idle {
            "after-seeks"
        } else {
            "after-idle"
        }
    );
    println!(
        "rss=whole-process-linux-VmRSS-or-NA rss_phase=after-idle,after-seeks rss_peak=VmHWM-process-lifetime-including-prior-samples-or-NA peak_assets=sampled-after-updates-not-allocation-counts"
    );
    println!(
        "sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes,seek_median_us,seek_p95_us,seek_max_us,gpu_morph_entities,peak_mesh_assets,peak_material_assets,peak_image_assets,rss_before_seeks,rss_after_seeks,cached_meshes,cached_mesh_payload_bytes,cached_tangent_payload_bytes,cached_assembly_payload_bytes,rss_process_peak"
    );
    for sample in 0..samples {
        if std::env::var_os("USD_BENCH_RAW_SAMPLES").is_some() {
            println!("sample_begin={sample}");
        }
        let m = if stepped {
            measure_with_steps(
                &path,
                gpu_prepared,
                seek,
                Some(&mut std::io::stdin().lock()),
            )?
        } else {
            measure(&path, gpu_prepared, seek)?
        };
        println!(
            "{sample},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{}",
            m.open.as_secs_f64() * 1000.0,
            m.idle.as_secs_f64() * 1_000_000.0,
            m.mesh_entities,
            m.subset_entities,
            m.mesh_assets,
            m.vertices,
            m.unreferenced_vertices,
            m.vertex_bytes,
            m.index_bytes,
            m.morph_bytes,
            m.image_bytes,
            m.seek_median.as_secs_f64() * 1_000_000.0,
            m.seek_p95.as_secs_f64() * 1_000_000.0,
            m.seek_max.as_secs_f64() * 1_000_000.0,
            m.gpu_morph_entities,
            m.peak_asset_counts[0],
            m.peak_asset_counts[1],
            m.peak_asset_counts[2],
            m.rss_before_seeks.map_or("NA".into(), |v| v.to_string()),
            m.rss_after_seeks.map_or("NA".into(), |v| v.to_string()),
            m.cached_meshes,
            m.cached_mesh_payload_bytes,
            m.cached_tangent_payload_bytes,
            m.cached_assembly_payload_bytes,
            m.rss_process_peak.map_or("NA".into(), |v| v.to_string())
        );
    }
    Ok(())
}

#[test]
fn raw_seek_phases_include_warmup_without_reindexing_samples() {
    for iteration in 0..SEEK_WARMUP {
        assert_eq!(seek_sample_phase(iteration), ("seek_warmup", iteration));
    }
    for iteration in 0..SeekMode::Repeated.samples() {
        assert_eq!(
            seek_sample_phase(iteration + SEEK_WARMUP),
            ("seek", iteration)
        );
    }
}

#[test]
fn quick_seek_uses_eight_samples_after_warmup() {
    assert_eq!(SeekMode::Quick.samples(), 8);
    for index in 0..12 {
        assert_eq!(
            SeekMode::Quick.clock(index),
            SeekMode::Repeated.clock(index)
        );
    }
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
    let clocks = (4..1004)
        .map(|i| SeekMode::Unique.clock(i))
        .collect::<Vec<_>>();
    assert_eq!(clocks.first(), Some(&0.01));
    assert_eq!(clocks.last(), Some(&10.0));
    assert!(clocks.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        clocks
            .iter()
            .map(|v| v.to_bits())
            .collect::<HashSet<_>>()
            .len(),
        1000
    );
    assert_eq!(parse_rss("Name: probe\nVmRSS:\t123 kB\n"), Some(123 * 1024));
    assert_eq!(parse_rss("VmRSS: 123 MB"), None);
    assert_eq!(parse_rss("VmRSS: missing kB"), None);
    assert_eq!(parse_rss("Name: probe"), None);
    assert_eq!(parse_rss(&format!("VmRSS: {} kB", usize::MAX)), None);
    assert_eq!(parse_rss("VmRSS:extra 123 kB"), None);
    assert_eq!(
        parse_status_bytes("VmRSS: 123 kB\nVmHWM: 456 kB", "VmHWM:"),
        Some(456 * 1024)
    );
    assert_eq!(parse_status_bytes("VmHWM: 123 MB", "VmHWM:"), None);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/morph_tangent_normals.usda");
    for gpu_prepared in [false, true] {
        let result = measure(&path, gpu_prepared, SeekMode::Unique).unwrap();
        assert!(result.seek_median > Duration::ZERO);
        assert!(result.seek_median <= result.seek_p95 && result.seek_p95 <= result.seek_max);
        assert!(result.peak_asset_counts[0] >= result.mesh_assets);
        assert!(result.peak_asset_counts[0] <= 16, "{result:?}");
        assert!(result.cached_meshes <= 4, "{result:?}");
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
fn image_buffer_accounting_separates_capacity_from_payload() {
    use bevy::{asset::RenderAssetUsages, render::render_resource::*};
    let mut images = Assets::<Image>::default();
    let empty = image_buffer_report(&images);
    assert!(
        empty.contains(
            "image_assets=0 data_buffers=0 payload_bytes=0 capacity_bytes=0 spare_bytes=0"
        )
    );
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![51; 4],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.data.as_mut().unwrap().reserve_exact(60);
    let mut no_data = image.clone();
    no_data.data = None;
    let mut reserved = image.clone();
    reserved.data = Some(Vec::with_capacity(32));
    let capacity =
        image.data.as_ref().unwrap().capacity() + reserved.data.as_ref().unwrap().capacity();
    let handles = [images.add(image), images.add(no_data), images.add(reserved)];
    let report = image_buffer_report(&images);
    assert!(report.contains(&format!(
        "image_assets=3 data_buffers=2 payload_bytes=4 capacity_bytes={capacity} spare_bytes={}",
        capacity - 4
    )));
    assert!(report.contains("scope=main-world-Assets-Image-data"));
    assert!(report.contains("excludes=source,mesh,handles,allocator-overhead,scratch,gpu"));
    assert!(handles.iter().all(|handle| images.contains(handle.id())));
    assert_eq!(
        images
            .get(&handles[0])
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn editor_benchmark_counts_subset_payload_and_rejects_failed_opens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let m = measure(
        &root.join("assets/material_subsets.usda"),
        false,
        SeekMode::Idle,
    )
    .unwrap();
    assert_eq!(
        (m.mesh_entities, m.subset_entities, m.mesh_assets),
        (3, 2, 3)
    );
    assert_eq!((m.vertices, m.unreferenced_vertices), (8, 0));
    assert_eq!(
        (m.vertex_bytes, m.index_bytes, m.morph_bytes, m.image_bytes),
        (256, 48, 0, 0)
    );
    assert!(
        measure(
            &root.join("assets/missing-editor-benchmark.usda"),
            false,
            SeekMode::Idle
        )
        .is_err()
    );
}

#[test]
fn morph_payload_measurement_counts_inline_attributes() {
    use bevy::mesh::morph::MorphAttributes;
    let mut mesh = Mesh::from(Cuboid::default());
    assert_eq!(mesh.get_morph_targets().map_or(0, size_of_val), 0);
    mesh.set_morph_targets(vec![MorphAttributes::default(); 24]);
    assert_eq!(
        mesh.get_morph_targets().map_or(0, size_of_val),
        24 * size_of::<MorphAttributes>()
    );
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
