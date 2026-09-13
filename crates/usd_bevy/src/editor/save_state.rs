use std::collections::BTreeMap;
use openusd::usd::Stage;

fn content_hash(stage: &Stage, id: &str) -> Option<blake3::Hash> {
    let layer = stage.layer(id)?;
    let data = layer.data();
    let mut hash = blake3::Hasher::new();
    for path in data.spec_paths() {
        hash_part(&mut hash, path.as_str().as_bytes());
        hash_part(&mut hash, format!("{:?}", data.spec_type(&path)?).as_bytes());
        let mut fields = data.list_fields(&path)?;
        fields.sort_unstable();
        hash.update(&(fields.len() as u64).to_le_bytes());
        for field in fields {
            hash_part(&mut hash, field.as_bytes());
            let value = data.get_field(&path, &field).ok()?;
            let kind: &'static str = value.as_ref().into();
            hash_part(&mut hash, kind.as_bytes());
            if let Some(bytes) = numeric_array_bytes(&value) {
                hash_part(&mut hash, bytes);
            } else {
                hash_part(&mut hash, openusd::usda::TextWriter::value_to_string(&value).ok()?.as_bytes());
            }
        }
    }
    Some(hash.finalize())
}

fn hash_part(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn numeric_array_bytes(value: &openusd::sdf::Value) -> Option<&[u8]> {
    use openusd::sdf::Value;
    macro_rules! arrays {
        ($($kind:ident),*) => { match value {
            $(Value::$kind(values) => Some(bytemuck::cast_slice(values)),)*
            _ => None,
        } };
    }
    arrays!(UcharVec, IntVec, UintVec, Int64Vec, Uint64Vec, HalfVec, FloatVec, DoubleVec,
        Vec2hVec, Vec2fVec, Vec2dVec, Vec2iVec, Vec3hVec, Vec3fVec, Vec3dVec, Vec3iVec,
        Vec4hVec, Vec4fVec, Vec4dVec, Vec4iVec, QuathVec, QuatfVec, QuatdVec,
        Matrix2dVec, Matrix3dVec, Matrix4dVec)
}

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
    pub disk: Option<crate::source::DiskBaselines>,
    loaded_document: bool,
    baselines: BTreeMap<String, blake3::Hash>,
    cache: BTreeMap<String, (u64, Option<blake3::Hash>)>,
    structure: u64,
}

impl SaveState {
    pub fn reloaded_layer(&mut self, stage: &Stage, id: &str) {
        if let Some(hash) = content_hash(stage, id) {
            self.baselines.insert(id.to_string(), hash);
            self.cache.remove(id);
        }
    }

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
                *current = (revision, content_hash(stage, &id));
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
        if let Some(hash) = content_hash(stage, id) {
            self.baselines.insert(id.to_string(), hash);
            self.cache.remove(id);
        }
    }
}
