use super::*;
use bevy::ecs::system::SystemState;

fn config(steps: usize) -> Config {
    Config {
        prim: "/Test".into(),
        mesh: "/Test/Face".into(),
        steps,
    }
}

#[test]
fn residency_capture_configuration_is_bounded_and_explicit() {
    assert!(Config::parse(None).unwrap().is_none());
    for steps in 1..=4 {
        let parsed = Config::parse(Some(format!(
            r#"{{"prim":"/Test","mesh":"/Test/Face","steps":{steps}}}"#
        )))
        .unwrap()
        .unwrap();
        assert_eq!(parsed.steps, steps);
    }
    for value in [
        "",
        "null",
        "[]",
        r#"{"prim":"/Test","mesh":"/Test/Face","steps":0}"#,
        r#"{"prim":"/Test","mesh":"/Test/Face","steps":5}"#,
        r#"{"prim":"Test","mesh":"/Test/Face","steps":2}"#,
        r#"{"prim":"/","mesh":"/Test/Face","steps":2}"#,
        r#"{"prim":"/Test","mesh":"/Other/Face","steps":2}"#,
        r#"{"prim":"/Test","mesh":"/Test/Face","steps":2,"save":true}"#,
    ] {
        assert!(Config::parse(Some(value.into())).is_err(), "{value}");
    }
    assert!(Config::parse(Some(" ".repeat(8193))).is_err());
}

fn initial() -> Observation {
    Observation {
        document: 1,
        revision: 0,
        time: 7.0,
        entity: Entity::PLACEHOLDER,
        selected: None,
        visible: false,
        inherited_visible: false,
        undo: false,
        redo: false,
        resident: false,
        deferred: true,
        policy: true,
        camera: "camera_world=test\n".into(),
    }
}

#[test]
fn residency_capture_requires_acknowledgements_and_rejects_identity_drift() {
    let bridge = EditorBridge::default();
    let config = config(4);
    for field in 0..5 {
        let mut state = State::default();
        assert!(!state.step(&config, initial(), &bridge).unwrap());
        assert!(!state.step(&config, initial(), &bridge).unwrap());
        assert_eq!(state.observations.len(), 1);
        let mut observed = initial();
        match field {
            0 => observed.document += 1,
            1 => observed.time += 1.0,
            2 => observed.camera.push_str("different"),
            3 => observed.policy = false,
            4 => observed.entity = World::new().spawn_empty().id(),
            _ => unreachable!(),
        }
        assert!(state.step(&config, observed, &bridge).is_err());
    }
    let mut state = State::default();
    let mut bad = initial();
    bad.resident = true;
    assert!(state.step(&config, bad, &bridge).is_err());
    assert!(!state.step(&config, initial(), &bridge).unwrap());
    let mut bad = initial();
    bad.selected = Some(config.mesh.clone());
    bad.resident = true;
    assert!(state.step(&config, bad, &bridge).is_err());
}

#[test]
fn capture_sequence_observes_real_selection_reveal_history_and_residency() {
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/residency_reveal.usda");
    for gpu in [false, true] {
        for deferred in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::asset::AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                TransformPlugin,
                bevy::camera::visibility::VisibilityPlugin,
                usd_bevy::UsdPlugin,
                usd_bevy::live::LiveStagePlugin,
                usd_bevy::editor::EditorPlugin,
            ));
            app.init_asset::<Mesh>()
                .init_asset::<StandardMaterial>()
                .init_asset::<Image>();
            crate::residency_policy::configure(&mut app, deferred);
            if gpu {
                app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin);
            }
            let bridge = app.world().resource::<EditorBridge>().clone();
            bridge.send(EditorCommand::Open(source.into())).unwrap();
            bridge.send(EditorCommand::Seek(7.0)).unwrap();
            app.update();
            let mut inputs = SystemState::<Inputs>::new(app.world_mut());
            let mut state = State::default();
            let config = config(4);
            let mut completed = false;
            for _ in 0..70 {
                let ready = state
                    .advance(
                        &config,
                        &inputs.get(app.world()).unwrap(),
                        &bridge,
                        "fixed-camera".into(),
                    )
                    .unwrap();
                app.update();
                if ready {
                    completed = true;
                    break;
                }
            }
            assert!(completed);
            assert_eq!(state.observations.len(), 5);
            assert_eq!(state.observations[0].resident, !deferred);
            assert!(!state.observations[1].inherited_visible);
            assert!(state.observations[2].resident && state.observations[2].inherited_visible);
            assert!(state.observations[3].resident && !state.observations[3].inherited_visible);
            assert!(state.observations[4].resident && state.observations[4].inherited_visible);
            assert!(
                state
                    .report(&config)
                    .contains("residency_steps_observed=4\n")
            );
            assert!(
                state
                    .advance(
                        &config,
                        &inputs.get(app.world()).unwrap(),
                        &bridge,
                        "fixed-camera".into()
                    )
                    .unwrap()
            );
        }
    }
}
