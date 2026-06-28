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
    let prim = stage.prim(openusd::sdf::path(path)?);
    let mut editor = NamespaceEditor::new(stage);
    editor.rename_prim(&prim, new_name)?;
    editor.apply()?;
    Ok(())
}

/// Reparent the prim at `path` under `new_parent` (keeping its name).
pub fn reparent_prim(stage: &Stage, path: &str, new_parent: &str) -> Result<()> {
    let prim = stage.prim(openusd::sdf::path(path)?);
    let parent = stage.prim(openusd::sdf::path(new_parent)?);
    let mut editor = NamespaceEditor::new(stage);
    editor.reparent_prim(&prim, &parent)?;
    editor.apply()?;
    Ok(())
}

/// Move the prim at `old` to the absolute path `new` (rename + reparent in one).
pub fn move_prim(stage: &Stage, old: &str, new: &str) -> Result<()> {
    let mut editor = NamespaceEditor::new(stage);
    editor.move_prim(openusd::sdf::path(old)?, openusd::sdf::path(new)?);
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

/// Clear an authored attribute opinion (in the current edit target).
pub fn clear_attribute(stage: &Stage, prim: &str, name: &str) -> Result<bool> {
    let attr = openusd::sdf::path(prim)?.append_property(name)?;
    Ok(stage.remove_property(attr)?)
}

// ─── Persistence (P6) ───────────────────────────────────────────────

/// Serialize the stage's composed root layer to a `.usda` string.
pub fn export_stage_string(stage: &Stage) -> Result<String> {
    Ok(stage.root_layer().export_to_string()?)
}

/// Write the stage's root layer to `filename` (a `.usda`/`.usd` path).
pub fn save_stage_as(stage: &Stage, filename: &str) -> Result<()> {
    stage.root_layer().export(filename)?;
    Ok(())
}

/// Whether a prim with an authored type currently resolves on the stage.
pub fn prim_exists(stage: &Stage, path: &str) -> bool {
    openusd::sdf::path(path)
        .ok()
        .map(|p| stage.prim(p).type_name().map(|t| t.is_some()).unwrap_or(false))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage_with(root: &str) -> Stage {
        let stage = Stage::builder().in_memory("authoring_test.usda").unwrap();
        stage.define_prim(root).unwrap().set_type_name("Xform").unwrap();
        stage
    }

    #[test]
    fn define_and_remove() {
        let stage = stage_with("/World");
        define_prim(&stage, "/World/Box", "Cube").unwrap();
        assert_eq!(
            stage.prim(openusd::sdf::path("/World/Box").unwrap()).type_name().unwrap().as_deref(),
            Some("Cube")
        );
        assert!(remove_prim(&stage, "/World/Box").unwrap());
        assert!(
            stage.prim(openusd::sdf::path("/World/Box").unwrap()).type_name().unwrap().is_none(),
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
            stage.prim(openusd::sdf::path("/World/Renamed").unwrap()).type_name().unwrap().is_some(),
            "rename created /World/Renamed"
        );
        assert!(
            stage.prim(openusd::sdf::path("/World/A").unwrap()).type_name().unwrap().is_none(),
            "old /World/A is gone"
        );

        reparent_prim(&stage, "/World/Renamed/Child", "/World/B").unwrap();
        assert!(
            stage.prim(openusd::sdf::path("/World/B/Child").unwrap()).type_name().unwrap().is_some(),
            "child reparented under /World/B"
        );
    }

    #[test]
    fn set_attribute_roundtrips() {
        let stage = stage_with("/World");
        set_attribute(&stage, "/World", "radius", "double", Value::Double(2.5)).unwrap();
        let got = stage
            .prim(openusd::sdf::path("/World").unwrap())
            .attribute("radius")
            .get::<Value>()
            .unwrap();
        assert!(matches!(got, Some(Value::Double(d)) if (d - 2.5).abs() < 1e-9));
    }

    #[test]
    fn persistence_export_and_reopen() {
        let stage = stage_with("/World");
        define_prim(&stage, "/World/Saved", "Sphere").unwrap();
        set_attribute(&stage, "/World/Saved", "radius", "double", Value::Double(3.0)).unwrap();

        // String export mentions the authored prim.
        let usda = export_stage_string(&stage).unwrap();
        assert!(usda.contains("Saved"), "export should contain the prim, got:\n{usda}");

        // File export round-trips through a fresh open.
        let path = std::env::temp_dir().join("usd_bevy_persist_test.usda");
        let path_str = path.to_str().unwrap();
        save_stage_as(&stage, path_str).unwrap();
        let reopened = Stage::open(path_str).unwrap();
        assert!(
            reopened
                .prim(openusd::sdf::path("/World/Saved").unwrap())
                .type_name()
                .unwrap()
                .is_some(),
            "reopened stage has the saved prim"
        );
        let _ = std::fs::remove_file(&path);
    }
}
