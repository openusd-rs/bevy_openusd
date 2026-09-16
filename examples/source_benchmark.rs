use std::time::{Duration, Instant};
use bevy::prelude::*;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::UsdInstances;

#[derive(Component, PartialEq, Debug)]
struct RuntimeTag(usize);

struct Measurement {
    assembly: Duration,
    replacement_assembly: Duration,
    load: Duration,
    reload: Duration,
    idle: Duration,
    initial_meshes: usize,
    initial_materials: usize,
    projected_meshes: usize,
}

fn model(size: f64) -> UsdSource {
    UsdSource::snapshot("source-benchmark/model.usda", format!(
        "#usda 1.0\ndef Xform \"Template\" {{\n def Cube \"Shape\" {{\n double size = {size}\n }}\n}}\n"
    ).into_bytes()).unwrap()
}

fn profile(app: &mut App, phase: &str, roots: usize) {
    if let Some(mut timings) = app.world_mut().get_resource_mut::<usd_bevy::asset::UsdSceneTimings>() {
        assert_eq!((timings.attempts, timings.failures), (roots, 0));
        eprintln!("source_profile phase={phase} roots={roots} validation_reuses={} open_ms={:.3} overrides_ms={:.3} validation_ms={:.3} projection_ms={:.3}",
            timings.validation_reuses,
            timings.open.as_secs_f64()*1000.0, timings.overrides.as_secs_f64()*1000.0,
            timings.validation.as_secs_f64()*1000.0, timings.projection.as_secs_f64()*1000.0);
        *timings = default();
    }
    if let Some(mut timings) = app.world_mut().get_resource_mut::<usd_bevy::route::ProjectionTimings>() {
        let mut rows: Vec<_> = timings.0.iter().collect();
        rows.sort_by_key(|(_, row)| std::cmp::Reverse(row.matching + row.application));
        for (name, row) in rows {
            eprintln!("route_profile phase={phase} roots={roots} route={name} attempts={} matches={} match_ms={:.3} apply_ms={:.3}",
                row.attempts, row.matches, row.matching.as_secs_f64()*1000.0, row.application.as_secs_f64()*1000.0);
        }
        timings.0.clear();
    }
}

fn measure(count: usize, root_count: usize, profiled: bool, routes: bool) -> Measurement {
    let root_source = UsdSource::snapshot("source-benchmark/root.usda", b"#usda 1.0\n".as_slice()).unwrap();
    let paths: Vec<_> = (0..count).map(|i| format!("/Instance{i}")).collect();
    let assemble = |model: &UsdSource| root_source.with_instanceable_references(paths.iter().map(|path|
        (path.as_str(), model, "/Template", openusd::sdf::LayerOffset::IDENTITY))).unwrap();
    let initial_model = model(2.0);
    let start = Instant::now();
    let source = assemble(&initial_model);
    let assembly = start.elapsed();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    if profiled { app.init_resource::<usd_bevy::asset::UsdSceneTimings>(); }
    if routes { app.init_resource::<usd_bevy::route::ProjectionTimings>(); }
    app.update();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let roots: Vec<_> = (0..root_count).map(|_| app.world_mut().spawn(UsdSceneRoot(handle.clone())).id()).collect();
    let start = Instant::now();
    app.update();
    let load = start.elapsed();
    profile(&mut app, "load", root_count);
    let initial_meshes = app.world().resource::<Assets<Mesh>>().len();
    let initial_materials = app.world().resource::<Assets<StandardMaterial>>().len();
    assert_eq!((initial_meshes, initial_materials), (1, 1));
    let mut entities = Vec::new();
    let instances = app.world().non_send::<UsdInstances>();
    assert_eq!(instances.len(), root_count);
    for root in &roots {
        assert_eq!(app.world().get::<UsdSceneState>(*root), Some(&UsdSceneState::Ready));
        let stage = instances.stage(*root).unwrap();
        let prototype = stage.prim("/Instance0").unwrap().prototype().unwrap();
        assert!(prototype.is_some());
        for i in 0..count {
            let prim = stage.prim(format!("/Instance{i}")).unwrap();
            assert!(prim.is_instance().unwrap());
            assert_eq!(prim.prototype().unwrap(), prototype);
            let path = format!("/Instance{i}/Shape");
            entities.push((*root, path.clone(), instances.entity(*root, &path).unwrap()));
        }
    }
    assert_eq!(entities.iter().map(|(_, _, entity)| *entity).collect::<std::collections::HashSet<_>>().len(), count * root_count);
    for (i, (_, _, entity)) in entities.iter().enumerate() { app.world_mut().entity_mut(*entity).insert(RuntimeTag(i)); }
    let replacement_model = model(4.0);
    let start = Instant::now();
    let replacement = assemble(&replacement_model);
    let replacement_assembly = start.elapsed();
    let start = Instant::now();
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = replacement;
    app.update();
    let reload = start.elapsed();
    profile(&mut app, "reload", root_count);
    let shared = app.world().get::<Mesh3d>(entities[0].2).unwrap().0.clone();
    let instances = app.world().non_send::<UsdInstances>();
    for (i, (root, path, entity)) in entities.iter().enumerate() {
        assert_eq!(app.world().get::<UsdSceneState>(*root), Some(&UsdSceneState::Ready));
        assert_eq!(instances.entity(*root, path), Some(*entity));
        assert_eq!(app.world().get::<RuntimeTag>(*entity), Some(&RuntimeTag(i)));
        assert_eq!(app.world().get::<Mesh3d>(*entity).unwrap().0, shared);
    }
    let mesh = app.world().resource::<Assets<Mesh>>().get(&shared).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("cube positions") };
    let extent = positions.iter().flatten().map(|value| value.abs()).fold(0.0_f32, f32::max);
    assert!((extent - 2.0).abs() < 0.0001);
    let start = Instant::now();
    for _ in 0..100 { app.update(); }
    let idle = start.elapsed() / 100;
    if profiled {
        assert_eq!(app.world().resource::<usd_bevy::asset::UsdSceneTimings>().attempts, 0);
    } else {
        assert!(!app.world().contains_resource::<usd_bevy::asset::UsdSceneTimings>());
    }
    if routes { assert!(app.world().resource::<usd_bevy::route::ProjectionTimings>().0.is_empty()); }
    assert_eq!(root_source.dependencies().count(), 0);
    Measurement { assembly, replacement_assembly, load, reload, idle, initial_meshes, initial_materials, projected_meshes: entities.len() }
}

fn main() {
    let count = std::env::args().nth(1).map(|value| value.parse::<usize>().expect("instance count must be an integer")).unwrap_or(128);
    assert!((1..=1024).contains(&count), "instance count must be between 1 and 1024");
    let profiled = std::env::var_os("USD_PROFILE_SOURCES").is_some();
    let routes = std::env::var_os("USD_PROFILE_ROUTES").is_some();
    println!("instances_per_root={count} profile={} phase_timing={profiled} route_timing={routes} renderer=headless source=captured assembly=typed-instanceable-batch excludes=input-generation,disk-io,plugin-startup,gpu samples=3",
        if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("roots,sample,assembly_ms,replacement_assembly_ms,load_ms,reload_ms,idle_us,initial_meshes,initial_materials,projected_meshes");
    for sample in 0..3 {
        for roots in if sample % 2 == 0 { [1, 4] } else { [4, 1] } {
            let m = measure(count, roots, profiled, routes);
            println!("{roots},{sample},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{}", m.assembly.as_secs_f64()*1000.0,
                m.replacement_assembly.as_secs_f64()*1000.0, m.load.as_secs_f64()*1000.0,
                m.reload.as_secs_f64()*1000.0, m.idle.as_secs_f64()*1_000_000.0,
                m.initial_meshes, m.initial_materials, m.projected_meshes);
        }
    }
}

#[test]
fn source_benchmark_checks_real_roots_reload_and_sharing() {
    for roots in [1, 4] {
        for profiled in [false, true] {
            let measurement = measure(2, roots, profiled, profiled);
            assert_eq!(measurement.projected_meshes, 2 * roots);
            assert_eq!((measurement.initial_meshes, measurement.initial_materials), (1, 1));
        }
    }
}
