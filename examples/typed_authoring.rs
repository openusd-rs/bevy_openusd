use bevy::prelude::*;
use openusd_schemas::geom::{Sphere, SphereSchema};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};

#[derive(Component, Reflect, Default, Debug, PartialEq)]
#[reflect(Component, Default)]
struct Health { current: f64, max: f64 }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified typed USD schema and Bevy component authoring across two independent instances.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("typed.usda")?;
    let sphere = Sphere::define(&stage, "/Enemy")?;
    sphere.create_radius_attr()?.set(0.75_f64)?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>().register_type::<Health>();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let health = Health { current: 30.0, max: 100.0 };
    usd_bevy::sync::author_component_value(&registry.read(), &stage, "/Enemy", &health)?;
    let source = UsdSource::new("typed.usda", stage.root_layer().export_to_string()?.into_bytes())?;
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let first = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
    let second = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
    app.update();
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    let a = instances.entity(first, "/Enemy").unwrap();
    let b = instances.entity(second, "/Enemy").unwrap();
    assert_ne!(a, b);
    assert_eq!(app.world().get::<Health>(a), Some(&health));
    assert_eq!(app.world().get::<Health>(b), Some(&health));
    let first_stage = instances.stage(first).unwrap().clone();
    usd_bevy::sync::author_component_value(&registry.read(), &first_stage, "/Enemy", &Health { current: 7.0, max: 100.0 })?;
    app.update();
    assert_eq!(app.world().get::<Health>(a).unwrap().current, 7.0);
    assert_eq!(app.world().get::<Health>(b).unwrap().current, 30.0);
    usd_bevy::sync::author_component_presence::<Health>(&registry.read(), &first_stage, "/Enemy", false)?;
    app.update();
    assert!(app.world().get::<Health>(a).is_none());
    assert_eq!(app.world().get::<Health>(b), Some(&health));
    usd_bevy::sync::author_component_presence::<Health>(&registry.read(), &first_stage, "/Enemy", true)?;
    app.update();
    assert_eq!(app.world().get::<Health>(a).unwrap().current, 7.0);
    assert_eq!(app.world().get::<Health>(b), Some(&health));
    let mut overrides = usd_bevy::instance::UsdInstanceOverrides::default();
    overrides.set_component(&registry.read(), "/Enemy", &Health { current: 11.0, max: 100.0 })?;
    app.world_mut().entity_mut(first).insert(overrides);
    usd_bevy::sync::author_component_value(&registry.read(), &stage, "/Enemy", &Health { current: 55.0, max: 100.0 })?;
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
        UsdSource::new("typed.usda", stage.root_layer().export_to_string()?.into_bytes())?;
    app.update();
    assert_eq!(app.world().get::<Health>(a).unwrap().current, 11.0);
    assert_eq!(app.world().get::<Health>(b).unwrap().current, 55.0);
    Ok(())
}

#[test]
fn typed_values_project_and_remain_instance_local() { verify().unwrap(); }
