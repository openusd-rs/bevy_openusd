use bevy::prelude::*;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState};
use usd_bevy::instance::{UsdAttributeOverride, UsdInstanceOverrides, UsdInstanceTime, UsdInstances};

fn main() {
    verify_instances();
    println!("Verified: shared source, independent clocks and overrides, isolated updates.");
}

fn verify_instances() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin {
        file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
        ..default()
    }, UsdPlugin, UsdAssetPlugin));
    app.insert_resource(Assets::<Mesh>::default());
    app.insert_resource(Assets::<StandardMaterial>::default());
    let asset: Handle<UsdScene> = app.world().resource::<AssetServer>().load("animated_spinner.usda");
    let first = app.world_mut().spawn((UsdSceneRoot(asset.clone()), UsdInstanceTime { current: 0.0 })).id();
    let second = app.world_mut().spawn((UsdSceneRoot(asset), UsdInstanceTime { current: 12.0 },
        UsdInstanceOverrides {
            attributes: vec![UsdAttributeOverride {
                prim: "/World/Static".into(), name: "size".into(), type_name: "double".into(),
                value: openusd::sdf::Value::Double(2.0),
            }],
            ..default()
        })).id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.update();
        for root in [first, second] {
            if let Some(UsdSceneState::Failed(error)) = app.world().get::<UsdSceneState>(root) {
                panic!("USD load failed: {error}");
            }
        }
        if [first, second].into_iter().all(|root| app.world().get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)) { break; }
        assert!(std::time::Instant::now() < deadline, "USD assets did not become ready");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let instances = app.world().get_non_send::<UsdInstances>().unwrap();
    assert_eq!(instances.len(), 2);
    let a = instances.entity(first, "/World/Spinner").unwrap();
    let b = instances.entity(second, "/World/Spinner").unwrap();
    assert_ne!(a, b);
    let size = |root| instances.stage(root).unwrap()
        .prim(openusd::sdf::path("/World/Static").unwrap()).unwrap()
        .attribute("size").get::<f64>().unwrap().unwrap();
    assert_eq!(size(first), 0.4);
    assert_eq!(size(second), 2.0);
    let first_rotation = app.world().get::<Transform>(a).unwrap().rotation;
    let second_rotation = app.world().get::<Transform>(b).unwrap().rotation;
    assert!(first_rotation.angle_between(second_rotation) > 1.0);
    app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 24.0;
    app.update();
    assert!(first_rotation.angle_between(app.world().get::<Transform>(a).unwrap().rotation) > 3.0);
    assert!(second_rotation.angle_between(app.world().get::<Transform>(b).unwrap().rotation) < 0.001);
    app.world_mut().entity_mut(second).remove::<UsdInstanceOverrides>();
    app.update();
    let instances = app.world().get_non_send::<UsdInstances>().unwrap();
    assert_eq!(instances.entity(second, "/World/Spinner"), Some(b));
    assert_eq!(instances.stage(second).unwrap().prim(openusd::sdf::path("/World/Static").unwrap()).unwrap()
        .attribute("size").get::<f64>().unwrap(), Some(0.4));
}

#[cfg(test)]
mod tests {
    #[test]
    fn independent_instances_showcase() { super::verify_instances(); }
}
