use std::time::{Duration, Instant};
use bevy::prelude::*;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::UsdInstances;

#[derive(Component, PartialEq, Debug)]
struct RuntimeTag(usize);

struct Measurement {
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

fn measure(count: usize, root_count: usize) -> Measurement {
    let mut text = String::from("#usda 1.0\n");
    for i in 0..count {
        text.push_str(&format!("def Xform \"Instance{i}\" (instanceable = true\n prepend references = @model.usda@</Template>) {{}}\n"));
    }
    let root_source = UsdSource::snapshot("source-benchmark/root.usda", text.into_bytes()).unwrap();
    let source = root_source.with_dependency(&model(2.0)).unwrap();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    app.update();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let roots: Vec<_> = (0..root_count).map(|_| app.world_mut().spawn(UsdSceneRoot(handle.clone())).id()).collect();
    let start = Instant::now();
    app.update();
    let load = start.elapsed();
    let initial_meshes = app.world().resource::<Assets<Mesh>>().len();
    let initial_materials = app.world().resource::<Assets<StandardMaterial>>().len();
    assert_eq!((initial_meshes, initial_materials), (1, 1));
    let mut entities = Vec::new();
    let instances = app.world().non_send::<UsdInstances>();
    assert_eq!(instances.len(), root_count);
    for root in &roots {
        assert_eq!(app.world().get::<UsdSceneState>(*root), Some(&UsdSceneState::Ready));
        for i in 0..count {
            let path = format!("/Instance{i}/Shape");
            entities.push((*root, path.clone(), instances.entity(*root, &path).unwrap()));
        }
    }
    assert_eq!(entities.iter().map(|(_, _, entity)| *entity).collect::<std::collections::HashSet<_>>().len(), count * root_count);
    for (i, (_, _, entity)) in entities.iter().enumerate() { app.world_mut().entity_mut(*entity).insert(RuntimeTag(i)); }
    let replacement = root_source.with_dependency(&model(4.0)).unwrap();
    let start = Instant::now();
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = replacement;
    app.update();
    let reload = start.elapsed();
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
    Measurement { load, reload, idle, initial_meshes, initial_materials, projected_meshes: entities.len() }
}

fn main() {
    let count = std::env::args().nth(1).map(|value| value.parse::<usize>().expect("instance count must be an integer")).unwrap_or(128);
    assert!((1..=1024).contains(&count), "instance count must be between 1 and 1024");
    println!("instances_per_root={count} profile={} renderer=headless source=captured excludes=disk-io,plugin-startup,gpu samples=3",
        if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("roots,sample,load_ms,reload_ms,idle_us,initial_meshes,initial_materials,projected_meshes");
    for sample in 0..3 {
        for roots in if sample % 2 == 0 { [1, 4] } else { [4, 1] } {
            let m = measure(count, roots);
            println!("{roots},{sample},{:.3},{:.3},{:.3},{},{},{}", m.load.as_secs_f64()*1000.0,
                m.reload.as_secs_f64()*1000.0, m.idle.as_secs_f64()*1_000_000.0,
                m.initial_meshes, m.initial_materials, m.projected_meshes);
        }
    }
}

#[test]
fn source_benchmark_checks_real_roots_reload_and_sharing() {
    for roots in [1, 4] {
        let measurement = measure(2, roots);
        assert_eq!(measurement.projected_meshes, 2 * roots);
        assert_eq!((measurement.initial_meshes, measurement.initial_materials), (1, 1));
    }
}
