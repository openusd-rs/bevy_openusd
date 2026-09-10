//! Composition-aware editor state over one authoritative live stage.

use openusd::sdf::Value;
use openusd::usd::{EditTarget, Stage, UndoStage};
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum EditorCommand {
    Open(String),
    Select(Option<String>),
    Edit(EditorEdit),
    EditLayer(String),
    Payload { prim: String, loaded: bool },
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
        app.init_resource::<EditorBridge>().init_resource::<EditorPlayback>()
            .init_resource::<crate::route::StageTime>()
            .add_systems(PreUpdate, (process_commands, advance_editor_time).chain())
            .add_systems(Last, publish_projection_issues);
    }
}

fn publish_projection_issues(
    bridge: Res<EditorBridge>,
    prims: Option<Res<crate::live::PrimEntities>>,
    issues: Query<&crate::route::reflect::UsdReflectIssues>,
    rendering: Query<(Option<&crate::route::subdivision::UsdSubdivisionError>,
        Option<&crate::route::skel::UsdDeformationError>, Option<&crate::route::instancer::UsdInstancerWarning>,
        Option<&crate::route::shapes::UsdShapeError>, Option<&crate::route::curves::UsdCurveError>)>,
) {
    let Ok(mut state) = bridge.0.lock() else { return };
    state.view.document.reflect_issues = state.view.document.selected.as_deref()
        .and_then(|path| prims.as_ref()?.entity(path))
        .and_then(|entity| issues.get(entity).ok())
        .map_or_else(Vec::new, |issues| issues.0.clone());
    state.view.document.render_issues = state.view.document.selected.as_deref()
        .and_then(|path| prims.as_ref()?.entity(path))
        .and_then(|entity| rendering.get(entity).ok())
        .map_or_else(Vec::new, |(subdivision, deformation, instancer, shape, curve)| {
            [subdivision.map(|error| format!("Subdivision: {}", error.0)),
                deformation.map(|error| format!("Deformation: {}", error.0)),
                instancer.map(|error| format!("Point instancer: {}", error.0)),
                shape.map(|error| format!("Shape: {}", error.0)),
                curve.map(|error| format!("Curve: {}", error.0))]
                .into_iter().flatten().collect()
        });
}

fn advance_editor_time(world: &mut World) {
    let Some(stage) = world.get_non_send::<EditorSession>().map(|editor| editor.stage().clone()) else { return };
    let delta = world.get_resource::<Time>().map_or(0.0, Time::delta_secs_f64);
    let current = world.resource::<crate::route::StageTime>().current;
    let current = world.resource_mut::<EditorPlayback>().0.advance(current, delta, &stage);
    world.resource_mut::<crate::route::StageTime>().current = current;
    let timeline = EditorTimeline { current, start: stage.start_time_code(), end: stage.end_time_code(),
        playing: world.resource::<EditorPlayback>().0.playing };
    let bridge = world.resource::<EditorBridge>();
    if let Ok(mut state) = bridge.0.lock() { state.view.timeline = timeline; }
}

fn process_commands(world: &mut World) {
    let bridge = world.resource::<EditorBridge>().clone();
    let commands = match bridge.0.lock() {
        Ok(mut state) => std::mem::take(&mut state.commands),
        Err(_) => return,
    };
    let mut session = world.remove_non_send::<EditorSession>();
    let external = session.as_mut().is_some_and(EditorSession::synchronize_external_edits);
    if commands.is_empty() && !external {
        if let Some(session) = session { world.insert_non_send(session); }
        return;
    }
    let mut status = if external { "External edits detected; undo history reset".into() } else { String::new() };
    let mut texture_dirty = external;
    for command in commands {
        let movement = session.as_ref().and_then(|editor| match &command {
            EditorCommand::Edit(edit) => edit.namespace_move(),
            EditorCommand::Undo => editor.undo.last().and_then(|entry| entry.edit.namespace_move()).map(|(old, new)| (new, old)),
            EditorCommand::Redo => editor.redo.last().and_then(|entry| entry.edit.namespace_move()),
            _ => None,
        });
        if matches!(&command, EditorCommand::Edit(_) | EditorCommand::Undo | EditorCommand::Redo | EditorCommand::Payload { .. }) {
            texture_dirty = true;
        }
        let result = if let EditorCommand::Open(path) = &command {
            std::fs::read(path).map_err(anyhow::Error::from)
                .and_then(|bytes| crate::UsdSource::new(path, bytes).map_err(anyhow::Error::from)).and_then(|source| {
                    let stage = source.open_stage()?;
                    crate::UsdSource::validate_composition(&stage)?;
                    let textures = prepare_textures(&stage, &source)?;
                    if !textures.is_empty() && !world.contains_resource::<Assets<Image>>() {
                        anyhow::bail!("image assets are unavailable for this document");
                    }
                    world.remove_non_send::<crate::live::LiveStage>();
                    if let Some(map) = world.remove_resource::<crate::live::PrimEntities>() {
                        if let Some(root) = map.entity("/") { world.despawn(root); }
                    }
                    world.insert_resource(crate::live::PrimEntities::default());
                    world.insert_non_send(crate::live::LiveStage::new(stage.clone()));
                    let mut editor = EditorSession::new(stage);
                    editor.source = Some(source);
                    install_textures(world, textures)?;
                    world.resource_mut::<EditorPlayback>().0.playing = false;
                    texture_dirty = false;
                    session = Some(editor);
                    Ok(())
                })
        } else if let Some(editor) = &mut session {
            match command {
                EditorCommand::Select(path) => editor.select(path),
                EditorCommand::Edit(edit) => editor.edit(edit),
                EditorCommand::EditLayer(identifier) => editor.set_edit_layer(&identifier),
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
                EditorCommand::Open(_) => unreachable!(),
            }
        } else {
            Err(anyhow::anyhow!("no USD document is open"))
        };
        if result.is_ok() {
            if let Some((old, new)) = movement { crate::live::remap_namespace(world, &old, &new); }
        }
        status = match result { Ok(()) => "Ready".into(), Err(error) => format!("Failed: {error:#}") };
    }
    if let Some(editor) = session.as_ref().filter(|_| texture_dirty) {
        if let Some(source) = &editor.source {
            match prepare_textures(editor.stage(), source).and_then(|textures| install_textures(world, textures)) {
                Ok(()) => {
                    if let Some(live) = world.get_non_send::<crate::live::LiveStage>() { live.enqueue_resync("/"); }
                }
                Err(error) => status = format!("Texture loading failed: {error:#}"),
            }
        }
    }
    let document = session.as_ref().map(EditorSession::snapshot).transpose();
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

fn prepare_textures(stage: &Stage, source: &crate::UsdSource) -> anyhow::Result<PreparedTextures> {
    let mut images = Vec::new();
    for (path, srgb) in crate::UsdSource::stage_texture_requests(stage).map_err(anyhow::Error::msg)? {
        let bytes = source.read_asset(&path)?;
        let inner = openusd::ar::split_package_relative_path_inner(&path).map(|(_, inner)| inner).unwrap_or_else(|| path.clone());
        let extension = std::path::Path::new(&inner).extension().and_then(|extension| extension.to_str())
            .ok_or_else(|| anyhow::anyhow!("texture has no extension: {path}"))?;
        let image = Image::from_buffer(&bytes, bevy::image::ImageType::Extension(extension),
            bevy::image::CompressedImageFormats::NONE, srgb, bevy::image::ImageSampler::default(),
            bevy::asset::RenderAssetUsages::default())?;
        images.push(((path, srgb), image));
    }
    Ok(images)
}

fn install_textures(world: &mut World, prepared: PreparedTextures) -> anyhow::Result<()> {
    anyhow::ensure!(prepared.is_empty() || world.contains_resource::<Assets<Image>>(), "image assets are unavailable for this document");
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
    ClearReferences { prim: String },
    RelationshipTargets { prim: String, name: String, targets: Vec<openusd::sdf::Path> },
    ClearRelationshipTargets { prim: String, name: String },
}

impl EditorEdit {
    fn namespace_move(&self) -> Option<(String, String)> {
        let old = match self {
            Self::Rename { path, .. } | Self::Reparent { path, .. } | Self::Move { path, .. } => path,
            _ => return None,
        };
        Some((old.clone(), self.selection_after(Some(old))?))
    }

    fn apply(&self, stage: &Stage) -> anyhow::Result<()> {
        use crate::authoring;
        match self {
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
            Self::ClearReferences { prim } => authoring::clear_references(stage, prim),
            Self::RelationshipTargets { prim, name, targets } => authoring::set_relationship_targets(stage, prim, name, targets),
            Self::ClearRelationshipTargets { prim, name } => authoring::clear_relationship_targets(stage, prim, name),
        }
    }

    fn selection_after(&self, selected: Option<&str>) -> Option<String> {
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
    transactions: usize,
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
    pub source: String,
    pub source_summary: String,
    pub sample_times: Vec<f64>,
    pub blocked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct EditorSnapshot {
    pub document_id: u64,
    pub asset_info: Option<crate::read::geom::CustomDict>,
    pub render_issues: Vec<String>,
    pub reflect_issues: Vec<crate::route::reflect::ReflectIssue>,
    pub relationships: Vec<(String, Vec<String>)>,
    pub material_warnings: Vec<String>,
    pub prims: Vec<String>,
    pub visibility: std::collections::HashMap<String, bool>,
    pub layers: Vec<String>,
    pub edit_layer: String,
    pub selected: Option<String>,
    pub attributes: Vec<AttributeSnapshot>,
    pub variants: Vec<(String, String)>,
    pub variant_choices: std::collections::BTreeMap<String, Vec<String>>,
    pub selected_loaded: Option<bool>,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Main-thread editor model. Clone `stage()` into `LiveStage` to project the
/// same document; route undoable authoring through `edit()`.
pub struct EditorSession {
    document_id: u64,
    source: Option<crate::UsdSource>,
    stage: UndoStage,
    selected: Option<String>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

impl EditorSession {
    pub fn new(stage: Stage) -> Self {
        static NEXT_DOCUMENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            document_id: NEXT_DOCUMENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            source: None,
            stage: UndoStage::with_capacity(stage, usize::MAX),
            selected: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn stage(&self) -> &Stage { &self.stage }

    pub fn select(&mut self, path: Option<String>) -> anyhow::Result<()> {
        if let Some(path) = &path {
            let path = openusd::sdf::path(path)?;
            anyhow::ensure!(self.stage.prim(path)?.is_valid()?, "selected prim does not exist");
        }
        self.selected = path;
        Ok(())
    }

    pub fn set_edit_layer(&self, identifier: &str) -> anyhow::Result<()> {
        self.stage.set_edit_target(EditTarget::for_layer(identifier))?;
        Ok(())
    }

    /// Changes runtime load rules without authoring layer opinions.
    pub fn set_payload_loaded(&self, path: &str, loaded: bool) -> anyhow::Result<()> {
        let path = openusd::sdf::path(path)?;
        anyhow::ensure!(self.stage.prim(path.clone())?.is_valid()?, "payload prim does not exist");
        if loaded { self.stage.load(path, openusd::usd::LoadPolicy::WithDescendants)?; }
        else { self.stage.unload(path)?; }
        Ok(())
    }

    pub fn edit(&mut self, edit: EditorEdit) -> anyhow::Result<()> {
        self.synchronize_external_edits();
        let target = self.stage.edit_target();
        let before = self.stage.undo_depth();
        if let Err(error) = edit.apply(&self.stage) {
            while self.stage.undo_depth() > before { self.stage.undo()?; }
            return Err(error);
        }
        let transactions = self.stage.undo_depth() - before;
        if transactions > 0 {
            let selection_before = self.selected.clone();
            self.selected = edit.selection_after(self.selected.as_deref());
            self.undo.push(HistoryEntry { edit, target, transactions, selection_before, selection_after: self.selected.clone() });
            self.redo.clear();
        }
        Ok(())
    }

    pub fn undo(&mut self) -> anyhow::Result<bool> {
        self.synchronize_external_edits();
        let Some(entry) = self.undo.last_mut() else { return Ok(false) };
        while entry.transactions > 0 {
            anyhow::ensure!(self.stage.undo()?, "editor transaction history is inconsistent");
            entry.transactions -= 1;
        }
        if entry.selection_before != entry.selection_after { self.selected = entry.selection_before.clone(); }
        self.redo.push(self.undo.pop().unwrap());
        Ok(true)
    }

    pub fn redo(&mut self) -> anyhow::Result<bool> {
        self.synchronize_external_edits();
        let Some(entry) = self.redo.last() else { return Ok(false) };
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
        if entry.selection_before != entry.selection_after { self.selected = entry.selection_after.clone(); }
        self.undo.push(entry);
        Ok(true)
    }

    /// Establishes a new history baseline when edits bypass the command model.
    pub fn synchronize_external_edits(&mut self) -> bool {
        let tracked: usize = self.undo.iter().map(|entry| entry.transactions).sum();
        if tracked == self.stage.undo_depth() { return false; }
        self.stage.reset();
        self.undo.clear();
        self.redo.clear();
        true
    }

    pub fn save(&self, filename: &str, mode: SaveMode) -> anyhow::Result<()> {
        match mode {
            SaveMode::RootLayer => crate::persistence::export_layer(&self.stage, &self.stage.root_layer(), filename)?,
            SaveMode::EditLayer => {
                let target = self.stage.edit_target();
                let layer = self.stage.layer(target.layer_identifier())
                    .ok_or_else(|| anyhow::anyhow!("edit layer is unavailable"))?;
                crate::persistence::export_layer(&self.stage, &layer, filename)?;
            }
            SaveMode::Flattened => {
                crate::UsdSource::validate_composition(&self.stage)?;
                crate::persistence::export_layer(&self.stage, &self.stage.flatten()?, filename)?;
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> anyhow::Result<EditorSnapshot> {
        let mut snapshot = EditorSnapshot {
            document_id: self.document_id,
            layers: self.stage.layer_stack(),
            edit_layer: self.stage.edit_target().layer_identifier().to_string(),
            selected: self.selected.clone(),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            ..Default::default()
        };
        let predicate = openusd::usd::PrimPredicate::new(
            openusd::usd::PrimStatus::ACTIVE.union(openusd::usd::PrimStatus::DEFINED),
            openusd::usd::PrimStatus::ABSTRACT,
        ).with_instance_proxies(true);
        self.stage.traverse(predicate, |path: &openusd::sdf::Path| {
            snapshot.prims.push(path.as_str().to_owned());
        })?;
        for path in &snapshot.prims {
            let value = self.stage.prim(openusd::sdf::path(path)?)?.attribute("visibility").get::<Value>()?;
            snapshot.visibility.insert(path.clone(), !matches!(value, Some(Value::Token(token)) if token.as_str() == "invisible"));
        }
        if let Some(path) = &self.selected {
            if !snapshot.prims.contains(path) { snapshot.selected = None; return Ok(snapshot); }
            let prim = self.stage.prim(openusd::sdf::path(path)?)?;
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
                snapshot.attributes.push(AttributeSnapshot {
                    name: attribute.path().as_str().rsplit('.').next().unwrap_or_default().to_string(),
                    type_name: attribute.type_name()?.map(|ty| ty.to_string()).unwrap_or_default(),
                    value: attribute.get::<Value>()?,
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
    use super::*;

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
    fn selected_render_failures_publish_and_clear() {
        use crate::route::{subdivision::UsdSubdivisionError, skel::UsdDeformationError, instancer::UsdInstancerWarning, shapes::UsdShapeError, curves::UsdCurveError};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EditorPlugin));
        app.init_resource::<crate::live::PrimEntities>();
        let entity = app.world_mut().spawn((UsdSubdivisionError("unsupported holes".into()),
            UsdDeformationError("invalid influences".into()), UsdInstancerWarning("missing prototype".into()), UsdShapeError("invalid dimensions".into()), UsdCurveError("invalid counts".into()))).id();
        app.world_mut().resource_mut::<crate::live::PrimEntities>().insert("/Prim", entity);
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.0.lock().unwrap().view.document.selected = Some("/Prim".into());
        app.update();
        assert_eq!(bridge.view().unwrap().document.render_issues,
            ["Subdivision: unsupported holes", "Deformation: invalid influences", "Point instancer: missing prototype", "Shape: invalid dimensions", "Curve: invalid counts"]);
        app.world_mut().entity_mut(entity).remove::<(UsdSubdivisionError, UsdDeformationError, UsdInstancerWarning, UsdShapeError, UsdCurveError)>();
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
    fn timeline_commands_scrub_loop_and_pause_without_authoring() {
        let stage = crate::UsdSource::new("timeline.usda", &br#"#usda 1.0
( startTimeCode = 0 endTimeCode = 10 timeCodesPerSecond = 10 )
def Xform "Mover" {
    double3 xformOp:translate.timeSamples = { 0: (0,0,0), 10: (10,0,0) }
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
        assert!(!bridge.view().unwrap().document.can_undo);
        bridge.send(EditorCommand::Seek(f64::NAN)).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().timeline.current, 5.0);
        assert!(bridge.view().unwrap().status.contains("finite"));
        bridge.send(EditorCommand::Seek(9.0)).unwrap();
        bridge.send(EditorCommand::Play(true)).unwrap();
        app.update();
        assert!(bridge.view().unwrap().timeline.current.abs() < 1e-6);
        app.update();
        assert!((bridge.view().unwrap().timeline.current - 1.0).abs() < 1e-6);
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
