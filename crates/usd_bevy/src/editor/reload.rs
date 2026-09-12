//! Native disk-layer refresh for the live editor document.

use super::*;
use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf, time::{Duration, Instant, SystemTime}};

#[derive(Resource, Default, Debug)]
pub struct EditorReloadStatus {
    pub error: Option<String>,
    pub files: usize,
}

#[derive(Resource, Default)]
pub(super) struct WatchState {
    document: Option<u64>,
    next: Option<Instant>,
    observed: BTreeMap<PathBuf, Option<(SystemTime, u64)>>,
    pending: BTreeSet<PathBuf>,
}

fn outer_path(id: &str) -> PathBuf {
    openusd::ar::split_package_relative_path_outer(id)
        .map_or_else(|| PathBuf::from(id), |(outer, _)| PathBuf::from(outer))
}

pub(super) fn watch(
    session: Option<NonSend<EditorSession>>,
    bridge: Res<EditorBridge>,
    mut state: ResMut<WatchState>,
    mut status: ResMut<EditorReloadStatus>,
) {
    let now = Instant::now();
    if state.next.is_some_and(|next| now < next) { return; }
    state.next = Some(now + Duration::from_millis(250));
    let Some(editor) = session else { return; };
    let Some(disk) = editor.save_state.borrow().disk.clone() else { return; };
    let paths = disk.lock().expect("disk baselines").keys().cloned().collect::<Vec<_>>();
    let observed: BTreeMap<_, _> = paths.into_iter().map(|path| {
        let stamp = std::fs::metadata(&path).ok().and_then(|meta| Some((meta.modified().ok()?, meta.len())));
        (path, stamp)
    }).collect();
    status.files = observed.len();
    if state.document != Some(editor.document_id) {
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
        self.reload_paths(None)
    }

    pub(super) fn reload_paths(&mut self, paths: Option<&[PathBuf]>) -> anyhow::Result<()> {
        let disk = self.save_state.borrow().disk.clone()
            .ok_or_else(|| anyhow::anyhow!("document has no disk provenance"))?;
        let baselines = disk.lock().expect("disk baselines").clone();
        let mut changed = BTreeMap::new();
        for (path, baseline) in baselines {
            if paths.is_some_and(|paths| !paths.contains(&path)) { continue; }
            let bytes = std::fs::read(&path)?;
            let hash = blake3::hash(&bytes);
            if hash != baseline { changed.insert(path, (bytes, hash)); }
        }
        if changed.is_empty() { return Ok(()); }
        let states = self.save_state.borrow_mut().states(self.stage(), &self.layer_changes.revisions(), self.layer_changes.structural_revision());
        let mut replacements = Vec::new();
        let mut accepted = BTreeSet::new();
        for id in self.stage.layer_identifiers() {
            let Some(layer) = self.stage.layer(&id) else { continue; };
            let Some(path) = layer.resolved_path() else { continue; };
            let key = crate::persistence::destination_identity(&outer_path(path))?;
            let Some((bytes, _)) = changed.get(&key) else { continue; };
            anyhow::ensure!(states.get(&id) == Some(&save_state::LayerSaveState::Clean),
                "reload conflict: {id} has unsaved edits; save elsewhere or undo them before reloading");
            anyhow::ensure!(!openusd::ar::is_package_relative_path(path), "package layer reload is not yet available: {path}");
            replacements.push((id, bytes.clone()));
            accepted.insert(key);
        }
        if replacements.is_empty() { return Ok(()); }
        let plan = crate::reload::LayerReload::prepare(self.stage(), &replacements)?;
        plan.apply(self.stage())?;
        self.synchronize_external_edits();
        for (id, _) in &replacements {
            self.save_state.borrow_mut().saved_to_source(self.stage(), id, id);
        }
        disk.lock().expect("disk baselines").extend(changed.into_iter()
            .filter(|(path, _)| accepted.contains(path)).map(|(path, (_, hash))| (path, hash)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
