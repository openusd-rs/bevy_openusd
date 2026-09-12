use bevy::prelude::*;
use openusd::sdf::{LayerOffset, Path};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::{UsdInstances, UsdInstanceTime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified native prototypes, retimed mesh sharing, stable entities and root cleanup.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let root = UsdSource::snapshot("instanced/root.usda", b"#usda 1.0\n".as_slice())?;
    let model = UsdSource::snapshot("instanced/model.usda", br#"#usda 1.0
(defaultPrim = "Model")
def Xform "Model" {
    def Cube "Geometry" { double size.timeSamples = {0: 1, 10: 3} }
}
"#.as_slice())?;
    let source = root.with_instanceable_references([
        ("/First", &model, Path::default(), LayerOffset::IDENTITY),
        ("/Second", &model, Path::default(), LayerOffset::IDENTITY),
        ("/Retimed", &model, Path::default(), LayerOffset::new(10.0, 2.0)),
    ])?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let root = app.world_mut().spawn((UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
    app.update();
    assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
    let instances = app.world().non_send::<UsdInstances>();
    let paths = ["/First/Geometry", "/Second/Geometry", "/Retimed/Geometry"];
    let entities = paths.map(|path| instances.entity(root, path).unwrap());
    assert_ne!(entities[0], entities[1]);
    assert_eq!(app.world().get::<Mesh3d>(entities[0]).unwrap().0,
        app.world().get::<Mesh3d>(entities[1]).unwrap().0);
    let stage = instances.stage(root).unwrap();
    assert!(stage.prim("/First")?.is_instance()?);
    assert_eq!(stage.prim("/First")?.prototype()?, stage.prim("/Second")?.prototype()?);
    assert!(stage.prim("/Retimed")?.is_instance()?);
    assert_ne!(stage.prim("/First")?.prototype()?, stage.prim("/Retimed")?.prototype()?);
    app.world_mut().entity_mut(entities[2]).insert(Name::new("retimed runtime name"));
    for (time, sizes) in [(10.0, [3.0, 3.0, 1.0]), (20.0, [3.0, 3.0, 2.0]),
        (30.0, [3.0, 3.0, 3.0]), (10.0, [3.0, 3.0, 1.0])] {
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time;
        app.update();
        assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
        let meshes = entities.map(|entity| app.world().get::<Mesh3d>(entity).unwrap().0.clone());
        assert_eq!(meshes[0], meshes[1]);
        if sizes[0] != sizes[2] { assert_ne!(meshes[0], meshes[2]); }
        for ((path, entity), (mesh, size)) in paths.into_iter().zip(entities).zip(meshes.iter().zip(sizes)) {
            assert_eq!(app.world().non_send::<UsdInstances>().entity(root, path), Some(entity));
            let mesh = app.world().resource::<Assets<Mesh>>().get(mesh).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let extent = positions.iter().map(|position| position[0].abs()).fold(0.0_f32, f32::max);
            assert!((extent * 2.0 - size).abs() < 0.0001, "{path} at {time}: {extent}");
        }
        assert_eq!(app.world().get::<Name>(entities[2]).unwrap().as_str(), "retimed runtime name");
    }
    app.world_mut().despawn(root);
    app.update();
    assert!(entities.iter().all(|entity| app.world().get_entity(*entity).is_err()));
    assert!(app.world().non_send::<UsdInstances>().is_empty());
    Ok(())
}

#[test]
fn instanceable_sources_project_shared_meshes() { verify().unwrap(); }
