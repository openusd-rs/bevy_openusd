use std::collections::{BTreeMap, BTreeSet};
use openusd::{sdf::Path, usd::Stage};

#[derive(PartialEq, Eq)]
struct Revision {
    layers: Vec<String>,
    authored: BTreeMap<String, u64>,
    structure: u64,
    load_rules: openusd::pcp::LoadRules,
    mask: openusd::usd::StagePopulationMask,
    muted: Vec<String>,
}

impl Revision {
    fn read(stage: &Stage, changes: &super::layer_changes::LayerChanges) -> Self {
        let authored = changes.revisions();
        Self { layers: stage.layer_identifiers(), authored, structure: changes.structural_revision(),
            load_rules: stage.load_rules(), mask: stage.mask(), muted: stage.muted_layers() }
    }

    fn reusable(&self) -> bool {
        self.structure != u64::MAX && self.authored.values().all(|revision| *revision != u64::MAX)
    }
}

#[derive(Default)]
pub(super) struct TextureIndex {
    revision: Option<Revision>,
    paths: Vec<Path>,
    #[cfg(test)]
    scans: usize,
}

impl TextureIndex {
    pub(super) fn requests(&mut self, stage: &Stage, changes: &super::layer_changes::LayerChanges) -> Result<BTreeSet<(String, bool)>, String> {
        let revision = Revision::read(stage, changes);
        if !revision.reusable() || self.revision.as_ref() != Some(&revision) {
            self.revision = None;
            self.paths = crate::UsdSource::stage_texture_prims(stage)?;
            #[cfg(test)]
            { self.scans += 1; }
            if revision.reusable() && Revision::read(stage, changes) == revision { self.revision = Some(revision); }
        }
        crate::UsdSource::texture_requests_for_prims(stage, &self.paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_index_reuses_paths_but_authoring_and_load_rules_rebuild() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Xform "Plain" {}
def DomeLight "Dome" { asset inputs:texture:file = @first.exr@ }
"#).open_stage().unwrap();
        let changes = super::super::layer_changes::LayerChanges::new(&stage);
        let mut index = TextureIndex::default();
        let first = index.requests(&stage, &changes).unwrap();
        assert_eq!(first, crate::UsdSource::stage_texture_requests(&stage).unwrap());
        assert_eq!(index.scans, 1);
        assert_eq!(index.requests(&stage, &changes).unwrap(), first);
        assert_eq!(index.scans, 1);
        stage.prim("/Plain").unwrap().set_type_name("DomeLight").unwrap();
        stage.create_attribute("/Plain.inputs:texture:file", "asset").unwrap()
            .set(openusd::sdf::Value::AssetPath(openusd::sdf::AssetPath::new("second.exr"))).unwrap();
        let second = index.requests(&stage, &changes).unwrap();
        assert_eq!(second, crate::UsdSource::stage_texture_requests(&stage).unwrap());
        assert_eq!(second.len(), 2);
        assert_eq!(index.scans, 2);
        stage.set_load_rules(openusd::pcp::LoadRules::none());
        assert_eq!(index.requests(&stage, &changes).unwrap(), crate::UsdSource::stage_texture_requests(&stage).unwrap());
        assert_eq!(index.scans, 3);
    }

    #[test]
    fn texture_resolution_is_fresh_without_rebuilding_candidate_paths() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root.usda");
        let bytes = b"#usda 1.0\ndef DomeLight \"Dome\" { asset inputs:texture:file = @late.exr@ }\n";
        std::fs::write(&root, bytes).unwrap();
        let stage = crate::UsdSource::new(&root, bytes.as_slice()).unwrap().open_stage().unwrap();
        let changes = super::super::layer_changes::LayerChanges::new(&stage);
        let mut index = TextureIndex::default();
        let before = index.requests(&stage, &changes).unwrap();
        std::fs::write(directory.path().join("late.exr"), b"resolver probe").unwrap();
        let after = index.requests(&stage, &changes).unwrap();
        assert_eq!(after, crate::UsdSource::stage_texture_requests(&stage).unwrap());
        assert_ne!(before, after);
        assert_eq!(index.scans, 1);
    }

    #[test]
    fn muting_and_unmuting_layers_invalidate_candidates() {
        let weak = crate::UsdSource::snapshot("weak.usda", b"#usda 1.0\ndef DomeLight \"Dome\" { asset inputs:texture:file = @dome.exr@ }\n".as_slice()).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", b"#usda 1.0\n(subLayers = [@weak.usda@])\n".as_slice()).unwrap()
            .with_dependency(&weak).unwrap();
        let stage = root.open_stage().unwrap();
        let changes = super::super::layer_changes::LayerChanges::new(&stage);
        let mut index = TextureIndex::default();
        assert_eq!(index.requests(&stage, &changes).unwrap().len(), 1);
        stage.mute_layer(weak.identifier());
        assert!(index.requests(&stage, &changes).unwrap().is_empty());
        stage.unmute_layer(weak.identifier());
        assert_eq!(index.requests(&stage, &changes).unwrap().len(), 1);
        assert_eq!(index.scans, 3);
    }

    #[test]
    fn variant_switches_and_external_layer_edits_invalidate_candidates() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Xform "Model" (
    variants = { string look = "plain" }
    prepend variantSets = "look"
) {
    variantSet "look" = {
        "plain" {}
        "lit" { def DomeLight "Dome" { asset inputs:texture:file = @dome.exr@ } }
    }
}
"#).open_stage().unwrap();
        let changes = super::super::layer_changes::LayerChanges::new(&stage);
        let mut index = TextureIndex::default();
        assert!(index.requests(&stage, &changes).unwrap().is_empty());
        crate::authoring::set_variant(&stage, "/Model", "look", "lit").unwrap();
        assert_eq!(index.requests(&stage, &changes).unwrap().len(), 1);
        crate::authoring::set_variant(&stage, "/Model", "look", "plain").unwrap();
        assert!(index.requests(&stage, &changes).unwrap().is_empty());
        assert_eq!(index.scans, 3);
        let root = stage.root_layer().identifier().to_owned();
        stage.layer_mut(&root).unwrap().edit(|layer| {
            openusd::sdf::PrimSpec::new(layer.data_mut(), "/External", openusd::sdf::Specifier::Def, "DomeLight")?;
            Ok(())
        }).unwrap();
        let requests = index.requests(&stage, &changes).unwrap();
        assert_eq!(requests, crate::UsdSource::stage_texture_requests(&stage).unwrap());
        assert!(index.paths.iter().any(|path| path.as_str() == "/External"));
        assert_eq!(index.scans, 4);
    }
}
