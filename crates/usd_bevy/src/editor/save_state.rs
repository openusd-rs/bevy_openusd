use std::collections::BTreeMap;
use openusd::usd::Stage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Authored content relative to its loaded baseline or last successful in-place save.
/// Clean does not assert that another process has left the backing file unchanged.
pub enum LayerSaveState {
    Unknown,
    Clean,
    Modified,
}

#[derive(Default)]
pub(super) struct SaveState {
    loaded_document: bool,
    baselines: BTreeMap<String, blake3::Hash>,
    cache: BTreeMap<String, (u64, Option<blake3::Hash>)>,
    structure: u64,
}

impl SaveState {
    pub fn opened(&mut self, stage: &Stage, revisions: &BTreeMap<String, u64>) {
        self.loaded_document = true;
        let _ = self.states(stage, revisions, 0);
    }

    pub fn states(&mut self, stage: &Stage, revisions: &BTreeMap<String, u64>, structure: u64) -> BTreeMap<String, LayerSaveState> {
        if self.structure != structure || structure == u64::MAX {
            self.cache.clear();
            self.structure = structure;
        }
        let mut states = BTreeMap::new();
        for id in stage.layer_identifiers() {
            let revision = revisions.get(&id).copied().unwrap_or_default();
            let current = self.cache.entry(id.clone()).or_insert((u64::MAX, None));
            if current.0 != revision || revision == u64::MAX {
                *current = (revision, stage.layer(&id).and_then(|layer| layer.export_to_string().ok())
                    .map(|text| blake3::hash(text.as_bytes())));
            }
            if self.loaded_document && revision == 0 && !self.baselines.contains_key(&id)
                && stage.layer(&id).is_some_and(|layer| layer.resolved_path().is_some()) {
                if let Some(hash) = current.1 { self.baselines.insert(id.clone(), hash); }
            }
            let state = match (self.baselines.get(&id), current.1) {
                (Some(baseline), Some(hash)) if *baseline == hash => LayerSaveState::Clean,
                (Some(_), Some(_)) => LayerSaveState::Modified,
                _ => LayerSaveState::Unknown,
            };
            states.insert(id, state);
        }
        for id in self.baselines.keys() {
            states.entry(id.clone()).or_insert(LayerSaveState::Unknown);
        }
        states
    }

    pub fn saved_to_source(&mut self, stage: &Stage, id: &str, filename: &str) {
        let Some(layer) = stage.layer(id) else { return };
        let Some(source) = layer.resolved_path() else { return };
        let (Ok(source), Ok(destination)) = (std::fs::canonicalize(source), std::fs::canonicalize(filename)) else { return };
        if source != destination { return; }
        if let Ok(text) = layer.export_to_string() {
            self.baselines.insert(id.to_string(), blake3::hash(text.as_bytes()));
            self.cache.remove(id);
        }
    }
}
