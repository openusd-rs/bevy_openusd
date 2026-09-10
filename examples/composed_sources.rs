use bevy::prelude::*;
use openusd_schemas::geom::{Sphere, SphereSchema};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified inline assembly of a typed USD model with independent Bevy roots and shared meshes.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("model.usda")?;
    Sphere::define(&stage, "/Model")?.create_radius_attr()?.set(1.5_f64)?;
    let model = UsdSource::snapshot("composed_sources/model.usda",
        stage.root_layer().export_to_string()?.into_bytes())?;
    let assembly = usd_bevy::usd!(r#"#usda 1.0
def "First" (prepend references = @model.usda@</Model>) {}
def "Second" (prepend references = @model.usda@</Model>) {}
"#);
    let source = UsdSource::snapshot("composed_sources/root.usda", assembly.text().as_bytes())?.with_dependency(&model)?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let first_root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
    let second_root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
    app.update();
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    let a = instances.entity(first_root, "/First").unwrap();
    let b = instances.entity(first_root, "/Second").unwrap();
    let c = instances.entity(second_root, "/First").unwrap();
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(app.world().get::<Mesh3d>(a).unwrap().0, app.world().get::<Mesh3d>(b).unwrap().0);
    assert_eq!(app.world().get::<Mesh3d>(a).unwrap().0, app.world().get::<Mesh3d>(c).unwrap().0);
    let first_stage = instances.stage(first_root).unwrap().clone();
    usd_bevy::authoring::set_attribute(&first_stage, "/First", "radius", "double", openusd::sdf::Value::Double(2.0))?;
    app.update();
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    assert_eq!(instances.stage(first_root).unwrap().prim("/First")?.attribute("radius").get::<f64>()?, Some(2.0));
    assert_eq!(instances.stage(first_root).unwrap().prim("/Second")?.attribute("radius").get::<f64>()?, Some(1.5));
    assert_eq!(instances.stage(second_root).unwrap().prim("/First")?.attribute("radius").get::<f64>()?, Some(1.5));
    assert_eq!(instances.entity(first_root, "/First"), Some(a));
    assert_ne!(app.world().get::<Mesh3d>(a).unwrap().0, app.world().get::<Mesh3d>(b).unwrap().0);
    Ok(())
}

#[test]
fn inline_typed_sources_compose_and_project() { verify().unwrap(); }
