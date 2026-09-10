use bevy::prelude::*;
use openusd_schemas::geom::{Sphere, SphereSchema};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};

#[derive(Component, Debug, PartialEq)]
struct RuntimeTag(u32);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified typed/inline composition, shared meshes, reload isolation, failure recovery and root cleanup.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("model.usda")?;
    let sphere = Sphere::define(&stage, "/Model")?;
    sphere.create_radius_attr()?.set(1.5_f64)?;
    let model = UsdSource::snapshot("composed_sources/model.usda",
        stage.root_layer().export_to_string()?.into_bytes())?;
    let assembly = usd_bevy::usd!(r#"#usda 1.0
(upAxis = "Y")
"#);
    let root_source = UsdSource::snapshot("composed_sources/root.usda", assembly.text().as_bytes())?;
    let compose = |model: &UsdSource| -> Result<UsdSource, Box<dyn std::error::Error>> {
        Ok(root_source.with_reference("/First", model, "/Model")?
            .with_reference("/Second", model, "/Model")?)
    };
    let source = compose(&model)?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let handle = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
    let first_root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
    let second_root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
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

    app.world_mut().entity_mut(a).insert(RuntimeTag(73));
    app.world_mut().entity_mut(first_root).insert(usd_bevy::instance::UsdInstanceOverrides {
        attributes: vec![usd_bevy::instance::UsdAttributeOverride {
            prim: "/First".into(), name: "radius".into(), type_name: "double".into(),
            value: openusd::sdf::Value::Double(4.0),
        }], ..default()
    });
    sphere.create_radius_attr()?.set(3.0_f64)?;
    let replacement = UsdSource::snapshot("composed_sources/model.usda", stage.root_layer().export_to_string()?.into_bytes())?;
    let replacement_source = compose(&replacement)?;
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = replacement_source.clone();
    app.update();
    assert_radius(&app, first_root, "/First", 4.0)?;
    assert_radius(&app, first_root, "/Second", 3.0)?;
    assert_radius(&app, second_root, "/First", 3.0)?;
    assert_eq!(app.world().get::<RuntimeTag>(a), Some(&RuntimeTag(73)));
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    assert_eq!(instances.entity(first_root, "/First"), Some(a));
    assert_eq!(instances.entity(first_root, "/Second"), Some(b));
    assert_eq!(instances.entity(second_root, "/First"), Some(c));
    assert_eq!(app.world().get::<Mesh3d>(b).unwrap().0, app.world().get::<Mesh3d>(c).unwrap().0);

    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
        UsdSource::snapshot("composed_sources/root.usda", &b"invalid USD"[..])?;
    app.update();
    for root in [first_root, second_root] {
        assert!(matches!(app.world().get::<UsdSceneState>(root), Some(UsdSceneState::Failed(_))));
    }
    assert_radius(&app, first_root, "/First", 4.0)?;
    assert_radius(&app, second_root, "/First", 3.0)?;
    assert_eq!(app.world().get::<RuntimeTag>(a), Some(&RuntimeTag(73)));
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = replacement_source;
    app.update();
    for root in [first_root, second_root] {
        assert!(matches!(app.world().get::<UsdSceneState>(root), Some(UsdSceneState::Ready)));
    }
    app.world_mut().entity_mut(first_root).remove::<usd_bevy::instance::UsdInstanceOverrides>();
    app.update();
    assert_radius(&app, first_root, "/First", 3.0)?;
    assert_eq!(app.world().get::<RuntimeTag>(a), Some(&RuntimeTag(73)));
    assert_eq!(app.world().get::<Mesh3d>(a).unwrap().0, app.world().get::<Mesh3d>(c).unwrap().0);
    app.world_mut().entity_mut(first_root).remove::<UsdSceneRoot>();
    app.update();
    assert!(app.world().get_entity(first_root).is_ok());
    assert!(app.world().get_entity(a).is_err());
    assert!(app.world().get_entity(b).is_err());
    assert!(app.world().get::<UsdSceneState>(first_root).is_none());
    assert_radius(&app, second_root, "/First", 3.0)?;
    assert_eq!(app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap().len(), 1);
    app.world_mut().despawn(second_root);
    app.update();
    assert!(app.world().get_entity(c).is_err());
    assert!(app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap().is_empty());
    assert_eq!(app.world_mut().query::<&usd_bevy::UsdPrimRef>().iter(app.world()).count(), 0);
    Ok(())
}

fn assert_radius(app: &App, root: Entity, path: &str, expected: f64) -> Result<(), Box<dyn std::error::Error>> {
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    assert_eq!(instances.stage(root).unwrap().prim(path)?.attribute("radius").get::<f64>()?, Some(expected));
    Ok(())
}

#[test]
fn inline_typed_sources_compose_and_project() { verify().unwrap(); }
