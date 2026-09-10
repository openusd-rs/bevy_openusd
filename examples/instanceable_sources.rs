use bevy::prelude::*;
use openusd::sdf::{LayerOffset, Path};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};
use usd_bevy::instance::UsdInstances;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified native prototype assembly, shared Bevy meshes and root cleanup.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let root = UsdSource::snapshot("instanced/root.usda", b"#usda 1.0\n".as_slice())?;
    let model = UsdSource::snapshot("instanced/model.usda", br#"#usda 1.0
(defaultPrim = "Model")
def Xform "Model" {
    def Cube "Geometry" { double size = 2 }
}
"#.as_slice())?;
    let source = root.with_instanceable_references([
        ("/First", &model, Path::default(), LayerOffset::IDENTITY),
        ("/Second", &model, Path::default(), LayerOffset::IDENTITY),
    ])?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
    app.update();
    assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
    let instances = app.world().non_send::<UsdInstances>();
    let entities = ["/First/Geometry", "/Second/Geometry"].map(|path| instances.entity(root, path).unwrap());
    assert_ne!(entities[0], entities[1]);
    assert_eq!(app.world().get::<Mesh3d>(entities[0]).unwrap().0,
        app.world().get::<Mesh3d>(entities[1]).unwrap().0);
    let stage = instances.stage(root).unwrap();
    assert!(stage.prim("/First")?.is_instance()?);
    assert_eq!(stage.prim("/First")?.prototype()?, stage.prim("/Second")?.prototype()?);
    app.world_mut().despawn(root);
    app.update();
    assert!(entities.iter().all(|entity| app.world().get_entity(*entity).is_err()));
    assert!(app.world().non_send::<UsdInstances>().is_empty());
    Ok(())
}

#[test]
fn instanceable_sources_project_shared_meshes() { verify().unwrap(); }
