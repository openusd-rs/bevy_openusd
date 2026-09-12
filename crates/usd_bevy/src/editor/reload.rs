//! Native disk-layer refresh for the live editor document.

use super::*;
use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf, time::{Duration, Instant, SystemTime}};

#[derive(Resource, Default, Debug)]
pub struct EditorReloadStatus {
    pub error: Option<String>,
    pub files: usize,
}

#[derive(Resource, Debug)]
pub struct EditorReloadSettings {
    pub enabled: bool,
    pub interval: Duration,
}

impl Default for EditorReloadSettings {
    fn default() -> Self { Self { enabled: true, interval: Duration::from_millis(250) } }
}

#[derive(Resource, Default)]
pub(super) struct WatchState {
    document: Option<u64>,
    next: Option<Instant>,
    observed: BTreeMap<PathBuf, Option<(SystemTime, u64)>>,
    pending: BTreeSet<PathBuf>,
    retry: Option<Instant>,
    revision: u64,
}

fn outer_path(id: &str) -> PathBuf {
    openusd::ar::split_package_relative_path_outer(id)
        .map_or_else(|| PathBuf::from(id), |(outer, _)| PathBuf::from(outer))
}

fn layer_files(stage: &Stage) -> BTreeSet<PathBuf> {
    stage.layer_identifiers().into_iter().filter_map(|id| {
        let layer = stage.layer(&id)?;
        let path = outer_path(layer.resolved_path()?);
        Some(crate::persistence::destination_identity(&path).unwrap_or(path))
    }).collect()
}

pub(super) fn watch(
    session: Option<NonSend<EditorSession>>,
    textures: Option<Res<EditorTextureRequests>>,
    settings: Res<EditorReloadSettings>,
    bridge: Res<EditorBridge>,
    mut state: ResMut<WatchState>,
    mut status: ResMut<EditorReloadStatus>,
) {
    if !settings.enabled { return; }
    let now = Instant::now();
    if state.next.is_some_and(|next| now < next) { return; }
    state.next = Some(now + settings.interval.max(Duration::from_millis(10)));
    let Some(editor) = session else { return; };
    if editor.source.as_ref().is_none_or(|source| !source.filesystem_backed()) { return; }
    if status.error.is_some() && (state.revision != editor.revision
        || (status.error.as_ref().is_some_and(|error| !error.contains("unsaved edits"))
            && state.retry.is_none_or(|retry| now >= retry))) {
        let paths = state.observed.keys().cloned().collect::<Vec<_>>();
        state.pending.extend(paths);
        state.retry = Some(now + Duration::from_secs(1));
    }
    state.revision = editor.revision;
    if editor.save_state.borrow().disk.is_none() { return; }
    let mut paths = layer_files(editor.stage());
    if let Some(textures) = textures {
        paths.extend(textures.0.iter().map(|path| outer_path(path)).filter(|path| path.is_absolute()));
    }
    let observed: BTreeMap<_, _> = paths.into_iter().map(|path| {
        let stamp = std::fs::metadata(&path).ok().and_then(|meta| Some((meta.modified().ok()?, meta.len())));
        (path, stamp)
    }).collect();
    if status.files != observed.len() { status.files = observed.len(); }
    if state.document != Some(editor.document_id) {
        status.error = None;
        state.retry = None;
        state.document = Some(editor.document_id);
        state.pending = observed.keys().cloned().collect();
        state.observed = observed;
    } else if state.observed != observed {
        let changed = observed.iter().filter(|(path, stamp)| state.observed.get(*path) != Some(*stamp))
            .map(|(path, _)| path.clone()).collect::<Vec<_>>();
        state.pending.extend(changed);
        state.observed = observed;
    } else if !state.pending.is_empty() {
        let paths = std::mem::take(&mut state.pending).into_iter().collect();
        if let Err(error) = bridge.send(EditorCommand::ReloadSources(paths)) { status.error = Some(error.to_string()); }
    }
}

impl EditorSession {
    /// Reloads externally changed layers without replacing the stage or entities.
    pub fn reload_sources(&mut self) -> anyhow::Result<()> {
        self.reload_paths(None, |_, _| Ok(())).map(|_| ())
    }

    pub(super) fn reload_paths(&mut self, paths: Option<&[PathBuf]>,
        preflight: impl FnOnce(&mut TexturePublication, &Stage) -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<TexturePublication>> {
        anyhow::ensure!(self.source.as_ref().is_some_and(crate::UsdSource::filesystem_backed),
            "document is not backed by filesystem assets");
        self.synchronize_external_edits();
        let disk = self.save_state.borrow().disk.clone()
            .ok_or_else(|| anyhow::anyhow!("document has no disk provenance"))?;
        let baselines = disk.lock().expect("disk baselines").clone();
        let old_requests = crate::UsdSource::stage_texture_requests(self.stage()).map_err(anyhow::Error::msg)?;
        let layers = layer_files(self.stage());
        let mut watched = layers.clone();
        watched.extend(old_requests.iter().map(|(path, _)| outer_path(path)));
        let mut changed = BTreeMap::new();
        let mut missing_textures = BTreeMap::new();
        for path in watched {
            if paths.is_some_and(|paths| !paths.contains(&path)) { continue; }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if !layers.contains(&path) => { missing_textures.insert(path, error); continue; }
                Err(error) => return Err(error.into()),
            };
            let hash = blake3::hash(&bytes);
            if baselines.get(&path) != Some(&hash) { changed.insert(path, (bytes, hash)); }
        }
        if changed.is_empty() {
            if let Some((path, error)) = missing_textures.into_iter().next() {
                anyhow::bail!("cannot read texture {}: {error}", path.display());
            }
            return Ok(None);
        }
        let states = self.save_state.borrow_mut().states(self.stage(), &self.layer_changes.revisions(), self.layer_changes.structural_revision());
        let mut replacements = Vec::new();
        let mut accepted = BTreeSet::new();
        let mut source_bytes = BTreeMap::new();
        for id in self.stage.layer_identifiers() {
            let Some(layer) = self.stage.layer(&id) else { continue; };
            let Some(path) = layer.resolved_path() else { continue; };
            let key = crate::persistence::destination_identity(&outer_path(path))?;
            let Some((bytes, _)) = changed.get(&key) else { continue; };
            anyhow::ensure!(states.get(&id) == Some(&save_state::LayerSaveState::Clean),
                "reload conflict: {id} has unsaved edits; save elsewhere or undo them before reloading");
            let replacement = if let Some((outer, inner)) = openusd::ar::split_package_relative_path_outer(path) {
                source_bytes.insert(outer, bytes.clone());
                openusd::ar::read_package_entry(Box::new(std::io::Cursor::new(bytes.clone())), &inner)?
            } else {
                source_bytes.insert(path.to_owned(), bytes.clone());
                bytes.clone()
            };
            replacements.push((id, replacement));
            accepted.insert(key);
        }
        let plan = if replacements.is_empty() { None } else {
            Some(crate::reload::LayerReload::prepare(self.stage(), &replacements)?)
        };
        let candidate = plan.as_ref().map_or(self.stage(), |plan| plan.candidate());
        let requests = crate::UsdSource::stage_texture_requests(candidate).map_err(anyhow::Error::msg)?;
        for (path, _) in &requests {
            if let Some(error) = missing_textures.get(&outer_path(path)) {
                anyhow::bail!("cannot read texture {path}: {error}");
            }
        }
        let mut source = self.source.clone().ok_or_else(|| anyhow::anyhow!("document has no source snapshot"))?;
        for (path, bytes) in &source_bytes { source.replace_file_bytes(path.clone(), bytes.clone()); }
        let changed_requests: BTreeSet<_> = requests.iter().filter(|request|
            !old_requests.contains(*request) || changed.contains_key(&outer_path(&request.0))).cloned().collect();
        for (path, _) in &changed_requests {
            let outer = outer_path(path);
            if !changed.contains_key(&outer) {
                let bytes = std::fs::read(&outer)?;
                let hash = blake3::hash(&bytes);
                changed.insert(outer.clone(), (bytes, hash));
            }
            if let Some((bytes, _)) = changed.get(&outer) {
                source.replace_file_bytes(outer.to_string_lossy().into_owned(), bytes.clone());
                accepted.insert(outer);
            }
        }
        let prepared = decode_textures(&source, changed_requests)?;
        let mut publication = TexturePublication { requests, prepared, consumers: Vec::new() };
        preflight(&mut publication, candidate)?;
        for (path, (_, hash)) in &changed {
            anyhow::ensure!(blake3::hash(&std::fs::read(path)?) == *hash, "source changed during reload: {}", path.display());
        }
        if let Some(plan) = &plan {
            for (path, bytes) in plan.disk.snapshots.lock().expect("prepared source snapshots").iter() {
                let key = crate::persistence::destination_identity(&outer_path(path))?;
                if !baselines.contains_key(&key) {
                    source_bytes.insert(path.clone(), bytes.to_vec());
                }
            }
        }
        let previous = disk.replacements.lock().expect("editor source replacements").clone();
        disk.replacements.lock().expect("editor source replacements")
            .extend(source_bytes.iter().map(|(path, bytes)| (path.clone(), Arc::from(bytes.clone()))));
        if let Err(error) = self.stage.without_recording(|stage|
            plan.as_ref().map_or(Ok(()), |plan| plan.apply(stage))) {
            *disk.replacements.lock().expect("editor source replacements") = previous;
            return Err(error);
        }
        self.source = Some(source);
        if !replacements.is_empty() {
            let conflicts = |entry: &HistoryEntry|
                replacements.iter().any(|(id, _)| entry.layers.contains(id));
            let keep: Vec<_> = self.undo.iter().flat_map(|entry|
                std::iter::repeat_n(!conflicts(entry), entry.transactions)).collect();
            assert!(self.stage.retain_transactions(&keep), "editor transaction history is inconsistent");
            self.undo.retain(|entry| !conflicts(entry));
            self.redo.retain(|entry| !conflicts(entry));
            self.revision = self.revision.wrapping_add(1);
        }
        for (id, _) in &replacements {
            self.save_state.borrow_mut().reloaded_layer(self.stage(), id);
        }
        disk.lock().expect("disk baselines").extend(changed.into_iter()
            .filter(|(path, _)| accepted.contains(path)).map(|(path, (_, hash))| (path, hash)));
        Ok(Some(publication))
    }
}

pub(super) struct TexturePublication {
    pub requests: BTreeSet<(String, bool)>,
    pub prepared: PreparedTextures,
    consumers: Vec<String>,
}

impl TexturePublication {
    pub fn preflight(&mut self, world: &World, stage: &Stage) -> anyhow::Result<()> {
        anyhow::ensure!(self.prepared.is_empty() || world.contains_resource::<Assets<Image>>(), "image assets are unavailable");
        let changed: BTreeSet<_> = self.prepared.iter().map(|((path, _), _)| path.as_str()).collect();
        let time = world.resource::<crate::route::StageTime>().current;
        let mut consumers = Vec::new();
        if !changed.is_empty() && let Some(map) = world.get_resource::<crate::live::PrimEntities>() {
            for (path, _) in map.iter() {
                if path == "/" { continue; }
                let prim = stage.prim(path)?;
                if !prim.is_valid()? { continue; }
                if matches!(prim.type_name()?.as_deref(), Some("DomeLight" | "DomeLight_1")) {
                    let texture = crate::route::dome::asset_string(prim.attribute("inputs:texture:file").get_at::<Value>(Some(openusd::usd::TimeCode::new(time)))?);
                    if changed.contains(texture.as_str()) { consumers.push(path.to_string()); }
                }
                let Some(material) = crate::read::shade::read_material_binding(stage, &openusd::sdf::path(path)?)? else { continue; };
                let Some(read) = crate::read::shade::read_preview_material_at(stage, &material, Some(time))? else { continue; };
                if [&read.diffuse_texture, &read.emissive_texture, &read.normal_texture, &read.metallic_texture,
                    &read.roughness_texture, &read.occlusion_texture, &read.opacity_texture].into_iter()
                    .flatten().any(|path| changed.contains(path.as_str())) {
                    let path = if prim.type_name()?.as_deref() == Some("GeomSubset") {
                        path.rsplit_once('/').map_or(path, |(parent, _)| parent)
                    } else { path };
                    consumers.push(path.to_string());
                }
            }
        }
        self.consumers = consumers;
        Ok(())
    }

    pub fn install(self, world: &mut World) {
        let mut textures = world.remove_resource::<crate::asset::SnapshotTextures>().unwrap_or_default();
        textures.0.retain(|key, _| self.requests.contains(key));
        for (key, image) in self.prepared {
            let mut assets = world.resource_mut::<Assets<Image>>();
            if textures.0.get(&key).and_then(|handle| assets.get(handle)).is_some_and(|old|
                old.data == image.data && old.texture_descriptor == image.texture_descriptor) { continue; }
            textures.0.insert(key, assets.add(image));
        }
        set_texture_requests(world, self.requests.into_iter().map(|(path, _)| path).collect());
        world.insert_resource(textures);
        if let Some(live) = world.get_non_send::<crate::live::LiveStage>() {
            for path in self.consumers { live.enqueue_resync(&path); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_only_documents_do_not_reload_same_named_disk_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        std::fs::write(&root, "#usda 1.0\ndef Sphere \"Disk\" {}\n").unwrap();
        let source = crate::UsdSource::snapshot(&root, b"#usda 1.0\ndef Cube \"Snapshot\" {}\n".as_slice()).unwrap();
        let mut editor = EditorSession::from_source(source).unwrap();
        assert!(editor.reload_sources().unwrap_err().to_string().contains("not backed by filesystem"));
        assert!(editor.stage().prim("/Snapshot").unwrap().is_valid().unwrap());
        assert!(!editor.stage().prim("/Disk").unwrap().is_valid().unwrap());
    }

    #[test]
    fn missing_image_assets_reject_reload_before_stage_publication() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let original = b"#usda 1.0\ndef Cube \"Model\" {}\n";
        std::fs::write(&root, original).unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        std::fs::write(directory.path().join("pixel.png"), png).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(root.to_string_lossy().into_owned())).unwrap();
        app.update();
        let before = app.world().non_send::<EditorSession>().stage().root_layer().export_to_string().unwrap();
        let revision = bridge.view().unwrap().document.revision;
        let replacement = r#"#usda 1.0
def Cube "Model" { rel material:binding = </Mat> }
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
"#;
        std::fs::write(&root, replacement).unwrap();
        bridge.send(EditorCommand::ReloadSources(vec![root.clone()])).unwrap();
        app.update();
        assert!(bridge.view().unwrap().status.contains("image assets are unavailable"));
        let editor = app.world().non_send::<EditorSession>();
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        assert_eq!(bridge.view().unwrap().document.revision, revision);
        assert_eq!(editor.save_state.borrow().disk.as_ref().unwrap().lock().unwrap()[&root], blake3::hash(original));
        app.init_resource::<Assets<Image>>();
        bridge.send(EditorCommand::ReloadSources(vec![root])).unwrap();
        app.update();
        assert_eq!(bridge.view().unwrap().status, "Ready");
        assert!(app.world().non_send::<EditorSession>().stage().prim("/Mat").unwrap().is_valid().unwrap());
        assert_eq!(app.world().resource::<crate::asset::SnapshotTextures>().0.len(), 1);
    }

    #[test]
    fn unrelated_layer_reload_preserves_undo_and_redo() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child.usda");
        let write_child = |size| std::fs::write(&child,
            format!("#usda 1.0\ndef Cube \"Model\" {{ double size = {size} }}\n")).unwrap();
        write_child(1);
        let root = directory.path().join("root.usda");
        let text = b"#usda 1.0\ndef Xform \"External\" (prepend references = @child.usda@</Model>) {}\ndef Cube \"Local\" { double size = 1 }\n";
        std::fs::write(&root, text).unwrap();
        let mut editor = EditorSession::from_source(crate::UsdSource::new(&root, text.as_slice()).unwrap()).unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Local".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(2.0) }).unwrap();
        write_child(3);
        editor.reload_sources().unwrap();
        assert_eq!(editor.undo.len(), 1);
        assert_eq!(editor.stage.undo_depth(), 1);
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().prim("/Local").unwrap().attribute("size").get::<f64>().unwrap(), Some(1.0));
        write_child(5);
        editor.reload_sources().unwrap();
        assert_eq!(editor.redo.len(), 1);
        assert!(editor.redo().unwrap());
        assert_eq!(editor.stage().prim("/Local").unwrap().attribute("size").get::<f64>().unwrap(), Some(2.0));
        assert_eq!(editor.stage().prim("/External").unwrap().attribute("size").get::<f64>().unwrap(), Some(5.0));
    }

    #[test]
    fn reload_invalidates_redo_that_would_overwrite_the_source() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let text = b"#usda 1.0\ndef Cube \"Model\" { double size = 1 }\n";
        std::fs::write(&root, text).unwrap();
        let mut editor = EditorSession::from_source(crate::UsdSource::new(&root, text.as_slice()).unwrap()).unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Model".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(2.0) }).unwrap();
        editor.undo().unwrap();
        std::fs::write(&root, "#usda 1.0\ndef Cube \"Model\" { double size = 3 }\n").unwrap();
        editor.reload_sources().unwrap();
        assert!(!editor.redo().unwrap());
        assert_eq!(editor.stage().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap(), Some(3.0));
    }

    #[test]
    fn reload_prunes_conflicting_commands_without_dropping_unrelated_undo() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child.usda");
        std::fs::write(&child, "#usda 1.0\ndef Cube \"Model\" { double size = 1 }\n").unwrap();
        let root = directory.path().join("root.usda");
        let text = b"#usda 1.0\n(subLayers = [@child.usda@])\ndef Cube \"Local\" { double size = 1 }\n";
        std::fs::write(&root, text).unwrap();
        let mut editor = EditorSession::from_source(crate::UsdSource::new(&root, text.as_slice()).unwrap()).unwrap();
        let edit = |prim: &str, value| EditorEdit::Attribute {
            prim: prim.into(), name: "size".into(), type_name: "double".into(), value: Value::Double(value),
        };
        editor.set_edit_layer(child.to_str().unwrap()).unwrap();
        editor.edit(edit("/Model", 2.0)).unwrap();
        editor.edit(edit("/Model", 1.0)).unwrap();
        editor.set_edit_layer(root.to_str().unwrap()).unwrap();
        editor.edit(edit("/Local", 2.0)).unwrap();
        assert_eq!(editor.undo.len(), 3);
        std::fs::write(&child, "#usda 1.0\ndef Cube \"Model\" { double size = 3 }\n").unwrap();
        editor.reload_sources().unwrap();
        assert_eq!(editor.undo.len(), 1);
        assert_eq!(editor.stage.undo_depth(), 1);
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().prim("/Local").unwrap().attribute("size").get::<f64>().unwrap(), Some(1.0));
        assert_eq!(editor.stage().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap(), Some(3.0));
        assert!(!editor.undo().unwrap());
    }

    #[test]
    fn capture_suspension_is_nested_and_panic_safe() {
        let stage = UndoStage::from(Stage::builder().in_memory("capture.usda").unwrap());
        stage.define_prim("/Before").unwrap();
        stage.without_recording(|_| {
            stage.without_recording(|stage| { stage.define_prim("/Nested").unwrap(); });
            stage.define_prim("/Outer").unwrap();
        });
        assert_eq!(stage.undo_depth(), 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stage.without_recording(|stage| {
                stage.define_prim("/BeforePanic").unwrap();
                panic!("test unwind");
            });
        }));
        assert!(result.is_err());
        stage.define_prim("/After").unwrap();
        assert_eq!(stage.undo_depth(), 2);
        assert!(!stage.retain_transactions(&[false]));
        assert_eq!(stage.undo_depth(), 2);
        stage.undo().unwrap();
        assert!(!stage.prim("/After").unwrap().is_valid().unwrap());
        assert!(stage.prim("/BeforePanic").unwrap().is_valid().unwrap());
    }

    fn tick_until(app: &mut App, predicate: impl Fn(&World) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            app.update();
            if predicate(app.world()) { return; }
            assert!(Instant::now() < deadline, "reload timed out: {:?}", app.world().resource::<EditorBridge>().view().unwrap().status);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn native_save_updates_referenced_instances_and_preserves_other_mesh() {
        #[derive(Component, PartialEq, Debug)]
        struct Runtime(u32);
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child.usda");
        let model = |size| format!("#usda 1.0\ndef Cube \"Model\" {{ double size = {size} }}\n");
        std::fs::write(&child, model(1)).unwrap();
        let root = directory.path().join("root.usda");
        std::fs::write(&root, "#usda 1.0\ndef Xform \"A\" (prepend references = @child.usda@</Model>) {}\ndef Xform \"B\" (prepend references = @child.usda@</Model>) {}\ndef Cube \"Other\" { double size = 7 }\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<Assets<Image>>().init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(root.to_string_lossy().into_owned())).unwrap();
        app.update();
        let map = app.world().resource::<crate::live::PrimEntities>();
        let entities = ["/A", "/B", "/Other"].map(|path| map.entity(path).unwrap());
        let mesh = app.world().get::<Mesh3d>(entities[2]).unwrap().0.clone();
        let transform_tick = app.world().entity(entities[2]).get_ref::<Transform>().unwrap().last_changed();
        app.world_mut().entity_mut(entities[2]).insert(Runtime(42));
        let document = bridge.view().unwrap().document.document_id;
        std::fs::write(&child, model(3)).unwrap();
        tick_until(&mut app, |world| world.non_send::<EditorSession>().stage().prim("/A").unwrap().attribute("size").get::<f64>().unwrap() == Some(3.0));
        assert_eq!(bridge.view().unwrap().document.document_id, document);
        for (path, entity) in ["/A", "/B", "/Other"].into_iter().zip(entities) {
            assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity(path), Some(entity));
        }
        assert_eq!(app.world().get::<Mesh3d>(entities[2]).unwrap().0, mesh);
        assert_eq!(app.world().get::<Runtime>(entities[2]), Some(&Runtime(42)));
        assert_eq!(app.world().entity(entities[2]).get_ref::<Transform>().unwrap().last_changed(), transform_tick);
        std::fs::write(&child, "broken").unwrap();
        tick_until(&mut app, |_| bridge.view().unwrap().status.starts_with("Failed"));
        assert_eq!(app.world().non_send::<EditorSession>().stage().prim("/A").unwrap().attribute("size").get::<f64>().unwrap(), Some(3.0));
        let temporary = directory.path().join("export.tmp");
        std::fs::write(&temporary, model(5)).unwrap();
        std::fs::rename(&temporary, &child).unwrap();
        tick_until(&mut app, |world| world.non_send::<EditorSession>().stage().prim("/B").unwrap().attribute("size").get::<f64>().unwrap() == Some(5.0));
    }

    #[test]
    fn dirty_layer_conflict_retains_edits_and_disk_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let original = b"#usda 1.0\ndef Cube \"Model\" { double size = 1 }\n";
        std::fs::write(&root, original).unwrap();
        let mut editor = EditorSession::from_source(crate::UsdSource::new(&root, original.as_slice()).unwrap()).unwrap();
        crate::authoring::set_attribute(editor.stage(), "/Model", "size", "double", Value::Double(2.0)).unwrap();
        std::fs::write(&root, "#usda 1.0\ndef Cube \"Model\" { double size = 3 }\n").unwrap();
        assert!(editor.reload_sources().unwrap_err().to_string().contains("unsaved edits"));
        assert_eq!(editor.stage().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap(), Some(2.0));
    }

    #[test]
    fn package_layer_save_updates_in_place_repeatedly() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("model.usdz");
        let package = |size| {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            zip.start_file("root.usda", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            write!(zip, "#usda 1.0\ndef Cube \"Model\" {{ double size = {size} }}\n").unwrap();
            zip.finish().unwrap().into_inner()
        };
        let bytes = package(1);
        std::fs::write(&file, &bytes).unwrap();
        let mut editor = EditorSession::from_source(crate::UsdSource::new(&file, bytes).unwrap()).unwrap();
        let document = editor.document_id();
        for size in [3, 5] {
            std::fs::write(&file, package(size)).unwrap();
            editor.reload_sources().unwrap();
            assert_eq!(editor.document_id(), document);
            assert_eq!(editor.stage().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap(), Some(size as f64));
        }
    }

    #[test]
    fn missing_new_reference_recovers_when_dependency_appears() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        std::fs::write(&root, "#usda 1.0\ndef Xform \"A\" {}\n").unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<Assets<Image>>().init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(root.to_string_lossy().into_owned())).unwrap();
        app.update();
        let entity = app.world().resource::<crate::live::PrimEntities>().entity("/A").unwrap();
        std::fs::write(&root, "#usda 1.0\ndef Xform \"A\" (prepend references = @new.usda@</Model>) {}\n").unwrap();
        tick_until(&mut app, |_| bridge.view().unwrap().status.starts_with("Failed"));
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/A"), Some(entity));
        std::fs::write(directory.path().join("new.usda"), "#usda 1.0\ndef Xform \"Model\" { def Cube \"Child\" {} }\n").unwrap();
        tick_until(&mut app, |world| world.resource::<crate::live::PrimEntities>().entity("/A/Child").is_some());
        assert_eq!(app.world().resource::<crate::live::PrimEntities>().entity("/A"), Some(entity));
        std::fs::write(directory.path().join("new.usda"), "#usda 1.0\ndef Xform \"Model\" { def Sphere \"Next\" {} }\n").unwrap();
        tick_until(&mut app, |world| world.resource::<crate::live::PrimEntities>().entity("/A/Next").is_some());
        assert!(app.world().resource::<crate::live::PrimEntities>().entity("/A/Child").is_none());
    }

    #[test]
    fn native_texture_save_updates_only_its_consumer_and_recovers() {
        let png = |pixel: [u8; 4]| {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&pixel).unwrap();
            }
            bytes
        };
        let directory = tempfile::tempdir().unwrap();
        let texture = directory.path().join("pixel.png");
        std::fs::write(&texture, png([255, 0, 0, 255])).unwrap();
        let root = directory.path().join("root.usda");
        std::fs::write(&root, r#"#usda 1.0
def Cube "A" { rel material:binding = </Mat> }
def Cube "Other" {}
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
"#).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin, EditorPlugin));
        app.init_resource::<Assets<Image>>().init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(root.to_string_lossy().into_owned())).unwrap();
        app.update();
        let other = app.world().resource::<crate::live::PrimEntities>().entity("/Other").unwrap();
        let tick = app.world().entity(other).get_ref::<Transform>().unwrap().last_changed();
        let handle = app.world().get::<MeshMaterial3d<StandardMaterial>>(other).unwrap().0.clone();
        let key = (texture.to_string_lossy().into_owned(), true);
        let pixels = |world: &World| {
            let handle = &world.resource::<crate::asset::SnapshotTextures>().0[&key];
            world.resource::<Assets<Image>>().get(handle).unwrap().data.clone().unwrap()
        };
        std::fs::write(&texture, png([0, 255, 0, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 255, 0, 255]);
        assert_eq!(app.world().entity(other).get_ref::<Transform>().unwrap().last_changed(), tick);
        assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(other).unwrap().0, handle);
        std::fs::write(&texture, b"broken").unwrap();
        tick_until(&mut app, |_| bridge.view().unwrap().status.starts_with("Failed"));
        assert_eq!(pixels(app.world()), [0, 255, 0, 255]);
        std::fs::write(&texture, png([0, 0, 255, 255])).unwrap();
        tick_until(&mut app, |world| pixels(world) == [0, 0, 255, 255]);
        let next = directory.path().join("next.png");
        std::fs::write(&next, png([255, 255, 0, 255])).unwrap();
        let original_root = std::fs::read_to_string(&root).unwrap();
        std::fs::write(&root, original_root.replace("@pixel.png@", "@next.png@")).unwrap();
        std::fs::remove_file(&texture).unwrap();
        let next_key = (next.to_string_lossy().into_owned(), true);
        tick_until(&mut app, |world| world.resource::<crate::asset::SnapshotTextures>().0.contains_key(&next_key));
        tick_until(&mut app, |world| world.resource::<EditorReloadStatus>().files == 2
            && !world.resource::<WatchState>().observed.contains_key(&texture));
        assert!(!app.world().resource::<WatchState>().observed.contains_key(&texture));
        let next_pixels = |world: &World| {
            let handle = &world.resource::<crate::asset::SnapshotTextures>().0[&next_key];
            world.resource::<Assets<Image>>().get(handle).unwrap().data.clone().unwrap()
        };
        std::fs::write(&next, png([255, 0, 255, 255])).unwrap();
        tick_until(&mut app, |world| next_pixels(world) == [255, 0, 255, 255]);
        std::fs::write(&root, &original_root).unwrap();
        tick_until(&mut app, |_| bridge.view().unwrap().status.starts_with("Failed"));
        assert!(app.world().resource::<crate::asset::SnapshotTextures>().0.contains_key(&next_key));
        std::fs::write(&texture, png([0, 255, 255, 255])).unwrap();
        tick_until(&mut app, |world| world.resource::<crate::asset::SnapshotTextures>().0.contains_key(&key));
        assert_eq!(pixels(app.world()), [0, 255, 255, 255]);
    }
}
