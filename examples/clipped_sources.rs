use bevy::prelude::*;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::{UsdInstances, UsdInstanceTime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified captured value clips with independent clocks and stable entities.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let source = UsdSource::snapshot("clipped/root.usda", br#"#usda 1.0
def Sphere "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@clip.usda@]
            double2[] active = [(0, 0)]
            double2[] times = [(0, 0), (20, 10)]
            string primPath = "/Model"
        }
    }
) {
    double radius
}
"#.as_slice())?;
    let clip = UsdSource::snapshot("clipped/clip.usda", br#"#usda 1.0
def Sphere "Model" {
    double radius.timeSamples = {0: 1, 10: 3}
}
"#.as_slice())?;
    let source = source.with_dependency(&clip)?;
    let stage = source.open_stage()?;
    let attribute = stage.attribute("/Model.radius")?;
    assert!(attribute.time_samples()?.is_none());
    assert_eq!(attribute.time_sample_times()?, [0.0, 20.0]);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let roots = [0.0, 20.0].map(|current|
        app.world_mut().spawn((UsdSceneRoot(handle.clone()), UsdInstanceTime { current })).id());
    app.update();
    let entities = roots.map(|root| app.world().non_send::<UsdInstances>().entity(root, "/Model").unwrap());
    for (index, entity) in entities.into_iter().enumerate() {
        app.world_mut().entity_mut(entity).insert(Name::new(format!("runtime {index}")));
    }
    for (times, expected) in [([0.0, 20.0], [1.0, 3.0]), ([10.0, 0.0], [2.0, 1.0]), ([20.0, 10.0], [3.0, 2.0]), ([0.0, 20.0], [1.0, 3.0])] {
        for (root, time) in roots.into_iter().zip(times) {
            app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time;
        }
        app.update();
        for index in 0..2 {
            let root = roots[index];
            let entity = entities[index];
            assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
            assert_eq!(app.world().non_send::<UsdInstances>().entity(root, "/Model"), Some(entity));
            assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), format!("runtime {index}"));
            let mesh = app.world().resource::<Assets<Mesh>>()
                .get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let radius = positions.iter().map(|position| Vec3::from_array(*position).length()).fold(0.0_f32, f32::max);
            assert!((radius - expected[index]).abs() < 0.0001, "root {index} at {}: {radius} != {}", times[index], expected[index]);
        }
    }
    for root in roots { app.world_mut().despawn(root); }
    app.update();
    for entity in entities { assert!(app.world().get_entity(entity).is_err()); }
    Ok(())
}

#[test]
fn captured_clips_update_independent_projected_geometry() { verify().unwrap(); }
