//! Composition-aware editor state over one authoritative live stage.

use openusd::sdf::Value;
use openusd::usd::{EditTarget, Stage, UndoStage};
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

mod layer_changes;
pub mod reload;
pub mod save_state;

#[cfg(all(feature = "file_watcher", not(target_arch = "wasm32")))]
pub mod texture_watch;

#[derive(Debug, Clone)]
pub enum EditorCommand {
    Open(String),
    OpenChecked { filename: String, document_id: u64, revision: u64 },
    /// Reload source-backed images without replacing the document or edit history.
    RefreshTextures,
    /// Refreshes changed disk layers in the current document.
    ReloadSources(Vec<std::path::PathBuf>),
    Select(Option<String>),
    Visibility { prim: String, visible: bool },
    Edit(EditorEdit),
    EditChecked { edit: EditorEdit, document_id: u64, revision: u64, target: EditTarget },
    EditLayer(String),
    EditLayerChecked { identifier: String, document_id: u64, revision: u64 },
    LayerMuteChecked { identifier: String, muted: bool, document_id: u64, revision: u64 },
    Payload { prim: String, loaded: bool },
    PayloadChecked { prim: String, loaded: bool, document_id: u64, revision: u64 },
    Undo,
    Redo,
    Seek(f64),
    Play(bool),
    Save { filename: String, mode: SaveMode },
    SaveChecked { filename: String, mode: SaveMode, document_id: u64, edit_layer: String },
}

#[derive(Debug, Clone, Default)]
pub struct EditorView {
    pub document: EditorSnapshot,
    pub status: String,
    pub timeline: EditorTimeline,
}

/// Accumulated command-driven inspector snapshot work.
#[derive(Resource, Default)]
pub struct EditorSnapshotTiming {
    pub snapshots: usize,
    pub elapsed: std::time::Duration,
}

#[derive(Debug, Clone, Default)]
pub struct EditorTimeline {
    pub current: f64,
    pub start: f64,
    pub end: f64,
    pub playing: bool,
}

#[derive(Resource, Default)]
struct EditorPlayback(crate::instance::UsdPlayback);

#[derive(Default)]
struct BridgeState {
    commands: VecDeque<EditorCommand>,
    view: EditorView,
}

/// Sendable command queue and owned UI snapshot; USD stages stay on the main thread.
#[derive(Resource, Clone, Default)]
pub struct EditorBridge(Arc<Mutex<BridgeState>>);

impl EditorBridge {
    pub fn send(&self, command: EditorCommand) -> anyhow::Result<()> {
        self.0.lock().map_err(|_| anyhow::anyhow!("editor queue is poisoned"))?.commands.push_back(command);
        Ok(())
    }

    pub fn view(&self) -> anyhow::Result<EditorView> {
        Ok(self.0.lock().map_err(|_| anyhow::anyhow!("editor queue is poisoned"))?.view.clone())
    }
}

/// Processes editor commands for the same stage used by `LiveStagePlugin`.
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("USD_PROFILE_LOADING").is_some() { app.init_resource::<EditorSnapshotTiming>(); }
        app.init_resource::<EditorBridge>().init_resource::<EditorPlayback>()
            .init_resource::<crate::route::StageTime>()
            .add_systems(PreUpdate, (process_commands, advance_editor_time).chain())
            .add_systems(Last, publish_projection_issues);
        #[cfg(not(target_arch = "wasm32"))]
        app.init_resource::<reload::EditorReloadStatus>()
            .init_resource::<reload::EditorReloadSettings>()
            .init_resource::<reload::WatchState>()
            .add_systems(PreUpdate, reload::watch.before(process_commands));
    }
}

fn publish_projection_issues(
    bridge: Res<EditorBridge>,
    prims: Option<Res<crate::live::PrimEntities>>,
    issues: Query<&crate::route::reflect::UsdReflectIssues>,
    rendering: Query<(Option<&crate::route::subdivision::UsdSubdivisionError>,
        Option<&crate::route::skel::UsdDeformationError>, Option<&crate::route::instancer::UsdInstancerWarning>,
        Option<&crate::route::shapes::UsdShapeError>, Option<&crate::route::curves::UsdCurveError>,
        Option<&crate::route::xform::UsdTransformError>, Option<&crate::route::material::UsdMaterialWarning>,
        Option<&crate::route::subset::UsdSubsetWarning>, Option<&crate::route::gpu_skin::UsdCpuSkinFallback>)>,
    generated: Query<(Option<&Children>, Option<&crate::route::subset::UsdSubset>,
        Option<&crate::route::instancer::UsdPrototypePart>, Has<crate::route::instancer::UsdInstance>,
        Option<&crate::route::instancer::UsdInstanceId>, Option<&crate::route::material::UsdMaterialWarning>)>,
) {
    let Ok(mut state) = bridge.0.lock() else { return };
    state.view.document.reflect_issues = state.view.document.selected.as_deref()
        .and_then(|path| prims.as_ref()?.entity(path))
        .and_then(|entity| issues.get(entity).ok())
        .map_or_else(Vec::new, |issues| issues.0.clone());
    state.view.document.render_issues = state.view.document.selected.as_deref()
        .and_then(|path| prims.as_ref()?.entity(path))
        .and_then(|entity| rendering.get(entity).ok())
        .map_or_else(Vec::new, |(subdivision, deformation, instancer, shape, curve, transform, material, subset, fallback)| {
            [subdivision.map(|error| format!("Subdivision: {}", error.0)),
                deformation.map(|error| format!("Deformation: {}", error.0)),
                instancer.map(|error| format!("Point instancer: {}", error.0)),
                shape.map(|error| format!("Shape: {}", error.0)),
                curve.map(|error| format!("Curve: {}", error.0)),
                transform.map(|error| format!("Transform: {}", error.0)),
                material.map(|warning| format!("Material: {}", warning.0)),
                subset.map(|warning| format!("Material subsets: {}", warning.0)),
                fallback.map(|warning| format!("CPU skinning fallback: {}", warning.0))]
                .into_iter().flatten().collect()
        });
    let selected = state.view.document.selected.as_deref().and_then(|path| prims.as_ref()?.entity(path));
    if let Some(entity) = selected {
        let mut pending = generated.get(entity).ok().and_then(|data| data.0)
            .map(|children| children.iter().rev().take(4097).map(|child| (child, String::new())).collect::<Vec<_>>()).unwrap_or_default();
        let mut visited = 0;
        let mut reported = 0;
        while let Some((entity, parent)) = pending.pop() {
            if visited == 4096 || reported == 64 {
                state.view.document.render_issues.push("Generated material diagnostics truncated (4096 entities / 64 warnings)".into());
                break;
            }
            visited += 1;
            let Ok((children, subset, part, instance, id, warning)) = generated.get(entity) else { continue };
            let label = if let Some(subset) = subset { format!("subset {}", subset.0) }
                else if let Some(part) = part { format!("prototype {}", part.0) }
                else if instance { id.map_or_else(|| "instance".into(), |id| format!("instance {}", id.0)) }
                else { continue };
            let label = if parent.is_empty() { label } else { format!("{parent} / {label}") };
            if let Some(warning) = warning {
                state.view.document.render_issues.push(format!("Material ({label}): {}", warning.0));
                reported += 1;
            }
            if let Some(children) = children {
                let remaining = 4097usize.saturating_sub(visited + pending.len());
                pending.extend(children.iter().rev().take(remaining).map(|child| (child, label.clone())));
            }
        }
    }
}

fn advance_editor_time(world: &mut World) {
    let Some(stage) = world.get_non_send::<EditorSession>().map(|editor| editor.stage().clone()) else { return };
    let delta = world.get_resource::<Time>().map_or(0.0, Time::delta_secs_f64);
    let current = world.resource::<crate::route::StageTime>().current;
    let current = world.resource_mut::<EditorPlayback>().0.advance(current, delta, &stage);
    world.resource_mut::<crate::route::StageTime>().current = current;
    let timeline = EditorTimeline { current, start: stage.start_time_code(), end: stage.end_time_code(),
        playing: world.resource::<EditorPlayback>().0.playing };
    let refresh = world.resource::<EditorBridge>().0.lock()
        .is_ok_and(|state| state.view.document.sample_time != Some(current));
    let snapshot = refresh.then(|| world.get_non_send::<EditorSession>().unwrap().snapshot_at(Some(current)));
    let bridge = world.resource::<EditorBridge>();
    if let Ok(mut state) = bridge.0.lock() {
        state.view.timeline = timeline;
        if let Some(snapshot) = snapshot {
            match snapshot {
                Ok(document) => state.view.document = document,
                Err(error) => state.view.status = format!("Inspection failed: {error:#}"),
            }
        }
    }
}

fn process_commands(world: &mut World) {
    let bridge = world.resource::<EditorBridge>().clone();
    let mut commands = match bridge.0.lock() {
        Ok(mut state) => std::mem::take(&mut state.commands),
        Err(_) => return,
    };
    let mut session = world.remove_non_send::<EditorSession>();
    if let Some(mut pending) = world.get_resource_mut::<PendingInitialOpen>() {
        if pending.retry && session.is_none() && !commands.iter().any(|command| matches!(command, EditorCommand::Open(_) | EditorCommand::OpenChecked { .. })) {
            commands.push_front(EditorCommand::Open(pending.path.clone()));
        }
        pending.retry = false;
    }
    let external = session.as_mut().is_some_and(EditorSession::synchronize_external_edits);
    if commands.is_empty() && !external {
        if let Some(session) = session { world.insert_non_send(session); }
        return;
    }
    let mut status = if external { "External edits detected; undo history reset".into() } else { String::new() };
    let mut texture_dirty = external;
    let mut inspect = external;
    for command in commands {
        inspect |= !matches!(&command, EditorCommand::ReloadSources(_));
        let command = if let EditorCommand::EditChecked { edit, document_id, revision, target } = command {
            texture_dirty |= session.as_mut().is_some_and(EditorSession::synchronize_external_edits);
            if session.as_ref().is_none_or(|editor| editor.document_id != document_id
                || editor.revision != revision || editor.stage.edit_target() != target) {
                status = "Failed: document or edit target changed before applying the edit; review and retry".into();
                continue;
            }
            EditorCommand::Edit(edit)
        } else { command };
        if let EditorCommand::OpenChecked { document_id, revision, .. } = &command {
            texture_dirty |= session.as_mut().is_some_and(EditorSession::synchronize_external_edits);
            let current = session.as_ref().map_or((0, 0), |editor| (editor.document_id, editor.revision));
            if current != (*document_id, *revision) {
                status = "Failed: document changed while choosing an open file; choose again".into();
                continue;
            }
        }
        let command = match command {
            EditorCommand::EditLayerChecked { identifier, document_id, revision } => {
                if session.as_ref().is_none_or(|editor| (editor.document_id, editor.revision) != (document_id, revision)) {
                    status = "Failed: document changed before selecting an edit layer; review and retry".into();
                    continue;
                }
                EditorCommand::EditLayer(identifier)
            }
            EditorCommand::PayloadChecked { prim, loaded, document_id, revision } => {
                if session.as_ref().is_none_or(|editor| (editor.document_id, editor.revision) != (document_id, revision)) {
                    status = "Failed: document changed before changing payload loading; review and retry".into();
                    continue;
                }
                EditorCommand::Payload { prim, loaded }
            }
            command => command,
        };
        let movements = session.as_ref().map_or_else(Vec::new, |editor| match &command {
            EditorCommand::Edit(edit) => edit.namespace_moves(),
            EditorCommand::Undo => editor.undo.last().map(|entry| entry.edit.namespace_moves().into_iter().rev().map(|(old, new)| (new, old)).collect()).unwrap_or_default(),
            EditorCommand::Redo => editor.redo.last().map(|entry| entry.edit.namespace_moves()).unwrap_or_default(),
            _ => Vec::new(),
        });
        if matches!(&command, EditorCommand::Edit(_) | EditorCommand::Undo | EditorCommand::Redo | EditorCommand::Payload { .. }) {
            texture_dirty = true;
        }
        let result = if let EditorCommand::Open(path) | EditorCommand::OpenChecked { filename: path, .. } = &command {
            let started = bevy::platform::time::Instant::now();
            let profiling = std::env::var_os("USD_PROFILE_LOADING").is_some();
            let profile = |phase: &str| {
                if profiling { eprintln!("editor_load phase={phase} elapsed_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0); }
            };
            world.remove_resource::<PendingInitialOpen>();
            if session.is_none() { set_texture_requests(world, Default::default()); }
            std::fs::read(path).map_err(anyhow::Error::from)
                .and_then(|bytes| crate::UsdSource::new(path, bytes).map_err(anyhow::Error::from)).and_then(|source| {
                    profile("source-bytes");
                    let (stage, disk_baselines) = source.open_stage_for_editor()?;
                    profile("stage-open");
                    crate::UsdSource::validate_composition(&stage)?;
                    profile("composition-validated");
                    let textures = match prepare_textures(&stage, &source) {
                        Ok(textures) => textures,
                        Err(error) => {
                            if session.is_none() {
                                let requests = crate::UsdSource::stage_texture_requests(&stage).map_err(anyhow::Error::msg)?;
                                set_texture_requests(world, texture_request_paths(&stage, &requests));
                                world.insert_resource(PendingInitialOpen { path: path.clone(), retry: false });
                            }
                            return Err(error);
                        }
                    };
                    if !textures.is_empty() && !world.contains_resource::<Assets<Image>>() {
                        anyhow::bail!("image assets are unavailable for this document");
                    }
                    profile("textures-decoded");
                    world.remove_non_send::<crate::live::LiveStage>();
                    if let Some(map) = world.remove_resource::<crate::live::PrimEntities>() {
                        if let Some(root) = map.entity("/") { world.despawn(root); }
                    }
                    world.insert_resource(crate::live::PrimEntities::default());
                    world.insert_non_send(crate::live::LiveStage::new(stage.clone()));
                    let mut editor = EditorSession::new(stage);
                    profile("editor-created");
                    editor.save_state.borrow_mut().disk = Some(disk_baselines);
                    editor.save_state.borrow_mut().opened(editor.stage(), &editor.layer_changes.revisions());
                    profile("save-baselines");
                    editor.source = Some(source);
                    install_textures(world, textures)?;
                    profile("textures-installed");
                    world.resource_mut::<EditorPlayback>().0.playing = false;
                    texture_dirty = false;
                    session = Some(editor);
                    Ok(())
                })
        } else if let Some(editor) = &mut session {
            match command {
                EditorCommand::ReloadSources(paths) => {
                    let started = bevy::platform::time::Instant::now();
                    let revision = editor.revision;
                    let mut published = false;
                    let result = editor.reload_paths(Some(&paths), |publication, stage| publication.preflight(world, stage)).map(|publication| {
                        if let Some(publication) = publication { published = true; publication.install(world); }
                    });
                    inspect |= published || result.is_err() || editor.revision != revision;
                    if std::env::var_os("USD_PROFILE_LOADING").is_some() {
                        eprintln!("editor_reload files={} elapsed_ms={:.3} success={}", paths.len(), started.elapsed().as_secs_f64()*1000.0, result.is_ok());
                    }
                    if let Some(mut status) = world.get_resource_mut::<reload::EditorReloadStatus>() {
                        status.error = result.as_ref().err().map(|error| format!("{error:#}"));
                    }
                    result
                }
                EditorCommand::Visibility { prim, visible } => {
                    let time = world.resource::<crate::route::StageTime>().current;
                    visibility_edit(editor.stage(), prim, visible, time).and_then(|edit| editor.edit(edit))
                        .map(|_| world.resource_mut::<EditorPlayback>().0.playing = false)
                }
                EditorCommand::RefreshTextures => editor.source.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("document has no source snapshot"))
                    .and_then(|source| refresh_textures(world, editor.stage(), source))
                    .map(|_| {
                        if let Some(live) = world.get_non_send::<crate::live::LiveStage>() { live.enqueue_resync("/"); }
                    }),
                EditorCommand::Select(path) => editor.select(path),
                EditorCommand::Edit(edit) => editor.edit(edit),
                EditorCommand::EditLayer(identifier) => editor.set_edit_layer(&identifier),
                EditorCommand::LayerMuteChecked { identifier, muted, document_id, revision } => {
                    if editor.document_id != document_id || editor.revision != revision {
                        Err(anyhow::anyhow!("document changed before changing layer muting; review and retry"))
                    } else {
                        editor.set_layer_muted(&identifier, muted).map(|_| {
                            texture_dirty = true;
                            if let Some(live) = world.get_non_send::<crate::live::LiveStage>() { live.enqueue_resync("/"); }
                        })
                    }
                }
                EditorCommand::Payload { prim, loaded } => editor.set_payload_loaded(&prim, loaded).map(|_| {
                    if let Some(live) = world.get_non_send::<crate::live::LiveStage>() {
                        live.enqueue_resync(&prim);
                    }
                }),
                EditorCommand::Undo => editor.undo().map(|_| ()),
                EditorCommand::Redo => editor.redo().map(|_| ()),
                EditorCommand::Seek(current) => {
                    if current.is_finite() {
                        world.resource_mut::<crate::route::StageTime>().current = current;
                        world.resource_mut::<EditorPlayback>().0.playing = false;
                        Ok(())
                    } else { Err(anyhow::anyhow!("time code must be finite")) }
                }
                EditorCommand::Play(playing) => { world.resource_mut::<EditorPlayback>().0.playing = playing; Ok(()) }
                EditorCommand::Save { filename, mode } => editor.save(&filename, mode),
                EditorCommand::SaveChecked { filename, mode, document_id, edit_layer } => {
                    if editor.document_id != document_id || editor.stage.edit_target().layer_identifier() != edit_layer {
                        Err(anyhow::anyhow!("document or edit layer changed while choosing a save destination; choose again"))
                    } else { editor.save(&filename, mode) }
                }
                EditorCommand::Open(_) | EditorCommand::OpenChecked { .. } | EditorCommand::EditChecked { .. }
                    | EditorCommand::EditLayerChecked { .. } | EditorCommand::PayloadChecked { .. } => unreachable!(),
            }
        } else {
            Err(anyhow::anyhow!("no USD document is open"))
        };
        if result.is_ok() {
            for (old, new) in movements { crate::live::remap_namespace(world, &old, &new); }
        }
        status = match result { Ok(()) => "Ready".into(), Err(error) => format!("Failed: {error:#}") };
    }
    if let Some(editor) = session.as_ref().filter(|_| texture_dirty) {
        if let Some(source) = &editor.source {
            match refresh_textures(world, editor.stage(), source) {
                Ok(()) => {
                    if let Some(live) = world.get_non_send::<crate::live::LiveStage>() { live.enqueue_resync("/"); }
                }
                Err(error) => status = format!("Texture loading failed: {error:#}"),
            }
        }
    }
    let time = world.resource::<crate::route::StageTime>().current;
    let started = world.contains_resource::<EditorSnapshotTiming>().then(bevy::platform::time::Instant::now);
    let inspector = session.as_ref().filter(|_| inspect || texture_dirty);
    let document = inspector.map(|session| session.snapshot_at(Some(time))).transpose();
    if inspector.is_some() && let Some(started) = started {
        let elapsed = started.elapsed();
        let mut timing = world.resource_mut::<EditorSnapshotTiming>();
        timing.snapshots += 1;
        timing.elapsed += elapsed;
        if std::env::var_os("USD_PROFILE_LOADING").is_some() {
            eprintln!("editor_snapshot elapsed_ms={:.3}", elapsed.as_secs_f64()*1000.0);
        }
    }
    if let Ok(mut state) = bridge.0.lock() {
        match document {
            Ok(Some(document)) => state.view.document = document,
            Ok(None) => {}
            Err(error) => status = format!("Inspection failed: {error:#}"),
        }
        state.view.status = status;
    }
    if let Some(session) = session { world.insert_non_send(session); }
}

type PreparedTextures = Vec<((String, bool), Image)>;

fn visibility_edit(stage: &Stage, prim: String, visible: bool, time: f64) -> anyhow::Result<EditorEdit> {
    anyhow::ensure!(time.is_finite(), "visibility edit time must be finite");
    let sampled = !stage.prim(openusd::sdf::path(&prim)?)?.attribute("visibility").time_sample_times()?.is_empty();
    let value = Value::Token(if visible { "inherited" } else { "invisible" }.into());
    Ok(if sampled {
        EditorEdit::AttributeSample { prim, name: "visibility".into(), type_name: "token".into(), value, time }
    } else {
        EditorEdit::Attribute { prim, name: "visibility".into(), type_name: "token".into(), value }
    })
}

#[derive(Resource, Default)]
pub(crate) struct EditorTextureRequests(pub std::collections::BTreeSet<String>);

#[derive(Resource)]
struct PendingInitialOpen {
    path: String,
    retry: bool,
}

fn set_texture_requests(world: &mut World, paths: std::collections::BTreeSet<String>) {
    if world.get_resource::<EditorTextureRequests>().is_none_or(|existing| existing.0 != paths) {
        world.insert_resource(EditorTextureRequests(paths));
    }
}

fn refresh_textures(world: &mut World, stage: &Stage, source: &crate::UsdSource) -> anyhow::Result<()> {
    let requests = crate::UsdSource::stage_texture_requests(stage).map_err(anyhow::Error::msg)?;
    set_texture_requests(world, texture_request_paths(stage, &requests));
    let textures = decode_textures(source, requests)?;
    install_textures(world, textures)
}

fn texture_request_paths(stage: &Stage, requests: &std::collections::BTreeSet<(String, bool)>) -> std::collections::BTreeSet<String> {
    let mut paths = std::collections::BTreeSet::new();
    for (path, _) in requests {
        if std::path::Path::new(path).is_absolute() || openusd::ar::is_package_relative_path(path) {
            paths.insert(path.clone());
        } else {
            for layer in stage.layer_identifiers() {
                if openusd::ar::is_package_relative_path(&layer) { continue; }
                let layer = std::path::Path::new(&layer);
                if layer.is_absolute() && let Some(parent) = layer.parent() {
                    paths.insert(parent.join(path).to_string_lossy().into_owned());
                }
            }
        }
    }
    paths
}

fn prepare_textures(stage: &Stage, source: &crate::UsdSource) -> anyhow::Result<PreparedTextures> {
    decode_textures(source, crate::UsdSource::stage_texture_requests(stage).map_err(anyhow::Error::msg)?)
}

fn decode_textures(source: &crate::UsdSource, requests: std::collections::BTreeSet<(String, bool)>) -> anyhow::Result<PreparedTextures> {
    let mut images = Vec::new();
    for (path, srgb) in requests {
        let bytes = source.read_asset(&path)
            .map_err(|error| anyhow::anyhow!("cannot read texture {path}: {error}"))?;
        let inner = openusd::ar::split_package_relative_path_inner(&path).map(|(_, inner)| inner).unwrap_or_else(|| path.clone());
        let extension = std::path::Path::new(&inner).extension().and_then(|extension| extension.to_str())
            .ok_or_else(|| anyhow::anyhow!("texture has no extension: {path}"))?;
        let image = Image::from_buffer(&bytes, bevy::image::ImageType::Extension(extension),
            bevy::image::CompressedImageFormats::NONE, srgb, bevy::image::ImageSampler::default(),
            bevy::asset::RenderAssetUsages::default())
            .map_err(|error| anyhow::anyhow!("cannot decode texture {path}: {error}"))?;
        images.push(((path, srgb), image));
    }
    Ok(images)
}

fn install_textures(world: &mut World, prepared: PreparedTextures) -> anyhow::Result<()> {
    anyhow::ensure!(prepared.is_empty() || world.contains_resource::<Assets<Image>>(), "image assets are unavailable for this document");
    set_texture_requests(world, prepared.iter().map(|((path, _), _)| path.clone()).collect());
    let old = world.remove_resource::<crate::asset::SnapshotTextures>().unwrap_or_default();
    let mut textures = crate::asset::SnapshotTextures::default();
    for (key, image) in prepared {
        let mut assets = world.resource_mut::<Assets<Image>>();
        let reused = old.0.get(&key).filter(|handle| assets.get(*handle).is_some_and(|existing|
            existing.data == image.data && existing.texture_descriptor.format == image.texture_descriptor.format
                && existing.texture_descriptor.size == image.texture_descriptor.size)).cloned();
        textures.0.insert(key, reused.unwrap_or_else(|| assets.add(image)));
    }
    world.insert_resource(textures);
    Ok(())
}

/// An undoable document edit.
#[derive(Debug, Clone)]
pub enum EditorEdit {
    /// Applies edits in order as one undoable command.
    Batch(Vec<EditorEdit>),
    /// Replaces the local stack with a static affine matrix and explicit reset state.
    TransformMatrix { prim: String, matrix: [f64; 16], reset: bool },
    AttributeSample { prim: String, name: String, type_name: String, value: Value, time: f64 },
    ClearAttributeSample { prim: String, name: String, time: f64 },
    ClearAttributeValues { prim: String, name: String },
    BlockAttributeValues { prim: String, name: String },
    Attribute { prim: String, name: String, type_name: String, value: Value },
    Variant { prim: String, set: String, selection: String },
    Define { path: String, type_name: String },
    Remove { path: String },
    Rename { path: String, name: String },
    Reparent { path: String, parent: String },
    Move { path: String, destination: String },
    References { prim: String, references: Vec<openusd::sdf::Reference> },
    ReferenceListOp { prim: String, operation: openusd::sdf::ReferenceListOp },
    ClearReferences { prim: String },
    Payloads { prim: String, payloads: Vec<openusd::sdf::Payload> },
    PayloadListOp { prim: String, operation: openusd::sdf::PayloadListOp },
    ClearPayloads { prim: String },
    RelationshipTargets { prim: String, name: String, targets: Vec<openusd::sdf::Path> },
    ClearRelationshipTargets { prim: String, name: String },
}

impl EditorEdit {
    fn namespace_moves(&self) -> Vec<(String, String)> {
        if let Self::Batch(edits) = self { return edits.iter().flat_map(Self::namespace_moves).collect(); }
        let old = match self {
            Self::Rename { path, .. } | Self::Reparent { path, .. } | Self::Move { path, .. } => path,
            _ => return Vec::new(),
        };
        self.selection_after(Some(old)).map(|new| vec![(old.clone(), new)]).unwrap_or_default()
    }

    fn apply(&self, stage: &Stage) -> anyhow::Result<()> {
        use crate::authoring;
        match self {
            Self::Batch(edits) => {
                for edit in edits { edit.apply(stage)?; }
                Ok(())
            }
            Self::TransformMatrix { prim, matrix, reset } => {
                let value = bevy::math::DMat4::from_cols_array(matrix);
                anyhow::ensure!(value.is_finite() && value.row(3) == bevy::math::DVec4::W
                    && value.as_mat4().is_finite(), "transform must be a finite affine matrix representable as f32");
                let owner = stage.prim(openusd::sdf::path(prim)?)?;
                anyhow::ensure!(owner.is_valid()?, "transform owner does not exist");
                let path = openusd::sdf::path(prim)?.append_property("xformOp:transform")?;
                let attribute = stage.create_attribute(path, "matrix4d")?;
                attribute.clear()?.set(Value::Matrix4d(openusd::gf::Matrix4d(*matrix)))?;
                let mut order = Vec::new();
                if *reset { order.push("!resetXformStack!".into()); }
                order.push("xformOp:transform".into());
                stage.create_attribute(openusd::sdf::path(prim)?.append_property("xformOpOrder")?, "token[]")?
                    .clear()?.set(Value::TokenVec(order))?;
                Ok(())
            }
            Self::AttributeSample { prim, name, type_name, value, time } =>
                authoring::set_attribute_sample(stage, prim, name, type_name, value.clone(), *time),
            Self::ClearAttributeSample { prim, name, time } => authoring::clear_attribute_sample(stage, prim, name, *time),
            Self::ClearAttributeValues { prim, name } => authoring::clear_attribute_values(stage, prim, name),
            Self::BlockAttributeValues { prim, name } => authoring::block_attribute_values(stage, prim, name),
            Self::Attribute { prim, name, type_name, value } =>
                authoring::set_attribute(stage, prim, name, type_name, value.clone()),
            Self::Variant { prim, set, selection } => {
                let choices = variant_choices(stage, prim)?;
                anyhow::ensure!(choices.get(set).is_some_and(|names| selection.is_empty() || names.contains(selection)),
                    "unknown variant {set}={selection} on {prim}");
                authoring::set_variant(stage, prim, set, selection)
            }
            Self::Define { path, type_name } => authoring::define_prim(stage, path, type_name),
            Self::Remove { path } => authoring::remove_prim(stage, path).map(|_| ()),
            Self::Rename { path, name } => authoring::rename_prim(stage, path, name),
            Self::Reparent { path, parent } => authoring::reparent_prim(stage, path, parent),
            Self::Move { path, destination } => authoring::move_prim(stage, path, destination),
            Self::References { prim, references } => authoring::set_references(stage, prim, references),
            Self::ReferenceListOp { prim, operation } => authoring::set_reference_list_op(stage, prim, operation),
            Self::ClearReferences { prim } => authoring::clear_references(stage, prim),
            Self::Payloads { prim, payloads } => authoring::set_payloads(stage, prim, payloads),
            Self::PayloadListOp { prim, operation } => authoring::set_payload_list_op(stage, prim, operation),
            Self::ClearPayloads { prim } => authoring::clear_payloads(stage, prim),
            Self::RelationshipTargets { prim, name, targets } => authoring::set_relationship_targets(stage, prim, name, targets),
            Self::ClearRelationshipTargets { prim, name } => authoring::clear_relationship_targets(stage, prim, name),
        }
    }

    fn selection_after(&self, selected: Option<&str>) -> Option<String> {
        if let Self::Batch(edits) = self {
            return edits.iter().fold(selected.map(String::from), |selected, edit| edit.selection_after(selected.as_deref()));
        }
        let selected = selected?;
        let (old, new) = match self {
            Self::Rename { path, name } => (path, Some(format!("{}/{name}", path.rsplit_once('/').map_or("", |(parent, _)| parent)))),
            Self::Reparent { path, parent } => (path, Some(format!("{}/{}", parent.trim_end_matches('/'), path.rsplit('/').next().unwrap_or("")))),
            Self::Move { path, destination } => (path, Some(destination.clone())),
            Self::Remove { path } => (path, None),
            _ => return Some(selected.into()),
        };
        if selected == old { return new; }
        if let Some(suffix) = selected.strip_prefix(old).filter(|suffix| suffix.starts_with('/')) {
            return new.map(|new| format!("{new}{suffix}"));
        }
        Some(selected.into())
    }
}

struct HistoryEntry {
    edit: EditorEdit,
    target: EditTarget,
    local_target: bool,
    transactions: usize,
    layers: std::collections::BTreeSet<String>,
    selection_before: Option<String>,
    selection_after: Option<String>,
}

/// Explicit output semantics for saving a USD document.
#[derive(Debug, Clone, Copy)]
pub enum SaveMode {
    RootLayer,
    EditLayer,
    Flattened,
}

#[derive(Debug, Clone)]
pub struct AttributeSnapshot {
    pub name: String,
    pub type_name: String,
    pub value: Option<Value>,
    pub sampled_matrix: Option<[f64; 16]>,
    pub source: String,
    pub source_summary: String,
    pub sample_times: Vec<f64>,
    pub blocked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct EditorSnapshot {
    pub document_id: u64,
    pub revision: u64,
    /// Authored commit counts per layer since this editor session opened; not saved-state flags.
    pub layer_revisions: std::collections::BTreeMap<String, u64>,
    pub layer_save_states: std::collections::BTreeMap<String, save_state::LayerSaveState>,
    pub sample_time: Option<f64>,
    pub asset_info: Option<crate::read::geom::CustomDict>,
    pub render_issues: Vec<String>,
    pub reflect_issues: Vec<crate::route::reflect::ReflectIssue>,
    pub relationships: Vec<(String, Vec<String>)>,
    pub payload_opinions: Vec<PayloadOpinion>,
    pub reference_opinions: Vec<ReferenceOpinion>,
    pub material_warnings: Vec<String>,
    pub prims: Vec<String>,
    pub visibility: std::collections::HashMap<String, bool>,
    pub layers: Vec<String>,
    pub root_layer: String,
    pub muted_layers: Vec<String>,
    pub mute_protected_layers: Vec<String>,
    pub edit_layer: String,
    pub edit_target: Option<EditTarget>,
    pub selected: Option<String>,
    pub attributes: Vec<AttributeSnapshot>,
    pub variants: Vec<(String, String)>,
    pub variant_choices: std::collections::BTreeMap<String, Vec<String>>,
    pub selected_loaded: Option<bool>,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Authored payload list operation at one contributing prim spec, strongest first.
#[derive(Debug, Clone)]
pub struct PayloadOpinion {
    pub layer: String,
    pub prim: openusd::sdf::Path,
    pub offset: openusd::sdf::LayerOffset,
    pub operation: openusd::sdf::PayloadListOp,
}

/// Authored reference list operation at one contributing prim spec, strongest first.
#[derive(Debug, Clone)]
pub struct ReferenceOpinion {
    pub layer: String,
    pub prim: openusd::sdf::Path,
    pub offset: openusd::sdf::LayerOffset,
    pub operation: openusd::sdf::ReferenceListOp,
}

impl EditorSnapshot {
    /// Captures the document revision and complete authoring target for a queued edit.
    pub fn checked_edit(&self, edit: EditorEdit) -> Option<EditorCommand> {
        Some(EditorCommand::EditChecked {
            edit, document_id: self.document_id, revision: self.revision, target: self.edit_target.clone()?,
        })
    }
}

/// Main-thread editor model. Clone `stage()` into `LiveStage` to project the
/// same document; route undoable authoring through `edit()`.
pub struct EditorSession {
    document_id: u64,
    revision: u64,
    source: Option<crate::UsdSource>,
    stage: UndoStage,
    selected: Option<String>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    history_limit: usize,
    layer_changes: layer_changes::LayerChanges,
    save_state: std::cell::RefCell<save_state::SaveState>,
}

impl EditorSession {
    pub fn new(stage: Stage) -> Self {
        static NEXT_DOCUMENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let layer_changes = layer_changes::LayerChanges::new(&stage);
        Self {
            document_id: NEXT_DOCUMENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            revision: 0,
            source: None,
            stage: UndoStage::with_capacity(stage, usize::MAX),
            selected: None,
            undo: Vec::new(),
            redo: Vec::new(),
            history_limit: 128,
            layer_changes,
            save_state: Default::default(),
        }
    }

    pub fn stage(&self) -> &Stage { &self.stage }

    /// Opens a source with resolver-byte baselines for guarded in-place saves.
    pub fn from_source(source: crate::UsdSource) -> anyhow::Result<Self> {
        let (stage, disk) = source.open_stage_for_editor()?;
        crate::UsdSource::validate_composition(&stage)?;
        let mut editor = Self::new(stage);
        editor.save_state.borrow_mut().disk = Some(disk);
        editor.save_state.borrow_mut().opened(editor.stage(), &editor.layer_changes.revisions());
        editor.source = Some(source);
        Ok(editor)
    }

    pub fn document_id(&self) -> u64 { self.document_id }

    /// Maximum retained undo and redo commands; defaults to 128.
    pub fn history_limit(&self) -> usize { self.history_limit }

    /// Retains newest undo commands, then nearest redo commands. Zero disables history.
    /// Limits command count, not diff bytes or transactions inside an active command.
    pub fn set_history_limit(&mut self, limit: usize) {
        self.synchronize_external_edits();
        self.history_limit = limit;
        self.trim_history();
    }

    fn trim_history(&mut self) {
        let excess = self.undo.len().saturating_sub(self.history_limit);
        let transactions = self.undo.drain(..excess).map(|entry| entry.transactions).sum();
        self.stage.discard_oldest(transactions);
        let excess = (self.undo.len() + self.redo.len()).saturating_sub(self.history_limit);
        self.redo.drain(..excess);
    }

    pub fn select(&mut self, path: Option<String>) -> anyhow::Result<()> {
        if let Some(path) = &path {
            let path = openusd::sdf::path(path)?;
            anyhow::ensure!(self.stage.prim(path)?.is_valid()?, "selected prim does not exist");
        }
        self.selected = path;
        Ok(())
    }

    pub fn set_edit_layer(&self, identifier: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.stage.is_layer_muted(identifier), "cannot edit a muted layer; unmute it first");
        self.stage.set_edit_target(EditTarget::for_layer(identifier))?;
        Ok(())
    }

    /// Changes runtime layer participation without authoring layer contents or adding undo commands.
    pub fn set_layer_muted(&mut self, identifier: &str, muted: bool) -> anyhow::Result<()> {
        self.synchronize_external_edits();
        anyhow::ensure!(self.stage.layer(identifier).is_some() || self.stage.is_layer_muted(identifier), "unknown layer");
        if muted {
            anyhow::ensure!(self.stage.root_layer().identifier() != identifier, "cannot mute the root layer");
            anyhow::ensure!(self.stage.edit_target().layer_identifier() != identifier, "cannot mute the active edit layer");
            anyhow::ensure!(!self.stage.sub_layers(identifier).iter().any(|layer| layer == self.stage.edit_target().layer_identifier()),
                "cannot mute a sublayer ancestor of the active edit layer");
        }
        if self.stage.is_layer_muted(identifier) == muted { return Ok(()); }
        if muted { self.stage.mute_layer(identifier); } else { self.stage.unmute_layer(identifier); }
        anyhow::ensure!(self.stage.is_layer_muted(identifier) == muted, "layer muting did not change");
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    /// Changes runtime load rules without authoring layer opinions.
    pub fn set_payload_loaded(&mut self, path: &str, loaded: bool) -> anyhow::Result<()> {
        self.synchronize_external_edits();
        let path = openusd::sdf::path(path)?;
        anyhow::ensure!(self.stage.prim(path.clone())?.is_valid()?, "payload prim does not exist");
        let before = self.stage.load_rules();
        if loaded { self.stage.load(path, openusd::usd::LoadPolicy::WithDescendants)?; }
        else { self.stage.unload(path)?; }
        if self.stage.load_rules() != before { self.revision = self.revision.wrapping_add(1); }
        Ok(())
    }

    pub fn edit(&mut self, edit: EditorEdit) -> anyhow::Result<()> {
        self.synchronize_external_edits();
        let target = self.stage.edit_target();
        let local_target = self.stage.layer_stack().iter().any(|layer| layer == target.layer_identifier());
        let before = self.stage.undo_depth();
        if let Err(error) = edit.apply(&self.stage) {
            while self.stage.undo_depth() > before { self.stage.undo()?; }
            return Err(error);
        }
        let transactions = self.stage.undo_depth() - before;
        if transactions > 0 {
            self.revision = self.revision.wrapping_add(1);
            let selection_before = self.selected.clone();
            self.selected = edit.selection_after(self.selected.as_deref());
            let layers = self.stage.recent_layer_identifiers(transactions);
            self.undo.push(HistoryEntry { edit, target, local_target, transactions, layers, selection_before, selection_after: self.selected.clone() });
            self.redo.clear();
            self.trim_history();
        }
        Ok(())
    }

    pub fn undo(&mut self) -> anyhow::Result<bool> {
        self.synchronize_external_edits();
        let Some(entry) = self.undo.last_mut() else { return Ok(false) };
        anyhow::ensure!(!self.stage.is_layer_muted(entry.target.layer_identifier()), "unmute {} before undo", entry.target.layer_identifier());
        anyhow::ensure!(!entry.local_target || self.stage.layer_stack().iter().any(|layer| layer == entry.target.layer_identifier()),
            "restore layer participation for {} before undo", entry.target.layer_identifier());
        while entry.transactions > 0 {
            anyhow::ensure!(self.stage.undo()?, "editor transaction history is inconsistent");
            entry.transactions -= 1;
            self.revision = self.revision.wrapping_add(1);
        }
        if entry.selection_before != entry.selection_after { self.selected = entry.selection_before.clone(); }
        self.redo.push(self.undo.pop().unwrap());
        Ok(true)
    }

    pub fn redo(&mut self) -> anyhow::Result<bool> {
        self.synchronize_external_edits();
        let Some(entry) = self.redo.last() else { return Ok(false) };
        anyhow::ensure!(!self.stage.is_layer_muted(entry.target.layer_identifier()), "unmute {} before redo", entry.target.layer_identifier());
        anyhow::ensure!(!entry.local_target || self.stage.layer_stack().iter().any(|layer| layer == entry.target.layer_identifier()),
            "restore layer participation for {} before redo", entry.target.layer_identifier());
        let target = self.stage.edit_target();
        self.stage.set_edit_target(entry.target.clone())?;
        let before = self.stage.undo_depth();
        let result = entry.edit.apply(&self.stage);
        if result.is_err() {
            while self.stage.undo_depth() > before { self.stage.undo()?; }
        }
        self.stage.set_edit_target(target)?;
        result?;
        let mut entry = self.redo.pop().unwrap();
        entry.transactions = self.stage.undo_depth() - before;
        entry.layers = self.stage.recent_layer_identifiers(entry.transactions);
        if entry.selection_before != entry.selection_after { self.selected = entry.selection_after.clone(); }
        self.undo.push(entry);
        self.revision = self.revision.wrapping_add(1);
        self.trim_history();
        Ok(true)
    }

    /// Establishes a new history baseline when edits bypass the command model.
    pub fn synchronize_external_edits(&mut self) -> bool {
        let tracked: usize = self.undo.iter().map(|entry| entry.transactions).sum();
        if tracked == self.stage.undo_depth() { return false; }
        self.revision = self.revision.wrapping_add(1);
        self.stage.reset();
        self.undo.clear();
        self.redo.clear();
        true
    }

    pub fn save(&self, filename: &str, mode: SaveMode) -> anyhow::Result<()> {
        let key = crate::persistence::destination_identity(std::path::Path::new(filename))?;
        let disk = self.save_state.borrow().disk.clone();
        let expected = disk.as_ref().and_then(|disk| disk.lock().expect("disk baselines").get(&key).copied());
        let published = match mode {
            SaveMode::RootLayer => crate::persistence::export_layer(&self.stage, &self.stage.root_layer(), filename, expected)?,
            SaveMode::EditLayer => {
                let target = self.stage.edit_target();
                let layer = self.stage.layer(target.layer_identifier())
                    .ok_or_else(|| anyhow::anyhow!("edit layer is unavailable"))?;
                crate::persistence::export_layer(&self.stage, &layer, filename, expected)?
            }
            SaveMode::Flattened => {
                crate::UsdSource::validate_composition(&self.stage)?;
                crate::persistence::export_layer(&self.stage, &crate::persistence::flatten::preserving_instances(&self.stage)?, filename, expected)?
            }
        };
        let disk = disk.unwrap_or_default();
        disk.lock().expect("disk baselines").insert(key, published);
        self.save_state.borrow_mut().disk = Some(disk);
        let saved_layer = match mode {
            SaveMode::RootLayer => Some(self.stage.root_layer().identifier().to_string()),
            SaveMode::EditLayer => Some(self.stage.edit_target().layer_identifier().to_string()),
            SaveMode::Flattened => None,
        };
        if let Some(layer) = saved_layer { self.save_state.borrow_mut().saved_to_source(&self.stage, &layer, filename); }
        Ok(())
    }

    pub fn snapshot(&self) -> anyhow::Result<EditorSnapshot> {
        self.snapshot_at(None)
    }

    /// Inspects defaults and optionally samples scalar matrix attributes at scene time.
    pub fn snapshot_at(&self, time: Option<f64>) -> anyhow::Result<EditorSnapshot> {
        anyhow::ensure!(time.is_none_or(f64::is_finite), "inspection time must be finite");
        let mut snapshot = EditorSnapshot {
            document_id: self.document_id,
            revision: self.revision,
            layer_revisions: self.layer_changes.revisions(),
            sample_time: time,
            layers: self.stage.layer_stack(),
            root_layer: self.stage.root_layer().identifier().to_string(),
            muted_layers: self.stage.muted_layers(),
            edit_layer: self.stage.edit_target().layer_identifier().to_string(),
            edit_target: Some(self.stage.edit_target()),
            selected: self.selected.clone(),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            ..Default::default()
        };
        snapshot.mute_protected_layers = snapshot.layers.iter().filter(|layer| {
            *layer == &snapshot.root_layer || *layer == &snapshot.edit_layer
                || self.stage.sub_layers(layer).contains(&snapshot.edit_layer)
        }).cloned().collect();
        snapshot.layer_save_states = self.save_state.borrow_mut().states(&self.stage, &snapshot.layer_revisions, self.layer_changes.structural_revision());
        let predicate = openusd::usd::PrimPredicate::new(
            openusd::usd::PrimStatus::ACTIVE.union(openusd::usd::PrimStatus::DEFINED),
            openusd::usd::PrimStatus::ABSTRACT,
        ).with_instance_proxies(true);
        self.stage.traverse(predicate, |path: &openusd::sdf::Path| {
            snapshot.prims.push(path.as_str().to_owned());
        })?;
        for path in &snapshot.prims {
            let value = self.stage.prim(openusd::sdf::path(path)?)?.attribute("visibility")
                .get_at::<Value>(time.map(openusd::usd::TimeCode::new))?;
            snapshot.visibility.insert(path.clone(), !matches!(value, Some(Value::Token(token)) if token.as_str() == "invisible"));
        }
        if let Some(path) = &self.selected {
            if !snapshot.prims.contains(path) { snapshot.selected = None; return Ok(snapshot); }
            let prim = self.stage.prim(openusd::sdf::path(path)?)?;
            for site in prim.prim_stack()? {
                let layer = self.stage.layer(&site.layer)
                    .ok_or_else(|| anyhow::anyhow!("composition opinion layer is unavailable"))?;
                if let Some(value) = layer.data().try_field(&site.path, "references")? {
                    if let Value::ReferenceListOp(operation) = value.as_ref() {
                        snapshot.reference_opinions.push(ReferenceOpinion {
                            layer: site.layer.clone(), prim: site.path.clone(), offset: site.offset, operation: operation.clone(),
                        });
                    }
                }
                if let Some(value) = layer.data().try_field(&site.path, "payload")? {
                    if let Value::PayloadListOp(operation) = value.as_ref() {
                        snapshot.payload_opinions.push(PayloadOpinion {
                            layer: site.layer, prim: site.path, offset: site.offset, operation: operation.clone(),
                        });
                    }
                }
            }
            snapshot.asset_info = crate::read::geom::read_asset_info(&self.stage, prim.path())?;
            let material = if prim.type_name()?.as_deref() == Some("Material") {
                Some(openusd::sdf::path(path)?)
            } else { crate::read::shade::read_material_binding(&self.stage, &openusd::sdf::path(path)?)? };
            if let Some(material) = material {
                match crate::read::shade::read_preview_material(&self.stage, &material) {
                    Ok(Some(read)) => snapshot.material_warnings = read.warnings,
                    Ok(None) => snapshot.material_warnings.push("Material surface is not supported".into()),
                    Err(error) => snapshot.material_warnings.push(format!("Material read failed: {error:#}")),
                }
            }
            snapshot.variants = prim.variant_sets().get_all_variant_selections()?;
            snapshot.variant_choices = variant_choices(&self.stage, path)?;
            snapshot.selected_loaded = Some(prim.is_loaded()?);
            for relationship in prim.relationships()? {
                snapshot.relationships.push((relationship.path().as_str().rsplit('.').next().unwrap_or_default().into(),
                    relationship.targets()?.into_iter().map(|path| path.to_string()).collect()));
            }
            snapshot.relationships.sort_by(|a, b| a.0.cmp(&b.0));
            for attribute in prim.attributes()? {
                let info = attribute.resolve_info_at(None)?;
                let type_name = attribute.type_name()?.map(|ty| ty.to_string()).unwrap_or_default();
                let sampled_matrix = if type_name == "matrix4d" && time.is_some() {
                    match attribute.get_at::<Value>(time.map(openusd::usd::TimeCode::new))? {
                        Some(Value::Matrix4d(matrix)) => Some(matrix.0),
                        _ => None,
                    }
                } else { None };
                snapshot.attributes.push(AttributeSnapshot {
                    name: attribute.path().as_str().rsplit('.').next().unwrap_or_default().to_string(),
                    type_name,
                    value: attribute.get::<Value>()?,
                    sampled_matrix,
                    source: format!("{:?}: {:?}", info.source(), info.node()),
                    source_summary: match info.node() {
                        Some(node) => format!("{:?} / {:?}\n{}", info.source(), node.arc(), node.path()),
                        None => format!("{:?}", info.source()),
                    },
                    sample_times: attribute.time_sample_times()?,
                    blocked: info.value_is_blocked(),
                });
            }
            snapshot.attributes.sort_by(|a,b| a.name.cmp(&b.name));
        }
        Ok(snapshot)
    }
}

fn variant_choices(stage: &Stage, path: &str) -> anyhow::Result<std::collections::BTreeMap<String, Vec<String>>> {
    let prim = stage.prim(openusd::sdf::path(path)?)?;
    let mut sets: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
    for site in prim.prim_stack()? {
        let layer = stage.layer(&site.layer).ok_or_else(|| anyhow::anyhow!("variant layer is unavailable"))?;
        let Some(names) = layer.data().try_field(&site.path, "variantSetChildren")? else { continue };
        let Value::TokenVec(names) = names.as_ref() else { continue };
        for name in names {
            let options = sets.entry(name.as_str().to_string()).or_default();
            let set_path = site.path.append_variant_selection(name.as_str(), "")?;
            if let Some(children) = layer.data().try_field(&set_path, "variantChildren")? {
                if let Value::TokenVec(children) = children.as_ref() {
                    options.extend(children.iter().map(|name| name.as_str().to_string()));
                }
            }
        }
    }
    Ok(sets.into_iter().map(|(name, options)| (name, options.into_iter().collect())).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_provenance_preserves_variant_sites_custom_data_and_time() {
        use super::*;
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
class Xform "Model" {}
def Xform "Source" (
    variants = { string choice = "a" }
    prepend variantSets = "choice"
) {
    variantSet "choice" = {
        "a" ( prepend references = </Model> (customData = { string label = "inner" }) ) {}
    }
}
"#[..]).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
def Xform "Root" ( prepend references = @weak.usda@</Source> (offset = 10; scale = 2) ) {}
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let mut editor = EditorSession::new(root.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let opinions = editor.snapshot().unwrap().reference_opinions;
        assert_eq!(opinions.len(), 2);
        assert!(opinions[0].layer.ends_with("root.usda"));
        assert_eq!(opinions[0].prim.as_str(), "/Root");
        assert_eq!(opinions[0].offset, openusd::sdf::LayerOffset::default());
        assert_eq!(opinions[0].operation.prepended_items[0].asset_path, "weak.usda");
        assert!(opinions[1].layer.ends_with("weak.usda"));
        assert_eq!(opinions[1].prim.as_str(), "/Source{choice=a}");
        assert_eq!(opinions[1].offset, openusd::sdf::LayerOffset::new(10., 2.));
        assert_eq!(opinions[1].operation.prepended_items[0].prim_path.as_str(), "/Model");
        assert_eq!(opinions[1].operation.prepended_items[0].custom_data.get("label"), Some(&openusd::sdf::Value::String("inner".into())));
        editor.edit(EditorEdit::References { prim: "/Root".into(), references: vec![] }).unwrap();
        let blocked = editor.snapshot().unwrap().reference_opinions;
        assert_eq!(blocked.len(), 1);
        assert!(blocked[0].operation.explicit);
        assert!(blocked[0].operation.explicit_items.is_empty());
        editor.undo().unwrap();
        assert_eq!(editor.snapshot().unwrap().reference_opinions.len(), 2);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        editor.select(None).unwrap();
        assert!(editor.snapshot().unwrap().reference_opinions.is_empty());
    }

    fn assert_reference_custom_data(stage: &openusd::usd::Stage) {
        let field = stage.root_layer().data().try_field(&openusd::sdf::path("/Root").unwrap(), "references").unwrap().unwrap().into_owned();
        let openusd::sdf::Value::ReferenceListOp(op) = field else { panic!("reference list op") };
        assert_eq!(op.prepended_items[0].custom_data.get("label"), Some(&openusd::sdf::Value::String("retained".into())));
        assert!(matches!(op.prepended_items[0].custom_data.get("settings"), Some(openusd::sdf::Value::Dictionary(values)) if values.get("version") == Some(&openusd::sdf::Value::Int(3))));
        assert_eq!(op.prepended_items[0].layer_offset, openusd::sdf::LayerOffset::new(10., 2.));
        assert_eq!(stage.prim("/Root/Shape").unwrap().type_name().unwrap().as_deref(), Some("Cube"));
    }

    #[test]
    fn reference_custom_data_survives_all_root_export_formats() {
        use super::*;
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/reference_custom_data.usda");
        let source = crate::UsdSource::new(input, std::fs::read(input).unwrap()).unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        let original = editor.stage().root_layer().export_to_string().unwrap();
        let directory = tempfile::tempdir().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let exported = directory.path().join(format!("exported.{extension}"));
            editor.save(exported.to_str().unwrap(), SaveMode::RootLayer).unwrap();
            let reread = crate::UsdSource::new(&exported, std::fs::read(&exported).unwrap()).unwrap().open_stage().unwrap();
            assert_reference_custom_data(&reread);
            if extension == "usdz" {
                let portable = crate::UsdSource::snapshot("relocated/reference.usdz", std::fs::read(&exported).unwrap()).unwrap().open_stage().unwrap();
                assert_reference_custom_data(&portable);
            }
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), original);
        }
    }

    #[test]
    #[ignore = "requires native usdcat"]
    fn native_reference_custom_data_round_trip() {
        use super::*;
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/reference_custom_data.usda");
        let source = crate::UsdSource::new(input, std::fs::read(input).unwrap()).unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        let directory = tempfile::tempdir().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let exported = directory.path().join(format!("exported.{extension}"));
            let native = directory.path().join(format!("native-{extension}.usda"));
            editor.save(exported.to_str().unwrap(), SaveMode::RootLayer).unwrap();
            let mut command = std::process::Command::new("usdcat");
            command.arg(&exported).arg("--out").arg(&native);
            if extension == "usdz" { command.arg("--flatten"); }
            let result = command.output().unwrap();
            assert!(result.status.success(), "{extension}: {}", String::from_utf8_lossy(&result.stderr));
            let reread = crate::UsdSource::new(&native, std::fs::read(&native).unwrap()).unwrap().open_stage().unwrap();
            if extension == "usdz" {
                assert_eq!(reread.prim("/Root/Shape").unwrap().type_name().unwrap().as_deref(), Some("Cube"));
            } else { assert_reference_custom_data(&reread); }
        }
    }

    #[test]
    fn reference_list_ops_preserve_composition_custom_data_and_history() {
        use super::*;
        use openusd::sdf::{Reference, ReferenceListOp};
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
class Xform "A" { double score = 1 }
class Xform "B" { double score = 2 }
class Xform "C" { double score = 3 }
def Xform "Root" ( prepend references = [</A>, </B>] ) {}
"#[..]).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
(subLayers = [@weak.usda@])
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let mut editor = EditorSession::new(root.open_stage().unwrap());
        let reference = |path: &str| Reference { prim_path: openusd::sdf::path(path).unwrap(), ..Default::default() };
        let score = |stage: &Stage| stage.prim("/Root").unwrap().attribute("score").get::<f64>().unwrap();
        let baseline = editor.stage().root_layer().export_to_string().unwrap();
        let mut annotated = reference("/C");
        annotated.custom_data.insert("label".into(), openusd::sdf::Value::String("retained".into()));
        for (operation, expected) in [
            (ReferenceListOp::prepended([annotated]), Some(3.)),
            (ReferenceListOp::appended([reference("/C")]), Some(1.)),
            (ReferenceListOp::added([reference("/C")]), Some(1.)),
            (ReferenceListOp::deleted([reference("/A")]), Some(2.)),
            (ReferenceListOp::ordered([reference("/B"), reference("/A")]), Some(2.)),
            (ReferenceListOp::explicit([]), None),
            (ReferenceListOp { deleted_items: vec![reference("/A")], prepended_items: vec![reference("/C")], ..Default::default() }, Some(3.)),
        ] {
            editor.edit(EditorEdit::ReferenceListOp { prim: "/Root".into(), operation: operation.clone() }).unwrap();
            assert_eq!(score(editor.stage()), expected, "{operation:?}");
            let authored = editor.stage().root_layer().export_to_string().unwrap();
            let reopened = crate::UsdSource::snapshot("saved.usda", authored.as_bytes()).unwrap()
                .with_dependency(&weak).unwrap().open_stage().unwrap();
            assert_eq!(score(&reopened), expected);
            let field = reopened.root_layer().data().try_field(&openusd::sdf::path("/Root").unwrap(), "references").unwrap().unwrap().into_owned();
            assert_eq!(field, openusd::sdf::Value::ReferenceListOp(operation));
            editor.undo().unwrap();
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), baseline);
            editor.redo().unwrap();
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), authored);
            editor.undo().unwrap();
        }
    }

    #[test]
    fn explicit_identity_payload_delete_matches_native_fixture() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_identity.usda");
        let bytes = std::fs::read(path).unwrap();
        let source = crate::UsdSource::new(path, bytes.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        assert!(stage.prim("/Root").unwrap().is_valid().unwrap());
        assert!(!stage.prim("/Root/Shape").unwrap().is_valid().unwrap());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn payload_list_ops_compose_undo_and_reopen_without_flattening() {
        use super::*;
        use openusd::sdf::{Payload, PayloadListOp};
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
class Xform "A" { double score = 1 }
class Xform "B" { double score = 2 }
class Xform "C" { double score = 3 }
def Xform "Root" ( prepend payload = [</A>, </B>] ) {}
"#[..]).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
(subLayers = [@weak.usda@])
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let mut editor = EditorSession::new(root.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let payload = |path: &str| Payload { prim_path: openusd::sdf::path(path).unwrap(), ..Default::default() };
        let score = |stage: &Stage| stage.prim("/Root").unwrap().attribute("score").get::<f64>().unwrap();
        let baseline = editor.stage().root_layer().export_to_string().unwrap();
        for (operation, expected) in [
            (PayloadListOp::prepended([payload("/C")]), Some(3.)),
            (PayloadListOp::appended([payload("/C")]), Some(1.)),
            (PayloadListOp::added([payload("/C")]), Some(1.)),
            (PayloadListOp::deleted([payload("/A")]), Some(2.)),
            (PayloadListOp::deleted([Payload { layer_offset: Some(openusd::sdf::LayerOffset::default()), ..payload("/A") }]), Some(2.)),
            (PayloadListOp::ordered([payload("/B"), payload("/A")]), Some(2.)),
            (PayloadListOp::ordered([Payload { layer_offset: Some(openusd::sdf::LayerOffset::default()), ..payload("/B") }, payload("/A")]), Some(2.)),
            (PayloadListOp::explicit([]), None),
            (PayloadListOp { deleted_items: vec![payload("/A")], prepended_items: vec![payload("/C")], ..Default::default() }, Some(3.)),
        ] {
            editor.edit(EditorEdit::PayloadListOp { prim: "/Root".into(), operation: operation.clone() }).unwrap();
            assert_eq!(score(editor.stage()), expected, "{operation:?}");
            assert_eq!(editor.snapshot().unwrap().payload_opinions[0].operation, operation);
            let authored = editor.stage().root_layer().export_to_string().unwrap();
            let reopened = crate::UsdSource::snapshot("saved.usda", authored.as_bytes()).unwrap()
                .with_dependency(&weak).unwrap().open_stage().unwrap();
            assert_eq!(score(&reopened), expected);
            editor.undo().unwrap();
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), baseline);
            assert_eq!(score(editor.stage()), Some(1.));
            editor.redo().unwrap();
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), authored);
            editor.undo().unwrap();
        }
    }

    #[test]
    fn payload_provenance_preserves_reference_and_variant_spec_mapping() {
        use super::*;
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
class Xform "Model" {}
def Xform "Source" (
    variants = { string choice = "a" }
    prepend variantSets = "choice"
) {
    variantSet "choice" = {
        "a" ( prepend payload = </Model> ) {}
    }
}
"#[..]).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
def Xform "Root" ( prepend references = @weak.usda@</Source> (offset = 10; scale = 2) ) {}
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let mut editor = EditorSession::new(root.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let opinions = editor.snapshot().unwrap().payload_opinions;
        assert_eq!(opinions.len(), 1);
        assert!(opinions[0].layer.ends_with("weak.usda"));
        assert_eq!(opinions[0].prim.as_str(), "/Source{choice=a}");
        assert_eq!(opinions[0].offset, openusd::sdf::LayerOffset::new(10., 2.));
        assert_eq!(opinions[0].operation.prepended_items[0].prim_path.as_str(), "/Model");
    }

    #[test]
    fn payload_provenance_retains_weaker_ops_and_authored_anchors() {
        use super::*;
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_authoring.usda");
        let source = crate::UsdSource::new(path, std::fs::read(path).unwrap()).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let original = editor.snapshot().unwrap().payload_opinions;
        assert_eq!(original.len(), 1);
        assert!(original[0].layer.ends_with("payload_authoring_weak.usda"));
        assert_eq!(original[0].prim.as_str(), "/Root");
        assert!(!original[0].operation.explicit);
        assert_eq!(original[0].operation.prepended_items[0].asset_path, "payload_authoring_content.usda");
        assert_eq!(original[0].operation.prepended_items[0].prim_path.as_str(), "/Box");
        editor.edit(EditorEdit::Payloads { prim: "/Root".into(), payloads: vec![] }).unwrap();
        let blocked = editor.snapshot().unwrap().payload_opinions;
        assert_eq!(blocked.len(), 2);
        assert!(blocked[0].layer.ends_with("payload_authoring.usda"));
        assert!(blocked[0].operation.explicit);
        assert!(blocked[0].operation.explicit_items.is_empty());
        assert_eq!(blocked[1].operation, original[0].operation);
        editor.edit(EditorEdit::ClearPayloads { prim: "/Root".into() }).unwrap();
        assert_eq!(editor.snapshot().unwrap().payload_opinions.len(), 1);
        editor.undo().unwrap();
        assert_eq!(editor.snapshot().unwrap().payload_opinions.len(), 2);
        editor.select(None).unwrap();
        assert!(editor.snapshot().unwrap().payload_opinions.is_empty());
    }

    #[test]
    fn checked_open_rejects_edits_and_replacements_but_allows_current_context() {
        use super::*;
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.usda").to_string_lossy().into_owned();
        let second = directory.path().join("second.usda").to_string_lossy().into_owned();
        std::fs::write(&first, "#usda 1.0\ndef Xform \"First\" {}\n").unwrap();
        std::fs::write(&second, "#usda 1.0\ndef Xform \"Second\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        let open = |filename: &str, snapshot: &EditorSnapshot| EditorCommand::OpenChecked {
            filename: filename.into(), document_id: snapshot.document_id, revision: snapshot.revision,
        };
        app.world_mut().insert_resource(PendingInitialOpen { path: second.clone(), retry: true });
        bridge.send(open(&first, &EditorSnapshot::default())).unwrap();
        app.update();
        let initial = bridge.view().unwrap().document;
        assert_ne!(initial.document_id, 0);
        assert!(initial.prims.contains(&"/First".into()));
        app.world_mut().non_send_mut::<EditorSession>().set_history_limit(0);
        bridge.send(EditorCommand::Edit(EditorEdit::Define { path: "/Unsaved".into(), type_name: "Xform".into() })).unwrap();
        bridge.send(open(&second, &initial)).unwrap();
        app.update();
        let edited = bridge.view().unwrap();
        assert!(edited.status.contains("document changed"));
        assert_eq!(edited.document.document_id, initial.document_id);
        assert!(edited.document.prims.contains(&"/Unsaved".into()));
        assert!(!edited.document.can_undo);
        assert_ne!(edited.document.revision, initial.revision);
        let stage = app.world().non_send::<EditorSession>().stage().clone();
        stage.define_prim("/External").unwrap();
        bridge.send(open(&second, &edited.document)).unwrap();
        app.update();
        let external = bridge.view().unwrap();
        assert!(external.status.contains("document changed"));
        assert!(external.document.prims.contains(&"/External".into()));
        bridge.send(EditorCommand::Select(Some("/First".into()))).unwrap();
        bridge.send(EditorCommand::Seek(10.0)).unwrap();
        bridge.send(open(&second, &external.document)).unwrap();
        app.update();
        let replaced = bridge.view().unwrap();
        assert_eq!(replaced.status, "Ready");
        assert_ne!(replaced.document.document_id, initial.document_id);
        assert!(replaced.document.prims.contains(&"/Second".into()));
        bridge.send(open(&first, &external.document)).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.contains("document changed"));
        assert_eq!(bridge.view().unwrap().document.document_id, replaced.document.document_id);
        bridge.send(open(&directory.path().join("missing.usda").to_string_lossy(), &replaced.document)).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().document.document_id, replaced.document.document_id);
        bridge.send(EditorCommand::Open(first)).unwrap();
        app.update();
        assert!(bridge.view().unwrap().document.prims.contains(&"/First".into()));
    }

    #[test]
    fn payload_list_authoring_retimes_and_roundtrips_history() {
        let stage = crate::UsdSource::snapshot("payload-authoring.usda", &br#"#usda 1.0
class Xform "Model" {
    double score.timeSamples = {0: 1, 10: 3}
}
def Xform "Instance" {}
"#[..]).unwrap().open_stage().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let before = stage.root_layer().export_to_string().unwrap();
        let payload = openusd::sdf::Payload {
            prim_path: openusd::sdf::path("/Model").unwrap(),
            layer_offset: Some(openusd::sdf::LayerOffset::new(10.,2.)), ..Default::default()
        };
        editor.edit(EditorEdit::Payloads { prim: "/Instance".into(), payloads: vec![payload.clone()] }).unwrap();
        for (time, expected) in [(10.,1.), (20.,2.), (30.,3.)] {
            assert_eq!(stage.prim("/Instance").unwrap().attribute("score")
                .get_at::<f64>(Some(openusd::usd::TimeCode::new(time))).unwrap(), Some(expected));
        }
        let authored = stage.root_layer().export_to_string().unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), authored);
        for scale in [0., -1., f64::NAN, f64::INFINITY] {
            let invalid = openusd::sdf::Payload { layer_offset: Some(openusd::sdf::LayerOffset::new(0.,scale)), ..payload.clone() };
            assert!(editor.edit(EditorEdit::Payloads { prim: "/Instance".into(), payloads: vec![payload.clone(), invalid] }).is_err());
            assert_eq!(stage.root_layer().export_to_string().unwrap(), authored);
        }
        editor.edit(EditorEdit::Payloads { prim: "/Instance".into(), payloads: vec![] }).unwrap();
        assert!(stage.prim("/Instance").unwrap().attribute("score").get::<f64>().unwrap().is_none());
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), authored);
        editor.edit(EditorEdit::ClearPayloads { prim: "/Instance".into() }).unwrap();
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), authored);
    }

    #[test]
    fn document_revision_tracks_undo_redo_not_failed_or_empty_edits() {
        use super::*;
        let stage = Stage::builder().in_memory("revision.usda").unwrap();
        let mut editor = EditorSession::new(stage);
        editor.edit(EditorEdit::Batch(Vec::new())).unwrap();
        assert_eq!(editor.snapshot().unwrap().revision, 0);
        editor.edit(EditorEdit::Define { path: "/A".into(), type_name: "Xform".into() }).unwrap();
        let authored = editor.snapshot().unwrap().revision;
        assert!(authored > 0);
        assert!(editor.edit(EditorEdit::TransformMatrix { prim: "/Missing".into(), matrix: [0.0;16], reset: false }).is_err());
        assert_eq!(editor.snapshot().unwrap().revision, authored);
        editor.undo().unwrap();
        let undone = editor.snapshot().unwrap().revision;
        assert!(undone > authored);
        editor.redo().unwrap();
        assert!(editor.snapshot().unwrap().revision > undone);
    }
    #[test]
    fn default_history_limit_bounds_retained_transactions() {
        use super::*;
        let stage = Stage::builder().in_memory("history-retention.usda").unwrap();
        stage.define_prim("/Model").unwrap();
        let mut editor = EditorSession::new(stage.clone());
        for value in 0..512 {
            editor.edit(EditorEdit::Attribute { prim: "/Model".into(), name: "counter".into(),
                type_name: "int".into(), value: Value::Int(value) }).unwrap();
            assert!(editor.undo.len() <= 128);
            assert_eq!(editor.stage.undo_depth(), editor.undo.iter().map(|entry| entry.transactions).sum::<usize>());
        }
        assert_eq!(editor.undo.len(), 128);
        for _ in 0..128 { assert!(editor.undo().unwrap()); }
        assert!(!editor.undo().unwrap());
        assert_eq!(stage.prim("/Model").unwrap().attribute("counter").get::<Value>().unwrap(), Some(Value::Int(383)));
        for _ in 0..128 { assert!(editor.redo().unwrap()); }
        assert!(!editor.redo().unwrap());
        assert_eq!(stage.prim("/Model").unwrap().attribute("counter").get::<Value>().unwrap(), Some(Value::Int(511)));
    }
    #[test]
    fn history_limit_evicts_whole_commands_after_atomic_edits() {
        use super::*;
        let stage = Stage::builder().in_memory("bounded-history.usda").unwrap();
        let mut editor = EditorSession::new(stage.clone());
        assert_eq!(editor.history_limit(), 128);
        editor.set_history_limit(2);
        let define = |path: &str| EditorEdit::Define { path: path.into(), type_name: "Xform".into() };
        editor.edit(define("/Permanent")).unwrap();
        editor.edit(EditorEdit::Batch(vec![define("/A"), define("/B")])).unwrap();
        assert!(editor.undo.last().unwrap().transactions > 1);
        editor.edit(define("/Newest")).unwrap();
        assert_eq!(editor.undo.len(), 2);
        assert!(editor.undo().unwrap());
        assert!(editor.undo().unwrap());
        assert!(!editor.undo().unwrap());
        assert!(stage.prim("/Permanent").unwrap().is_valid().unwrap());
        for path in ["/A", "/B", "/Newest"] { assert!(!stage.prim(path).unwrap().is_valid().unwrap()); }
        assert!(editor.redo().unwrap());
        assert!(editor.redo().unwrap());
        assert!(!editor.redo().unwrap());
        editor.set_history_limit(1);
        let before = stage.root_layer().export_to_string().unwrap();
        let depth = editor.stage.undo_depth();
        let mut edits = (0..32).map(|i| define(&format!("/Temporary{i}"))).collect::<Vec<_>>();
        edits.push(EditorEdit::TransformMatrix { prim: "/Missing".into(), matrix: [0.0; 16], reset: false });
        assert!(editor.edit(EditorEdit::Batch(edits.clone())).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert_eq!(editor.stage.undo_depth(), depth);
        assert_eq!(editor.undo.len(), 1);
        editor.set_history_limit(0);
        assert!(!editor.undo().unwrap());
        assert!(!editor.redo().unwrap());
        assert_eq!(editor.stage.undo_depth(), 0);
        assert!(editor.edit(EditorEdit::Batch(edits)).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        editor.edit(define("/Untracked")).unwrap();
        assert_eq!(editor.stage.undo_depth(), 0);
        assert!(!editor.undo().unwrap());
        assert!(stage.prim("/Untracked").unwrap().is_valid().unwrap());
    }

    #[test]
    fn history_limit_preserves_nearest_redo_and_external_baselines() {
        use super::*;
        let stage = Stage::builder().in_memory("redo-limit.usda").unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let define = |path: &str| EditorEdit::Define { path: path.into(), type_name: "Xform".into() };
        for path in ["/A", "/B", "/C", "/D"] { editor.edit(define(path)).unwrap(); }
        for _ in 0..3 { assert!(editor.undo().unwrap()); }
        editor.set_history_limit(2);
        assert_eq!(editor.redo.len(), 1);
        assert!(editor.redo().unwrap());
        assert!(!editor.redo().unwrap());
        assert!(stage.prim("/B").unwrap().is_valid().unwrap());
        assert!(!stage.prim("/C").unwrap().is_valid().unwrap());
        stage.define_prim("/External").unwrap();
        editor.set_history_limit(1);
        assert!(!editor.undo().unwrap());
        editor.edit(define("/Recent")).unwrap();
        assert!(editor.undo().unwrap());
        assert!(stage.prim("/External").unwrap().is_valid().unwrap());
        assert!(stage.prim("/B").unwrap().is_valid().unwrap());
    }
    #[test]
    fn visibility_commands_edit_samples_or_defaults_and_undo() {
        let source = crate::UsdSource::snapshot("visibility.usda", include_bytes!("../../../assets/visibility_animation.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.insert_non_send(EditorSession::new(stage.clone()));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Seek(10.0)).unwrap();
        app.update();
        assert!(!bridge.view().unwrap().document.visibility["/Animated"]);
        bridge.send(EditorCommand::Play(true)).unwrap();
        bridge.send(EditorCommand::Visibility { prim: "/Animated".into(), visible: true }).unwrap();
        app.update();
        let view = bridge.view().unwrap();
        assert_eq!(view.status, "Ready");
        assert_eq!(view.timeline.current, 10.0);
        assert!(!view.timeline.playing);
        assert!(view.document.visibility["/Animated"]);
        let attr = stage.prim("/Animated").unwrap().attribute("visibility");
        assert_eq!(attr.time_sample_times().unwrap(), vec![0.0, 10.0, 20.0]);
        assert_eq!(attr.get::<Value>().unwrap(), Some(Value::Token("inherited".into())));
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        assert!(!bridge.view().unwrap().document.visibility["/Animated"]);
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        bridge.send(EditorCommand::Redo).unwrap();
        app.update();
        assert!(bridge.view().unwrap().document.visibility["/Animated"]);
        bridge.send(EditorCommand::Visibility { prim: "/Visible".into(), visible: false }).unwrap();
        app.update();
        assert!(!bridge.view().unwrap().document.visibility["/Visible"]);
        assert!(stage.prim("/Visible").unwrap().attribute("visibility").time_sample_times().unwrap().is_empty());
        for _ in 0..2 { bridge.send(EditorCommand::Undo).unwrap(); app.update(); }
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(visibility_edit(&stage, "/Animated".into(), true, f64::NAN).is_err());
    }
    #[test]
    fn failed_texture_refresh_retains_images_and_records_requested_paths() {
        use super::*;
        let directory = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::snapshot(directory.path().join("scene.usda"), br#"#usda 1.0
def DomeLight "Sky" { asset inputs:texture:file = @missing.png@ }
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<Image>::default());
        install_textures(&mut world, vec![(("previous.png".into(), false), Image::default())]).unwrap();
        let previous = world.resource::<crate::asset::SnapshotTextures>().0.clone();
        let error = refresh_textures(&mut world, &stage, &source).unwrap_err();
        assert!(error.to_string().contains("missing.png"));
        assert_eq!(world.resource::<crate::asset::SnapshotTextures>().0, previous);
        assert_eq!(world.resource::<EditorTextureRequests>().0,
            [directory.path().join("missing.png").to_string_lossy().into_owned()].into());
        install_textures(&mut world, Vec::new()).unwrap();
        assert!(world.resource::<EditorTextureRequests>().0.is_empty());
        assert!(world.resource::<crate::asset::SnapshotTextures>().0.is_empty());
    }

    use super::*;

    #[test]
    fn sampled_matrix_inspection_maps_scene_time_without_authoring() {
        let stage = crate::UsdSource::new("sampled-matrix-inspection.usda", include_bytes!("../../../assets/xform_animation.usda").as_slice())
            .unwrap().open_stage().unwrap();
        stage.define_prim("/M").unwrap().set_type_name("Xform").unwrap();
        crate::authoring::set_references(&stage, "/M", &[openusd::sdf::Reference {
            prim_path: openusd::sdf::path("/Affine").unwrap(),
            layer_offset: openusd::sdf::LayerOffset { offset: 10.0, scale: 2.0 },
            ..Default::default()
        }]).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        editor.select(Some("/M".into())).unwrap();
        let defaults = editor.snapshot().unwrap();
        assert_eq!(defaults.sample_time, None);
        assert!(defaults.attributes.iter().all(|attribute| attribute.sampled_matrix.is_none()));
        for (time, shear) in [(10.0, 0.0), (15.0, 0.5), (20.0, 1.0)] {
            let snapshot = editor.snapshot_at(Some(time)).unwrap();
            assert_eq!(snapshot.sample_time, Some(time));
            let attribute = snapshot.attributes.iter().find(|attribute| attribute.name == "xformOp:transform").unwrap();
            assert_eq!(attribute.value, None);
            assert_eq!(attribute.sample_times, vec![10.0, 20.0, 30.0]);
            assert_eq!(attribute.sampled_matrix.unwrap(), [1.0,0.0,0.5 * shear,0.0, 0.75 * shear,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]);
        }
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(editor.snapshot_at(Some(invalid)).is_err());
        }
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(!editor.snapshot().unwrap().can_undo);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.insert_non_send(editor);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Seek(15.0)).unwrap();
        app.update();
        let published = bridge.view().unwrap();
        assert_eq!(published.document.sample_time, Some(15.0));
        assert_eq!(published.timeline.current, 15.0);
        let sampled = published.document.attributes.iter().find(|attribute| attribute.name == "xformOp:transform").unwrap().sampled_matrix.unwrap();
        assert_eq!(sampled[2], 0.25);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(100)));
        app.world_mut().resource_mut::<EditorPlayback>().0.range = Some((10.0, 30.0));
        bridge.send(EditorCommand::Play(true)).unwrap();
        app.update();
        let published = bridge.view().unwrap();
        assert!(published.timeline.current > 15.0);
        assert_eq!(published.document.sample_time, Some(published.timeline.current));
        let sampled = published.document.attributes.iter().find(|attribute| attribute.name == "xformOp:transform").unwrap().sampled_matrix.unwrap();
        assert!((sampled[2] - (published.timeline.current - 10.0) * 0.05).abs() < 1e-6);
        assert!(!published.document.can_undo);
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn matrix_edit_restores_exact_animated_stack_on_undo() {
        let stage = crate::UsdSource::new("matrix-edit.usda", include_bytes!("../../../assets/xform_animation.usda").as_slice())
            .unwrap().open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut editor = EditorSession::new(stage);
        let matrix = [1.0,0.0,0.5,0.0, 0.75,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 3.0,4.0,5.0,1.0];
        editor.edit(EditorEdit::TransformMatrix { prim: "/Affine".into(), matrix, reset: true }).unwrap();
        let path = openusd::sdf::path("/Affine").unwrap();
        for time in [0.0, 2.5, 5.0, 10.0] {
            let (actual, reset) = crate::read::xform::read_transform_stack_at(editor.stage(), &path, Some(time)).unwrap().unwrap();
            assert_eq!(actual, matrix.map(|value| value as f32));
            assert!(reset);
        }
        let after = editor.stage().root_layer().export_to_string().unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), after);
        assert!(editor.undo().unwrap());
        for invalid in [f64::NAN, f64::INFINITY, f64::MAX] {
            let mut bad = matrix;
            bad[0] = invalid;
            assert!(editor.edit(EditorEdit::TransformMatrix { prim: "/Affine".into(), matrix: bad, reset: false }).is_err());
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        }
        let mut projective = matrix;
        projective[3] = 0.25;
        assert!(editor.edit(EditorEdit::TransformMatrix { prim: "/Affine".into(), matrix: projective, reset: false }).is_err());
        assert!(editor.edit(EditorEdit::TransformMatrix { prim: "/Missing".into(), matrix, reset: false }).is_err());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), after);
    }

    #[test]
    fn matrix_edit_undo_reveals_weaker_animation_without_changing_its_layer() {
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/xform_animation.usda");
        let stage = crate::UsdSource::new("matrix-overlay.usda", format!("#usda 1.0\n( subLayers = [@{fixture}@] )\n").into_bytes())
            .unwrap().open_stage().unwrap();
        let root_before = stage.root_layer().export_to_string().unwrap();
        let path = openusd::sdf::path("/Affine").unwrap();
        let original = crate::read::xform::read_transform_stack_at(&stage, &path, Some(5.0)).unwrap();
        let layers_before: Vec<_> = stage.layer_identifiers().into_iter().map(|identifier| {
            let text = stage.layer(&identifier).unwrap().export_to_string().unwrap();
            (identifier, text)
        }).collect();
        let mut editor = EditorSession::new(stage);
        let matrix = bevy::math::DMat4::from_translation(bevy::math::DVec3::new(9.0, 8.0, 7.0)).to_cols_array();
        editor.edit(EditorEdit::TransformMatrix { prim: "/Affine".into(), matrix, reset: false }).unwrap();
        assert_eq!(crate::read::xform::read_transform_stack_at(editor.stage(), &path, Some(5.0)).unwrap(), Some((matrix.map(|v| v as f32), false)));
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), root_before);
        assert_eq!(crate::read::xform::read_transform_stack_at(editor.stage(), &path, Some(5.0)).unwrap(), original);
        for (identifier, before) in layers_before {
            let layer = editor.stage().layer(&identifier).unwrap();
            assert_eq!(layer.export_to_string().unwrap(), before);
        }
    }

    #[test]
    fn matrix_edit_maps_reference_target_and_survives_save_reopen() {
        let stage = crate::UsdSource::new("mapped-matrix.usda", br#"#usda 1.0
def Xform "Source" {
    double3 xformOp:translate.timeSamples = {0: (1,2,3), 10: (4,5,6)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "M" {}
"#.as_slice()).unwrap().open_stage().unwrap();
        crate::authoring::set_references(&stage, "/M", &[openusd::sdf::Reference {
            prim_path: openusd::sdf::path("/Source").unwrap(),
            layer_offset: openusd::sdf::LayerOffset { offset: 10.0, scale: 2.0 },
            ..Default::default()
        }]).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let path = openusd::sdf::path("/M").unwrap();
        let root_target = stage.edit_target();
        stage.set_edit_target(stage.edit_target_for_node(&path, openusd::usd::EditTargetArc::Reference).unwrap()).unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let matrix = [1.0,0.0,0.5,0.0, 0.75,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 3.0,4.0,5.0,1.0];
        editor.edit(EditorEdit::TransformMatrix { prim: "/M".into(), matrix, reset: true }).unwrap();
        assert!(!stage.root_layer().data().has_spec(&path.append_property("xformOp:transform").unwrap()));
        let source_path = openusd::sdf::path("/Source").unwrap();
        assert!(stage.root_layer().data().has_spec(&source_path.append_property("xformOp:transform").unwrap()));
        let after = stage.root_layer().export_to_string().unwrap();
        stage.set_edit_target(root_target.clone()).unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(stage.edit_target(), root_target);
        assert_eq!(stage.root_layer().export_to_string().unwrap(), after);
        let directory = tempfile::tempdir().unwrap();
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            for extension in ["usda", "usdc", "usd"] {
                let output = directory.path().join(format!("{name}.{extension}"));
                editor.save(output.to_str().unwrap(), mode).unwrap();
                let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(output.to_str().unwrap()).unwrap();
                for prim in [&path, &source_path] {
                    for time in [0.0, 10.0, 20.0, 30.0] {
                        assert_eq!(crate::read::xform::read_transform_stack_at(&reopened, prim, Some(time)).unwrap(), Some((matrix.map(|v| v as f32), true)));
                    }
                }
            }
        }
    }

    #[test]
    fn sample_edits_map_reference_time_and_preserve_defaults() {
        let stage = crate::UsdSource::new("sample-edits.usda", br#"#usda 1.0
def Xform "Source" { double score = 5 }
def Xform "M" {}
"#.as_slice()).unwrap().open_stage().unwrap();
        crate::authoring::set_references(&stage, "/M", &[openusd::sdf::Reference {
            prim_path: openusd::sdf::Path::new("/Source").unwrap(),
            layer_offset: openusd::sdf::LayerOffset { offset: 10.0, scale: 2.0 },
            ..Default::default()
        }]).unwrap();
        let path = openusd::sdf::Path::new("/M").unwrap();
        let target = stage.edit_target_for_node(&path, openusd::usd::EditTargetArc::Reference).unwrap();
        stage.set_edit_target(target).unwrap();
        let mut editor = EditorSession::new(stage.clone());
        editor.selected = Some("/M".into());
        let snapshot = editor.snapshot().unwrap();
        let score = snapshot.attributes.iter().find(|attribute| attribute.name == "score").unwrap();
        assert_eq!(score.source_summary, "Default / Reference\n/Source");
        assert!(score.source.contains("ResolveNode"));
        let source = stage.attribute("/Source.score").unwrap();
        let composed = stage.attribute("/M.score").unwrap();
        editor.edit(EditorEdit::AttributeSample { prim: "/M".into(), name: "score".into(),
            type_name: "double".into(), value: Value::Double(9.0), time: 14.0 }).unwrap();
        assert_eq!(source.time_samples().unwrap().unwrap(), vec![(2.0, Value::Double(9.0))]);
        assert_eq!(composed.time_samples().unwrap().unwrap(), vec![(14.0, Value::Double(9.0))]);
        assert_eq!(editor.snapshot().unwrap().attributes.iter().find(|attribute| attribute.name == "score").unwrap().sample_times, vec![14.0]);
        assert_eq!(composed.get::<f64>().unwrap(), Some(5.0));
        assert_eq!(composed.get_at::<f64>(Some(openusd::usd::TimeCode::new(14.0))).unwrap(), Some(9.0));
        assert!(!stage.root_layer().data().has_spec(&openusd::sdf::Path::new("/M.score").unwrap()));
        editor.undo().unwrap();
        assert!(source.time_samples().unwrap().unwrap_or_default().is_empty());
        editor.redo().unwrap();
        assert_eq!(source.time_samples().unwrap().unwrap(), vec![(2.0, Value::Double(9.0))]);
        for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let before = editor.stage.undo_depth();
            assert!(editor.edit(EditorEdit::AttributeSample { prim: "/M".into(), name: "newValue".into(),
                type_name: "double".into(), value: Value::Double(1.0), time }).is_err());
            assert_eq!(editor.stage.undo_depth(), before);
            assert!(!stage.root_layer().data().has_spec(&openusd::sdf::Path::new("/Source.newValue").unwrap()));
        }
        editor.edit(EditorEdit::ClearAttributeSample { prim: "/M".into(), name: "score".into(), time: 14.0 }).unwrap();
        assert!(source.time_samples().unwrap().unwrap_or_default().is_empty());
        assert_eq!(source.get::<f64>().unwrap(), Some(5.0));
        editor.undo().unwrap();
        assert_eq!(source.time_samples().unwrap().unwrap(), vec![(2.0, Value::Double(9.0))]);
        editor.redo().unwrap();
        assert!(source.time_samples().unwrap().unwrap_or_default().is_empty());
    }

    #[test]
    fn attribute_value_block_and_clear_are_undoable_without_removing_metadata() {
        let stage = crate::UsdSource::new("value-ops.usda", br#"#usda 1.0
over "M" {
    double score = 5 ( documentation = "keep this metadata" )
    double score.timeSamples = { 0: 8, 10: 10 }
}
"#.as_slice()).unwrap().open_stage().unwrap();
        let weak = openusd::sdf::Layer::from_bytes("value-ops-weak.usda", br#"#usda 1.0
def Xform "M" { double score = 2 }
"#.to_vec()).unwrap();
        let root = stage.root_layer().identifier().to_string();
        stage.insert_layer(&root, 0, weak, openusd::sdf::LayerOffset::default()).unwrap();
        let attribute = stage.attribute("/M.score").unwrap();
        let mut editor = EditorSession::new(stage.clone());
        editor.select(Some("/M".into())).unwrap();
        editor.edit(EditorEdit::BlockAttributeValues { prim: "/M".into(), name: "score".into() }).unwrap();
        assert!(attribute.get::<f64>().unwrap().is_none());
        assert!(attribute.get_at::<f64>(Some(openusd::usd::TimeCode::new(5.0))).unwrap().is_none());
        assert!(editor.snapshot().unwrap().attributes.iter().find(|attr| attr.name == "score").unwrap().blocked);
        editor.undo().unwrap();
        assert_eq!(attribute.get::<f64>().unwrap(), Some(5.0));
        assert_eq!(attribute.get_at::<f64>(Some(openusd::usd::TimeCode::new(5.0))).unwrap(), Some(9.0));
        editor.redo().unwrap();
        assert!(attribute.get::<f64>().unwrap().is_none());
        editor.edit(EditorEdit::ClearAttributeValues { prim: "/M".into(), name: "score".into() }).unwrap();
        assert_eq!(attribute.get::<f64>().unwrap(), Some(2.0));
        assert_eq!(attribute.get_at::<f64>(Some(openusd::usd::TimeCode::new(5.0))).unwrap(), Some(2.0));
        assert_eq!(attribute.get_metadata::<String>("documentation").unwrap().as_deref(), Some("keep this metadata"));
        assert!(!editor.snapshot().unwrap().attributes.iter().find(|attr| attr.name == "score").unwrap().blocked);
        editor.undo().unwrap();
        assert!(attribute.get::<f64>().unwrap().is_none());
        editor.redo().unwrap();
        assert_eq!(attribute.get::<f64>().unwrap(), Some(2.0));
        editor.edit(EditorEdit::BlockAttributeValues { prim: "/M".into(), name: "score".into() }).unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/M".into(), name: "score".into(),
            type_name: "double".into(), value: Value::Double(42.0) }).unwrap();
        assert_eq!(attribute.get::<f64>().unwrap(), Some(42.0));
        editor.undo().unwrap();
        assert!(attribute.get::<f64>().unwrap().is_none());
        editor.redo().unwrap();
        assert_eq!(attribute.get::<f64>().unwrap(), Some(42.0));
        assert_eq!(stage.edit_target().layer_identifier(), root);
    }

    #[test]
    fn asset_info_composes_layers_and_follows_editor_selection() {
        let stage = crate::UsdSource::new("asset-info.usda", br#"#usda 1.0
over "Model" (
    assetInfo = { string name = "strong" dictionary details = { string version = "2" } }
) {}
def Xform "Empty" {}
"#.as_slice()).unwrap().open_stage().unwrap();
        let weak = openusd::sdf::Layer::from_bytes("asset-info-weak.usda", br#"#usda 1.0
def Xform "Model" (
    assetInfo = {
        string name = "weak"
        asset identifier = @model.usda@
        dictionary details = { string version = "1" string author = "Artist" }
    }
) {}
"#.to_vec()).unwrap();
        let root = stage.root_layer().identifier().to_string();
        stage.insert_layer(&root, 0, weak, openusd::sdf::LayerOffset::default()).unwrap();
        let path = openusd::sdf::Path::new("/Model").unwrap();
        let info = crate::read::geom::read_asset_info(&stage, &path).unwrap().unwrap();
        assert_eq!(info.get("name").unwrap().as_str(), Some("strong"));
        assert_eq!(info.get("identifier").unwrap().as_str(), Some("model.usda"));
        assert_eq!(info.get_nested("details.version").unwrap().as_str(), Some("2"));
        assert_eq!(info.get_nested("details.author").unwrap().as_str(), Some("Artist"));
        assert_eq!(info.entries.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["details", "identifier", "name"]);
        let mut editor = EditorSession::new(stage);
        editor.select(Some("/Model".into())).unwrap();
        assert_eq!(editor.snapshot().unwrap().asset_info, Some(info));
        editor.select(Some("/Empty".into())).unwrap();
        assert!(editor.snapshot().unwrap().asset_info.is_none());
        editor.select(None).unwrap();
        assert!(editor.snapshot().unwrap().asset_info.is_none());
    }

    #[test]
    fn selected_projection_issues_publish_and_clear() {
        use crate::route::reflect::{ReflectIssue, ReflectIssueKind, UsdReflectIssues};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.init_resource::<crate::live::PrimEntities>();
        let issue = ReflectIssue { type_segment: "Health".into(), field: Some("current".into()), kind: ReflectIssueKind::UnsupportedValue };
        let entity = app.world_mut().spawn(UsdReflectIssues(vec![issue.clone()])).id();
        app.world_mut().resource_mut::<crate::live::PrimEntities>().insert("/Prim", entity);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.0.lock().unwrap().view.document.selected = Some("/Prim".into());
        app.update();
        assert_eq!(bridge.view().unwrap().document.reflect_issues, vec![issue.clone()]);
        app.world_mut().entity_mut(entity).remove::<UsdReflectIssues>();
        app.update();
        assert!(bridge.view().unwrap().document.reflect_issues.is_empty());
        app.world_mut().entity_mut(entity).insert(UsdReflectIssues(vec![issue]));
        bridge.0.lock().unwrap().view.document.selected = Some("/Missing".into());
        app.update();
        assert!(bridge.view().unwrap().document.reflect_issues.is_empty());
    }

    #[test]
    fn checked_save_rejects_stale_document_and_layer_without_touching_output() {
        let source = crate::UsdSource::new("checked.usda", &b"#usda 1.0\ndef Scope \"Model\" {}\n"[..]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("saved.usda");
        std::fs::write(&output, "original").unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        let snapshot = editor.snapshot().unwrap();
        let replacement = EditorSession::new(source.open_stage().unwrap());
        assert_ne!(snapshot.document_id, replacement.snapshot().unwrap().document_id);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.insert_non_send(editor);
        let bridge = app.world().resource::<EditorBridge>().clone();
        let send_save = |document_id, edit_layer: String| bridge.send(EditorCommand::SaveChecked {
            filename: output.to_string_lossy().into_owned(), mode: SaveMode::RootLayer, document_id, edit_layer,
        }).unwrap();
        send_save(snapshot.document_id, "different-layer.usda".into());
        app.update();
        assert!(bridge.view().unwrap().status.contains("document or edit layer changed"));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "original");
        app.insert_non_send(replacement);
        send_save(snapshot.document_id, snapshot.edit_layer.clone());
        app.update();
        assert!(bridge.view().unwrap().status.contains("document or edit layer changed"));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "original");
        let current = app.world().get_non_send::<EditorSession>().unwrap().snapshot().unwrap();
        send_save(current.document_id, current.edit_layer);
        app.update();
        assert_eq!(bridge.view().unwrap().status, "Ready");
        let reopened = crate::UsdSource::new(&output, std::fs::read(&output).unwrap()).unwrap().open_stage().unwrap();
        assert_eq!(reopened.prim("/Model").unwrap().type_name().unwrap().as_deref(), Some("Scope"));
    }

    #[test]
    fn source_save_baselines_reject_external_edits_and_follow_own_saves() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let weak = directory.path().join("weak.usda");
        let initial = "#usda 1.0\n(subLayers = [@weak.usda@])\n";
        std::fs::write(&root, initial).unwrap();
        std::fs::write(&weak, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let editor = EditorSession::from_source(crate::UsdSource::new(&root, initial.as_bytes()).unwrap()).unwrap();
        let weak_id = editor.stage().layer_identifiers().into_iter().find(|id| id.ends_with("weak.usda")).unwrap();
        editor.set_edit_layer(&weak_id).unwrap();
        std::fs::write(&weak, "#usda 1.0\ndef Xform \"External\" {}\n").unwrap();
        let error = editor.save(weak.to_str().unwrap(), SaveMode::EditLayer).unwrap_err();
        assert!(error.to_string().contains("loaded document"), "{error:#}");
        assert!(std::fs::read_to_string(&weak).unwrap().contains("External"));
        editor.save(root.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        editor.save(root.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let copy = directory.path().join("copy.usda");
        editor.save(copy.to_str().unwrap(), SaveMode::EditLayer).unwrap();
        editor.save(copy.to_str().unwrap(), SaveMode::EditLayer).unwrap();
        std::fs::write(&copy, "external copy").unwrap();
        assert!(editor.save(copy.to_str().unwrap(), SaveMode::EditLayer).is_err());
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "external copy");
        std::fs::remove_file(&root).unwrap();
        assert!(editor.save(root.to_str().unwrap(), SaveMode::RootLayer).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn source_save_baseline_tracks_outer_package_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usdz");
        let mut archive = openusd::usdz::ArchiveWriter::new(std::io::Cursor::new(Vec::new()));
        archive.add_layer("root.usda", b"#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        std::fs::write(&root, &bytes).unwrap();
        let editor = EditorSession::from_source(crate::UsdSource::new(&root, bytes).unwrap()).unwrap();
        editor.save(root.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        editor.save(root.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let external = b"external package replacement";
        std::fs::write(&root, external).unwrap();
        let error = editor.save(root.to_str().unwrap(), SaveMode::RootLayer).unwrap_err();
        assert!(error.to_string().contains("loaded document"), "{error:#}");
        assert_eq!(std::fs::read(&root).unwrap(), external);
    }

    #[test]
    fn source_save_baseline_uses_supplied_root_bytes_not_later_disk_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let source = crate::UsdSource::new(&root, b"#usda 1.0\ndef Xform \"Original\" {}\n".as_slice()).unwrap();
        std::fs::write(&root, "#usda 1.0\ndef Xform \"External\" {}\n").unwrap();
        let editor = EditorSession::from_source(source).unwrap();
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer, SaveMode::Flattened] {
            assert!(editor.save(root.to_str().unwrap(), mode).is_err());
        }
        assert!(std::fs::read_to_string(&root).unwrap().contains("External"));
    }

    #[test]
    fn layer_save_states_follow_content_and_only_source_saves() {
        use save_state::LayerSaveState::{Clean, Modified, Unknown};
        let directory = tempfile::tempdir().unwrap();
        let root_file = directory.path().join("root.usda");
        let weak_file = directory.path().join("weak.usda");
        std::fs::write(&root_file, "#usda 1.0\n(subLayers = [@weak.usda@])\nover \"Root\" { point3f[] points = [(0,0,0), (1,0,0)] }\n").unwrap();
        std::fs::write(&weak_file, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let source = crate::UsdSource::new(&root_file, std::fs::read(&root_file).unwrap()).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        assert!(editor.snapshot().unwrap().layer_save_states.values().all(|state| *state == Unknown));
        editor.save_state.borrow_mut().opened(editor.stage(), &editor.layer_changes.revisions());
        let initial = editor.snapshot().unwrap();
        assert!(initial.layer_save_states.values().all(|state| *state == Clean));
        let root = initial.root_layer;
        let weak = initial.layers.into_iter().find(|id| id != &root).unwrap();
        let edit = || EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(4.) };
        editor.edit(edit()).unwrap();
        editor.set_edit_layer(&weak).unwrap();
        editor.edit(edit()).unwrap();
        let both = editor.snapshot().unwrap().layer_save_states;
        assert_eq!(both[&root], Modified);
        assert_eq!(both[&weak], Modified);
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer, SaveMode::Flattened] {
            editor.save(directory.path().join("copy.usda").to_str().unwrap(), mode).unwrap();
            assert_eq!(editor.snapshot().unwrap().layer_save_states, both);
        }
        assert!(editor.save(directory.path().join("missing/out.usda").to_str().unwrap(), SaveMode::RootLayer).is_err());
        assert_eq!(editor.snapshot().unwrap().layer_save_states, both);
        editor.save(root_file.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&root], Clean);
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Modified);
        editor.undo().unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Clean);
        editor.redo().unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Modified);
        editor.save(weak_file.to_str().unwrap(), SaveMode::EditLayer).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Clean);
        editor.undo().unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Modified);
        editor.redo().unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&weak], Clean);
        editor.set_edit_layer(&root).unwrap();
        editor.set_layer_muted(&weak, true).unwrap();
        assert!(editor.snapshot().unwrap().layer_save_states.values().all(|state| *state == Clean));
        editor.stage().prim("/Root").unwrap().attribute("score").set(9_f64).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&root], Modified);
        editor.stage().prim("/Root").unwrap().attribute("score").set(4_f64).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&root], Clean);
        let points = editor.stage().prim("/Root").unwrap().attribute("points");
        points.clone().set(Value::Vec3fVec(vec![[0.0, 0.0, 0.0].into(), [1.0, 0.5, 0.0].into()])).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&root], Modified);
        points.set(Value::Vec3fVec(vec![[0.0, 0.0, 0.0].into(), [1.0, 0.0, 0.0].into()])).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_save_states[&root], Clean);
    }

    #[test]
    fn layer_revisions_track_authored_changes_not_runtime_or_exports() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/layer_muting.usda");
        let stage = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap().open_stage().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let snapshot = editor.snapshot().unwrap();
        assert!(snapshot.layer_revisions.is_empty());
        let root = snapshot.root_layer;
        let weak = snapshot.layers.into_iter().find(|id| id != &root).unwrap();
        let edit = || EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(4.) };
        editor.edit(edit()).unwrap();
        let first = editor.snapshot().unwrap().layer_revisions;
        assert!(first[&root] > 0);
        assert!(!first.contains_key(&weak));
        editor.set_edit_layer(&weak).unwrap();
        editor.edit(edit()).unwrap();
        let second = editor.snapshot().unwrap().layer_revisions;
        assert!(second[&weak] > 0);
        assert_eq!(second[&root], first[&root]);
        editor.undo().unwrap();
        let undone = editor.snapshot().unwrap().layer_revisions;
        assert!(undone[&weak] > second[&weak]);
        editor.redo().unwrap();
        stage.prim("/Root").unwrap().attribute("score").set(7_f64).unwrap();
        let external = editor.snapshot().unwrap().layer_revisions;
        assert!(external[&weak] > undone[&weak]);
        editor.set_edit_layer(&root).unwrap();
        editor.set_layer_muted(&weak, true).unwrap();
        editor.set_layer_muted(&weak, false).unwrap();
        editor.select(Some("/Root".into())).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_revisions, external);
        let output = tempfile::tempdir().unwrap();
        editor.save(output.path().join("copy.usda").to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert_eq!(editor.snapshot().unwrap().layer_revisions, external);
    }

    #[test]
    fn muting_ancestor_of_edit_layer_is_rejected_without_changes() {
        let leaf = crate::UsdSource::snapshot("leaf.usda", &br#"#usda 1.0
def Xform "Root" { def Cube "Shape" {} }
"#[..]).unwrap();
        let parent = crate::UsdSource::snapshot("parent.usda", &br#"#usda 1.0
(subLayers = [@leaf.usda@])
"#[..]).unwrap().with_dependency(&leaf).unwrap();
        let source = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
(subLayers = [@parent.usda@])
"#[..]).unwrap().with_dependency(&parent).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        let layers = editor.stage().layer_stack();
        assert_eq!(layers.len(), 3);
        editor.set_edit_layer(&layers[2]).unwrap();
        let before = editor.snapshot().unwrap();
        let contents: Vec<_> = layers.iter().map(|id| editor.stage().layer(id).unwrap().export_to_string().unwrap()).collect();
        assert_eq!(before.mute_protected_layers, layers);
        assert!(editor.set_layer_muted(&layers[1], true).is_err());
        let after = editor.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.edit_target, before.edit_target);
        assert!(after.muted_layers.is_empty());
        assert!(!after.can_undo);
        assert!(editor.stage().prim("/Root/Shape").unwrap().is_valid().unwrap());
        for (id, text) in layers.iter().zip(contents) {
            assert_eq!(editor.stage().layer(id).unwrap().export_to_string().unwrap(), text);
        }
        editor.set_edit_layer(&layers[0]).unwrap();
        assert_eq!(editor.snapshot().unwrap().mute_protected_layers, [layers[0].clone()]);
        editor.set_layer_muted(&layers[1], true).unwrap();
        assert!(!editor.stage().prim("/Root/Shape").unwrap().is_valid().unwrap());
        editor.set_layer_muted(&layers[1], false).unwrap();
        assert!(editor.stage().prim("/Root/Shape").unwrap().is_valid().unwrap());
        editor.set_edit_layer(&layers[2]).unwrap();
        let original = editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(4.) }).unwrap();
        let edited = editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap();
        editor.set_edit_layer(&layers[0]).unwrap();
        editor.set_layer_muted(&layers[1], true).unwrap();
        let revision = editor.snapshot().unwrap().revision;
        assert!(editor.undo().is_err());
        assert_eq!(editor.snapshot().unwrap().revision, revision);
        assert!(editor.snapshot().unwrap().can_undo);
        assert_eq!(editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap(), edited);
        editor.set_layer_muted(&layers[1], false).unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap(), original);
        editor.set_layer_muted(&layers[1], true).unwrap();
        let revision = editor.snapshot().unwrap().revision;
        assert!(editor.redo().is_err());
        assert_eq!(editor.snapshot().unwrap().revision, revision);
        assert!(editor.snapshot().unwrap().can_redo);
        assert_eq!(editor.stage().edit_target().layer_identifier(), layers[0]);
        assert_eq!(editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap(), original);
        editor.set_layer_muted(&layers[1], false).unwrap();
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().layer(&layers[2]).unwrap().export_to_string().unwrap(), edited);
    }

    #[test]
    fn layer_muting_preserves_authored_history_and_rejects_stale_commands() {
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
def Xform "Root" { def Sphere "FromWeak" {} }
"#[..]).unwrap();
        let source = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
(subLayers = [@weak.usda@])
over "Root" {}
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let independent = source.open_stage().unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        let root = editor.stage().root_layer().identifier().to_string();
        let weak = editor.snapshot().unwrap().layers.into_iter().find(|id| id != &root).unwrap();
        let baseline = editor.stage().root_layer().export_to_string().unwrap();
        assert!(editor.set_layer_muted(&root, true).is_err());
        assert!(editor.set_layer_muted("unknown.usda", true).is_err());
        editor.set_edit_layer(&weak).unwrap();
        assert!(editor.set_layer_muted(&weak, true).is_err());
        editor.set_edit_layer(&root).unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(2.) }).unwrap();
        let authored = editor.stage().root_layer().export_to_string().unwrap();
        let before = editor.snapshot().unwrap();
        editor.set_layer_muted(&weak, true).unwrap();
        assert_eq!(editor.snapshot().unwrap().muted_layers, [weak.clone()]);
        assert!(!editor.stage().prim("/Root/FromWeak").unwrap().is_valid().unwrap());
        assert!(independent.prim("/Root/FromWeak").unwrap().is_valid().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), authored);
        assert!(editor.set_edit_layer(&weak).is_err());
        assert!(!editor.synchronize_external_edits());
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), baseline);
        assert!(editor.stage().is_layer_muted(&weak));
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), authored);
        let revision = editor.snapshot().unwrap().revision;
        editor.set_layer_muted(&weak, true).unwrap();
        assert_eq!(editor.snapshot().unwrap().revision, revision);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.insert_non_send(editor);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::LayerMuteChecked { identifier: weak.clone(), muted: false, document_id: before.document_id, revision: before.revision }).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.contains("document changed"));
        assert!(app.world().get_non_send::<EditorSession>().unwrap().stage().is_layer_muted(&weak));
        let current = app.world().get_non_send::<EditorSession>().unwrap().snapshot().unwrap();
        bridge.send(EditorCommand::LayerMuteChecked { identifier: weak.clone(), muted: false, document_id: current.document_id, revision: current.revision }).unwrap();
        app.update();
        let editor = app.world().get_non_send::<EditorSession>().unwrap();
        assert!(editor.snapshot().unwrap().muted_layers.is_empty());
        assert!(editor.stage().prim("/Root/FromWeak").unwrap().is_valid().unwrap());
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), authored);
    }

    #[test]
    fn payload_load_changes_invalidate_queued_edits_but_noops_do_not() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_authoring.usda");
        let stage = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap().open_stage().unwrap();
        let editor = EditorSession::new(stage);
        let snapshot = editor.snapshot().unwrap();
        let original = editor.stage().root_layer().export_to_string().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.insert_non_send(editor);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::PayloadChecked { prim: "/Root".into(), loaded: false,
            document_id: snapshot.document_id, revision: snapshot.revision }).unwrap();
        bridge.send(snapshot.checked_edit(EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(),
            type_name: "double".into(), value: Value::Double(4.) }).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.contains("changed before applying the edit"));
        let mut editor = app.world_mut().remove_non_send::<EditorSession>().unwrap();
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), original);
        assert!(!editor.snapshot().unwrap().can_undo);
        let unloaded = editor.snapshot().unwrap().revision;
        assert_eq!(unloaded, snapshot.revision + 1);
        editor.set_payload_loaded("/Root", false).unwrap();
        assert_eq!(editor.snapshot().unwrap().revision, unloaded);
        editor.set_payload_loaded("/Root", true).unwrap();
        let loaded = editor.snapshot().unwrap().revision;
        assert_eq!(loaded, unloaded + 1);
        editor.set_payload_loaded("/Root", true).unwrap();
        assert_eq!(editor.snapshot().unwrap().revision, loaded);
        assert!(editor.set_payload_loaded("/Missing", false).is_err());
        assert_eq!(editor.snapshot().unwrap().revision, loaded);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), original);
    }

    #[test]
    fn checked_inspector_runtime_actions_reject_stale_documents_and_revisions() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_authoring.usda");
        let source = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap();
        for payload in [false, true] {
            let old = EditorSession::new(source.open_stage().unwrap()).snapshot().unwrap();
            let editor = EditorSession::new(source.open_stage().unwrap());
            let current = editor.snapshot().unwrap();
            let weak = current.layers.iter().find(|id| *id != &current.root_layer).unwrap().clone();
            let original = editor.stage().root_layer().export_to_string().unwrap();
            let command = |document_id, revision| if payload {
                EditorCommand::PayloadChecked { prim: "/Root".into(), loaded: false, document_id, revision }
            } else { EditorCommand::EditLayerChecked { identifier: weak.clone(), document_id, revision } };
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, EditorPlugin));
            app.insert_non_send(editor);
            let bridge = app.world().resource::<EditorBridge>().clone();
            for (id, revision) in [(old.document_id, old.revision), (current.document_id, current.revision.wrapping_add(1))] {
                bridge.send(command(id, revision)).unwrap();
                app.update();
                assert!(bridge.view().unwrap().status.contains("document changed"));
                let editor = app.world().get_non_send::<EditorSession>().unwrap();
                assert_eq!(editor.stage().edit_target().layer_identifier(), current.root_layer);
                assert!(editor.stage().prim("/Root").unwrap().is_loaded().unwrap());
                assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), original);
            }
            bridge.send(command(current.document_id, current.revision)).unwrap();
            app.update();
            assert_eq!(bridge.view().unwrap().status, "Ready");
            let editor = app.world().get_non_send::<EditorSession>().unwrap();
            if payload { assert!(!editor.stage().prim("/Root").unwrap().is_loaded().unwrap()); }
            else { assert_eq!(editor.stage().edit_target().layer_identifier(), weak); }
            assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), original);
            assert!(!editor.snapshot().unwrap().can_undo);
        }
    }

    #[test]
    fn history_for_muted_layers_requires_unmute_without_losing_commands() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/layer_muting.usda");
        let stage = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap().open_stage().unwrap();
        let mut editor = EditorSession::new(stage);
        let snapshot = editor.snapshot().unwrap();
        let weak = snapshot.layers.iter().find(|id| *id != &snapshot.root_layer).unwrap().clone();
        editor.set_edit_layer(&weak).unwrap();
        let original = editor.stage().layer(&weak).unwrap().export_to_string().unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(4.) }).unwrap();
        let edited = editor.stage().layer(&weak).unwrap().export_to_string().unwrap();
        editor.set_edit_layer(&snapshot.root_layer).unwrap();
        editor.set_layer_muted(&weak, true).unwrap();
        let revision = editor.snapshot().unwrap().revision;
        assert!(editor.undo().unwrap_err().to_string().contains("unmute"));
        assert_eq!(editor.snapshot().unwrap().revision, revision);
        assert_eq!(editor.stage().edit_target().layer_identifier(), snapshot.root_layer);
        assert!(editor.snapshot().unwrap().can_undo);
        editor.set_layer_muted(&weak, false).unwrap();
        assert_eq!(editor.stage().layer(&weak).unwrap().export_to_string().unwrap(), edited);
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().layer(&weak).unwrap().export_to_string().unwrap(), original);
        editor.set_layer_muted(&weak, true).unwrap();
        let revision = editor.snapshot().unwrap().revision;
        assert!(editor.redo().unwrap_err().to_string().contains("unmute"));
        assert_eq!(editor.snapshot().unwrap().revision, revision);
        assert_eq!(editor.stage().edit_target().layer_identifier(), snapshot.root_layer);
        assert!(editor.snapshot().unwrap().can_redo);
        editor.set_layer_muted(&weak, false).unwrap();
        assert_eq!(editor.stage().layer(&weak).unwrap().export_to_string().unwrap(), original);
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().layer(&weak).unwrap().export_to_string().unwrap(), edited);
    }

    #[test]
    fn pseudo_root_attribute_queries_do_not_poison_layer_muting() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/layer_muting.usda");
        let stage = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap().open_stage().unwrap();
        let root = stage.root_layer().identifier().to_string();
        let weak = stage.layer_stack().into_iter().find(|id| id != &root).unwrap();
        for name in ["purpose", "visibility"] {
            let attribute = stage.prim("/").unwrap().attribute(name);
            assert!(!attribute.resolve_info().unwrap().has_authored_value());
            assert!(attribute.get::<Value>().unwrap().is_none());
            assert!(!attribute.is_defined().unwrap());
        }
        stage.mute_layer(&weak);
        assert!(stage.is_layer_muted(&weak));
        assert!(!stage.prim("/Root/Shape").unwrap().is_valid().unwrap());
        stage.unmute_layer(&weak);
        assert!(stage.prim("/Root/Shape").unwrap().is_valid().unwrap());
    }

    #[test]
    fn checked_layer_muting_reconciles_live_geometry_without_replacing_root() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/layer_muting.usda");
        let stage = crate::UsdSource::new(filename, std::fs::read(filename).unwrap()).unwrap().open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin, crate::live::LiveStagePlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.insert_non_send(crate::live::LiveStage::new(stage.clone()));
        app.insert_non_send(EditorSession::new(stage));
        app.update();
        let root = app.world().resource::<crate::live::PrimEntities>().entity("/Root").unwrap();
        app.world_mut().entity_mut(root).insert(Name::new("runtime name"));
        let runtime_child = app.world_mut().spawn((ChildOf(root), Name::new("runtime child"))).id();
        let original_shape = app.world().resource::<crate::live::PrimEntities>().entity("/Root/Shape").unwrap();
        let snapshot = app.world().get_non_send::<EditorSession>().unwrap().snapshot().unwrap();
        let weak = snapshot.layers.iter().find(|id| *id != &snapshot.root_layer).unwrap().clone();
        let bridge = app.world().resource::<EditorBridge>().clone();
        for muted in [true, false, true, false] {
            let snapshot = app.world().get_non_send::<EditorSession>().unwrap().snapshot().unwrap();
            bridge.send(EditorCommand::LayerMuteChecked { identifier: weak.clone(), muted, document_id: snapshot.document_id, revision: snapshot.revision }).unwrap();
            app.update();
            let map = app.world().resource::<crate::live::PrimEntities>();
            assert_eq!(map.entity("/Root"), Some(root));
            assert_eq!(map.entity("/Root/Shape").is_none(), muted);
            assert_eq!(app.world().get::<Name>(root).unwrap().as_str(), "runtime name");
            assert_eq!(app.world().get::<ChildOf>(runtime_child).unwrap().parent(), root);
            assert!(app.world().get_entity(original_shape).is_err());
            if !muted { assert!(app.world().get::<Mesh3d>(map.entity("/Root/Shape").unwrap()).is_some()); }
        }
    }

    #[test]
    fn selected_render_failures_publish_and_clear() {
        use crate::route::{subset::UsdSubsetWarning, gpu_skin::UsdCpuSkinFallback};
        use crate::route::{subdivision::UsdSubdivisionError, skel::UsdDeformationError, instancer::UsdInstancerWarning, shapes::UsdShapeError, curves::UsdCurveError};
        use crate::route::xform::UsdTransformError;
        use crate::route::material::UsdMaterialWarning;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.init_resource::<crate::live::PrimEntities>();
        let entity = app.world_mut().spawn((UsdSubdivisionError("unsupported holes".into()),
            UsdDeformationError("invalid influences".into()), UsdInstancerWarning("missing prototype".into()), UsdShapeError("invalid dimensions".into()), UsdCurveError("invalid counts".into()), UsdTransformError("projective matrix".into()), UsdMaterialWarning("missing tangent frame".into()),
            UsdSubsetWarning("overlapping faces".into()), UsdCpuSkinFallback("unsupported joint layout".into()))).id();
        app.world_mut().resource_mut::<crate::live::PrimEntities>().insert("/Prim", entity);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.0.lock().unwrap().view.document.selected = Some("/Prim".into());
        app.update();
        assert_eq!(bridge.view().unwrap().document.render_issues,
            ["Subdivision: unsupported holes", "Deformation: invalid influences", "Point instancer: missing prototype", "Shape: invalid dimensions", "Curve: invalid counts", "Transform: projective matrix", "Material: missing tangent frame",
            "Material subsets: overlapping faces", "CPU skinning fallback: unsupported joint layout"]);
        app.world_mut().entity_mut(entity).remove::<(UsdSubdivisionError, UsdDeformationError, UsdInstancerWarning, UsdShapeError, UsdCurveError, UsdTransformError, UsdMaterialWarning, UsdSubsetWarning, UsdCpuSkinFallback)>();
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.is_empty());
        app.world_mut().entity_mut(entity).insert(UsdSubdivisionError("unsupported holes".into()));
        bridge.0.lock().unwrap().view.document.selected = Some("/Missing".into());
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.is_empty());
        bridge.0.lock().unwrap().view.document.selected = None;
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.is_empty());
    }

    #[test]
    fn projected_subset_failure_reaches_inspector_and_recovers() {
        let stage = crate::UsdSource::snapshot("subset.usda", include_bytes!("../../../assets/material_subset_warning.usda").as_slice()).unwrap().open_stage().unwrap();
        let live = crate::live::LiveStage::new(stage);
        let indices = live.stage.prim("/Root/Faces").unwrap().attribute("indices");
        indices.clone().set(Value::IntVec(vec![99])).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>().init_resource::<Assets<Image>>();
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(app.world_mut(), &live, &mut map);
        let root = map.entity("/Root").unwrap();
        app.insert_resource(map);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.0.lock().unwrap().view.document.selected = Some("/Root".into());
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.iter().any(|issue| issue.starts_with("Material subsets:")));
        assert!(app.world().get::<Mesh3d>(root).is_some());
        indices.set(Value::IntVec(vec![0])).unwrap();
        let mut map = app.world_mut().remove_resource::<crate::live::PrimEntities>().unwrap();
        crate::live::apply_changes(app.world_mut(), &live, &mut map);
        assert_eq!(map.entity("/Root"), Some(root));
        app.insert_resource(map);
        app.update();
        let issues = bridge.view().unwrap().document.render_issues;
        assert!(!issues.iter().any(|issue| issue.starts_with("Material subsets:")));
        assert!(issues.iter().any(|issue| issue.starts_with("Material (subset Faces):")));
    }

    #[test]
    fn generated_material_issues_follow_owned_children_and_clear() {
        use crate::route::{material::UsdMaterialWarning, subset::UsdSubset,
            instancer::{UsdInstance, UsdInstanceId, UsdPrototypePart}};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.init_resource::<crate::live::PrimEntities>();
        let root = app.world_mut().spawn_empty().id();
        let subset = app.world_mut().spawn((ChildOf(root), UsdSubset("/Root/Faces".into()), UsdMaterialWarning("no UV0".into()))).id();
        let instance = app.world_mut().spawn((ChildOf(root), UsdInstance, UsdInstanceId(7))).id();
        let part = app.world_mut().spawn((ChildOf(instance), UsdPrototypePart("/Proto/Mesh".into()), UsdMaterialWarning("no tangent frame".into()))).id();
        let unrelated = app.world_mut().spawn((ChildOf(root), UsdMaterialWarning("runtime warning".into()))).id();
        app.world_mut().spawn((ChildOf(unrelated), UsdSubset("unrelated".into()), UsdMaterialWarning("hidden".into())));
        app.world_mut().resource_mut::<crate::live::PrimEntities>().insert("/Root", root);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.0.lock().unwrap().view.document.selected = Some("/Root".into());
        app.update();
        assert_eq!(bridge.view().unwrap().document.render_issues, [
            "Material (subset /Root/Faces): no UV0",
            "Material (instance 7 / prototype /Proto/Mesh): no tangent frame"]);
        app.world_mut().entity_mut(subset).remove::<UsdMaterialWarning>();
        app.world_mut().entity_mut(part).remove::<UsdMaterialWarning>();
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.is_empty());
        app.world_mut().entity_mut(part).insert(UsdMaterialWarning("returned".into()));
        bridge.0.lock().unwrap().view.document.selected = None;
        app.update();
        assert!(bridge.view().unwrap().document.render_issues.is_empty());
    }

    #[test]
    fn generated_material_issue_collection_reports_limits() {
        use crate::route::{material::UsdMaterialWarning, subset::UsdSubset};
        for (count, warnings, expected, truncated) in [(64, true, 64, false), (65, true, 65, true),
            (4096, false, 0, false), (4097, false, 1, true)] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, EditorPlugin));
            app.init_resource::<crate::live::PrimEntities>();
            let root = app.world_mut().spawn_empty().id();
            for i in 0..count {
                let mut child = app.world_mut().spawn((ChildOf(root), UsdSubset(format!("/Root/Part{i}"))));
                if warnings { child.insert(UsdMaterialWarning("missing coordinates".into())); }
            }
            app.world_mut().resource_mut::<crate::live::PrimEntities>().insert("/Root", root);
            let bridge = app.world().resource::<EditorBridge>().clone();
            bridge.0.lock().unwrap().view.document.selected = Some("/Root".into());
            app.update();
            let issues = bridge.view().unwrap().document.render_issues;
            assert_eq!(issues.len(), expected);
            assert_eq!(issues.last().is_some_and(|issue| issue.contains("diagnostics truncated")), truncated);
        }
    }

    #[test]
    fn timeline_commands_scrub_loop_and_pause_without_authoring() {
        let stage = crate::UsdSource::new("timeline.usda", &br#"#usda 1.0
( startTimeCode = 0 endTimeCode = 10 timeCodesPerSecond = 10 )
def Xform "Mover" {
    double3 xformOp:translate.timeSamples = { 0: (0,0,0), 10: (10,0,0) }
    token visibility.timeSamples = { 0: "inherited", 5: "invisible", 10: "inherited" }
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#[..]).unwrap().open_stage().unwrap();
        let baseline = crate::authoring::export_stage_string(&stage).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(100)));
        app.world_mut().insert_non_send(crate::live::LiveStage::new(stage.clone()));
        app.world_mut().insert_non_send(EditorSession::new(stage));
        app.update();
        let bridge = app.world().resource::<EditorBridge>().clone();
        let entity = app.world().resource::<crate::live::PrimEntities>().entity("/Mover").unwrap();
        bridge.send(EditorCommand::Seek(5.0)).unwrap();
        app.update();
        assert_eq!(app.world().get::<Transform>(entity).unwrap().translation.x, 5.0);
        assert_eq!(bridge.view().unwrap().timeline.current, 5.0);
        assert_eq!(bridge.view().unwrap().document.sample_time, Some(5.0));
        assert!(!bridge.view().unwrap().document.visibility["/Mover"]);
        assert!(!bridge.view().unwrap().document.can_undo);
        bridge.send(EditorCommand::Seek(f64::NAN)).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().timeline.current, 5.0);
        assert!(bridge.view().unwrap().status.contains("finite"));
        bridge.send(EditorCommand::Seek(9.0)).unwrap();
        bridge.send(EditorCommand::Play(true)).unwrap();
        app.update();
        assert!(bridge.view().unwrap().timeline.current.abs() < 1e-6);
        assert!(bridge.view().unwrap().document.visibility["/Mover"]);
        assert_eq!(bridge.view().unwrap().document.sample_time, Some(0.0));
        app.update();
        assert!((bridge.view().unwrap().timeline.current - 1.0).abs() < 1e-6);
        assert_eq!(bridge.view().unwrap().document.sample_time, Some(1.0));
        bridge.send(EditorCommand::Play(false)).unwrap();
        app.update();
        let paused = bridge.view().unwrap().timeline.current;
        app.update();
        assert_eq!(bridge.view().unwrap().timeline.current, paused);
        assert!(!bridge.view().unwrap().timeline.playing);
        assert!(!bridge.view().unwrap().document.can_undo);
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Mover"), Some(entity));
        assert_eq!(crate::authoring::export_stage_string(app.world().get_non_send::<EditorSession>().unwrap().stage()).unwrap(), baseline);
    }

    #[test]
    fn relationship_targets_distinguish_empty_from_cleared_and_undo() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("weak.usda"), "#usda 1.0\ndef Xform \"Owner\" { rel links = [</A>, </B.value>] }\n").unwrap();
        let source = crate::UsdSource::new(directory.path().join("root.usda"), &b"#usda 1.0\n( subLayers = [@weak.usda@] )\n"[..]).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Owner".into())).unwrap();
        let targets = |editor: &EditorSession| editor.snapshot().unwrap().relationships.into_iter().find(|(name, _)| name == "links").unwrap().1;
        assert_eq!(targets(&editor), vec!["/A", "/B.value"]);
        let baseline = crate::authoring::export_stage_string(editor.stage()).unwrap();
        editor.edit(EditorEdit::RelationshipTargets { prim: "/Owner".into(), name: "links".into(), targets: vec![] }).unwrap();
        assert!(targets(&editor).is_empty());
        editor.undo().unwrap();
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), baseline);
        assert_eq!(targets(&editor), vec!["/A", "/B.value"]);
        editor.redo().unwrap();
        editor.edit(EditorEdit::ClearRelationshipTargets { prim: "/Owner".into(), name: "links".into() }).unwrap();
        assert_eq!(targets(&editor), vec!["/A", "/B.value"]);
        editor.undo().unwrap();
        assert!(targets(&editor).is_empty());
        let before = crate::authoring::export_stage_string(editor.stage()).unwrap();
        assert!(editor.edit(EditorEdit::RelationshipTargets { prim: "/Owner".into(), name: "links".into(), targets: vec![openusd::sdf::path("relative").unwrap()] }).is_err());
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), before);
        editor.edit(EditorEdit::RelationshipTargets { prim: "/Owner".into(), name: "links".into(), targets: vec![openusd::sdf::path("/Future").unwrap()] }).unwrap();
        let output = directory.path().join("saved.usda");
        editor.save(output.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(output.to_str().unwrap()).unwrap();
        assert_eq!(reopened.prim(openusd::sdf::path("/Owner").unwrap()).unwrap().relationship("links").targets().unwrap(), vec![openusd::sdf::path("/Future").unwrap()]);
    }

    #[test]
    fn batch_edits_group_history_and_roll_back_failure() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("batch.usda").unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        editor.edit(EditorEdit::Batch(vec![
            EditorEdit::Define { path: "/Assembly".into(), type_name: "Xform".into() },
            EditorEdit::Batch(vec![
                EditorEdit::Define { path: "/Assembly/Ball".into(), type_name: "Sphere".into() },
                EditorEdit::Attribute { prim: "/Assembly/Ball".into(), name: "radius".into(), type_name: "double".into(), value: Value::Double(2.5) },
            ]),
        ])).unwrap();
        let after = stage.root_layer().export_to_string().unwrap();
        assert_eq!(editor.undo.len(), 1);
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(!editor.undo().unwrap());
        editor.edit(EditorEdit::Batch(vec![])).unwrap();
        assert!(editor.snapshot().unwrap().can_redo);
        let failed = EditorEdit::Batch(vec![
            EditorEdit::Define { path: "/Temporary".into(), type_name: "Xform".into() },
            EditorEdit::TransformMatrix { prim: "/Missing".into(), matrix: [0.0; 16], reset: false },
        ]);
        assert!(editor.edit(failed).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(!editor.snapshot().unwrap().can_undo);
        assert!(editor.redo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), after);
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn namespace_commands_reconcile_the_live_projection() {
        #[derive(Component, PartialEq, Debug)]
        struct RuntimeState(u32);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("namespace.usda");
        std::fs::write(&path, "#usda 1.0\ndef Cube \"Box\" {}\ndef Xform \"Group\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(path.to_string_lossy().into_owned())).unwrap();
        app.update();
        let original = app.world().resource::<crate::live::PrimEntities>().entity("/Box").unwrap();
        app.world_mut().entity_mut(original).insert(RuntimeState(42));
        let runtime_child = app.world_mut().spawn((RuntimeState(7), ChildOf(original))).id();
        bridge.send(EditorCommand::Select(Some("/Box".into()))).unwrap();
        bridge.send(EditorCommand::Edit(EditorEdit::Rename { path: "/Box".into(), name: "Renamed".into() })).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Renamed"));
        let renamed = app.world().resource::<crate::live::PrimEntities>().entity("/Renamed").unwrap();
        assert!(app.world().get::<Mesh3d>(renamed).is_some());
        assert_eq!(renamed, original);
        assert_eq!(app.world().get::<RuntimeState>(renamed), Some(&RuntimeState(42)));
        assert_eq!(app.world().get::<crate::UsdPrimRef>(renamed).unwrap().path, "/Renamed");
        assert_eq!(app.world().get::<ChildOf>(runtime_child).unwrap().parent(), renamed);
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Box").is_some());
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Renamed").is_none());
        assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Box"));
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Box"), Some(original));
        bridge.send(EditorCommand::Redo).unwrap();
        bridge.send(EditorCommand::Edit(EditorEdit::Move { path: "/Renamed".into(), destination: "/Group/Moved".into() })).unwrap();
        app.update();
        let map = app.world().resource::<crate::live::PrimEntities>();
        assert_eq!(map.entity("/Group/Moved"), Some(original));
        assert_eq!(app.world().get::<ChildOf>(original).unwrap().parent(), map.entity("/Group").unwrap());
        assert_eq!(app.world().get::<RuntimeState>(original), Some(&RuntimeState(42)));
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        let map = app.world().resource::<crate::live::PrimEntities>();
        assert_eq!(map.entity("/Renamed"), Some(original));
        assert_eq!(app.world().get::<ChildOf>(original).unwrap().parent(), map.entity("/").unwrap());
        assert_eq!(app.world().get::<RuntimeState>(runtime_child), Some(&RuntimeState(7)));
        bridge.send(EditorCommand::Redo).unwrap();
        app.update();
        bridge.send(EditorCommand::Edit(EditorEdit::Move { path: "/Group/Moved".into(), destination: "/Final".into() })).unwrap();
        bridge.send(EditorCommand::Edit(EditorEdit::Remove { path: "/Group".into() })).unwrap();
        app.update();
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Final"), Some(original));
        assert_eq!(app.world().get::<RuntimeState>(original), Some(&RuntimeState(42)));
        assert_eq!(app.world().get::<RuntimeState>(runtime_child), Some(&RuntimeState(7)));
        bridge.send(EditorCommand::Edit(EditorEdit::Batch(vec![
            EditorEdit::Rename { path: "/Final".into(), name: "Intermediate".into() },
            EditorEdit::Batch(vec![EditorEdit::Move { path: "/Intermediate".into(), destination: "/Batched".into() }]),
        ]))).unwrap();
        app.update();
        for path in ["/Batched", "/Final", "/Batched"] {
            if path == "/Final" { bridge.send(EditorCommand::Undo).unwrap(); app.update(); }
            else if bridge.view().unwrap().document.selected.as_deref() == Some("/Final") {
                bridge.send(EditorCommand::Redo).unwrap(); app.update();
            }
            assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some(path));
            assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity(path), Some(original));
            assert_eq!(app.world().get::<RuntimeState>(original), Some(&RuntimeState(42)));
            assert_eq!(app.world().get::<ChildOf>(runtime_child).unwrap().parent(), original);
        }
    }

    #[test]
    fn namespace_edits_preserve_subtrees_and_selection_through_history() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("namespace.usda").unwrap();
        crate::authoring::define_prim(&stage, "/World/A/Child", "Cube").unwrap();
        crate::authoring::define_prim(&stage, "/World/B", "Xform").unwrap();
        crate::authoring::set_attribute(&stage, "/World/A/Child", "size", "double", Value::Double(7.0)).unwrap();
        let mut editor = EditorSession::new(stage);
        editor.select(Some("/World/A/Child".into())).unwrap();
        editor.edit(EditorEdit::Rename { path: "/World/A".into(), name: "Renamed".into() }).unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/World/Renamed/Child"));
        editor.undo().unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/World/A/Child"));
        editor.redo().unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/World/Renamed/Child"));
        editor.edit(EditorEdit::Reparent { path: "/World/Renamed".into(), parent: "/World/B".into() }).unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/World/B/Renamed/Child"));
        editor.edit(EditorEdit::Move { path: "/World/B/Renamed".into(), destination: "/Moved".into() }).unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/Moved/Child"));
        assert_eq!(editor.stage().prim(openusd::sdf::path("/Moved/Child").unwrap()).unwrap().attribute("size").get::<Value>().unwrap(), Some(Value::Double(7.0)));
        let before = crate::authoring::export_stage_string(editor.stage()).unwrap();
        assert!(editor.edit(EditorEdit::Move { path: "/Moved".into(), destination: "/World/B".into() }).is_err());
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), before);
        assert_eq!(editor.selected.as_deref(), Some("/Moved/Child"));
        editor.edit(EditorEdit::Remove { path: "/Moved".into() }).unwrap();
        assert!(editor.selected.is_none());
        editor.undo().unwrap();
        assert_eq!(editor.selected.as_deref(), Some("/Moved/Child"));
        assert_eq!(editor.stage().prim(openusd::sdf::path("/Moved/Child").unwrap()).unwrap().attribute("size").get::<Value>().unwrap(), Some(Value::Double(7.0)));
        let source = crate::UsdSource::new("reopened.usda", crate::authoring::export_stage_string(editor.stage()).unwrap().into_bytes()).unwrap();
        assert!(crate::authoring::prim_exists(&source.open_stage().unwrap(), "/Moved/Child"));
    }

    #[test]
    fn editor_loads_filesystem_and_packaged_images_and_reprojects_material_edits() {
        let directory = tempfile::tempdir().unwrap();
        let text = r#"#usda 1.0
def Mesh "Mesh" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    rel material:binding = </Mat>
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Tex.outputs:rgb>
        float inputs:roughness.connect = </Mat/Tex.outputs:g>
        float inputs:metallic.connect = </Mat/Tex.outputs:b>
        token outputs:surface
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @pixel.png@
        float3 outputs:rgb
        float outputs:g
        float outputs:b
    }
}
"#;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[200, 80, 20, 255]).unwrap();
        }
        std::fs::write(directory.path().join("pixel.png"), &png).unwrap();
        std::fs::write(directory.path().join("scene.usda"), text).unwrap();
        let mut archive = openusd::usdz::ArchiveWriter::new(std::io::Cursor::new(Vec::new()));
        archive.add_layer("scene.usda", text.as_bytes()).unwrap();
        archive.add_layer("pixel.png", &png).unwrap();
        std::fs::write(directory.path().join("scene.usdz"), archive.finish().unwrap().into_inner()).unwrap();
        for filename in ["scene.usda", "scene.usdz"] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
            app.insert_resource(Assets::<Image>::default());
            app.insert_resource(Assets::<Mesh>::default());
            app.insert_resource(Assets::<StandardMaterial>::default());
            let bridge = app.world().resource::<EditorBridge>().clone();
            bridge.send(EditorCommand::Open(directory.path().join(filename).to_string_lossy().into_owned())).unwrap();
            app.update();
            assert_eq!(bridge.view().unwrap().status, "Ready");
            let entity = app.world().resource::<crate::live::PrimEntities>().entity("/Mesh").unwrap();
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            let texture = material.base_color_texture.clone().unwrap();
            let packed = material.metallic_roughness_texture.clone().unwrap();
            assert_eq!(app.world().resource::<Assets<Image>>().get(&texture).unwrap().data.as_deref(), Some([200,80,20,255].as_slice()));
            assert_eq!(app.world().resource::<Assets<Image>>().get(&packed).unwrap().data.as_deref(), Some([255,80,20,255].as_slice()));
            bridge.send(EditorCommand::Edit(EditorEdit::Attribute {
                prim: "/Mat/Surface".into(), name: "inputs:ior".into(), type_name: "float".into(), value: Value::Float(1.7),
            })).unwrap();
            app.update();
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.ior, 1.7);
            assert_eq!(material.base_color_texture.as_ref(), Some(&texture));
            assert_eq!(material.metallic_roughness_texture.as_ref(), Some(&packed));
            if filename == "scene.usda" {
                bridge.send(EditorCommand::Select(Some("/Mesh".into()))).unwrap();
                app.world_mut().entity_mut(entity).insert(Name::new("runtime name"));
                let child = app.world_mut().spawn((Name::new("runtime child"), ChildOf(entity))).id();
                app.update();
                let before = app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap();
                let document_id = bridge.view().unwrap().document.document_id;
                std::fs::remove_file(directory.path().join("pixel.png")).unwrap();
                bridge.send(EditorCommand::RefreshTextures).unwrap();
                app.update();
                let status = bridge.view().unwrap().status;
                assert!(status.starts_with("Failed: cannot read texture "));
                assert!(status.contains("pixel.png"), "{status}");
                assert_eq!(app.world().resource::<Assets<StandardMaterial>>()
                    .get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap()
                    .base_color_texture.as_ref(), Some(&texture));
                std::fs::write(directory.path().join("pixel.png"), b"broken image").unwrap();
                bridge.send(EditorCommand::RefreshTextures).unwrap();
                app.update();
                let status = bridge.view().unwrap().status;
                assert!(status.starts_with("Failed: cannot decode texture "));
                assert!(status.contains(directory.path().join("pixel.png").to_str().unwrap()));
                assert_eq!(app.world().resource::<Assets<StandardMaterial>>()
                    .get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap()
                    .base_color_texture.as_ref(), Some(&texture));
                let mut changed = Vec::new();
                {
                    let mut encoder = png::Encoder::new(&mut changed, 1, 1);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder.write_header().unwrap().write_image_data(&[20, 160, 240, 255]).unwrap();
                }
                std::fs::write(directory.path().join("pixel.png"), changed).unwrap();
                bridge.send(EditorCommand::RefreshTextures).unwrap();
                app.update();
                assert_eq!(bridge.view().unwrap().status, "Ready");
                assert_eq!(bridge.view().unwrap().document.document_id, document_id);
                assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Mesh"));
                assert!(bridge.view().unwrap().document.can_undo);
                assert_eq!(app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap(), before);
                assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Mesh"), Some(entity));
                assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime name");
                assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
                let material = app.world().resource::<Assets<StandardMaterial>>()
                    .get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
                assert_eq!(material.ior, 1.7);
                assert_ne!(material.base_color_texture.as_ref(), Some(&texture));
                assert_eq!(app.world().resource::<Assets<Image>>().get(material.base_color_texture.as_ref().unwrap()).unwrap().data.as_deref(), Some([20,160,240,255].as_slice()));
                assert_eq!(app.world().resource::<Assets<Image>>().get(material.metallic_roughness_texture.as_ref().unwrap()).unwrap().data.as_deref(), Some([255,160,240,255].as_slice()));
                bridge.send(EditorCommand::Undo).unwrap();
                app.update();
                assert_eq!(app.world().non_send::<EditorSession>().stage().prim("/Mat/Surface").unwrap()
                    .attribute("inputs:ior").get::<Value>().unwrap(), None);
                bridge.send(EditorCommand::RefreshTextures).unwrap();
                app.update();
                assert!(bridge.view().unwrap().document.can_redo);
                bridge.send(EditorCommand::Redo).unwrap();
                app.update();
                assert_eq!(app.world().non_send::<EditorSession>().stage().prim("/Mat/Surface").unwrap()
                    .attribute("inputs:ior").get::<Value>().unwrap(), Some(Value::Float(1.7)));
                std::fs::write(directory.path().join("pixel.png"), &png).unwrap();
            }
            bridge.send(EditorCommand::Open(directory.path().join("missing.usda").to_string_lossy().into_owned())).unwrap();
            app.update();
            assert!(bridge.view().unwrap().status.starts_with("Failed:"));
            assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Mesh"), Some(entity));
        }
    }

    fn session() -> EditorSession {
        let source = crate::UsdSource::new("editor.usda", &b"#usda 1.0\ndef Cube \"Box\" {}\n"[..]).unwrap();
        EditorSession::new(source.open_stage().unwrap())
    }

    #[test]
    fn typed_references_compose_reusable_assets_and_undo() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("part.usda"),
            "#usda 1.0\n(defaultPrim = \"Part\")\ndef Xform \"Part\" { def Cube \"Shape\" {} }\n").unwrap();
        let source = crate::UsdSource::new(directory.path().join("assembly.usda"),
            &b"#usda 1.0\ndef Xform \"Left\" {}\ndef Xform \"Right\" {}\n"[..]).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        let reference = openusd::sdf::Reference { asset_path: "part.usda".into(), ..Default::default() };
        for prim in ["/Left", "/Right"] {
            editor.edit(EditorEdit::References { prim: prim.into(), references: vec![reference.clone()] }).unwrap();
        }
        let paths = editor.snapshot().unwrap().prims;
        assert!(paths.contains(&"/Left/Shape".to_string()));
        assert!(paths.contains(&"/Right/Shape".to_string()));
        editor.undo().unwrap();
        assert!(!editor.snapshot().unwrap().prims.contains(&"/Right/Shape".to_string()));
        assert!(editor.snapshot().unwrap().prims.contains(&"/Left/Shape".to_string()));
        editor.redo().unwrap();
        let before = crate::authoring::export_stage_string(editor.stage()).unwrap();
        let invalid = openusd::sdf::Reference { prim_path: openusd::sdf::path("/Left.size").unwrap(), ..Default::default() };
        assert!(editor.edit(EditorEdit::References { prim: "/Left".into(), references: vec![invalid] }).is_err());
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), before);
        let output = directory.path().join("assembly-out.usda");
        editor.save(output.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(output.to_str().unwrap()).unwrap();
        assert!(reopened.prim("/Right/Shape").unwrap().is_valid().unwrap());
    }

    #[test]
    fn external_authoring_resets_history_instead_of_undoing_the_wrong_edit() {
        let mut editor = session();
        editor.edit(EditorEdit::Attribute {
            prim: "/Box".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(4.0),
        }).unwrap();
        crate::authoring::set_attribute(editor.stage(), "/Box", "size", "double", Value::Double(9.0)).unwrap();
        assert!(!editor.undo().unwrap());
        assert!(!editor.snapshot().unwrap().can_undo);
        assert_eq!(editor.stage().prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(9.0));
        editor.edit(EditorEdit::Attribute {
            prim: "/Box".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(10.0),
        }).unwrap();
        editor.undo().unwrap();
        assert_eq!(editor.stage().prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(9.0));
    }

    #[test]
    fn variant_choices_follow_referenced_namespace_and_undo_selection() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("model.usda"), r#"#usda 1.0
def Xform "Asset" (prepend variantSets = "shape") {
    variantSet "shape" = {
        "box" { def Xform "Box" {} }
        "round" { def Xform "Round" {} }
    }
}
"#).unwrap();
        let source = crate::UsdSource::new(directory.path().join("root.usda"),
            &b"#usda 1.0\ndef Xform \"Mounted\" ( prepend references = @model.usda@</Asset> ) {}\n"[..]).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Mounted".into())).unwrap();
        assert_eq!(editor.snapshot().unwrap().variant_choices["shape"], ["box", "round"]);
        assert!(editor.edit(EditorEdit::Variant {
            prim: "/Mounted".into(), set: "shape".into(), selection: "typo".into(),
        }).is_err());
        editor.edit(EditorEdit::Variant {
            prim: "/Mounted".into(), set: "shape".into(), selection: "box".into(),
        }).unwrap();
        assert!(editor.snapshot().unwrap().prims.contains(&"/Mounted/Box".to_string()));
        editor.undo().unwrap();
        assert!(!editor.snapshot().unwrap().prims.contains(&"/Mounted/Box".to_string()));
        editor.redo().unwrap();
        assert!(editor.snapshot().unwrap().prims.contains(&"/Mounted/Box".to_string()));
    }

    #[test]
    fn visible_variant_fixture_preserves_selection_and_authored_history() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/editor_variants.usda");
        let bytes = std::fs::read(&path).unwrap();
        let source = crate::UsdSource::new(&path, bytes.clone()).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Model".into())).unwrap();
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let size = |editor: &EditorSession| editor.stage().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap();
        assert_eq!(size(&editor), Some(1.0));
        editor.edit(EditorEdit::Variant { prim: "/Model".into(), set: "look".into(), selection: "blue".into() }).unwrap();
        let blue = editor.stage().root_layer().export_to_string().unwrap();
        assert_eq!(size(&editor), Some(2.0));
        assert_eq!(editor.snapshot().unwrap().selected.as_deref(), Some("/Model"));
        assert!(editor.undo().unwrap());
        assert_eq!(size(&editor), Some(1.0));
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(size(&editor), Some(2.0));
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), blue);
        assert_eq!(editor.snapshot().unwrap().selected.as_deref(), Some("/Model"));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn checked_edits_reject_changed_document_revision_and_target() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_authoring.usda");
        bridge.send(EditorCommand::Open(filename.into())).unwrap();
        app.update();
        let initial = bridge.view().unwrap().document;
        let edit = || EditorEdit::Attribute { prim: "/Root".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(7.) };
        let stage = app.world().non_send::<EditorSession>().stage().clone();
        bridge.send(EditorCommand::Select(Some("/Root".into()))).unwrap();
        bridge.send(EditorCommand::Seek(5.)).unwrap();
        bridge.send(initial.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert_eq!(stage.prim("/Root").unwrap().attribute("score").get::<f64>().unwrap(), Some(7.));
        bridge.send(initial.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        let current = bridge.view().unwrap().document;
        let before = stage.root_layer().export_to_string().unwrap();
        let weak = current.layers.iter().find(|layer| *layer != &current.edit_layer).unwrap().clone();
        bridge.send(EditorCommand::EditLayer(weak)).unwrap();
        bridge.send(current.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        bridge.send(EditorCommand::EditLayer(current.edit_layer.clone())).unwrap();
        app.update();
        let mut mismapped = bridge.view().unwrap().document;
        mismapped.edit_target = Some(EditTarget::for_local_direct_variant(&mismapped.edit_layer, "/Root{choice=a}").unwrap());
        bridge.send(mismapped.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        let external = bridge.view().unwrap().document;
        stage.create_attribute("/Root.external", "double").unwrap().set(Value::Double(1.)).unwrap();
        bridge.send(external.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        let replaced = bridge.view().unwrap().document;
        bridge.send(EditorCommand::Open(filename.into())).unwrap();
        bridge.send(replaced.checked_edit(edit()).unwrap()).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert!(app.world().non_send::<EditorSession>().stage().prim("/Root").unwrap().attribute("score").get::<f64>().unwrap().is_none());
        assert!(EditorSnapshot::default().checked_edit(edit()).is_none());
    }

    #[test]
    fn authored_external_payloads_reconcile_live_history_and_save() {
        let directory = tempfile::tempdir().unwrap();
        let part = directory.path().join("part.usda");
        let part_text = "#usda 1.0\n(defaultPrim = \"Part\")\ndef Xform \"Part\" { def Cube \"Shape\" {} }\n";
        std::fs::write(&part, part_text).unwrap();
        let root = directory.path().join("assembly.usda");
        std::fs::write(&root, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::UsdPlugin, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(root.to_string_lossy().into_owned())).unwrap();
        bridge.send(EditorCommand::Select(Some("/Root".into()))).unwrap();
        app.update();
        let parent = app.world().resource::<crate::live::PrimEntities>().entity("/Root").unwrap();
        app.world_mut().entity_mut(parent).insert(Name::new("runtime parent"));
        bridge.send(EditorCommand::Edit(EditorEdit::Payloads {
            prim: "/Root".into(), payloads: vec![openusd::sdf::Payload { asset_path: "part.usda".into(), ..Default::default() }],
        })).unwrap();
        app.update();
        for (command, visible) in [(None, true), (Some(EditorCommand::Undo), false), (Some(EditorCommand::Redo), true),
            (Some(EditorCommand::Payload { prim: "/Root".into(), loaded: false }), false),
            (Some(EditorCommand::Payload { prim: "/Root".into(), loaded: true }), true)] {
            if let Some(command) = command { bridge.send(command).unwrap(); app.update(); }
            let map = app.world().resource::<crate::live::PrimEntities>();
            assert_eq!(map.entity("/Root"), Some(parent));
            assert_eq!(app.world().get::<Name>(parent).unwrap().as_str(), "runtime parent");
            assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Root"));
            assert_eq!(map.entity("/Root/Shape").is_some(), visible);
            if visible { assert!(app.world().get::<Mesh3d>(map.entity("/Root/Shape").unwrap()).is_some()); }
        }
        let output = directory.path().join("saved.usda");
        bridge.send(EditorCommand::Save { filename: output.to_string_lossy().into_owned(), mode: SaveMode::RootLayer }).unwrap();
        app.update();
        let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(output.to_str().unwrap()).unwrap();
        assert!(reopened.prim("/Root/Shape").unwrap().is_valid().unwrap());
        assert!(matches!(reopened.prim("/Root").unwrap().get_metadata::<Value>("payload").unwrap(), Some(Value::PayloadListOp(_))));
        assert_eq!(std::fs::read_to_string(part).unwrap(), part_text);
        let stage = app.world().non_send::<EditorSession>().stage().clone();
        let before = stage.root_layer().export_to_string().unwrap();
        bridge.send(EditorCommand::Edit(EditorEdit::Batch(vec![
            EditorEdit::ClearPayloads { prim: "/Root".into() },
            EditorEdit::Payloads { prim: "/Root".into(), payloads: vec![openusd::sdf::Payload {
                asset_path: "part.usda".into(), prim_path: openusd::sdf::path("/Part.size").unwrap(), ..Default::default()
            }] },
        ]))).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Root/Shape").is_some());
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Root"), Some(parent));
    }

    #[test]
    fn payload_commands_reconcile_and_keep_unloaded_prim_selectable() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/payload_test.usda");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(path.into())).unwrap();
        bridge.send(EditorCommand::Select(Some("/Root".into()))).unwrap();
        app.update();
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Root/Child").is_some());
        bridge.send(EditorCommand::Payload { prim: "/Root".into(), loaded: false }).unwrap();
        app.update();
        let document = bridge.view().unwrap().document;
        assert_eq!(document.selected.as_deref(), Some("/Root"));
        assert_eq!(document.selected_loaded, Some(false));
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Root/Child").is_none());
        assert!(!document.can_undo);
        bridge.send(EditorCommand::Payload { prim: "/Root".into(), loaded: true }).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().document.selected_loaded, Some(true));
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/Root/Child").is_some());
        bridge.send(EditorCommand::Payload { prim: "/Missing".into(), loaded: true }).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
    }

    #[test]
    fn flatten_rejects_timecode_clips_without_replacing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::snapshot(directory.path().join("source.usda"), &br#"#usda 1.0
def Xform "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@clip.usda@]
            double2[] active = [(0, 0)]
            double2[] times = [(0, 0), (10, 10)]
            string primPath = "/Model"
        }
    }
) {
    timecode score
}
"#[..]).unwrap();
        let clip = crate::UsdSource::snapshot(directory.path().join("clip.usda"),
            &b"#usda 1.0\ndef Xform \"Model\" { timecode score.timeSamples = {0: 1, 10: 3} }\n"[..]).unwrap();
        let source = source.with_dependency(&clip).unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        let before = editor.stage().root_layer().export_to_string().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let destination = directory.path().join(format!("output.{extension}"));
            std::fs::write(&destination, b"existing output").unwrap();
            let error = editor.save(destination.to_str().unwrap(), SaveMode::Flattened).unwrap_err();
            assert!(error.to_string().contains("timecode clip values at /Model.score"), "{error:#}");
            assert_eq!(std::fs::read(destination).unwrap(), b"existing output");
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 4);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer] {
            let destination = directory.path().join("authored.usda");
            editor.save(destination.to_str().unwrap(), mode).unwrap();
            assert!(std::fs::read_to_string(destination).unwrap().contains("clips"));
        }
    }

    #[test]
    fn flatten_rejects_incomplete_composition_without_publishing_or_mutating() {
        let directory = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::snapshot(directory.path().join("source.usda"), &b"#usda 1.0\ndef Scope \"Model\" (prepend references = @missing.usda@</Model>) {}\n"[..]).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Model".into())).unwrap();
        editor.edit(EditorEdit::Attribute {
            prim: "/Model".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(17.0),
        }).unwrap();
        let before = editor.stage().root_layer().export_to_string().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let path = directory.path().join(format!("flat.{extension}"));
            std::fs::write(&path, "existing output").unwrap();
            let error = editor.save(path.to_str().unwrap(), SaveMode::Flattened).unwrap_err();
            assert!(error.to_string().contains("missing.usda"), "{error:#}");
            assert_eq!(std::fs::read(&path).unwrap(), b"existing output");
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 4);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        assert_eq!(editor.snapshot().unwrap().selected.as_deref(), Some("/Model"));
        assert!(editor.snapshot().unwrap().can_undo);
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer] {
            let path = directory.path().join("authored.usda");
            editor.save(path.to_str().unwrap(), mode).unwrap();
            assert!(std::fs::read_to_string(path).unwrap().contains("missing.usda"));
        }
        editor.undo().unwrap();
        assert_eq!(editor.stage().prim("/Model").unwrap().attribute("score").get::<f64>().unwrap(), None);
    }

    #[test]
    fn invalid_composition_open_preserves_document_selection_and_history() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("current.usda");
        std::fs::write(&current, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        std::fs::write(directory.path().join("model.usda"), "#usda 1.0\ndef Scope \"Present\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(current.to_string_lossy().into_owned())).unwrap();
        bridge.send(EditorCommand::Select(Some("/Root".into()))).unwrap();
        bridge.send(EditorCommand::Edit(EditorEdit::Attribute {
            prim: "/Root".into(), name: "visibility".into(), type_name: "token".into(), value: Value::Token("invisible".into()),
        })).unwrap();
        app.update();
        let entity = app.world().resource::<crate::live::PrimEntities>().entity("/Root").unwrap();
        let document_id = bridge.view().unwrap().document.document_id;
        for (name, text) in [
            ("layer", "#usda 1.0\n(subLayers = [@missing.usda@])\ndef Scope \"Broken\" {}\n"),
            ("reference", "#usda 1.0\ndef Scope \"Broken\" (prepend references = @missing.usda@</Model>) {}\n"),
            ("root-target", "#usda 1.0\ndef Scope \"Broken\" (prepend references = @model.usda@</Absent>) {}\n"),
            ("subroot-target", "#usda 1.0\ndef Scope \"Broken\" (prepend references = @model.usda@</Present/Absent>) {}\n"),
            ("payload", "#usda 1.0\ndef Scope \"Broken\" (prepend payload = @missing.usda@</Model>) {}\n"),
        ] {
            let path = directory.path().join(format!("{name}.usda"));
            std::fs::write(&path, text).unwrap();
            bridge.send(EditorCommand::Open(path.to_string_lossy().into_owned())).unwrap();
            app.update();
            let view = bridge.view().unwrap();
            assert!(view.status.starts_with("Failed:"), "{name}: {}", view.status);
            assert_eq!(view.document.document_id, document_id);
            assert_eq!(view.document.selected.as_deref(), Some("/Root"));
            assert!(view.document.can_undo);
            assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Root"), Some(entity));
            assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        }
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        assert_ne!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        assert!(bridge.view().unwrap().document.can_redo);
    }

    #[test]
    fn command_bridge_edits_the_projected_document_and_preserves_it_on_open_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.usda");
        std::fs::write(&path, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(path.to_string_lossy().into_owned())).unwrap();
        bridge.send(EditorCommand::Select(Some("/Root".into()))).unwrap();
        app.update();
        let entity = app.world().resource::<crate::live::PrimEntities>().entity("/Root").unwrap();
        assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Root"));
        bridge.send(EditorCommand::Edit(EditorEdit::Attribute {
            prim: "/Root".into(), name: "visibility".into(), type_name: "token".into(),
            value: Value::Token("invisible".into()),
        })).unwrap();
        app.update();
        assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        assert!(bridge.view().unwrap().document.can_undo);
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        assert_ne!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        bridge.send(EditorCommand::Open(directory.path().join("absent.usda").to_string_lossy().into_owned())).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/Root"), Some(entity));
    }

    #[test]
    fn unchanged_reload_reuses_inspection_but_changes_and_errors_refresh_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.usda");
        std::fs::write(&path, "#usda 1.0\ndef Xform \"Root\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<EditorSnapshotTiming>();
        app.world_mut().resource_mut::<reload::EditorReloadSettings>().enabled = false;
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(path.to_string_lossy().into_owned())).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 1);
        let document = bridge.view().unwrap().document.document_id;
        bridge.send(EditorCommand::ReloadSources(vec![path.clone()])).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 1);
        assert_eq!(bridge.view().unwrap().status, "Ready");
        std::fs::write(&path, "#usda 1.0\ndef Xform \"Root\" {}\ndef Xform \"Added\" {}\n").unwrap();
        bridge.send(EditorCommand::ReloadSources(vec![path.clone()])).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 2);
        assert!(bridge.view().unwrap().document.prims.contains(&"/Added".to_owned()));
        bridge.send(EditorCommand::Select(Some("/Added".into()))).unwrap();
        bridge.send(EditorCommand::ReloadSources(vec![path.clone()])).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 3);
        assert_eq!(bridge.view().unwrap().document.selected.as_deref(), Some("/Added"));
        std::fs::write(&path, "not a USD layer").unwrap();
        bridge.send(EditorCommand::ReloadSources(vec![path.clone()])).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 4);
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert_eq!(bridge.view().unwrap().document.document_id, document);
        assert!(bridge.view().unwrap().document.prims.contains(&"/Added".to_owned()));
        std::fs::write(&path, "#usda 1.0\ndef Xform \"Root\" {}\ndef Xform \"Added\" {}\n").unwrap();
        app.world().non_send::<crate::live::LiveStage>().stage.define_prim("/External").unwrap().set_type_name("Xform").unwrap();
        bridge.send(EditorCommand::ReloadSources(vec![path])).unwrap();
        app.update();
        assert_eq!(app.world().resource::<EditorSnapshotTiming>().snapshots, 5);
        assert!(bridge.view().unwrap().document.prims.contains(&"/External".to_owned()));
    }

    #[test]
    fn undo_restores_unauthored_fallback_and_redo_replays_edit() {
        let mut editor = session();
        editor.select(Some("/Box".into())).unwrap();
        let before = crate::authoring::export_stage_string(editor.stage()).unwrap();
        editor.edit(EditorEdit::Attribute {
            prim: "/Box".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(5.0),
        }).unwrap();
        assert!(editor.snapshot().unwrap().can_undo);
        assert_eq!(editor.stage().prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(5.0));
        assert!(editor.undo().unwrap());
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), before);
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(5.0));
    }

    #[test]
    fn failed_edit_rolls_back_attribute_creation() {
        let mut editor = session();
        let before = crate::authoring::export_stage_string(editor.stage()).unwrap();
        assert!(editor.edit(EditorEdit::Attribute {
            prim: "/Box".into(), name: "bad".into(), type_name: "double".into(),
            value: Value::String("not a double".into()),
        }).is_err());
        assert_eq!(crate::authoring::export_stage_string(editor.stage()).unwrap(), before);
        assert!(!editor.snapshot().unwrap().can_undo);
    }

    #[test]
    fn selection_and_live_projection_use_the_same_stage() {
        use bevy::prelude::*;
        let mut editor = session();
        let live = crate::live::LiveStage::new(editor.stage().clone());
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        assert!(editor.select(Some("/Missing".into())).is_err());
        editor.select(Some("/Box".into())).unwrap();
        editor.edit(EditorEdit::Remove { path: "/Box".into() }).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(map.entity("/Box").is_none());
        assert!(editor.snapshot().unwrap().selected.is_none());
        editor.undo().unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(map.entity("/Box").is_some());
    }

    #[test]
    fn layer_target_undo_and_save_modes_preserve_composition() {
        let directory = tempfile::tempdir().unwrap();
        let weak = directory.path().join("weak.usda");
        std::fs::write(&weak, "#usda 1.0\ndef Cube \"Box\" { double size = 3 }\n").unwrap();
        let source = crate::UsdSource::new(directory.path().join("root.usda"),
            &b"#usda 1.0\n( subLayers = [@weak.usda@] )\n"[..]).unwrap();
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        let root = editor.stage().edit_target().layer_identifier().to_string();
        editor.set_edit_layer(weak.to_str().unwrap()).unwrap();
        editor.edit(EditorEdit::Attribute {
            prim: "/Box".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(7.0),
        }).unwrap();
        editor.set_edit_layer(&root).unwrap();
        editor.undo().unwrap();
        assert_eq!(editor.stage().prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(3.0));
        editor.redo().unwrap();
        assert_eq!(editor.stage().edit_target().layer_identifier(), root);
        editor.select(Some("/Box".into())).unwrap();
        let snapshot = editor.snapshot().unwrap();
        assert_eq!(snapshot.layers.len(), 2);
        assert!(snapshot.attributes.iter().any(|a| a.name == "size" && a.value == Some(Value::Double(7.0))));
        let flat = directory.path().join("flat.usda");
        editor.save(flat.to_str().unwrap(), SaveMode::Flattened).unwrap();
        let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(flat.to_str().unwrap()).unwrap();
        assert_eq!(reopened.prim("/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(7.0));
        let root_out = directory.path().join("root_out.usda");
        editor.save(root_out.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert!(std::fs::read_to_string(root_out).unwrap().contains("subLayers"));
        editor.set_edit_layer(weak.to_str().unwrap()).unwrap();
        let layer_out = directory.path().join("layer_out.usda");
        editor.save(layer_out.to_str().unwrap(), SaveMode::EditLayer).unwrap();
        assert!(std::fs::read_to_string(layer_out).unwrap().contains("size = 7"));
    }
}
