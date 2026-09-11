use std::sync::{Arc, Mutex};
use mara::ui::mara_core::{pod::Pod, vocab::Id};
use openusd::sdf::{LayerOffset, Payload, PayloadListOp};
use usd_bevy::editor::{EditorBridge, EditorEdit, EditorSnapshot};

#[derive(Clone)]
struct Row { asset: String, prim: String, offset: String, scale: String, mode: Mode }

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode { #[default] Replace, Prepend, Append, Add, Delete, Order }

impl Mode {
    fn label(self) -> &'static str {
        match self { Self::Replace => "Replace", Self::Prepend => "Prepend", Self::Append => "Append",
            Self::Add => "Add", Self::Delete => "Delete", Self::Order => "Order" }
    }

    fn next(self) -> Self {
        match self { Self::Replace => Self::Prepend, Self::Prepend => Self::Append, Self::Append => Self::Add,
            Self::Add => Self::Delete, Self::Delete => Self::Order, Self::Order => Self::Replace }
    }
}

impl Default for Row {
    fn default() -> Self { Self { asset: String::new(), prim: String::new(), offset: "0".into(), scale: "1".into(), mode: Mode::default() } }
}

fn parse_rows(rows: &[Row]) -> Result<PayloadListOp, String> {
    let explicit = rows.iter().all(|row| row.mode == Mode::Replace);
    if !explicit && rows.iter().any(|row| row.mode == Mode::Replace) {
        return Err("Replace cannot be mixed with other row modes".into());
    }
    let mut operation = PayloadListOp { explicit, ..Default::default() };
    for row in rows {
        let bucket = match row.mode {
            Mode::Replace => &mut operation.explicit_items, Mode::Prepend => &mut operation.prepended_items,
            Mode::Append => &mut operation.appended_items, Mode::Add => &mut operation.added_items,
            Mode::Delete => &mut operation.deleted_items, Mode::Order => &mut operation.ordered_items,
        };
        bucket.push(row.parse()?);
    }
    Ok(operation)
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

pub fn opinion_pods(snapshot: &EditorSnapshot) -> Vec<Pod> {
    snapshot.payload_opinions.iter().enumerate().map(|(index, opinion)| {
        let mut lines = vec![format!("Authored opinion {} (strongest first)", index+1)];
        lines.extend(super::inspector::path_lines(&opinion.layer));
        lines.extend(super::inspector::path_lines(opinion.prim.as_str()));
        lines.push(format!("Site time: offset {}, scale {}", opinion.offset.offset, opinion.offset.scale));
        let op = &opinion.operation;
        if op.explicit && op.explicit_items.is_empty() { lines.push("Explicit empty list (blocks weaker payloads)".into()); }
        for (name, entries) in [("Explicit", &op.explicit_items), ("Prepend", &op.prepended_items),
            ("Append", &op.appended_items), ("Add", &op.added_items), ("Delete", &op.deleted_items), ("Order", &op.ordered_items)] {
            for payload in entries {
                lines.push(name.into());
                lines.extend(super::inspector::path_lines(&format!("Asset: {}", if payload.asset_path.is_empty() { "(internal)" } else { &payload.asset_path })));
                lines.extend(super::inspector::path_lines(&format!("Prim: {}", if payload.prim_path.is_empty() { "(defaultPrim)" } else { payload.prim_path.as_str() })));
                if let Some(offset) = payload.layer_offset { lines.push(format!("Arc time: offset {}, scale {}", offset.offset, offset.scale)); }
            }
        }
        Pod::new(Id::new(("editor.payload.opinion", index))).with_custom_units(lines.len(), move |ui| {
            for line in lines { ui.label(&line); }
        })
    }).collect()
}

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
        ui.label("Payload list (click row mode to change)");
        ui.label("Asset paths are relative to the edit layer");
        ui.label("Draft starts empty; does not copy composed arcs");
        let mut remove = None;
        for (index, row) in state.rows.iter_mut().enumerate() {
            if ui.button(&format!("Payload {}: {}", index+1, row.mode.label())).clicked { row.mode = row.mode.next(); }
            ui.text_input(&mut row.asset, "Asset path (empty = internal)");
            ui.text_input(&mut row.prim, "Prim path (empty = defaultPrim)");
            ui.text_input(&mut row.offset, "Time offset");
            ui.text_input(&mut row.scale, "Time scale");
            if ui.button(&format!("Remove payload {}", index+1)).clicked { remove = Some(index); }
        }
        if let Some(index) = remove { state.rows.remove(index); }
        if ui.button("Add payload entry").clicked && state.rows.len() < 64 { state.rows.push(Row::default()); }
        let label = if state.rows.is_empty() { "Block all weaker payloads" } else { "Apply draft as local payload opinion" };
        if ui.button(label).clicked {
            match parse_rows(&state.rows) {
                Ok(operation) => {
                    state.error.clear();
                    if let Some(command) = snapshot.checked_edit(EditorEdit::PayloadListOp { prim: prim.clone(), operation }) { super::send(&bridge, command); }
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
    fn row_modes_build_complete_ops_and_reject_mixed_replacement() {
        let row = Row { prim: "/Model".into(), ..Default::default() };
        let payload = row.parse().unwrap();
        let mut mode = Mode::Replace;
        for expected in [PayloadListOp::explicit([payload.clone()]), PayloadListOp::prepended([payload.clone()]),
            PayloadListOp::appended([payload.clone()]), PayloadListOp::added([payload.clone()]),
            PayloadListOp::deleted([payload.clone()]), PayloadListOp::ordered([payload.clone()])] {
            assert_eq!(parse_rows(&[Row { mode, ..row.clone() }]).unwrap(), expected);
            mode = mode.next();
        }
        assert!(mode == Mode::Replace);
        assert_eq!(parse_rows(&[]).unwrap(), PayloadListOp::explicit([]));
        assert!(parse_rows(&[row.clone(), Row { mode: Mode::Delete, ..row.clone() }]).is_err());
        let mixed = parse_rows(&[Row { mode: Mode::Prepend, ..row.clone() }, Row { mode: Mode::Delete, ..row }]).unwrap();
        assert_eq!(mixed.prepended_items, vec![payload.clone()]);
        assert_eq!(mixed.deleted_items, vec![payload]);
        assert!(!mixed.explicit);
    }

    #[test]
    fn delete_row_matches_omitted_identity_offset_and_undo_restores_cube() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/payload_authoring.usda");
        let source = usd_bevy::UsdSource::new(path, std::fs::read(path).unwrap()).unwrap();
        let mut editor = usd_bevy::editor::EditorSession::new(source.open_stage().unwrap());
        let operation = parse_rows(&[Row { asset: "payload_authoring_content.usda".into(), prim: "/Box".into(), mode: Mode::Delete, ..Default::default() }]).unwrap();
        editor.edit(EditorEdit::PayloadListOp { prim: "/Root".into(), operation }).unwrap();
        assert!(!editor.stage().prim("/Root/Shape").unwrap().is_valid().unwrap());
        editor.undo().unwrap();
        assert_eq!(editor.stage().prim("/Root/Shape").unwrap().type_name().unwrap().as_deref(), Some("Cube"));
    }

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
