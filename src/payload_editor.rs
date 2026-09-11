use std::sync::{Arc, Mutex};
use mara::ui::mara_core::{pod::Pod, vocab::Id};
use openusd::sdf::{LayerOffset, Payload};
use usd_bevy::editor::{EditorBridge, EditorEdit, EditorSnapshot};

#[derive(Clone)]
struct Row { asset: String, prim: String, offset: String, scale: String }

impl Default for Row {
    fn default() -> Self { Self { asset: String::new(), prim: String::new(), offset: "0".into(), scale: "1".into() } }
}

impl Row {
    fn parse(&self) -> Result<Payload, String> {
        let prim_path = if self.prim.trim().is_empty() { openusd::sdf::Path::default() }
            else { openusd::sdf::path(self.prim.trim()).map_err(|error| error.to_string())? };
        if self.asset.is_empty() && prim_path.is_empty() { return Err("Specify an asset or an internal prim target".into()); }
        if !prim_path.is_empty() && (!prim_path.as_str().starts_with('/') || !prim_path.is_prim_path()
            || prim_path.as_str() == "/" || prim_path.contains_prim_variant_selection()) {
            return Err("Target must be an absolute prim path, or empty for defaultPrim".into());
        }
        let offset: f64 = self.offset.trim().parse().map_err(|_| "Invalid time offset")?;
        let scale: f64 = self.scale.trim().parse().map_err(|_| "Invalid time scale")?;
        if !offset.is_finite() || !scale.is_finite() || scale <= 0. { return Err("Use a finite offset and positive finite scale".into()); }
        Ok(Payload { asset_path: self.asset.clone(), prim_path, layer_offset: Some(LayerOffset::new(offset, scale)) })
    }
}

#[derive(Default)]
struct State {
    context: Option<(u64, String, String)>, target: Option<openusd::usd::EditTarget>,
    rows: Vec<Row>, error: String,
}

#[derive(Clone, Default)]
pub struct PayloadDraft(Arc<Mutex<State>>);

pub fn pod(snapshot: &EditorSnapshot, bridge: &EditorBridge, draft: &PayloadDraft) -> Option<Pod> {
    let prim = snapshot.selected.as_ref()?.clone();
    if prim == "/" { return None; }
    let key = (snapshot.document_id, snapshot.edit_layer.clone(), prim.clone());
    let count = {
        let mut state = draft.0.lock().ok()?;
        if state.context.as_ref() != Some(&key) || state.target != snapshot.edit_target {
            *state = State { context: Some(key.clone()), target: snapshot.edit_target.clone(), ..Default::default() };
        }
        state.rows.len()
    };
    let bridge = bridge.clone();
    let snapshot = EditorSnapshot {
        document_id: snapshot.document_id, revision: snapshot.revision,
        edit_target: snapshot.edit_target.clone(), ..Default::default()
    };
    let draft = draft.clone();
    Some(Pod::new(Id::new(("editor.payload.author", key.clone()))).with_custom_units(7+count*6, move |ui| {
        let Ok(mut state) = draft.0.lock() else { return };
        if state.context.as_ref() != Some(&key) || state.target != snapshot.edit_target { return; }
        ui.label("Payload list replacement");
        ui.label("Asset paths are relative to the edit layer");
        ui.label("Draft starts empty; does not copy composed arcs");
        let mut remove = None;
        for (index, row) in state.rows.iter_mut().enumerate() {
            ui.label(&format!("Payload {}", index+1));
            ui.text_input(&mut row.asset, "Asset path (empty = internal)");
            ui.text_input(&mut row.prim, "Prim path (empty = defaultPrim)");
            ui.text_input(&mut row.offset, "Time offset");
            ui.text_input(&mut row.scale, "Time scale");
            if ui.button(&format!("Remove payload {}", index+1)).clicked { remove = Some(index); }
        }
        if let Some(index) = remove { state.rows.remove(index); }
        if ui.button("Add payload entry").clicked && state.rows.len() < 64 { state.rows.push(Row::default()); }
        let label = if state.rows.is_empty() { "Block all weaker payloads" } else { "Replace payload list with draft" };
        if ui.button(label).clicked {
            match state.rows.iter().map(Row::parse).collect::<Result<Vec<_>,_>>() {
                Ok(payloads) => {
                    state.error.clear();
                    if let Some(command) = snapshot.checked_edit(EditorEdit::Payloads { prim: prim.clone(), payloads }) { super::send(&bridge, command); }
                }
                Err(error) => state.error = error,
            }
        }
        if ui.button("Clear local payload opinion").clicked {
            state.error.clear();
            if let Some(command) = snapshot.checked_edit(EditorEdit::ClearPayloads { prim: prim.clone() }) { super::send(&bridge, command); }
        }
        if !state.error.is_empty() { ui.label(&state.error); }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_payload_draft_replaces_and_clears_the_showcase() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/payload_authoring.usda");
        let bytes = std::fs::read(path).unwrap();
        let source = usd_bevy::UsdSource::new(path, bytes.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut editor = usd_bevy::editor::EditorSession::new(stage.clone());
        let shape = || stage.prim("/Root/Shape").unwrap().type_name().unwrap();
        assert_eq!(shape().as_deref(), Some("Cube"));
        let payload = Row { asset: "payload_authoring_content.usda".into(), prim: "/Ball".into(), ..Default::default() }.parse().unwrap();
        editor.edit(EditorEdit::Payloads { prim: "/Root".into(), payloads: vec![payload] }).unwrap();
        assert_eq!(shape().as_deref(), Some("Sphere"));
        editor.edit(EditorEdit::ClearPayloads { prim: "/Root".into() }).unwrap();
        assert_eq!(shape().as_deref(), Some("Cube"));
        editor.undo().unwrap();
        assert_eq!(shape().as_deref(), Some("Sphere"));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn payload_rows_preserve_asset_paths_and_validate_targets_and_time() {
        let row = Row { asset: "part with spaces.usda".into(), ..Default::default() };
        assert_eq!(row.parse().unwrap().asset_path, row.asset);
        assert!(Row::default().parse().is_err());
        assert!(Row { prim: "/Model".into(), ..Default::default() }.parse().is_ok());
        for prim in ["relative", "/", "/Model.size", "/Model{v=x}"] {
            assert!(Row { prim: prim.into(), ..row.clone() }.parse().is_err());
        }
        for scale in ["0", "-1", "NaN", "inf", "bad"] {
            assert!(Row { scale: scale.into(), ..row.clone() }.parse().is_err());
        }
        assert!(Row { offset: "NaN".into(), ..row }.parse().is_err());
    }

    #[test]
    fn payload_drafts_do_not_cross_document_prim_or_edit_layer() {
        let draft = PayloadDraft::default();
        let bridge = EditorBridge::default();
        let mut snapshot = EditorSnapshot { document_id: 1, selected: Some("/Root".into()), edit_layer: "root.usda".into(), ..Default::default() };
        for change in 0..5 {
            match change {
                1 => snapshot.document_id = 2,
                2 => snapshot.selected = Some("/Other".into()),
                3 => snapshot.edit_layer = "weak.usda".into(),
                4 => snapshot.edit_target = Some(openusd::usd::EditTarget::for_local_direct_variant("weak.usda", "/Other{v=a}").unwrap()),
                _ => {},
            }
            assert!(pod(&snapshot, &bridge, &draft).is_some());
            assert!(draft.0.lock().unwrap().rows.is_empty());
            draft.0.lock().unwrap().rows.push(Row::default());
            assert!(pod(&snapshot, &bridge, &draft).is_some());
            assert_eq!(draft.0.lock().unwrap().rows.len(), 1);
        }
    }
}
