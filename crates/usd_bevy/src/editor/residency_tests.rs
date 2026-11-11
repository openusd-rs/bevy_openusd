use super::*;
use crate::{
    live::{LiveStagePlugin, PrimEntities},
    route::{gpu_skin, preparation::MeshPreparationTiming, residency},
};
use std::path::Path;

fn scene(orientation: &str) -> String {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/skel_morph_blended_animated.usda"
    );
    format!(
        r#"#usda 1.0
(startTimeCode = 0; endTimeCode = 10)
def "Hidden" (references = @{source}@</Test>) {{
    token visibility = "invisible"
    over "Face" {{ uniform token orientation = "{orientation}" }}
}}
def "Visible" (references = @{source}@</Test>) {{}}
"#
    )
}

fn command(app: &mut App, command: EditorCommand) -> EditorView {
    let bridge = app.world().resource::<EditorBridge>().clone();
    bridge.send(command).unwrap();
    app.update();
    bridge.view().unwrap()
}

fn open(path: &Path, gpu: bool, defer: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        crate::UsdPlugin,
        LiveStagePlugin,
        EditorPlugin,
    ));
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>();
    app.init_resource::<MeshPreparationTiming>();
    if gpu {
        app.add_plugins(gpu_skin::UsdGpuSkinningPlugin);
    }
    if defer {
        app.init_resource::<residency::DeferHiddenMeshes>();
    }
    app.world_mut()
        .resource_mut::<reload::EditorReloadSettings>()
        .enabled = false;
    let view = command(
        &mut app,
        EditorCommand::Open(path.to_string_lossy().into_owned()),
    );
    assert_eq!(view.status, "Ready");
    app
}

fn entity(app: &App, path: &str) -> Entity {
    app.world().resource::<PrimEntities>().entity(path).unwrap()
}

fn compare(a: &App, b: &App, path: &str) {
    let (ae, be) = (entity(a, path), entity(b, path));
    let mesh = |app: &App, entity| {
        let handle = &app.world().get::<Mesh3d>(entity).unwrap().0;
        app.world()
            .resource::<Assets<Mesh>>()
            .get(handle)
            .unwrap()
            .clone()
    };
    let (am, bm) = (mesh(a, ae), mesh(b, be));
    assert_eq!(am.primitive_topology(), bm.primitive_topology());
    assert_eq!(am.get_index_buffer_bytes(), bm.get_index_buffer_bytes());
    assert_eq!(am.attributes().count(), bm.attributes().count());
    for (attribute, values) in am.attributes() {
        assert_eq!(
            values.get_bytes(),
            bm.attribute(attribute.id).unwrap().get_bytes(),
            "{path}: {}",
            attribute.name
        );
    }
    assert_eq!(
        am.get_morph_targets().map(bytemuck::cast_slice::<_, u8>),
        bm.get_morph_targets().map(bytemuck::cast_slice::<_, u8>)
    );
    assert_eq!(am.morph_target_names(), bm.morph_target_names());
    assert_eq!(
        a.world()
            .get::<gpu_skin::UsdGpuSkin>(ae)
            .map(|skin| &skin.matrices),
        b.world()
            .get::<gpu_skin::UsdGpuSkin>(be)
            .map(|skin| &skin.matrices)
    );
    assert_eq!(
        a.world()
            .get::<gpu_skin::UsdCpuSkinFallback>(ae)
            .map(|fallback| &fallback.0),
        b.world()
            .get::<gpu_skin::UsdCpuSkinFallback>(be)
            .map(|fallback| &fallback.0)
    );
    assert_eq!(
        a.world().get::<crate::route::skel::UsdDeformationError>(ae),
        b.world().get::<crate::route::skel::UsdDeformationError>(be)
    );
}

#[test]
fn editor_deferral_preserves_selection_reveal_history_reload_and_replacement() {
    for gpu in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().canonicalize().unwrap().join("scene.usda");
        let empty = directory.path().join("empty.usda");
        std::fs::write(&path, scene("rightHanded")).unwrap();
        std::fs::write(&empty, "#usda 1.0\ndef Xform \"Empty\" {}\n").unwrap();
        let mut eager = open(&path, gpu, false);
        let mut deferred = open(&path, gpu, true);
        let hidden = entity(&deferred, "/Hidden/Face");
        let document = deferred
            .world()
            .resource::<EditorBridge>()
            .view()
            .unwrap()
            .document
            .document_id;
        assert!(deferred.world().get::<Mesh3d>(hidden).is_none());
        assert!(
            deferred
                .world()
                .get::<residency::UsdDeferredMesh>(hidden)
                .is_some()
        );
        compare(&eager, &deferred, "/Visible/Face");
        for action in [
            EditorCommand::Select(Some("/Hidden/Face".into())),
            EditorCommand::Seek(7.0),
            EditorCommand::Edit(EditorEdit::Attribute {
                prim: "/Hidden/Face".into(),
                name: "orientation".into(),
                type_name: "token".into(),
                value: Value::Token("leftHanded".into()),
            }),
        ] {
            for app in [&mut eager, &mut deferred] {
                let view = command(app, action.clone());
                assert_eq!(view.status, "Ready");
                assert_eq!(view.document.selected.as_deref(), Some("/Hidden/Face"));
            }
            assert!(deferred.world().get::<Mesh3d>(hidden).is_none());
            compare(&eager, &deferred, "/Visible/Face");
        }
        let reveal = EditorCommand::Visibility {
            prim: "/Hidden".into(),
            visible: true,
        };
        for action in [reveal, EditorCommand::Undo, EditorCommand::Redo] {
            for app in [&mut eager, &mut deferred] {
                let view = command(app, action.clone());
                assert_eq!(view.status, "Ready");
                assert_eq!(view.timeline.current, 7.0);
                assert_eq!(view.document.selected.as_deref(), Some("/Hidden/Face"));
                assert_eq!(
                    view.document.visibility["/Hidden"],
                    !matches!(action, EditorCommand::Undo)
                );
            }
            assert_eq!(entity(&deferred, "/Hidden/Face"), hidden);
            assert!(
                deferred
                    .world()
                    .get::<residency::UsdDeferredMesh>(hidden)
                    .is_none()
            );
            compare(&eager, &deferred, "/Hidden/Face");
            compare(&eager, &deferred, "/Visible/Face");
        }
        for time in [2.0, 9.0, 2.0] {
            for app in [&mut eager, &mut deferred] {
                assert_eq!(command(app, EditorCommand::Seek(time)).status, "Ready");
            }
            compare(&eager, &deferred, "/Hidden/Face");
        }
        std::fs::write(&path, scene("leftHanded")).unwrap();
        for app in [&mut eager, &mut deferred] {
            let view = command(app, EditorCommand::ReloadSources(vec![path.clone()]));
            assert!(
                view.status.starts_with("Failed:") && view.status.contains("unsaved edits"),
                "{}",
                view.status
            );
        }
        assert_eq!(entity(&deferred, "/Hidden/Face"), hidden);
        compare(&eager, &deferred, "/Hidden/Face");
        for _ in 0..2 {
            for app in [&mut eager, &mut deferred] {
                assert_eq!(command(app, EditorCommand::Undo).status, "Ready");
            }
        }
        for app in [&mut eager, &mut deferred] {
            let view = command(app, EditorCommand::ReloadSources(vec![path.clone()]));
            assert_eq!(view.status, "Ready");
            assert_eq!(view.timeline.current, 2.0);
            assert_eq!(view.document.selected.as_deref(), Some("/Hidden/Face"));
        }
        assert_eq!(entity(&deferred, "/Hidden/Face"), hidden);
        compare(&eager, &deferred, "/Hidden/Face");
        std::fs::write(&path, "not a USD layer").unwrap();
        for app in [&mut eager, &mut deferred] {
            assert!(
                command(app, EditorCommand::ReloadSources(vec![path.clone()]))
                    .status
                    .starts_with("Failed:")
            );
        }
        compare(&eager, &deferred, "/Hidden/Face");
        std::fs::write(&path, scene("rightHanded")).unwrap();
        for app in [&mut eager, &mut deferred] {
            assert_eq!(
                command(app, EditorCommand::ReloadSources(vec![path.clone()])).status,
                "Ready"
            );
        }
        compare(&eager, &deferred, "/Hidden/Face");
        assert_eq!(
            deferred
                .world()
                .resource::<EditorBridge>()
                .view()
                .unwrap()
                .document
                .document_id,
            document
        );
        if bevy::tasks::ComputeTaskPool::get().thread_num() > 1 {
            assert!(deferred.world().resource::<MeshPreparationTiming>().jobs > 0);
        }
        let view = command(
            &mut deferred,
            EditorCommand::Open(empty.to_string_lossy().into_owned()),
        );
        assert_eq!(view.status, "Ready");
        assert_ne!(view.document.document_id, document);
        assert!(deferred.world().get_entity(hidden).is_err());
        assert_eq!(
            deferred
                .world_mut()
                .query::<&Mesh3d>()
                .iter(deferred.world())
                .count(),
            0
        );
        assert_eq!(
            deferred
                .world_mut()
                .query::<&residency::UsdDeferredMesh>()
                .iter(deferred.world())
                .count(),
            0
        );
        assert_eq!(
            deferred
                .world_mut()
                .query::<&gpu_skin::UsdGpuSkin>()
                .iter(deferred.world())
                .count(),
            0
        );
    }
}
