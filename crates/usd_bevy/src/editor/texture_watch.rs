//! Native watching of external images in the current editor document.

use super::{EditorBridge, EditorCommand, EditorSession};
use bevy::{
    asset::io::{AssetSourceEvent, file::FileWatcher},
    prelude::*,
};
use std::{collections::BTreeSet, path::PathBuf, time::{Duration, Instant}};

/// Watches requested external textures and queues document-preserving refreshes.
/// Requires EditorPlugin. Package entries and USD layers are not watched.
pub struct EditorTextureWatchPlugin;

/// Active file count and the latest watcher setup error.
#[derive(Resource, Default, Debug)]
pub struct EditorTextureWatchStatus {
    pub files: usize,
    pub error: Option<String>,
}

#[derive(Resource, Default)]
struct WatchState {
    document: Option<u64>,
    paths: BTreeSet<PathBuf>,
    pending: BTreeSet<PathBuf>,
    next_attempt: Option<Instant>,
    #[cfg(unix)]
    next_identity_check: Option<Instant>,
    #[cfg(unix)]
    identities: std::collections::BTreeMap<PathBuf, (u64, u64)>,
    watchers: Vec<(
        PathBuf,
        async_channel::Receiver<AssetSourceEvent>,
        FileWatcher,
    )>,
}

fn install_watchers(state: &mut WatchState, status: &mut EditorTextureWatchStatus, now: Instant) -> bool {
    if state.pending.is_empty() || state.next_attempt.is_some_and(|next| now < next) { return false; }
    let retry = state.next_attempt.is_some();
    let mut recovered = false;
    status.error = None;
    for parent in std::mem::take(&mut state.pending) {
        #[cfg(unix)]
        let identity = directory_identity(&parent);
        let (sender, receiver) = async_channel::unbounded();
        match FileWatcher::new(parent.clone(), sender, Duration::from_millis(300)) {
            Ok(watcher) => {
                #[cfg(unix)]
                {
                    if identity.is_none() || directory_identity(&parent) != identity {
                        status.error = Some(format!("texture watcher directory changed during setup: {}", parent.display()));
                        state.pending.insert(parent);
                        continue;
                    }
                    state.identities.insert(parent.clone(), identity.unwrap());
                }
                status.files += state.paths.iter().filter(|path| path.parent() == Some(parent.as_path())).count();
                state.watchers.push((parent, receiver, watcher));
                recovered |= retry;
            }
            Err(error) => {
                status.error = Some(format!("texture watcher {}: {error}", parent.display()));
                state.pending.insert(parent);
            }
        }
    }
    state.next_attempt = (!state.pending.is_empty()).then_some(now + Duration::from_secs(1));
    recovered
}

#[cfg(unix)]
fn directory_identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    metadata.is_dir().then(|| (metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
fn rearm_replaced_directories(state: &mut WatchState, now: Instant) -> Option<usize> {
    if state.next_identity_check.is_some_and(|next| now < next) { return None; }
    state.next_identity_check = Some(now + Duration::from_secs(1));
    let replaced: BTreeSet<_> = state.watchers.iter().filter_map(|(parent, _, _)| {
        (directory_identity(parent).as_ref() != state.identities.get(parent)).then(|| parent.clone())
    }).collect();
    if replaced.is_empty() { return None; }
    state.watchers.retain(|(parent, _, _)| !replaced.contains(parent));
    for parent in &replaced { state.identities.remove(parent); }
    state.pending.extend(replaced);
    let files = state.paths.iter().filter(|path|
        state.watchers.iter().any(|(parent, _, _)| path.parent() == Some(parent.as_path()))).count();
    state.next_attempt = Some(now);
    Some(files)
}

impl Plugin for EditorTextureWatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WatchState>()
            .init_resource::<EditorTextureWatchStatus>()
            .add_systems(PreUpdate, watch_textures.before(super::process_commands));
    }
}

fn watch_textures(
    editor: Option<NonSend<EditorSession>>,
    textures: Option<Res<super::EditorTextureRequests>>,
    bridge: Res<EditorBridge>,
    mut state: ResMut<WatchState>,
    mut status: ResMut<EditorTextureWatchStatus>,
) {
    let document = editor.as_ref().map(|editor| editor.document_id);
    if state.document != document
        || textures
            .as_ref()
            .is_some_and(|textures| textures.is_changed())
        || (textures.is_none() && !state.paths.is_empty())
    {
        let paths: BTreeSet<_> = textures
            .as_ref()
            .filter(|_| document.is_some())
            .into_iter()
            .flat_map(|textures| textures.0.iter())
            .filter(|path| !openusd::ar::is_package_relative_path(path))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .collect();
        if state.document != document || state.paths != paths {
            state.watchers.clear();
            #[cfg(unix)]
            {
                state.identities.clear();
                state.next_identity_check = None;
            }
            state.document = document;
            state.paths = paths;
            status.files = 0;
            status.error = None;
            state.next_attempt = None;
            state.pending = state
                .paths
                .iter()
                .filter_map(|path| path.parent().map(ToOwned::to_owned))
                .collect();
        }
    }
    let now = Instant::now();
    let mut changed = false;
    #[cfg(unix)]
    if let Some(files) = rearm_replaced_directories(&mut state, now) {
        status.files = files;
        changed = true;
    }
    if !state.pending.is_empty() && state.next_attempt.is_none_or(|next| now >= next) {
        changed |= install_watchers(&mut state, &mut status, now);
    }
    for (parent, receiver, _) in &state.watchers {
        while let Ok(event) = receiver.try_recv() {
            let matches = |path: &std::path::Path| state.paths.contains(&parent.join(path));
            changed |= match event {
                AssetSourceEvent::AddedAsset(path)
                | AssetSourceEvent::ModifiedAsset(path)
                | AssetSourceEvent::RemovedAsset(path) => matches(&path),
                AssetSourceEvent::RenamedAsset { old, new } => matches(&old) || matches(&new),
                AssetSourceEvent::RemovedUnknown {
                    path,
                    is_meta: false,
                } => matches(&path),
                _ => false,
            };
        }
    }
    if changed {
        if let Err(error) = bridge.send(EditorCommand::RefreshTextures) {
            status.error = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_editor_watch_keeps_status_change_tick() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, super::super::EditorPlugin, EditorTextureWatchPlugin));
        app.update();
        let tick = app.world().get_resource_ref::<EditorTextureWatchStatus>().unwrap().last_changed();
        for _ in 0..3 { app.update(); }
        assert_eq!(app.world().get_resource_ref::<EditorTextureWatchStatus>().unwrap().last_changed(), tick);
    }

    #[test]
    #[ignore = "requires native filesystem events"]
    fn native_editor_texture_watch_retries_only_failed_directories() {
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let missing = directory.path().join("missing");
        std::fs::create_dir(&ready).unwrap();
        let mut state = WatchState {
            paths: [ready.join("a.png"), missing.join("b.png")].into(),
            pending: [ready.clone(), missing.clone()].into(),
            ..default()
        };
        let mut status = EditorTextureWatchStatus::default();
        let now = Instant::now();
        assert!(!install_watchers(&mut state, &mut status, now));
        assert_eq!(status.files, 1);
        assert!(status.error.as_ref().unwrap().contains("missing"));
        assert_eq!(state.pending, [missing.clone()].into());
        let retained = state.watchers[0].1.clone();
        std::fs::create_dir(&missing).unwrap();
        assert!(!install_watchers(&mut state, &mut status, now + Duration::from_millis(999)));
        assert_eq!(status.files, 1);
        assert!(install_watchers(&mut state, &mut status, now + Duration::from_secs(1)));
        assert_eq!(status.files, 2);
        assert!(status.error.is_none());
        assert!(state.pending.is_empty() && state.next_attempt.is_none());
        assert_eq!(state.watchers.len(), 2);
        assert!(retained.same_channel(&state.watchers[0].1));
        assert!(!install_watchers(&mut state, &mut status, now + Duration::from_secs(2)));
        assert_eq!(status.files, 2);
        #[cfg(unix)]
        {
            assert_eq!(rearm_replaced_directories(&mut state, now + Duration::from_secs(2)), None);
            std::fs::rename(&missing, directory.path().join("old-missing")).unwrap();
            std::fs::create_dir(&missing).unwrap();
            assert_eq!(rearm_replaced_directories(&mut state, now + Duration::from_millis(2999)), None);
            status.files = rearm_replaced_directories(&mut state, now + Duration::from_secs(3)).unwrap();
            assert_eq!(status.files, 1);
            assert_eq!(state.pending, [missing.clone()].into());
            assert!(retained.same_channel(&state.watchers[0].1));
            assert!(install_watchers(&mut state, &mut status, now + Duration::from_secs(3)));
            assert_eq!(status.files, 2);
            assert!(retained.same_channel(&state.watchers[0].1));
            assert_eq!(rearm_replaced_directories(&mut state, now + Duration::from_secs(4)), None);
        }
    }

    fn png(pixel: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixel)
                .unwrap();
        }
        bytes
    }

    fn tick_until(app: &mut App, condition: impl Fn(&World) -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.update();
            if condition(app.world()) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "editor watch timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    #[ignore = "requires native filesystem events"]
    fn native_editor_texture_watch_recovery_refreshes_missed_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let images = directory.path().join("images");
        std::fs::create_dir(&images).unwrap();
        let file = images.join("pixel.png");
        std::fs::write(&file, png([255, 0, 0, 255])).unwrap();
        let scene = directory.path().join("scene.usda");
        std::fs::write(&scene, r#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Texture.outputs:rgb>
        token outputs:surface
    }
    def Shader "Texture" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @images/pixel.png@
        float3 outputs:rgb
    }
}
"#).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin,
            super::super::EditorPlugin, EditorTextureWatchPlugin));
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(scene.to_string_lossy().into_owned())).unwrap();
        app.update();
        let id = bridge.view().unwrap().document.document_id;
        let before = app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap();
        let pixels = |world: &World| {
            let handle = world.resource::<crate::asset::SnapshotTextures>().0.values().next().unwrap();
            world.resource::<Assets<Image>>().get(handle).unwrap().data.clone().unwrap()
        };
        assert_eq!(pixels(app.world()), [255, 0, 0, 255]);
        assert!(app.world().resource::<WatchState>().watchers.is_empty());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&images).unwrap();
        app.update();
        assert!(app.world().resource::<EditorTextureWatchStatus>().error.is_some());
        assert_eq!(app.world().resource::<EditorTextureWatchStatus>().files, 0);
        bridge.send(EditorCommand::Select(Some("/Mat".into()))).unwrap();
        app.update();
        std::fs::create_dir(&images).unwrap();
        std::fs::write(&file, png([0, 0, 255, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 0, 255, 255]);
        assert_eq!(app.world().resource::<EditorTextureWatchStatus>().files, 1);
        assert!(app.world().resource::<EditorTextureWatchStatus>().error.is_none());
        let view = bridge.view().unwrap();
        assert_eq!(view.document.document_id, id);
        assert_eq!(view.document.selected.as_deref(), Some("/Mat"));
        assert_eq!(view.status, "Ready");
        assert_eq!(app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap(), before);
        bridge.send(EditorCommand::Edit(super::super::EditorEdit::Attribute {
            prim: "/Mat/Texture".into(), name: "inputs:file".into(), type_name: "asset".into(),
            value: openusd::sdf::Value::AssetPath(openusd::sdf::AssetPath::new("new-images/later.png")),
        })).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.contains("Texture loading failed"));
        assert_eq!(pixels(app.world()), [0, 0, 255, 255]);
        let requested = directory.path().join("new-images/later.png");
        assert!(app.world().resource::<super::super::EditorTextureRequests>().0.contains(requested.to_str().unwrap()), "requests={:?}; status={}", app.world().resource::<super::super::EditorTextureRequests>().0, bridge.view().unwrap().status);
        let edited = app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorTextureWatchStatus>().files, 0);
        assert!(app.world().resource::<EditorTextureWatchStatus>().error.is_some());
        std::fs::create_dir(requested.parent().unwrap()).unwrap();
        std::fs::write(&requested, png([0, 255, 0, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 255, 0, 255]);
        assert_eq!(bridge.view().unwrap().document.document_id, id);
        assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Mat"));
        assert_eq!(bridge.view().unwrap().status, "Ready");
        assert_eq!(app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap(), edited);
        std::fs::write(&requested, png([255, 255, 0, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [255, 255, 0, 255]);
        #[cfg(unix)]
        {
            let parent = requested.parent().unwrap();
            let moved = directory.path().join("replaced-images");
            std::fs::rename(parent, &moved).unwrap();
            std::fs::create_dir(parent).unwrap();
            std::fs::write(&requested, png([255, 0, 255, 255])).unwrap();
            tick_until(&mut app, |world| pixels(world) == [255, 0, 255, 255]);
            std::fs::write(&requested, png([0, 255, 255, 255])).unwrap();
            tick_until(&mut app, |world| pixels(world) == [0, 255, 255, 255]);
            assert_eq!(bridge.view().unwrap().document.document_id, id);
            assert_eq!(app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap(), edited);
        }
    }

    #[test]
    #[ignore = "requires native filesystem events"]
    fn native_editor_texture_watch_preserves_document_and_recovers() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("pixel.png");
        std::fs::write(&file, png([255, 0, 0, 255])).unwrap();
        let scene = directory.path().join("scene.usda");
        std::fs::write(
            &scene,
            r#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Texture.outputs:rgb>
        token outputs:surface
    }
    def Shader "Texture" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @pixel.png@
        float3 outputs:rgb
    }
}
"#,
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            crate::live::LiveStagePlugin,
            super::super::EditorPlugin,
            EditorTextureWatchPlugin,
        ));
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge
            .send(EditorCommand::Open(scene.to_string_lossy().into_owned()))
            .unwrap();
        tick_until(&mut app, |world| {
            world.resource::<EditorTextureWatchStatus>().files == 1
        });
        assert!(
            app.world()
                .resource::<EditorTextureWatchStatus>()
                .error
                .is_none()
        );
        bridge
            .send(EditorCommand::Select(Some("/Mat".into())))
            .unwrap();
        bridge
            .send(EditorCommand::Edit(super::super::EditorEdit::Attribute {
                prim: "/Mat/Surface".into(),
                name: "inputs:ior".into(),
                type_name: "float".into(),
                value: openusd::sdf::Value::Float(1.7),
            }))
            .unwrap();
        app.update();
        let before = app
            .world()
            .non_send::<EditorSession>()
            .stage()
            .root_layer()
            .export_to_string()
            .unwrap();
        let id = bridge.view().unwrap().document.document_id;
        let pixels = |world: &World| {
            let textures = world.resource::<crate::asset::SnapshotTextures>();
            let handle = textures.0.values().next().unwrap();
            world
                .resource::<Assets<Image>>()
                .get(handle)
                .unwrap()
                .data
                .clone()
                .unwrap()
        };
        assert_eq!(pixels(app.world()), [255, 0, 0, 255]);
        let texture_tick = app
            .world()
            .get_resource_ref::<crate::asset::SnapshotTextures>()
            .unwrap()
            .last_changed();
        std::fs::write(directory.path().join("unrelated.txt"), "unrelated change").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            app.update();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            app.world()
                .get_resource_ref::<crate::asset::SnapshotTextures>()
                .unwrap()
                .last_changed(),
            texture_tick
        );
        let temporary = directory.path().join("replacement.png");
        std::fs::write(&temporary, png([0, 0, 255, 255])).unwrap();
        std::fs::rename(&temporary, &file).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 0, 255, 255]);
        std::fs::remove_file(&file).unwrap();
        tick_until(&mut app, |_| {
            bridge.view().unwrap().status.starts_with("Failed:")
        });
        assert_eq!(pixels(app.world()), [0, 0, 255, 255]);
        std::fs::write(&file, png([0, 255, 0, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 255, 0, 255]);
        assert_eq!(bridge.view().unwrap().status, "Ready");
        assert_eq!(bridge.view().unwrap().document.document_id, id);
        assert_eq!(
            bridge.view().unwrap().document.selected.as_deref(),
            Some("/Mat")
        );
        assert!(bridge.view().unwrap().document.can_undo);
        assert_eq!(
            app.world()
                .non_send::<EditorSession>()
                .stage()
                .root_layer()
                .export_to_string()
                .unwrap(),
            before
        );
        let next_directory = tempfile::tempdir().unwrap();
        let next_scene = next_directory.path().join("scene.usda");
        let next_file = next_directory.path().join("pixel.png");
        std::fs::copy(&scene, &next_scene).unwrap();
        std::fs::write(&next_file, png([255, 255, 0, 255])).unwrap();
        bridge
            .send(EditorCommand::Open(next_scene.to_string_lossy().into_owned()))
            .unwrap();
        tick_until(&mut app, |world| {
            world.resource::<WatchState>().paths.contains(&next_file)
                && pixels(world) == [255, 255, 0, 255]
        });
        assert_ne!(bridge.view().unwrap().document.document_id, id);
        assert_eq!(app.world().resource::<WatchState>().paths.len(), 1);
        assert_eq!(app.world().resource::<WatchState>().watchers.len(), 1);
        assert_eq!(app.world().resource::<EditorTextureWatchStatus>().files, 1);
        let texture_tick = app
            .world()
            .get_resource_ref::<crate::asset::SnapshotTextures>()
            .unwrap()
            .last_changed();
        std::fs::remove_file(&file).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            app.update();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            app.world()
                .get_resource_ref::<crate::asset::SnapshotTextures>()
                .unwrap()
                .last_changed(),
            texture_tick
        );
        assert_eq!(bridge.view().unwrap().status, "Ready");
        let next_id = bridge.view().unwrap().document.document_id;
        bridge
            .send(EditorCommand::Open(
                next_directory.path().join("missing.usda").to_string_lossy().into_owned(),
            ))
            .unwrap();
        tick_until(&mut app, |_| {
            bridge.view().unwrap().status.starts_with("Failed:")
        });
        assert_eq!(bridge.view().unwrap().document.document_id, next_id);
        assert!(app.world().resource::<WatchState>().paths.contains(&next_file));
        assert_eq!(pixels(app.world()), [255, 255, 0, 255]);
        std::fs::write(&next_file, png([255, 0, 255, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [255, 0, 255, 255]);
        assert_eq!(bridge.view().unwrap().status, "Ready");
        app.world_mut().remove_non_send::<EditorSession>();
        app.update();
        assert_eq!(app.world().resource::<EditorTextureWatchStatus>().files, 0);
        assert!(app.world().resource::<WatchState>().watchers.is_empty());
    }
}
