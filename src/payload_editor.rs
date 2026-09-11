use std::sync::{Arc, Mutex};
use mara::ui::mara_core::{pod::Pod, vocab::Id};
use openusd::sdf::{LayerOffset, Payload, PayloadListOp};
use usd_bevy::editor::{EditorBridge, EditorEdit, EditorSnapshot};

#[derive(Clone, PartialEq)]
struct Row { asset: String, prim: String, offset: String, scale: String, mode: Mode, authored_offset: bool }

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
    fn default() -> Self { Self { asset: String::new(), prim: String::new(), offset: "0".into(), scale: "1".into(), mode: Mode::default(), authored_offset: true } }
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
        let layer_offset = (self.authored_offset || offset != 0. || scale != 1.).then_some(LayerOffset::new(offset, scale));
        Ok(Payload { asset_path: self.asset.clone(), prim_path, layer_offset })
    }
}

fn local_operation(snapshot: &EditorSnapshot) -> Option<&PayloadListOp> {
    let target = snapshot.edit_target.as_ref()?;
    let path = target.map_to_spec_path(&openusd::sdf::path(snapshot.selected.as_ref()?).ok()?)?;
    snapshot.payload_opinions.iter().find(|opinion|
        opinion.layer == target.layer_identifier() && opinion.prim == path).map(|opinion| &opinion.operation)
}

fn rows_from_operation(operation: &PayloadListOp) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for (mode, items) in [(Mode::Replace, &operation.explicit_items), (Mode::Prepend, &operation.prepended_items),
        (Mode::Append, &operation.appended_items), (Mode::Add, &operation.added_items),
        (Mode::Delete, &operation.deleted_items), (Mode::Order, &operation.ordered_items)] {
        for payload in items {
            if rows.len() == 64 { return Err("Local opinion exceeds the 64-row editor limit".into()); }
            let offset = payload.layer_offset.unwrap_or_default();
            rows.push(Row { asset: payload.asset_path.clone(), prim: payload.prim_path.to_string(),
                offset: offset.offset.to_string(), scale: offset.scale.to_string(), mode,
                authored_offset: payload.layer_offset.is_some() });
        }
    }
    if parse_rows(&rows)? != *operation { return Err("Local opinion cannot be represented by this draft".into()); }
    Ok(rows)
}

#[derive(Default)]
struct State {
    context: Option<(u64, String, String)>, target: Option<openusd::usd::EditTarget>,
    rows: Vec<Row>, error: String,
    baseline: Option<PayloadListOp>, touched: bool,
}

fn source_conflicts(state: &mut State, current: Option<&PayloadListOp>) -> bool {
    if state.baseline.as_ref() == current { return false; }
    if !state.touched || current.is_some_and(|current| parse_rows(&state.rows).is_ok_and(|draft| &draft == current)) {
        state.baseline = current.cloned();
        return false;
    }
    true
}

fn reload_source(state: &mut State, current: Option<&PayloadListOp>) -> Result<(), String> {
    let rows = current.map(rows_from_operation).transpose()?.unwrap_or_default();
    state.rows = rows; state.baseline = current.cloned(); state.touched = current.is_some(); state.error.clear();
    Ok(())
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
    let local = local_operation(snapshot).cloned();
    let (count, conflict) = {
        let mut state = draft.0.lock().ok()?;
        if state.context.as_ref() != Some(&key) || state.target != snapshot.edit_target {
            *state = State { context: Some(key.clone()), target: snapshot.edit_target.clone(), baseline: local.clone(), ..Default::default() };
        }
        (state.rows.len(), source_conflicts(&mut state, local.as_ref()))
    };
    let bridge = bridge.clone();
    let snapshot = EditorSnapshot {
        document_id: snapshot.document_id, revision: snapshot.revision,
        edit_target: snapshot.edit_target.clone(), ..Default::default()
    };
    let draft = draft.clone();
    Some(Pod::new(Id::new(("editor.payload.author", key.clone()))).with_custom_units(7+3*usize::from(conflict)+count*6+usize::from(local.is_some()), move |ui| {
        let Ok(mut state) = draft.0.lock() else { return };
        if state.context.as_ref() != Some(&key) || state.target != snapshot.edit_target { return; }
        ui.label("Payload list (click row mode to change)");
        ui.label("Asset paths are relative to the edit layer");
        ui.label("Draft starts empty; does not copy composed arcs");
        let mut ready = !source_conflicts(&mut state, local.as_ref());
        if !ready {
            ui.label("Payload source changed; draft preserved");
            if ui.button("Discard draft and reload current opinion").clicked {
                match reload_source(&mut state, local.as_ref()) {
                    Ok(()) => ready = true,
                    Err(error) => state.error = error,
                }
            }
            if ui.button("Keep draft over changed payload opinion").clicked {
                state.baseline = local.clone(); state.error.clear(); ready = true;
            }
        }
        let mut remove = None;
        let before = state.rows.clone();
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
        if before != state.rows { state.touched = true; }
        let label = if state.rows.is_empty() { "Block all weaker payloads" } else { "Apply draft as local payload opinion" };
        if ready && ui.button(label).clicked {
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
        if let Some(operation) = &local {
            if ui.button("Discard draft and load local opinion").clicked {
                match rows_from_operation(operation) {
                    Ok(rows) => { state.rows = rows; state.baseline = Some(operation.clone()); state.touched = true; state.error.clear(); }
                    Err(error) => state.error = error,
                }
            }
        }
        if !state.error.is_empty() { ui.label(&state.error); }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_source_conflicts_preserve_drafts_and_acknowledge_writes() {
        let first = PayloadListOp::prepended([Payload { asset_path: "first.usda".into(), ..Default::default() }]);
        let second = PayloadListOp::prepended([Payload { asset_path: "second.usda".into(), ..Default::default() }]);
        let mut state = State { baseline: Some(first.clone()), ..Default::default() };
        assert!(!source_conflicts(&mut state, Some(&second)));
        assert_eq!(state.baseline, Some(second.clone()));
        reload_source(&mut state, Some(&first)).unwrap();
        state.rows[0].offset = "7".into();
        assert!(source_conflicts(&mut state, Some(&second)));
        assert_eq!(state.baseline, Some(first.clone()));
        assert_eq!(state.rows[0].offset, "7");
        state.baseline = Some(second.clone());
        assert!(!source_conflicts(&mut state, Some(&second)));
        assert_eq!(state.rows[0].asset, "first.usda");
        let written = parse_rows(&state.rows).unwrap();
        assert!(!source_conflicts(&mut state, Some(&written)));
        assert_eq!(state.baseline, Some(written));
        assert!(source_conflicts(&mut state, None));
        state.rows[0].scale = "invalid".into();
        assert!(source_conflicts(&mut state, None));
        assert!(state.baseline.is_some());
        reload_source(&mut state, Some(&second)).unwrap();
        assert_eq!(parse_rows(&state.rows).unwrap(), second);
        let unrepresentable = PayloadListOp::default();
        assert!(reload_source(&mut state, Some(&unrepresentable)).is_err());
        assert_eq!(parse_rows(&state.rows).unwrap(), second);
        reload_source(&mut state, None).unwrap();
        assert!(state.rows.is_empty() && !state.touched && state.baseline.is_none());
    }

    #[test]
    fn local_draft_import_preserves_buckets_offsets_and_rejects_loss() {
        let payload = Payload { asset_path: "relative with spaces.usda".into(), prim_path: openusd::sdf::Path::default(), layer_offset: None };
        for operation in [PayloadListOp::explicit([]), PayloadListOp::explicit([payload.clone()]),
            PayloadListOp { prepended_items: vec![payload.clone()], deleted_items: vec![Payload {
                layer_offset: Some(LayerOffset::new(12.125, 0.375)), ..payload.clone()
            }], ordered_items: vec![Payload { layer_offset: Some(LayerOffset::default()), ..payload.clone() }], ..Default::default() }] {
            let rows = rows_from_operation(&operation).unwrap();
            assert_eq!(parse_rows(&rows).unwrap(), operation);
        }
        assert!(rows_from_operation(&PayloadListOp::default()).is_err());
        assert!(rows_from_operation(&PayloadListOp::explicit(vec![payload.clone(); 65])).is_err());
        assert_eq!(rows_from_operation(&PayloadListOp::explicit(vec![payload.clone(); 64])).unwrap().len(), 64);
        let mut rows = rows_from_operation(&PayloadListOp::explicit([payload])).unwrap();
        rows[0].offset = "2".into();
        assert_eq!(parse_rows(&rows).unwrap().explicit_items[0].layer_offset, Some(LayerOffset::new(2., 1.)));
    }

    #[test]
    fn local_draft_import_matches_complete_variant_spec_not_weaker_layer() {
        let mut snapshot = EditorSnapshot {
            selected: Some("/Root".into()),
            edit_target: Some(openusd::usd::EditTarget::for_layer("root.usda")),
            payload_opinions: vec![usd_bevy::editor::PayloadOpinion {
                layer: "weak.usda".into(), prim: openusd::sdf::path("/Root{choice=a}").unwrap(),
                offset: LayerOffset::default(), operation: PayloadListOp::explicit([]),
            }], ..Default::default()
        };
        assert!(local_operation(&snapshot).is_none());
        snapshot.edit_target = Some(openusd::usd::EditTarget::for_layer("weak.usda"));
        assert!(local_operation(&snapshot).is_none());
        snapshot.edit_target = Some(openusd::usd::EditTarget::for_local_direct_variant("weak.usda", "/Root{choice=b}").unwrap());
        assert!(local_operation(&snapshot).is_none());
        snapshot.edit_target = Some(openusd::usd::EditTarget::for_local_direct_variant("weak.usda", "/Root{choice=a}").unwrap());
        assert!(local_operation(&snapshot).unwrap().explicit);
    }

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
