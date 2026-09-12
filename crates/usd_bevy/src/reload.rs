//! Transactional, field-level updates of layers in an existing stage.

use openusd::{sdf::{AbstractData, Data, Layer}, usd::Stage};

/// Prepared layer changes validated against the current composed document.
pub struct LayerReload {
    layers: Vec<(String, Data)>,
}

impl LayerReload {
    /// Parses replacement bytes and validates composition without modifying `stage`.
    pub fn prepare(stage: &Stage, replacements: &[(String, Vec<u8>)]) -> anyhow::Result<Self> {
        let mut layers = Vec::new();
        for (id, bytes) in replacements {
            anyhow::ensure!(stage.layer(id).is_some(), "reload layer is not in the stage: {id}");
            anyhow::ensure!(!layers.iter().any(|(existing, _)| existing == id), "duplicate reload layer: {id}");
            let layer = Layer::from_bytes(id, bytes.clone())?;
            layers.push((id.clone(), Data::from_abstract(layer.data())?));
        }
        let root = stage.root_layer();
        let root_id = root.identifier().to_owned();
        let mut source = crate::UsdSource::new(&root_id, root.export_to_string()?.into_bytes())?;
        drop(root);
        for id in stage.layer_identifiers() {
            if id != root_id {
                source.insert_dependency(id.clone(), stage.layer(&id).unwrap().export_to_string()?.into_bytes());
            }
        }
        let candidate = source.open_stage()?;
        for id in stage.muted_layers() { candidate.mute_layer(id); }
        crate::UsdSource::validate_composition(&candidate)?;
        let plan = Self { layers };
        plan.apply(&candidate)?;
        crate::UsdSource::validate_composition(&candidate)?;
        Ok(plan)
    }

    /// Publishes changed fields in one transaction; unchanged fields are untouched.
    pub fn apply(&self, stage: &Stage) -> anyhow::Result<()> {
        let ids: Vec<_> = self.layers.iter().map(|(id, _)| id.as_str()).collect();
        stage.batch_edit(&ids, |edits| {
            for (edit, (_, replacement)) in edits.iter_mut().zip(&self.layers) {
                patch(edit.data_mut(), replacement);
            }
            Ok(())
        })?;
        Ok(())
    }
}

fn patch(target: &mut dyn AbstractData, source: &Data) {
    for path in target.spec_paths() {
        if !source.has_spec(&path) { target.erase_spec(&path); }
    }
    for path in source.spec_paths() {
        if target.spec_type(&path) != source.spec_type(&path) {
            target.create_spec(path.clone(), source.spec_type(&path).unwrap());
        }
        for field in target.list_fields(&path).unwrap_or_default() {
            if !source.has_field(&path, &field) { target.erase_field(&path, &field); }
        }
        for field in source.list_fields(&path).unwrap_or_default() {
            let value = source.get_field(&path, &field).expect("materialized layer data");
            if target.try_field(&path, &field).ok().flatten().as_deref() != Some(value.as_ref()) {
                target.set_field(&path, &field, value.into_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_layer_reload_updates_both_instances_not_unrelated_prims() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child.usda");
        std::fs::write(&child, "#usda 1.0\ndef Cube \"Model\" { double size = 1 }\n").unwrap();
        let root = directory.path().join("root.usda");
        let source = crate::UsdSource::new(&root, b"#usda 1.0\ndef Xform \"A\" (prepend references = @child.usda@</Model>) {}\ndef Xform \"B\" (prepend references = @child.usda@</Model>) {}\ndef Cube \"Other\" { double size = 7 }\n".as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        crate::UsdSource::validate_composition(&stage).unwrap();
        let live = crate::live::LiveStage::new(stage.clone());
        let plan = LayerReload::prepare(&stage, &[(child.to_string_lossy().into_owned(), b"#usda 1.0\ndef Cube \"Model\" { double size = 3 }\n".to_vec())]).unwrap();
        assert_eq!(stage.prim("/A").unwrap().attribute("size").get::<f64>().unwrap(), Some(1.0));
        plan.apply(&stage).unwrap();
        for path in ["/A", "/B"] {
            assert_eq!(stage.prim(path).unwrap().attribute("size").get::<f64>().unwrap(), Some(3.0));
        }
        assert_eq!(stage.prim("/Other").unwrap().attribute("size").get::<f64>().unwrap(), Some(7.0));
        let changes = live.drain_changes();
        assert!(!changes.is_empty());
        assert!(changes.iter().flat_map(|change| change.paths()).all(|path| path != "/" && !path.starts_with("/Other")));
    }

    #[test]
    fn malformed_reload_keeps_live_layer_unchanged() {
        let source = crate::UsdSource::new("reload.usda", b"#usda 1.0\ndef Cube \"Model\" {}\n".as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        assert!(LayerReload::prepare(&stage, &[(source.identifier().into(), b"broken".to_vec())]).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }
}
