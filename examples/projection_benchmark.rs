use std::time::{Duration, Instant};
use bevy::prelude::*;
use usd_bevy::live::{LiveStage, PrimEntities, apply_changes, project_stage};
use usd_bevy::route::{SchemaRegistry, cache::{ProjectionCache, MaterialCache}};

#[derive(Debug)]
struct Measurement {
    open: Duration,
    project: Duration,
    edit: Duration,
    idle: Duration,
    meshes: usize,
    materials: usize,
    cached_payload_bytes_after_edit: usize,
}

fn measure(count: usize, cached: bool) -> Measurement {
    let mut text = String::from("#usda 1.0\ndef Xform \"Template\" { def Cube \"Shape\" {} }\n");
    for i in 0..count {
        text.push_str(&format!("def Xform \"Instance{i}\" (instanceable = true\n prepend references = </Template>) {{}}\n"));
    }
    let source = usd_bevy::UsdSource::new("benchmark.usda", text.into_bytes()).unwrap();
    let start = Instant::now();
    let stage = source.open_stage().unwrap();
    let open = start.elapsed();
    let live = LiveStage::new(stage);
    let mut world = World::new();
    world.insert_resource(Assets::<Mesh>::default());
    world.insert_resource(Assets::<StandardMaterial>::default());
    world.insert_resource(SchemaRegistry::builtin());
    if std::env::var_os("USD_PROFILE_ROUTES").is_some() {
        world.init_resource::<usd_bevy::route::ProjectionTimings>();
    }
    if cached {
        world.insert_resource(ProjectionCache::default());
        world.insert_resource(MaterialCache::default());
    }
    let mut map = PrimEntities::default();
    let start = Instant::now();
    project_stage(&mut world, &live, &mut map);
    let project = start.elapsed();
    let meshes = world.resource::<Assets<Mesh>>().len();
    let materials = world.resource::<Assets<StandardMaterial>>().len();
    assert_eq!(meshes, if cached { 1 } else { count + 1 });
    let entities: Vec<_> = (0..count).map(|i| map.entity(&format!("/Instance{i}/Shape")).unwrap()).collect();
    let start = Instant::now();
    usd_bevy::authoring::set_attribute(&live.stage, "/Template/Shape", "size", "double", openusd::sdf::Value::Double(4.0)).unwrap();
    apply_changes(&mut world, &live, &mut map);
    let edit = start.elapsed();
    for (i, entity) in entities.into_iter().enumerate() {
        assert_eq!(map.entity(&format!("/Instance{i}/Shape")), Some(entity));
        let mesh = &world.get::<Mesh3d>(entity).unwrap().0;
        let mesh = world.resource::<Assets<Mesh>>().get(mesh).unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("missing cube positions") };
        let extent = positions.iter().flat_map(|position| position.iter()).map(|value| value.abs()).fold(0.0_f32, f32::max);
        assert!((extent - 2.0).abs() < 0.0001, "prototype size edit did not reach proxy geometry");
    }
    let start = Instant::now();
    for _ in 0..1000 { apply_changes(&mut world, &live, &mut map); }
    let idle = start.elapsed() / 1000;
    if let Some(timings) = world.get_resource::<usd_bevy::route::ProjectionTimings>() {
        let mut rows: Vec<_> = timings.0.iter().collect();
        rows.sort_by_key(|(_, row)| std::cmp::Reverse(row.matching + row.application));
        eprintln!("route_profile cached={cached} includes=project+edit");
        for (name, row) in rows {
            eprintln!("{name}: attempts={} matches={} match_ms={:.3} apply_ms={:.3}", row.attempts, row.matches,
                row.matching.as_secs_f64() * 1000.0, row.application.as_secs_f64() * 1000.0);
        }
    }
    let cached_payload_bytes_after_edit = world.get_resource::<ProjectionCache>().map_or(0, ProjectionCache::retained_payload_bytes);
    Measurement { open, project, edit, idle, meshes, materials, cached_payload_bytes_after_edit }
}

fn main() {
    let count = std::env::args().nth(1).map(|arg| arg.parse::<usize>().expect("instance count must be an integer")).unwrap_or(128);
    assert!((1..=4096).contains(&count), "instance count must be between 1 and 4096");
    println!("native_instances={count} profile={} renderer=headless samples=3", if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("cached,sample,open_ms,project_ms,edit_ms,idle_us,initial_meshes,initial_materials,cached_payload_bytes_after_edit");
    for sample in 0..3 {
        for cached in if sample % 2 == 0 { [false, true] } else { [true, false] } {
            let m = measure(count, cached);
            println!("{cached},{sample},{:.3},{:.3},{:.3},{:.3},{},{},{}", m.open.as_secs_f64() * 1000.0,
                m.project.as_secs_f64() * 1000.0, m.edit.as_secs_f64() * 1000.0, m.idle.as_secs_f64() * 1_000_000.0, m.meshes, m.materials, m.cached_payload_bytes_after_edit);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn benchmark_checks_cached_and_uncached_projection() {
        let uncached = super::measure(4, false);
        let cached = super::measure(4, true);
        assert_eq!(uncached.meshes, 5);
        assert_eq!(cached.meshes, 1);
        assert_eq!(uncached.materials, 5);
        assert_eq!(cached.materials, 1);
        assert_eq!(uncached.cached_payload_bytes_after_edit, 0);
        assert!(cached.cached_payload_bytes_after_edit > 0);
    }
}
