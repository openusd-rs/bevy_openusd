//! Bounded current-composition hierarchy and constant visibility reuse.

use super::{
    EditorSnapshot,
    layer_changes::{CompositionRevision, LayerChanges},
};
use openusd::{
    sdf::Path,
    usd::{PrimPredicate, PrimStatus, Stage, TimeCode},
    vt::Value,
};

struct Entry {
    path: Path,
    constant: Option<[Option<bool>; 2]>,
}

pub(super) struct HierarchyCache {
    revision: Option<CompositionRevision>,
    entries: Vec<Entry>,
    max_paths: usize,
    max_path_bytes: usize,
    #[cfg(test)]
    scans: usize,
    #[cfg(test)]
    reads: usize,
}

impl Default for HierarchyCache {
    fn default() -> Self {
        Self {
            revision: None,
            entries: Vec::new(),
            max_paths: 65_536,
            max_path_bytes: 8 * 1024 * 1024,
            #[cfg(test)]
            scans: 0,
            #[cfg(test)]
            reads: 0,
        }
    }
}

fn predicate() -> PrimPredicate {
    PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
    .with_instance_proxies(true)
}

fn visible(value: Option<Value>) -> bool {
    !matches!(value, Some(Value::Token(token)) if token.as_str() == "invisible")
}

impl HierarchyCache {
    pub(super) fn sample(
        &mut self,
        stage: &Stage,
        changes: &LayerChanges,
        time: Option<f64>,
        snapshot: &mut EditorSnapshot,
    ) -> anyhow::Result<()> {
        stage.flush()?;
        let revision = CompositionRevision::read(stage, changes);
        let slot = usize::from(time.is_some());
        if revision.reusable() && self.revision.as_ref() == Some(&revision) {
            for entry in &mut self.entries {
                let value = if let Some(value) = entry.constant.and_then(|values| values[slot]) {
                    value
                } else {
                    #[cfg(test)]
                    {
                        self.reads += 1;
                    }
                    let value = visible(
                        stage
                            .prim(&entry.path)?
                            .attribute("visibility")
                            .get_at::<Value>(time.map(TimeCode::new))?,
                    );
                    if let Some(values) = &mut entry.constant {
                        values[slot] = Some(value);
                    }
                    value
                };
                snapshot.prims.push(entry.path.to_string());
                snapshot.visibility.insert(entry.path.to_string(), value);
            }
            return Ok(());
        }
        self.revision = None;
        self.entries = Vec::new();
        #[cfg(test)]
        {
            self.scans += 1;
        }
        let mut entries = Vec::new();
        stage.traverse(predicate(), |path| {
            entries.push(Entry {
                path: path.clone(),
                constant: None,
            });
        })?;
        let retain = entries.len() <= self.max_paths
            && entries
                .iter()
                .map(|entry| entry.path.as_str().len())
                .try_fold(0usize, usize::checked_add)
                .is_some_and(|bytes| bytes <= self.max_path_bytes);
        for entry in &mut entries {
            let attribute = stage.prim(&entry.path)?.attribute("visibility");
            #[cfg(test)]
            {
                self.reads += 1;
            }
            let value = visible(attribute.get_at::<Value>(time.map(TimeCode::new))?);
            if retain
                && attribute.num_time_samples().is_ok_and(|count| count == 0)
                && attribute
                    .might_be_time_varying()
                    .is_ok_and(|varying| !varying)
            {
                let mut values = [None; 2];
                values[slot] = Some(value);
                entry.constant = Some(values);
            }
            snapshot.prims.push(entry.path.to_string());
            snapshot.visibility.insert(entry.path.to_string(), value);
        }
        if retain && revision.reusable() && CompositionRevision::read(stage, changes) == revision {
            self.revision = Some(revision);
            self.entries = entries;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{EditorEdit, EditorSession};

    fn check(editor: &EditorSession, time: Option<f64>) -> EditorSnapshot {
        let snapshot = editor.snapshot_at(time).unwrap();
        let stage = editor.stage();
        let mut prims = Vec::new();
        stage
            .traverse(predicate(), |path| prims.push(path.to_string()))
            .unwrap();
        assert_eq!(snapshot.prims, prims);
        let expected: std::collections::HashMap<_, _> = prims
            .into_iter()
            .map(|path| {
                let value = stage
                    .prim(openusd::sdf::path(&path).unwrap())
                    .unwrap()
                    .attribute("visibility")
                    .get_at::<Value>(time.map(TimeCode::new))
                    .unwrap();
                (path, visible(value))
            })
            .collect();
        assert_eq!(snapshot.visibility, expected);
        snapshot
    }

    fn editor(text: &str) -> EditorSession {
        EditorSession::new(crate::snippet::UsdSnippet::new(text).open_stage().unwrap())
    }

    #[test]
    fn constant_values_reuse_and_single_sample_defaults_stay_distinct() {
        let mut editor = editor(
            r#"#usda 1.0
def Xform "Static" { token visibility = "invisible" }
def Xform "Single" {
    token visibility = "inherited"
    token visibility.timeSamples = { 5: "invisible" }
}
def Xform "Moving" {
    token visibility.timeSamples = { 0: "inherited", 5: "invisible", 10: "inherited" }
}
"#,
        );
        let before = editor.stage().root_layer().export_to_string().unwrap();
        for time in [None, Some(0.0), Some(5.0), Some(10.0), Some(5.0), None] {
            let snapshot = check(&editor, time);
            assert!(!snapshot.visibility["/Static"]);
            assert_eq!(snapshot.visibility["/Single"], time.is_none());
        }
        assert_eq!(editor.hierarchy.borrow().scans, 1);
        assert_eq!(editor.hierarchy.borrow().reads, 3 + 5 * 2 + 1);
        assert!(editor.snapshot_at(Some(f64::NAN)).is_err());
        editor.select(Some("/Moving".into())).unwrap();
        assert_eq!(
            check(&editor, Some(5.0)).selected.as_deref(),
            Some("/Moving")
        );
        assert_eq!(editor.hierarchy.borrow().scans, 1);
        editor
            .edit(EditorEdit::Attribute {
                prim: "/Static".into(),
                name: "visibility".into(),
                type_name: "token".into(),
                value: Value::Token("inherited".into()),
            })
            .unwrap();
        assert!(check(&editor, Some(5.0)).visibility["/Static"]);
        editor.undo().unwrap();
        assert!(!check(&editor, Some(5.0)).visibility["/Static"]);
        editor.redo().unwrap();
        assert!(check(&editor, Some(5.0)).visibility["/Static"]);
        assert_eq!(editor.hierarchy.borrow().scans, 4);
        editor.undo().unwrap();
        assert_eq!(
            editor.stage().root_layer().export_to_string().unwrap(),
            before
        );
    }

    #[test]
    fn variants_and_direct_layer_edits_rebuild_hierarchy() {
        let editor = editor(
            r#"#usda 1.0
def Xform "Model" (
    variants = { string look = "plain" }
    prepend variantSets = "look"
) {
    variantSet "look" = {
        "plain" {}
        "child" { def Xform "Child" {} }
    }
}
"#,
        );
        check(&editor, None);
        crate::authoring::set_variant(editor.stage(), "/Model", "look", "child").unwrap();
        assert!(check(&editor, None).prims.contains(&"/Model/Child".into()));
        crate::authoring::set_variant(editor.stage(), "/Model", "look", "plain").unwrap();
        assert!(!check(&editor, None).prims.contains(&"/Model/Child".into()));
        editor
            .stage()
            .root_layer()
            .edit(|layer| {
                openusd::sdf::PrimSpec::new(
                    layer.data_mut(),
                    "/External",
                    openusd::sdf::Specifier::Def,
                    "Xform",
                )?;
                Ok(())
            })
            .unwrap();
        assert!(check(&editor, None).prims.contains(&"/External".into()));
        assert_eq!(editor.hierarchy.borrow().scans, 4);
    }

    #[test]
    fn clip_visibility_is_sampled_across_active_clips() {
        let first = crate::UsdSource::snapshot(
            "first.usda",
            b"#usda 1.0\ndef Xform \"Model\" { token visibility.timeSamples = { 0: \"invisible\" } }\n".as_slice(),
        )
        .unwrap();
        let second = crate::UsdSource::snapshot(
            "second.usda",
            b"#usda 1.0\ndef Xform \"Model\" { token visibility.timeSamples = { 0: \"inherited\" } }\n".as_slice(),
        )
        .unwrap();
        let source = crate::UsdSource::snapshot(
            "root.usda",
            br#"#usda 1.0
def Xform "Model" (
    clips = { dictionary default = {
        asset[] assetPaths = [@first.usda@, @second.usda@]
        double2[] active = [(0, 0), (10, 1)]
        string primPath = "/Model"
    } }
) { token visibility }
"#
            .as_slice(),
        )
        .unwrap()
        .with_dependency(&first)
        .unwrap()
        .with_dependency(&second)
        .unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        for time in [None, Some(0.0), Some(15.0), Some(5.0), Some(15.0), None] {
            let snapshot = check(&editor, time);
            if let Some(time) = time {
                assert_eq!(snapshot.visibility["/Model"], time >= 10.0);
            }
        }
        assert!(
            editor
                .hierarchy
                .borrow()
                .entries
                .iter()
                .all(|entry| entry.constant.is_none())
        );
    }

    #[test]
    fn muted_layers_and_payload_rules_invalidate_paths() {
        let weak = crate::UsdSource::snapshot(
            "weak.usda",
            b"#usda 1.0\ndef Xform \"Weak\" {}\n".as_slice(),
        )
        .unwrap();
        let payload = crate::UsdSource::snapshot(
            "payload.usda",
            b"#usda 1.0\ndef Xform \"Payload\" { def Xform \"Child\" {} }\n".as_slice(),
        )
        .unwrap();
        let root = crate::UsdSource::snapshot(
            "root.usda",
            b"#usda 1.0\n(subLayers = [@weak.usda@])\ndef Xform \"Root\" (prepend payload = @payload.usda@</Payload>) {}\n".as_slice(),
        ).unwrap().with_dependency(&weak).unwrap().with_dependency(&payload).unwrap();
        let editor = EditorSession::new(root.open_stage().unwrap());
        assert!(check(&editor, None).prims.contains(&"/Root/Child".into()));
        editor
            .stage()
            .set_load_rules(openusd::pcp::LoadRules::none());
        assert!(!check(&editor, None).prims.contains(&"/Root/Child".into()));
        editor
            .stage()
            .set_load_rules(openusd::pcp::LoadRules::all());
        assert!(check(&editor, None).prims.contains(&"/Root/Child".into()));
        editor.stage().mute_layer(weak.identifier());
        assert!(!check(&editor, None).prims.contains(&"/Weak".into()));
        editor.stage().unmute_layer(weak.identifier());
        assert!(check(&editor, None).prims.contains(&"/Weak".into()));
        assert_eq!(editor.hierarchy.borrow().scans, 5);
    }

    #[test]
    fn entry_and_text_limits_drop_retained_paths_without_changing_results() {
        for (max_paths, max_path_bytes) in [(0, usize::MAX), (usize::MAX, 0)] {
            let editor =
                editor("#usda 1.0\ndef Xform \"Box\" { token visibility = \"invisible\" }\n");
            check(&editor, None);
            {
                let mut cache = editor.hierarchy.borrow_mut();
                cache.max_paths = max_paths;
                cache.max_path_bytes = max_path_bytes;
            }
            editor.stage().define_prim("/Added").unwrap();
            check(&editor, None);
            check(&editor, Some(10.0));
            let cache = editor.hierarchy.borrow();
            assert_eq!(cache.scans, 3);
            assert!(cache.revision.is_none());
            assert!(cache.entries.is_empty());
            assert_eq!(cache.entries.capacity(), 0);
        }
    }

    #[test]
    fn native_instance_proxies_and_separate_population_masks_match_the_oracle() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("instances.usda");
        std::fs::write(
            &file,
            r#"#usda 1.0
class Xform "Abstract" { def Xform "Hidden" {} }
def Xform "Model" { def Xform "Child" { token visibility = "invisible" } }
def Xform "Instance" (instanceable = true prepend references = </Model>) {}
over "Undefined" { def Xform "Hidden" {} }
"#,
        )
        .unwrap();
        for masked in [false, true] {
            let mask = if masked {
                openusd::usd::StagePopulationMask::new(["/Model"]).unwrap()
            } else {
                openusd::usd::StagePopulationMask::all()
            };
            let stage = Stage::builder()
                .mask(mask)
                .open(file.to_str().unwrap())
                .unwrap();
            let editor = EditorSession::new(stage);
            for time in [None, Some(0.0), Some(20.0)] {
                let snapshot = check(&editor, time);
                assert!(!snapshot.prims.iter().any(|path| path.starts_with("/Abstract") || path.starts_with("/Undefined")));
                assert_eq!(snapshot.prims.contains(&"/Instance/Child".into()), !masked);
                assert!(!snapshot.visibility["/Model/Child"]);
            }
            assert_eq!(editor.hierarchy.borrow().scans, 1);
            editor
                .stage()
                .prim("/Model/Child")
                .unwrap()
                .set_active(false)
                .unwrap();
            assert!(!check(&editor, None).prims.contains(&"/Model/Child".into()));
            editor
                .stage()
                .prim("/Model/Child")
                .unwrap()
                .set_active(true)
                .unwrap();
            assert!(check(&editor, None).prims.contains(&"/Model/Child".into()));
            editor.stage().remove_prim("/Model/Child").unwrap();
            assert!(!check(&editor, None).prims.contains(&"/Model/Child".into()));
            assert_eq!(editor.hierarchy.borrow().scans, 4);
        }
    }

    #[test]
    fn source_reload_refreshes_warm_paths_and_preserves_failure_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().canonicalize().unwrap().join("reload.usda");
        let original = "#usda 1.0\ndef Xform \"Model\" { token visibility = \"invisible\" }\n";
        std::fs::write(&file, original).unwrap();
        let mut editor =
            EditorSession::from_source(crate::UsdSource::from_file(&file).unwrap()).unwrap();
        let document = editor.document_id();
        assert!(!check(&editor, Some(0.0)).visibility["/Model"]);
        std::fs::write(&file, "not a USD layer").unwrap();
        assert!(editor.reload_sources().is_err());
        assert!(!check(&editor, Some(5.0)).visibility["/Model"]);
        std::fs::write(
            &file,
            "#usda 1.0\ndef Xform \"Model\" { def Xform \"Child\" {} }\n",
        )
        .unwrap();
        editor.reload_sources().unwrap();
        let snapshot = check(&editor, Some(5.0));
        assert!(snapshot.visibility["/Model"]);
        assert!(snapshot.prims.contains(&"/Model/Child".into()));
        assert_eq!(editor.document_id(), document);
        let replacement =
            EditorSession::from_source(crate::UsdSource::from_file(&file).unwrap()).unwrap();
        assert_ne!(replacement.document_id(), document);
        assert!(check(&replacement, None).visibility["/Model"]);
        assert_eq!(replacement.hierarchy.borrow().scans, 1);
    }

    #[test]
    fn clip_manifest_defaults_do_not_replace_untimed_visibility() {
        let clip = crate::UsdSource::snapshot(
            "clip.usda",
            b"#usda 1.0\ndef Xform \"Model\" { token visibility }\n".as_slice(),
        )
        .unwrap();
        let manifest = crate::UsdSource::snapshot(
            "manifest.usda",
            b"#usda 1.0\nover \"Model\" { token visibility = \"invisible\" }\n".as_slice(),
        )
        .unwrap();
        let source = crate::UsdSource::snapshot(
            "root.usda",
            br#"#usda 1.0
def Xform "Model" (
    clips = { dictionary default = {
        asset[] assetPaths = [@clip.usda@]
        asset manifestAssetPath = @manifest.usda@
        double2[] active = [(0, 0)]
        string primPath = "/Model"
    } }
) { token visibility }
"#
            .as_slice(),
        )
        .unwrap()
        .with_dependency(&clip)
        .unwrap()
        .with_dependency(&manifest)
        .unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        for time in [None, Some(0.0), Some(20.0), None, Some(-10.0)] {
            check(&editor, time);
        }
    }
}
