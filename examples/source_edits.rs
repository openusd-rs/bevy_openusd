use bevy::prelude::*;
use openusd::sdf::Value;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource, editor::EditorEdit};

#[derive(Component)]
struct RuntimeOwned;

fn projected_width(app: &App, root: Entity, path: &str) -> f32 {
    let instances = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap();
    let entity = instances.entity(root, path).unwrap();
    let handle = &app.world().get::<Mesh3d>(entity).unwrap().0;
    let mesh = app.world().resource::<Assets<Mesh>>().get(handle).unwrap();
    let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("expected float positions") };
    let min = points.iter().map(|point| point[0]).fold(f32::INFINITY, f32::min);
    let max = points.iter().map(|point| point[0]).fold(f32::NEG_INFINITY, f32::max);
    max - min
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = UsdSource::snapshot("source_edits/model.usda", &b"#usda 1.0\ndef Cube \"Model\" {}\n"[..])?;
    let root = UsdSource::snapshot("source_edits/root.usda", &b"#usda 1.0\n"[..])?;
    let assembly = root.with_references([("/Small", &model, "/Model"), ("/Large", &model, "/Model")])?;
    let customized = assembly.with_edits([
        EditorEdit::Attribute { prim: "/Small".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(1.) },
        EditorEdit::Attribute { prim: "/Large".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(4.) },
    ])?;
    let stage = customized.open_stage()?;
    assert_eq!(stage.prim("/Small")?.attribute("size").get::<f64>()?, Some(1.));
    assert_eq!(stage.prim("/Large")?.attribute("size").get::<f64>()?, Some(4.));
    assert_eq!(assembly.open_stage()?.prim("/Small")?.attribute("size").get::<f64>()?, Some(2.));
    assert_eq!(customized.dependencies().count(), assembly.dependencies().count());
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
    app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
    let original = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source: assembly, textures: default() });
    let changed = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source: customized.clone(), textures: default() });
    let original_root = app.world_mut().spawn((UsdSceneRoot(original), RuntimeOwned)).id();
    let changed_root = app.world_mut().spawn((UsdSceneRoot(changed.clone()), RuntimeOwned)).id();
    app.update();
    assert_eq!(projected_width(&app, original_root, "/Small"), 2.);
    assert_eq!(projected_width(&app, changed_root, "/Small"), 1.);
    assert_eq!(projected_width(&app, changed_root, "/Large"), 4.);
    let small = app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap().entity(changed_root, "/Small").unwrap();
    app.world_mut().entity_mut(small).insert(RuntimeOwned);
    let updated = customized.with_edits([EditorEdit::Attribute {
        prim: "/Small".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(6.),
    }])?;
    app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&changed).unwrap().source = updated;
    app.update();
    assert_eq!(projected_width(&app, changed_root, "/Small"), 6.);
    assert_eq!(projected_width(&app, changed_root, "/Large"), 4.);
    assert_eq!(projected_width(&app, original_root, "/Small"), 2.);
    assert!(app.world().get::<RuntimeOwned>(original_root).is_some());
    assert!(app.world().get::<RuntimeOwned>(changed_root).is_some());
    assert_eq!(app.world().get_non_send::<usd_bevy::instance::UsdInstances>().unwrap().entity(changed_root, "/Small"), Some(small));
    assert!(app.world().get::<RuntimeOwned>(small).is_some());
    Ok(())
}

#[test]
fn customized_assembly_keeps_original_snapshot() { main().unwrap(); }
