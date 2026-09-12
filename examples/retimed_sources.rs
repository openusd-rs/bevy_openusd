use bevy::prelude::*;
use openusd::sdf::{LayerOffset, Path};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::{UsdInstances, UsdInstanceTime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified retimed reference geometry and stable entities across clock changes.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let root = UsdSource::snapshot("retimed/root.usda", b"#usda 1.0\n".as_slice())?;
    let model = UsdSource::snapshot("retimed/model.usda", br#"#usda 1.0
(defaultPrim = "Model")
def Sphere "Model" {
    double radius.timeSamples = {0: 1, 10: 3}
}
"#.as_slice())?;
    let source = root.with_offset_references([
        ("/Original", &model, Path::default(), LayerOffset::IDENTITY),
        ("/Retimed", &model, Path::default(), LayerOffset::new(10.0, 2.0)),
    ])?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let root = app.world_mut().spawn((UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
    app.update();
    let entities = ["/Original", "/Retimed"].map(|path|
        app.world().non_send::<UsdInstances>().entity(root, path).unwrap());
    app.world_mut().entity_mut(entities[1]).insert(Name::new("runtime name"));
    for (time, expected) in [(10.0, [3.0, 1.0]), (20.0, [3.0, 2.0]), (30.0, [3.0, 3.0])] {
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time;
        app.update();
        assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
        for ((path, entity), radius) in ["/Original", "/Retimed"].into_iter().zip(entities).zip(expected) {
            assert_eq!(app.world().non_send::<UsdInstances>().entity(root, path), Some(entity));
            let mesh = app.world().resource::<Assets<Mesh>>()
                .get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let extent = positions.iter().map(|position| Vec3::from_array(*position).length())
                .fold(0.0_f32, f32::max);
            assert!((extent - radius).abs() < 0.0001, "{path} at {time}: {extent} != {radius}");
        }
        assert_eq!(app.world().get::<Name>(entities[1]).unwrap().as_str(), "runtime name");
    }
    Ok(())
}

#[test]
fn retimed_sources_update_projected_geometry() { verify().unwrap(); }
