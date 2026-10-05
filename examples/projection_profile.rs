use bevy::prelude::*;
use std::time::{Duration, Instant};
use usd_bevy::live::ProjectionJob;
use usd_bevy::route::{
    ProjectionTimings, SchemaRegistry,
    cache::{MaterialCache, ProjectionCache},
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Projects the first prims of a stage headlessly and reports the rate and
/// the costliest routes. `UNLOADED=1` opens it with no payloads loaded.
/// Usage: projection_profile ASSET [PRIMS]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let limit: usize = args.get(1).map_or(Ok(200_000), |n| n.parse())?;
    let started = Instant::now();
    let source = usd_bevy::UsdSource::from_file(std::fs::canonicalize(&args[0])?)?;
    let stage = if std::env::var_os("UNLOADED").is_some() {
        source.open_stage_unloaded()?
    } else {
        source.open_stage()?
    };
    eprintln!("open_ms={:.0}", started.elapsed().as_secs_f64() * 1000.0);

    let mut world = World::new();
    world.insert_resource(Assets::<Mesh>::default());
    world.insert_resource(Assets::<StandardMaterial>::default());
    world.insert_resource(SchemaRegistry::builtin());
    world.insert_resource(ProjectionCache::default());
    world.insert_resource(MaterialCache::default());
    world.init_resource::<ProjectionTimings>();
    world.init_resource::<usd_bevy::route::gpu_instancing::UsdGpuInstancing>();
    world.init_resource::<usd_bevy::route::gpu_skin::GpuSkinningEnabled>();
    world.init_resource::<Assets<usd_bevy::route::flat_material::FlatMaterial>>();
    world.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
    let parent = world.spawn(Transform::default()).id();

    let started = Instant::now();
    let (mut job, mut map) = ProjectionJob::begin(&mut world, &stage, parent);
    let mut last = Instant::now();
    loop {
        let finished = job.step(&mut world, &stage, &mut map, Duration::from_millis(100));
        let (done, _) = job.progress();
        if last.elapsed() >= Duration::from_secs(5) {
            last = Instant::now();
            eprintln!(
                "prims={done} rate={:.0}/s",
                done as f64 / started.elapsed().as_secs_f64()
            );
        }
        if finished || done >= limit {
            break;
        }
    }
    let total = started.elapsed();
    let (done, _) = job.progress();
    eprintln!(
        "projected={done} ms={:.0} rate={:.0}/s entities={} meshes={} materials={}",
        total.as_secs_f64() * 1000.0,
        done as f64 / total.as_secs_f64(),
        world.entities().len(),
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<StandardMaterial>>().len(),
    );
    let (mut vertex_bytes, mut index_bytes, mut layouts) = (0usize, 0usize, Vec::new());
    for (_, mesh) in world.resource::<Assets<Mesh>>().iter() {
        let vertices = mesh.count_vertices() * mesh.get_vertex_size() as usize;
        vertex_bytes += vertices;
        index_bytes += mesh.indices().map_or(0, |indices| match indices {
            bevy::mesh::Indices::U16(values) => values.len() * 2,
            bevy::mesh::Indices::U32(values) => values.len() * 4,
        });
        let names: Vec<_> = mesh
            .attributes()
            .map(|(attribute, _)| attribute.name)
            .collect();
        match layouts.iter_mut().find(|(known, _, _)| *known == names) {
            Some((_, count, bytes)) => {
                *count += 1;
                *bytes += vertices;
            }
            None => layouts.push((names, 1, vertices)),
        }
    }
    eprintln!(
        "mesh_bytes vertices={:.0}MB indices={:.0}MB",
        vertex_bytes as f64 / 1e6,
        index_bytes as f64 / 1e6
    );
    for (names, count, bytes) in layouts {
        eprintln!(
            "mesh_layout meshes={count} vertex_mb={:.0} {names:?}",
            bytes as f64 / 1e6
        );
    }
    let timings = world.resource::<ProjectionTimings>();
    let mut rows: Vec<_> = timings.0.iter().collect();
    rows.sort_by_key(|(_, row)| std::cmp::Reverse(row.matching + row.application));
    let mut matching = Duration::ZERO;
    let mut application = Duration::ZERO;
    for (_, row) in &rows {
        matching += row.matching;
        application += row.application;
    }
    eprintln!(
        "routes match_ms={:.0} apply_ms={:.0}",
        matching.as_secs_f64() * 1000.0,
        application.as_secs_f64() * 1000.0
    );
    for (name, row) in rows.into_iter().take(15) {
        eprintln!(
            "route={name} attempts={} matches={} match_ms={:.0} apply_ms={:.0}",
            row.attempts,
            row.matches,
            row.matching.as_secs_f64() * 1000.0,
            row.application.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
