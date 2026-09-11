//! Stage authoring + persistence (RETHINK P5 ops + P6 persistence).
//!
//! The model layer the editor UI drives: namespace edits (define / remove /
//! rename / reparent / move), attribute authoring, and saving. Every edit
//! goes through the live `Stage` (`&self`, interior-mutable) and commits —
//! firing the `StageSink` so [`crate::live`] reprojects the affected
//! entities. The UI panels are a thin presentation layer on top of these;
//! these functions are headless and fully testable without a window.

use openusd::sdf::Value;
use openusd::usd::{NamespaceEditor, Stage};

type Result<T> = anyhow::Result<T>;

/// Replaces a relationship's target list in the current edit target.
pub fn set_relationship_targets(stage: &Stage, prim: &str, name: &str, targets: &[openusd::sdf::Path]) -> Result<()> {
    for target in targets {
        anyhow::ensure!(target.as_str().starts_with('/') && target.as_str() != "/"
            && (target.is_prim_path() || target.is_property_path()) && !target.contains_prim_variant_selection(),
            "relationship target must be an absolute prim or property path");
    }
    let prim = stage.prim(openusd::sdf::path(prim)?)?;
    anyhow::ensure!(prim.is_valid()?, "relationship owner does not exist");
    prim.create_relationship(name)?.set_targets(targets.iter().cloned())?;
    Ok(())
}

/// Clears only the local target-list opinion, allowing weaker targets to compose.
pub fn clear_relationship_targets(stage: &Stage, prim: &str, name: &str) -> Result<()> {
    stage.prim(openusd::sdf::path(prim)?)?.relationship(name).clear_targets()?;
    Ok(())
}

/// Replaces the reference list authored on a prim in the current edit target.
/// Relative asset paths remain relative to that layer; an empty list blocks
/// weaker references. Use `clear_references` to remove the local opinion.
/// Time mappings require a finite offset and positive finite scale.
pub fn set_references(stage: &Stage, prim: &str, references: &[openusd::sdf::Reference]) -> Result<()> {
    for reference in references {
        anyhow::ensure!(reference.prim_path.is_empty() || (
            reference.prim_path.as_str().starts_with('/') && reference.prim_path.is_prim_path()
                && !reference.prim_path.contains_prim_variant_selection()
                && reference.prim_path.as_str() != "/"
        ), "reference target must be an absolute prim path or empty for defaultPrim");
        anyhow::ensure!(reference.layer_offset.offset.is_finite()
            && reference.layer_offset.scale.is_finite() && reference.layer_offset.scale > 0.0,
            "reference time mapping must have a finite offset and positive finite scale");
    }
    let prim = stage.prim(openusd::sdf::path(prim)?)?;
    anyhow::ensure!(prim.is_valid()?, "reference owner does not exist");
    prim.set_metadata("references", Value::ReferenceListOp(openusd::sdf::ReferenceListOp::explicit(references.to_vec())))?;
    Ok(())
}

/// Removes the current edit target's reference-list opinion.
pub fn clear_references(stage: &Stage, prim: &str) -> Result<()> {
    stage.prim(openusd::sdf::path(prim)?)?.clear_metadata("references")?;
    Ok(())
}

/// Replaces the current edit target's payload list; an empty list blocks weaker payloads.
pub fn set_payloads(stage: &Stage, prim: &str, payloads: &[openusd::sdf::Payload]) -> Result<()> {
    for payload in payloads {
        anyhow::ensure!(payload.prim_path.is_empty() || (
            payload.prim_path.as_str().starts_with('/') && payload.prim_path.is_prim_path()
                && !payload.prim_path.contains_prim_variant_selection()
                && payload.prim_path.as_str() != "/"
        ), "payload target must be an absolute prim path or empty for defaultPrim");
        anyhow::ensure!(payload.layer_offset.is_none_or(|offset| offset.offset.is_finite()
            && offset.scale.is_finite() && offset.scale > 0.0),
            "payload time mapping must have a finite offset and positive finite scale");
    }
    let prim = stage.prim(openusd::sdf::path(prim)?)?;
    anyhow::ensure!(prim.is_valid()?, "payload owner does not exist");
    prim.set_metadata("payload", Value::PayloadListOp(openusd::sdf::PayloadListOp::explicit(payloads.to_vec())))?;
    Ok(())
}

/// Removes the current edit target's payload-list opinion.
pub fn clear_payloads(stage: &Stage, prim: &str) -> Result<()> {
    stage.prim(openusd::sdf::path(prim)?)?.clear_metadata("payload")?;
    Ok(())
}

// ─── Namespace ops ──────────────────────────────────────────────────

/// Define (create) a prim of `type_name` at `path`.
pub fn define_prim(stage: &Stage, path: &str, type_name: &str) -> Result<()> {
    let prim = openusd::sdf::path(path)?;
    stage.define_prim(prim)?.set_type_name(type_name)?;
    Ok(())
}

/// Remove the prim (and its subtree) at `path`. Returns whether anything was
/// removed.
pub fn remove_prim(stage: &Stage, path: &str) -> Result<bool> {
    let prim = openusd::sdf::path(path)?;
    Ok(stage.remove_prim(prim)?)
}

/// Rename the prim at `path` to `new_name` (last namespace component only).
pub fn rename_prim(stage: &Stage, path: &str, new_name: &str) -> Result<()> {
    let prim = stage.prim(openusd::sdf::path(path)?)?;
    let mut editor = NamespaceEditor::new(stage);
    editor.rename_prim(&prim, new_name)?;
    editor.apply()?;
    Ok(())
}

/// Reparent the prim at `path` under `new_parent` (keeping its name).
pub fn reparent_prim(stage: &Stage, path: &str, new_parent: &str) -> Result<()> {
    let prim = stage.prim(openusd::sdf::path(path)?)?;
    let parent = stage.prim(openusd::sdf::path(new_parent)?)?;
    let mut editor = NamespaceEditor::new(stage);
    editor.reparent_prim(&prim, &parent)?;
    editor.apply()?;
    Ok(())
}

/// Move the prim at `old` to the absolute path `new` (rename + reparent in one).
pub fn move_prim(stage: &Stage, old: &str, new: &str) -> Result<()> {
    let mut editor = NamespaceEditor::new(stage);
    editor.move_prim(openusd::sdf::path(old)?, openusd::sdf::path(new)?)?;
    editor.apply()?;
    Ok(())
}

// ─── Attribute ops ──────────────────────────────────────────────────

/// Author `value` onto attribute `name` of `prim` (creating it as
/// `type_name` if needed).
pub fn set_attribute(
    stage: &Stage,
    prim: &str,
    name: &str,
    type_name: &str,
    value: Value,
) -> Result<()> {
    let attr = openusd::sdf::path(prim)?.append_property(name)?;
    stage.create_attribute(attr, type_name)?.set(value)?;
    Ok(())
}

/// Authors a scene-time sample through the current edit target's time mapping.
pub fn set_attribute_sample(stage: &Stage, prim: &str, name: &str, type_name: &str, value: Value, time: f64) -> Result<()> {
    anyhow::ensure!(time.is_finite(), "attribute sample time must be finite");
    let path = openusd::sdf::path(prim)?.append_property(name)?;
    stage.create_attribute(path, type_name)?.set_at(value, openusd::usd::TimeCode::new(time))?;
    Ok(())
}

/// Removes the sample at scene time from the current edit target.
pub fn clear_attribute_sample(stage: &Stage, prim: &str, name: &str, time: f64) -> Result<()> {
    anyhow::ensure!(time.is_finite(), "attribute sample time must be finite");
    let path = openusd::sdf::path(prim)?.append_property(name)?;
    stage.attribute(path)?.clear_at(openusd::usd::TimeCode::new(time))?;
    Ok(())
}

/// Removes the attribute spec from the current edit target.
pub fn clear_attribute(stage: &Stage, prim: &str, name: &str) -> Result<bool> {
    let attr = openusd::sdf::path(prim)?.append_property(name)?;
    Ok(stage.remove_property(attr)?)
}

/// Removes local default and time-sample values while retaining property metadata.
pub fn clear_attribute_values(stage: &Stage, prim: &str, name: &str) -> Result<()> {
    let path = openusd::sdf::path(prim)?.append_property(name)?;
    stage.attribute(path)?.clear()?;
    Ok(())
}

/// Blocks weaker values and removes local time samples in the current edit target.
pub fn block_attribute_values(stage: &Stage, prim: &str, name: &str) -> Result<()> {
    let path = openusd::sdf::path(prim)?.append_property(name)?;
    stage.attribute(path)?.block()?;
    Ok(())
}

// ─── Variant selection (PLAN Phase 2) ───────────────────────────────

/// Select variant `selection` for variant set `set` on `prim` (non-destructive:
/// other sets' selections are preserved). This authors the prim's
/// `variantSelection` metadata — a **composition** change, so the commit fires a
/// `resynced` notice and the live loop reconciles the affected subtree.
pub fn set_variant(stage: &Stage, prim: &str, set: &str, selection: &str) -> Result<()> {
    let set = set.to_string();
    let selection = selection.to_string();
    stage
        .prim(openusd::sdf::path(prim)?).expect("validated USD path")
        .update_metadata("variantSelection", move |cur| {
            let mut map = match cur {
                Some(Value::VariantSelectionMap(m)) => m,
                _ => std::collections::HashMap::new(),
            };
            map.insert(set, selection);
            Some(Value::VariantSelectionMap(map))
        })?;
    Ok(())
}

// ─── Persistence (P6) ───────────────────────────────────────────────

/// Serialize the stage's composed root layer to a `.usda` string.
pub fn export_stage_string(stage: &Stage) -> Result<String> {
    Ok(stage.root_layer().export_to_string()?)
}

/// Atomically replace `filename` with the stage's root layer in its selected format.
pub fn save_stage_as(stage: &Stage, filename: &str) -> Result<()> {
    crate::persistence::export_layer(stage, &stage.root_layer(), filename)
}

/// Whether a prim with an authored type currently resolves on the stage.
pub fn prim_exists(stage: &Stage, path: &str) -> bool {
    openusd::sdf::path(path)
        .ok()
        .map(|p| {
            stage
                .prim(p).expect("validated USD path")
                .type_name()
                .map(|t| t.is_some())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage_with(root: &str) -> Stage {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("authoring_test.usda").unwrap();
        stage
            .define_prim(root)
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        stage
    }

    #[test]
    fn payload_clear_restores_weaker_opinions_and_validation_is_atomic() {
        let weak = crate::UsdSource::snapshot("weak.usda", &br#"#usda 1.0
class Xform "Model" { double score = 7 }
def Xform "Instance" ( prepend payload = </Model> ) {}
"#[..]).unwrap();
        let root = crate::UsdSource::snapshot("root.usda", &br#"#usda 1.0
(subLayers = [@weak.usda@])
"#[..]).unwrap().with_dependency(&weak).unwrap();
        let stage = root.open_stage().unwrap();
        let score = || stage.prim("/Instance").unwrap().attribute("score").get::<f64>().unwrap();
        assert_eq!(score(), Some(7.));
        set_payloads(&stage, "/Instance", &[]).unwrap();
        assert_eq!(score(), None);
        clear_payloads(&stage, "/Instance").unwrap();
        assert_eq!(score(), Some(7.));
        let before = stage.root_layer().export_to_string().unwrap();
        for target in ["relative", "/", "/Model.score", "/Model{choice=a}"] {
            let payload = openusd::sdf::Payload { prim_path: openusd::sdf::path(target).unwrap(), ..Default::default() };
            assert!(set_payloads(&stage, "/Instance", &[payload]).is_err());
            assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        }
        assert!(set_payloads(&stage, "/Absent", &[]).is_err());
        let invalid = openusd::sdf::Payload {
            prim_path: openusd::sdf::path("/Model").unwrap(),
            layer_offset: Some(openusd::sdf::LayerOffset::new(f64::INFINITY,1.)), ..Default::default()
        };
        assert!(set_payloads(&stage, "/Instance", &[invalid]).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn reference_authoring_retimes_and_rejects_unsupported_scales_atomically() {
        use openusd::sdf::{LayerOffset, Reference};
        let stage = crate::UsdSource::snapshot("reference-scales.usda", &br#"#usda 1.0
class Xform "Model" {
    double score.timeSamples = {0: 1, 10: 3}
}
def Xform "Instance" {}
"#[..]).unwrap().open_stage().unwrap();
        let reference = Reference {
            prim_path: openusd::sdf::path("/Model").unwrap(),
            layer_offset: LayerOffset::new(10.0, 2.0), ..Default::default()
        };
        set_references(&stage, "/Instance", &[reference.clone()]).unwrap();
        for (time, value) in [(10.0, 1.0), (20.0, 2.0), (30.0, 3.0)] {
            assert_eq!(stage.prim("/Instance").unwrap().attribute("score")
                .get_at::<f64>(Some(openusd::usd::TimeCode::new(time))).unwrap(), Some(value));
        }
        let before = stage.root_layer().export_to_string().unwrap();
        for scale in [-1.0, 0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let invalid = Reference { layer_offset: LayerOffset::new(0.0, scale), ..reference.clone() };
            assert!(set_references(&stage, "/Instance", &[reference.clone(), invalid]).is_err());
            assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        }
    }

    #[test]
    fn define_and_remove() {
        let stage = stage_with("/World");
        define_prim(&stage, "/World/Box", "Cube").unwrap();
        assert_eq!(
            stage
                .prim(openusd::sdf::path("/World/Box").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .as_deref(),
            Some("Cube")
        );
        assert!(remove_prim(&stage, "/World/Box").unwrap());
        assert!(
            stage
                .prim(openusd::sdf::path("/World/Box").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .is_none(),
            "removed prim is gone"
        );
    }

    #[test]
    fn rename_and_reparent() {
        let stage = stage_with("/World");
        define_prim(&stage, "/World/A", "Xform").unwrap();
        define_prim(&stage, "/World/B", "Xform").unwrap();
        define_prim(&stage, "/World/A/Child", "Cube").unwrap();

        rename_prim(&stage, "/World/A", "Renamed").unwrap();
        assert!(
            stage
                .prim(openusd::sdf::path("/World/Renamed").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .is_some(),
            "rename created /World/Renamed"
        );
        assert!(
            stage
                .prim(openusd::sdf::path("/World/A").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .is_none(),
            "old /World/A is gone"
        );

        reparent_prim(&stage, "/World/Renamed/Child", "/World/B").unwrap();
        assert!(
            stage
                .prim(openusd::sdf::path("/World/B/Child").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .is_some(),
            "child reparented under /World/B"
        );
    }

    #[test]
    fn set_attribute_roundtrips() {
        let stage = stage_with("/World");
        set_attribute(&stage, "/World", "radius", "double", Value::Double(2.5)).unwrap();
        let got = stage
            .prim(openusd::sdf::path("/World").unwrap()).expect("validated USD path")
            .attribute("radius")
            .get::<Value>()
            .unwrap();
        assert!(matches!(got, Some(Value::Double(d)) if (d - 2.5).abs() < 1e-9));
    }

    #[test]
    fn editor_history_preserves_existing_prim_and_authored_absence() {
        use crate::editor::{EditorEdit, EditorSession};
        let stage = stage_with("/World");
        define_prim(&stage, "/World/Box", "Cube").unwrap();
        set_attribute(&stage, "/World/Box", "custom", "string", Value::String("retained".into())).unwrap();
        let baseline = export_stage_string(&stage).unwrap();
        let mut editor = EditorSession::new(stage.clone());
        editor.edit(EditorEdit::Define { path: "/World/Box".into(), type_name: "Sphere".into() }).unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(export_stage_string(&stage).unwrap(), baseline);
        editor.edit(EditorEdit::Attribute {
            prim: "/World/Box".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(9.0),
        }).unwrap();
        assert!(editor.undo().unwrap());
        assert_eq!(export_stage_string(&stage).unwrap(), baseline);
        assert!(editor.redo().unwrap());
        assert_eq!(stage.prim(openusd::sdf::path("/World/Box").unwrap()).unwrap().attribute("size").get::<Value>().unwrap(), Some(Value::Double(9.0)));
        editor.edit(EditorEdit::Rename { path: "/World/Box".into(), name: "Crate".into() }).unwrap();
        assert!(prim_exists(&stage, "/World/Crate"));
        assert!(editor.undo().unwrap());
        assert!(prim_exists(&stage, "/World/Box"));
    }

    #[test]
    fn persistence_export_and_reopen() {
        let stage = stage_with("/World");
        define_prim(&stage, "/World/Saved", "Sphere").unwrap();
        set_attribute(
            &stage,
            "/World/Saved",
            "radius",
            "double",
            Value::Double(3.0),
        )
        .unwrap();

        // String export mentions the authored prim.
        let usda = export_stage_string(&stage).unwrap();
        assert!(
            usda.contains("Saved"),
            "export should contain the prim, got:\n{usda}"
        );

        // File export round-trips through a fresh open.
        let path = std::env::temp_dir().join("usd_bevy_persist_test.usda");
        let path_str = path.to_str().unwrap();
        save_stage_as(&stage, path_str).unwrap();
        let reopened = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path_str).unwrap();
        assert!(
            reopened
                .prim(openusd::sdf::path("/World/Saved").unwrap()).expect("validated USD path")
                .type_name()
                .unwrap()
                .is_some(),
            "reopened stage has the saved prim"
        );
        let _ = std::fs::remove_file(&path);
    }
}
