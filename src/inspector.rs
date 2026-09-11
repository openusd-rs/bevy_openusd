use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mara::ui::mara_core::{pane::PaneBody, pod::Pod, vocab::Id};
use openusd::sdf::Value;
use usd_bevy::editor::{EditorBridge, EditorCommand, EditorEdit, EditorSnapshot};

#[derive(Default, Clone)]
pub struct Drafts(
    Arc<Mutex<HashMap<String, (String, String, String)>>>,
    Arc<Mutex<HashMap<String, (String, String)>>>,
    Arc<Mutex<HashMap<String, bool>>>,
    crate::payload_editor::PayloadDraft,
    Arc<Mutex<Option<(u64, Option<openusd::usd::EditTarget>)>>>,
);

impl Drafts {
    fn synchronize_context(&self, snapshot: &EditorSnapshot) {
        let Ok(mut context) = self.4.lock() else { return };
        let current = (snapshot.document_id, snapshot.edit_target.clone());
        if context.as_ref() == Some(&current) { return; }
        if let Ok(mut values) = self.0.lock() { values.clear(); }
        if let Ok(mut times) = self.1.lock() { times.clear(); }
        if let Ok(mut expanded) = self.2.lock() { expanded.clear(); }
        *context = Some(current);
    }
}

pub(crate) fn path_lines(path: &str) -> Vec<String> {
    path.split('\n').flat_map(|line| {
        let chars: Vec<_> = line.chars().collect();
        if chars.is_empty() { vec![String::new()] }
        else { chars.chunks(40).map(|chunk| chunk.iter().collect()).collect() }
    }).collect()
}

fn send_edit(bridge: &EditorBridge, snapshot: &EditorSnapshot, edit: EditorEdit) {
    if let Some(command) = snapshot.checked_edit(edit) { super::send(bridge, command); }
}

fn draft_conflicts(draft: &mut (String, String, String), current: &str) -> bool {
    if draft.0 == current { return false; }
    if draft.1 == draft.0 || draft.1 == current {
        *draft = (current.into(), current.into(), String::new());
        false
    } else { true }
}

fn reconcile_draft(ui: &mut mara::ui::mara_core::MaraUi<'_>, draft: &mut (String, String, String), current: &str) -> bool {
    if !draft_conflicts(draft, current) { return true; }
    ui.label("Source changed; your draft is preserved");
    ui.label(&format!("Current: {}", current.chars().take(80).collect::<String>()));
    if ui.button("Reload current value (discard draft)").clicked {
        *draft = (current.into(), current.into(), String::new());
        return true;
    }
    if ui.button("Keep draft over current value").clicked {
        draft.0 = current.into();
        draft.2.clear();
        return true;
    }
    false
}

pub fn show(body: &mut PaneBody, snapshot: &EditorSnapshot, bridge: &EditorBridge, drafts: &Drafts, current_time: f64) {
    drafts.synchronize_context(snapshot);
    let edit_context = Arc::new(EditorSnapshot {
        document_id: snapshot.document_id, revision: snapshot.revision,
        edit_target: snapshot.edit_target.clone(), ..Default::default()
    });
    let target = path_lines(&snapshot.edit_layer);
    let mut layers = vec![Pod::new("editor.target").with_custom_units(target.len() + 1, move |ui| {
        ui.label("Edit target");
        for line in target { ui.label(&line); }
    })];
    for layer in &snapshot.layers {
        let layer = layer.clone();
        let lines = path_lines(&layer);
        let active = layer == snapshot.edit_layer;
        let bridge = bridge.clone();
        layers.push(Pod::new(Id::new(("editor.layer", &layer))).with_custom_units(lines.len() + 1, move |ui| {
            for line in lines { ui.label(&line); }
            if active { ui.label("Active edit target"); }
            else if ui.button("Use as edit target").clicked { super::send(&bridge, EditorCommand::EditLayer(layer)); }
        }));
    }
    body.add_normal("editor.layers", "Layers / edit target", "document", layers);
    let Some(path) = &snapshot.selected else {
        body.add_normal("editor.selection", "Selection", "options",
            vec![Pod::new("editor.none").with_readout("Prim", "Select a prim in the outliner")]);
        return;
    };
    let prim_path = path_lines(path);
    let mut pods = vec![Pod::new("editor.path").with_custom_units(prim_path.len() + 1, move |ui| {
        ui.label("Prim");
        for line in prim_path { ui.label(&line); }
    })];
    for (index, issue) in snapshot.render_issues.iter().enumerate() {
        let lines = crate::lighting::status_lines(issue);
        pods.push(Pod::new(Id::new(("editor.render.issue", index))).with_custom_units(lines.len() + 1, move |ui| {
            ui.label("Rendering issue");
            for line in lines { ui.label(&line); }
        }));
    }
    for (index, issue) in snapshot.reflect_issues.iter().enumerate() {
        let field = issue.field.as_deref().map_or(String::new(), |field| format!(" / {field}"));
        let message = format!("{}{}: {:?}", issue.type_segment, field, issue.kind);
        pods.push(Pod::new(Id::new(("editor.reflect.issue", index))).with_readout("Component issue", &message));
    }
    for (index, warning) in snapshot.material_warnings.iter().enumerate() {
        pods.push(Pod::new(Id::new(("editor.material.warning", index))).with_readout("Material warning", warning));
    }
    if let Some(info) = &snapshot.asset_info {
        for (name, value) in info.iter() {
            let lines = path_lines(&format!("{name}: {value:?}"));
            pods.push(Pod::new(Id::new(("editor.asset_info", name))).with_custom_units(lines.len() + 1, move |ui| {
                ui.label("Asset metadata");
                for line in lines { ui.label(&line); }
            }));
        }
    }
    if path != "/" {
        for (action, initial) in [("Rename", path.rsplit('/').next().unwrap_or("").to_string()), ("Move", path.clone())] {
            let key = format!("{path}:namespace:{action}");
            let path = path.clone();
            let drafts = drafts.clone();
            let bridge = bridge.clone();
            let edit_context = edit_context.clone();
            pods.push(Pod::new(Id::new(&key)).with_custom_units(2, move |ui| {
                let Ok(mut drafts) = drafts.0.lock() else { return };
                let draft = drafts.entry(key).or_insert_with(|| (initial.clone(), initial, String::new()));
                ui.text_input(&mut draft.1, if action == "Rename" { "New prim name" } else { "Absolute destination path" });
                if ui.button(action).clicked {
                    let edit = if action == "Rename" {
                        EditorEdit::Rename { path, name: draft.1.clone() }
                    } else {
                        EditorEdit::Move { path, destination: draft.1.clone() }
                    };
                    send_edit(&bridge, &edit_context, edit);
                }
            }));
        }
    }
    if let Some(loaded) = snapshot.selected_loaded {
        let bridge = bridge.clone();
        let path = path.clone();
        pods.push(Pod::new("editor.payload").with_custom_units(2, move |ui| {
            ui.readout("Payload state", if loaded { "Loaded" } else { "Unloaded" });
            if ui.button(if loaded { "Unload payloads" } else { "Load payloads" }).clicked {
                super::send(&bridge, EditorCommand::Payload { prim: path, loaded: !loaded });
            }
        }));
    }
    if let Some(payload) = crate::payload_editor::pod(snapshot, bridge, &drafts.3) {
        let mut payloads = vec![payload];
        payloads.extend(crate::payload_editor::opinion_pods(snapshot));
        payloads.extend(reference_opinion_pods(snapshot, bridge, drafts));
        payloads.extend(reference_edit_pods(snapshot, bridge, drafts));
        body.add_normal("editor.payloads", "Payloads / references", "document", payloads);
    }
    for (set, options) in &snapshot.variant_choices {
        let key = format!("{path}:variant:{set}");
        let selected = snapshot.variants.iter().find(|(name, _)| name == set)
            .map(|(_, value)| value.clone()).unwrap_or_default();
        let (path, set, options) = (path.clone(), set.clone(), options.clone());
        let bridge = bridge.clone();
        let edit_context = edit_context.clone();
        pods.push(Pod::new(Id::new(&key)).with_custom_units(options.len() + 1, move |ui| {
            ui.label(&format!("Variant: {set} = {selected}"));
            for selection in options {
                if ui.button(&selection).clicked && selection != selected {
                    send_edit(&bridge, &edit_context, EditorEdit::Variant {
                        prim: path.clone(), set: set.clone(), selection,
                    });
                }
            }
        }));
    }
    for (name, targets) in &snapshot.relationships {
        let key = format!("{path}:relationship:{name}");
        let current = targets.join(" ");
        let (path, name) = (path.clone(), name.clone());
        let drafts = drafts.clone();
        let bridge = bridge.clone();
        let edit_context = edit_context.clone();
        pods.push(Pod::new(Id::new(&key)).with_custom_units(9, move |ui| {
            ui.label(&format!("Relationship: {name}"));
            let Ok(mut drafts) = drafts.0.lock() else { return };
            let draft = drafts.entry(key).or_insert_with(|| (current.clone(), current.clone(), String::new()));
            if !reconcile_draft(ui, draft, &current) { return; }
            ui.text_input(&mut draft.1, "Absolute targets, separated by spaces");
            if ui.button("Set targets (empty blocks weaker targets)").clicked {
                match draft.1.split_whitespace().map(openusd::sdf::path).collect::<Result<Vec<_>, _>>() {
                    Ok(targets) => {
                        draft.2.clear();
                        send_edit(&bridge, &edit_context, EditorEdit::RelationshipTargets { prim: path.clone(), name: name.clone(), targets });
                    }
                    Err(error) => draft.2 = error.to_string(),
                }
            }
            if ui.button("Clear local target opinion").clicked {
                send_edit(&bridge, &edit_context, EditorEdit::ClearRelationshipTargets { prim: path, name });
            }
            if !draft.2.is_empty() { ui.label(&draft.2); }
        }));
    }
    let mut matrix_pods = Vec::new();
    for attribute in &snapshot.attributes {
        let matrix_attribute = attribute.type_name == "matrix4d";
        let sampled_time = snapshot.sample_time;
        let key = format!("{}:{}:{path}:{}", snapshot.document_id, snapshot.edit_layer, attribute.name);
        let attribute = attribute.clone();
        let path = path.clone();
        let drafts = drafts.clone();
        let bridge = bridge.clone();
        let source_lines = path_lines(&attribute.source);
        let edit_context = edit_context.clone();
        let summary_lines = path_lines(&attribute.source_summary);
        let expanded = drafts.2.lock().ok().and_then(|states| states.get(&key).copied()).unwrap_or(false);
        let sample_lines = path_lines(&sample_summary(&attribute.sample_times));
        let units = 20 + summary_lines.len() + sample_lines.len() + if expanded { source_lines.len() } else { 0 } + if matrix_attribute { 6 } else { 0 };
        let destination = if matrix_attribute { &mut matrix_pods } else { &mut pods };
        destination.push(Pod::new(Id::new(&key)).with_custom_units(units, move |ui| {
            ui.label(&format!("{} ({})", attribute.name, attribute.type_name));
            if matrix_attribute {
                if attribute.value.is_none() { ui.label("No authored default; edits are a draft"); }
                let value = attribute.value.clone().or_else(|| value_template("matrix4d")).unwrap();
                let Some(current) = editable_text(&value) else { ui.label("Invalid matrix value"); return };
                let Ok(mut state) = drafts.0.lock() else { return };
                let draft = state.entry(key.clone()).or_insert_with(|| (current.clone(), current.clone(), String::new()));
                if !reconcile_draft(ui, draft, &current) { return; }
                let mut loaded = None;
                if let Some((time, matrix)) = sampled_time.zip(attribute.sampled_matrix) {
                    if ui.button(&format!("Load sampled matrix at {time}")).clicked {
                        draft.1 = editable_text(&Value::Matrix4d(openusd::gf::Matrix4d(matrix))).unwrap();
                        draft.2.clear();
                        loaded = Some(time);
                    }
                }
                ui.label("USD rows; translation is in row 4");
                let mut rows: Vec<String> = draft.1.split('\n').map(str::to_owned).collect();
                rows.resize(4, String::new());
                for (index, row) in rows.iter_mut().enumerate() { ui.text_input(row, &format!("Row {}: four numbers", index + 1)); }
                draft.1 = rows.join("\n");
                drop(state);
                if let Some(time) = loaded {
                    if let Ok(mut times) = drafts.1.lock() { times.insert(key.clone(), (time.to_string(), String::new())); }
                }
            }
            ui.label("Source");
            for line in summary_lines { ui.label(&line); }
            if ui.button(if expanded { "Hide source details" } else { "Show source details" }).clicked {
                if let Ok(mut states) = drafts.2.lock() { states.insert(key.clone(), !expanded); }
            }
            if expanded { for line in source_lines { ui.label(&line); } }
            for line in sample_lines { ui.label(&line); }
            if attribute.blocked { ui.label("Value is blocked"); }
            if ui.button("Clear local values / samples").clicked {
                send_edit(&bridge, &edit_context, EditorEdit::ClearAttributeValues { prim: path.clone(), name: attribute.name.clone() });
            }
            if ui.button("Block values and samples").clicked {
                send_edit(&bridge, &edit_context, EditorEdit::BlockAttributeValues { prim: path.clone(), name: attribute.name.clone() });
            }
            let Ok(mut times) = drafts.1.lock() else { return };
            let time = times.entry(key.clone()).or_insert_with(|| (current_time.to_string(), String::new()));
            ui.text_input(&mut time.0, "Scene sample time");
            if ui.button("Use timeline time").clicked { time.0 = current_time.to_string(); time.1.clear(); }
            if ui.button("Clear sample at time").clicked {
                match parse_sample_time(&time.0) {
                    Ok(time_code) => {
                        time.1.clear();
                        send_edit(&bridge, &edit_context, EditorEdit::ClearAttributeSample {
                            prim: path.clone(), name: attribute.name.clone(), time: time_code,
                        });
                    }
                    Err(error) => time.1 = error,
                }
            }
            if !time.1.is_empty() { ui.label(&time.1); }
            if attribute.value.is_none() { ui.label("No default; apply to author one"); }
            let Some(value) = attribute.value.or_else(|| value_template(&attribute.type_name)) else {
                ui.label("This USD type is read-only");
                return;
            };
            let Some(current) = editable_text(&value) else { ui.label(&format!("{value:?}")); return };
            let Ok(mut drafts) = drafts.0.lock() else { return };
            let draft = drafts.entry(key).or_insert_with(|| (current.clone(), current.clone(), String::new()));
            if !reconcile_draft(ui, draft, &current) { return; }
            if !matrix_attribute { ui.text_input(&mut draft.1, "Value"); }
            if ui.button("Apply sample at time").clicked {
                match parse_sample_time(&time.0).and_then(|time| parse_value(&value, &draft.1).map(|value| (time, value))) {
                    Ok((time_code, value)) => {
                        draft.2.clear();
                        time.1.clear();
                        send_edit(&bridge, &edit_context, EditorEdit::AttributeSample {
                            prim: path.clone(), name: attribute.name.clone(), type_name: attribute.type_name.clone(), value, time: time_code,
                        });
                    }
                    Err(error) => draft.2 = error,
                }
            }
            if ui.button("Apply default value").clicked {
                match parse_value(&value, &draft.1) {
                    Ok(value) => {
                        draft.2.clear();
                        send_edit(&bridge, &edit_context, EditorEdit::Attribute {
                            prim: path, name: attribute.name, type_name: attribute.type_name, value,
                        });
                    }
                    Err(error) => draft.2 = error,
                }
            }
            if !draft.2.is_empty() { ui.label(&draft.2); }
        }));
    }
    if !matrix_pods.is_empty() { body.add_normal("editor.matrices", "Matrix attributes", "options", matrix_pods); }
    body.add_normal("editor.attributes", "Composed properties", "options", pods);
}

fn reference_opinion_lines(opinion: &usd_bevy::editor::ReferenceOpinion, index: usize) -> Vec<String> {
    let mut lines = vec![format!("Reference opinion {} (strongest first)", index+1), "Asset paths relative to source layer".into()];
    lines.extend(path_lines(&opinion.layer));
    lines.extend(path_lines(opinion.prim.as_str()));
    lines.push(format!("Site time: offset {}, scale {}", opinion.offset.offset, opinion.offset.scale));
    let op = &opinion.operation;
    if op.explicit && op.explicit_items.is_empty() { lines.push("Explicit empty list (blocks weaker references)".into()); }
    for (name, entries) in [("Explicit", &op.explicit_items), ("Prepend", &op.prepended_items),
        ("Append", &op.appended_items), ("Add", &op.added_items), ("Delete", &op.deleted_items), ("Order", &op.ordered_items)] {
        for reference in entries {
            lines.push(name.into());
            lines.push(format!("Asset: {}", if reference.asset_path.is_empty() { "(internal)" } else { &reference.asset_path }));
            lines.push(format!("Prim: {}", if reference.prim_path.is_empty() { "(defaultPrim)" } else { reference.prim_path.as_str() }));
            lines.push(format!("Arc time: offset {}, scale {}", reference.layer_offset.offset, reference.layer_offset.scale));
            let mut keys = reference.custom_data.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys { lines.push(format!("customData {key}: {:?}", reference.custom_data[key])); }
        }
    }
    lines.into_iter().flat_map(|line| path_lines(&line)).collect()
}

fn clear_reference_command(snapshot: &EditorSnapshot, opinion: &usd_bevy::editor::ReferenceOpinion) -> Option<EditorCommand> {
    let prim = snapshot.selected.as_ref()?;
    let target = snapshot.edit_target.as_ref()?;
    if opinion.layer != target.layer_identifier()
        || target.map_to_spec_path(&openusd::sdf::path(prim).ok()?)? != opinion.prim { return None; }
    snapshot.checked_edit(EditorEdit::ClearReferences { prim: prim.clone() })
}

fn reference_opinion_pods(snapshot: &EditorSnapshot, bridge: &EditorBridge, drafts: &Drafts) -> Vec<Pod> {
    snapshot.reference_opinions.iter().enumerate().map(|(index, opinion)| {
        let lines = reference_opinion_lines(opinion, index);
        let clear = clear_reference_command(snapshot, opinion);
        let expanded = drafts.2.clone();
        let key = format!("reference.edit:{}:{}", opinion.layer, opinion.prim);
        let bridge = bridge.clone();
        Pod::new(Id::new(("editor.reference.opinion", index))).with_custom_units(lines.len()+2*usize::from(clear.is_some()), move |ui| {
            for line in lines { ui.label(&line); }
            if let Some(command) = clear {
                if let Ok(mut expanded) = expanded.lock() {
                    let open = expanded.entry(key).or_default();
                    if ui.button(if *open { "Hide reference entry fields" } else { "Edit local reference entries" }).clicked { *open = !*open; }
                }
                if ui.button("Clear local reference opinion").clicked { super::send(&bridge, command); }
            }
        })
    }).collect()
}

fn edited_reference(base: &openusd::sdf::Reference, fields: &[String; 4]) -> Result<openusd::sdf::Reference, String> {
    let mut reference = base.clone();
    reference.asset_path = fields[0].clone();
    reference.prim_path = if fields[1].trim().is_empty() { openusd::sdf::Path::default() }
        else { openusd::sdf::path(fields[1].trim()).map_err(|error| error.to_string())? };
    if reference.asset_path.is_empty() && reference.prim_path.is_empty() { return Err("Specify an asset or internal target".into()); }
    if !reference.prim_path.is_empty() && (!reference.prim_path.as_str().starts_with('/')
        || !reference.prim_path.is_prim_path() || reference.prim_path.as_str() == "/"
        || reference.prim_path.contains_prim_variant_selection()) { return Err("Use an absolute prim target or empty defaultPrim".into()); }
    reference.layer_offset = openusd::sdf::LayerOffset::new(
        fields[2].trim().parse().map_err(|_| "Invalid time offset")?,
        fields[3].trim().parse().map_err(|_| "Invalid time scale")?);
    if !reference.layer_offset.offset.is_finite() || !reference.layer_offset.scale.is_finite()
        || reference.layer_offset.scale <= 0. { return Err("Use a finite offset and positive finite scale".into()); }
    Ok(reference)
}

fn reference_edit_pods(snapshot: &EditorSnapshot, bridge: &EditorBridge, drafts: &Drafts) -> Vec<Pod> {
    let mut pods = Vec::new();
    for opinion in &snapshot.reference_opinions {
        if clear_reference_command(snapshot, opinion).is_none() { continue; }
        let key = format!("reference.edit:{}:{}", opinion.layer, opinion.prim);
        if !drafts.2.lock().is_ok_and(|expanded| expanded.get(&key) == Some(&true)) { continue; }
        for (bucket, entries) in [("Explicit", &opinion.operation.explicit_items), ("Prepend", &opinion.operation.prepended_items),
            ("Append", &opinion.operation.appended_items), ("Add", &opinion.operation.added_items),
            ("Delete", &opinion.operation.deleted_items), ("Order", &opinion.operation.ordered_items)] {
            for (index, reference) in entries.iter().enumerate() {
                let key = format!("reference:{}:{}:{bucket}:{index}", opinion.layer, opinion.prim);
                let current = [reference.asset_path.clone(), reference.prim_path.to_string(),
                    reference.layer_offset.offset.to_string(), reference.layer_offset.scale.to_string()];
                let conflicts = drafts.0.lock().map(|values| current.iter().enumerate().filter(|(field, source)|
                    values.get(&format!("{key}:{field}")).is_some_and(|draft|
                        draft.0 != **source && draft.1 != draft.0 && draft.1 != **source)).count()).unwrap_or(0);
                let reference = reference.clone();
                let mut operation = opinion.operation.clone();
                let drafts = drafts.clone();
                let bridge = bridge.clone();
                let prim = snapshot.selected.clone().unwrap();
                let context = EditorSnapshot { document_id: snapshot.document_id, revision: snapshot.revision,
                    edit_target: snapshot.edit_target.clone(), ..Default::default() };
                pods.push(Pod::new(Id::new(&key)).with_custom_units(12+conflicts*4, move |ui| {
                    let Ok(active) = drafts.4.lock() else { return };
                    if active.as_ref() != Some(&(context.document_id, context.edit_target.clone())) { return; }
                    drop(active);
                    let Ok(mut values) = drafts.0.lock() else { return };
                    ui.label(&format!("Edit reference: {bucket} {}", index+1));
                    ui.label("Other entries and customData are retained");
                    let mut fields = current.clone();
                    let mut ready = true;
                    for (field, label) in ["Asset path", "Prim target (empty = defaultPrim)", "Time offset", "Time scale"].into_iter().enumerate() {
                        let draft = values.entry(format!("{key}:{field}")).or_insert_with(|| (current[field].clone(), current[field].clone(), String::new()));
                        ui.label(label);
                        if reconcile_draft(ui, draft, &current[field]) { ui.text_input(&mut draft.1, label); fields[field] = draft.1.clone(); }
                        else { ready = false; }
                    }
                    let error = &mut values.entry(format!("{key}:error")).or_default().2;
                    if ready && ui.button("Apply reference entry").clicked {
                        match edited_reference(&reference, &fields) {
                            Ok(reference) => {
                                let entries = match bucket { "Explicit" => &mut operation.explicit_items, "Prepend" => &mut operation.prepended_items,
                                    "Append" => &mut operation.appended_items, "Add" => &mut operation.added_items,
                                    "Delete" => &mut operation.deleted_items, _ => &mut operation.ordered_items };
                                entries[index] = reference;
                                error.clear();
                                send_edit(&bridge, &context, EditorEdit::ReferenceListOp { prim, operation });
                            }
                            Err(message) => *error = message,
                        }
                    }
                    if !error.is_empty() { ui.label(error); }
                }));
            }
        }
    }
    pods
}

fn parse_sample_time(input: &str) -> Result<f64, String> {
    input.trim().parse::<f64>().ok().filter(|time| time.is_finite())
        .ok_or_else(|| "Sample time must be a finite number".into())
}

fn sample_summary(times: &[f64]) -> String {
    if times.is_empty() { return "Scene samples: none".into(); }
    let shown = times.iter().take(6).map(ToString::to_string).collect::<Vec<_>>().join(", ");
    if times.len() > 6 { format!("Scene samples: {shown} ({} total)", times.len()) }
    else { format!("Scene samples: {shown}") }
}

fn value_template(type_name: &str) -> Option<Value> {
    Some(match type_name {
        "bool" => Value::Bool(false),
        "int" => Value::Int(0),
        "int64" => Value::Int64(0),
        "uint" => Value::Uint(0),
        "uint64" => Value::Uint64(0),
        "float" => Value::Float(0.0),
        "double" => Value::Double(0.0),
        "matrix4d" => Value::Matrix4d(openusd::gf::Matrix4d([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0])),
        "string" => Value::String(String::new()),
        "token" => Value::Token("".into()),
        "asset" => Value::AssetPath("".into()),
        "float3" | "point3f" | "vector3f" | "normal3f" | "color3f" => Value::Vec3f([0.0;3].into()),
        "double3" | "point3d" | "vector3d" | "normal3d" | "color3d" => Value::Vec3d([0.0;3].into()),
        _ => return None,
    })
}

fn editable_text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Bool(v) => v.to_string(),
        Value::Int(v) => v.to_string(),
        Value::Int64(v) => v.to_string(),
        Value::Uint(v) => v.to_string(),
        Value::Uint64(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Token(v) => v.as_str().into(),
        Value::AssetPath(v) => v.as_str().into(),
        Value::Vec3f(v) => format!("{} {} {}", v.x, v.y, v.z),
        Value::Vec3d(v) => format!("{} {} {}", v.x, v.y, v.z),
        Value::Matrix4d(v) => v.0.chunks_exact(4).map(|row| row.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n"),
        _ => return None,
    })
}

fn parse_value(template: &Value, input: &str) -> Result<Value, String> {
    let invalid = || "Invalid value for this USD type".to_string();
    let text = input.trim();
    Ok(match template {
        Value::Bool(_) => Value::Bool(text.parse().map_err(|_| invalid())?),
        Value::Int(_) => Value::Int(text.parse().map_err(|_| invalid())?),
        Value::Int64(_) => Value::Int64(text.parse().map_err(|_| invalid())?),
        Value::Uint(_) => Value::Uint(text.parse().map_err(|_| invalid())?),
        Value::Uint64(_) => Value::Uint64(text.parse().map_err(|_| invalid())?),
        Value::String(_) => Value::String(input.into()),
        Value::Token(_) => Value::Token(input.into()),
        Value::AssetPath(_) => Value::AssetPath(input.into()),
        Value::Float(_) => {
            let value: f32 = text.parse().map_err(|_| invalid())?;
            if !value.is_finite() { return Err(invalid()); }
            Value::Float(value)
        }
        Value::Double(_) => {
            let value: f64 = text.parse().map_err(|_| invalid())?;
            if !value.is_finite() { return Err(invalid()); }
            Value::Double(value)
        }
        Value::Matrix4d(_) => {
            let values = text.split_whitespace().map(str::parse::<f64>).collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Matrix needs sixteen finite numbers".to_string())?;
            let values: [f64; 16] = values.try_into().map_err(|_| "Matrix needs sixteen finite numbers".to_string())?;
            if !values.iter().all(|v| v.is_finite()) { return Err("Matrix needs sixteen finite numbers".into()); }
            Value::Matrix4d(openusd::gf::Matrix4d(values))
        }
        Value::Vec3f(_) | Value::Vec3d(_) => {
            let values: Vec<f64> = text.split_whitespace().map(str::parse)
                .collect::<Result<_, _>>().map_err(|_| invalid())?;
            let values: [f64; 3] = values.try_into().map_err(|_| invalid())?;
            if !values.iter().all(|v| v.is_finite()) { return Err(invalid()); }
            if matches!(template, Value::Vec3f(_)) {
                let values = values.map(|v| v as f32);
                if !values.iter().all(|v| v.is_finite()) { return Err(invalid()); }
                Value::Vec3f(values.into())
            } else { Value::Vec3d(values.into()) }
        }
        _ => return Err("This USD type is read-only in the inspector".into()),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_entry_edit_preserves_custom_data_and_undo() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/reference_custom_data.usda");
        let bytes = std::fs::read(path).unwrap();
        let source = usd_bevy::UsdSource::new(path, bytes.as_slice()).unwrap();
        let mut editor = usd_bevy::editor::EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let mut operation = editor.snapshot().unwrap().reference_opinions[0].operation.clone();
        let base = operation.prepended_items[0].clone();
        let fields = [base.asset_path.clone(), "/Ball".into(), "12.5".into(), "3".into()];
        let edited = super::edited_reference(&base, &fields).unwrap();
        assert_eq!(edited.custom_data, base.custom_data);
        assert_eq!(edited.layer_offset, openusd::sdf::LayerOffset::new(12.5, 3.));
        operation.prepended_items[0] = edited;
        editor.edit(usd_bevy::editor::EditorEdit::ReferenceListOp { prim: "/Root".into(), operation }).unwrap();
        assert_eq!(editor.stage().prim("/Root/Shape").unwrap().type_name().unwrap().as_deref(), Some("Sphere"));
        editor.undo().unwrap();
        assert_eq!(editor.snapshot().unwrap().reference_opinions[0].operation.prepended_items[0], base);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        for (field, value) in [(1, "relative"), (1, "/Root.attr"), (1, "/Root{v=a}"), (2, "NaN"), (3, "0"), (3, "-1")] {
            let mut invalid = fields.clone();
            invalid[field] = value.into();
            assert!(super::edited_reference(&base, &invalid).is_err());
        }
    }

    #[test]
    fn reference_clear_control_matches_mapped_target_and_retains_guard() {
        let opinion = usd_bevy::editor::ReferenceOpinion { layer: "weak.usda".into(),
            prim: openusd::sdf::path("/Source{choice=a}").unwrap(), offset: openusd::sdf::LayerOffset::default(),
            operation: openusd::sdf::ReferenceListOp::explicit([]) };
        let mut snapshot = usd_bevy::editor::EditorSnapshot { document_id: 4, revision: 9,
            selected: Some("/Source".into()), edit_target: Some(openusd::usd::EditTarget::for_layer("root.usda")), ..Default::default() };
        assert!(super::clear_reference_command(&snapshot, &opinion).is_none());
        snapshot.edit_target = Some(openusd::usd::EditTarget::for_layer("weak.usda"));
        assert!(super::clear_reference_command(&snapshot, &opinion).is_none());
        snapshot.edit_target = Some(openusd::usd::EditTarget::for_local_direct_variant("weak.usda", "/Source{choice=a}").unwrap());
        let Some(usd_bevy::editor::EditorCommand::EditChecked { edit: usd_bevy::editor::EditorEdit::ClearReferences { prim }, document_id, revision, target })
            = super::clear_reference_command(&snapshot, &opinion) else { panic!("checked clear") };
        assert_eq!((document_id, revision, prim.as_str()), (4, 9, "/Source"));
        assert_eq!(Some(target), snapshot.edit_target);
    }

    #[test]
    fn reference_readout_keeps_source_identity_buckets_and_custom_data() {
        use openusd::sdf::{LayerOffset, Reference, ReferenceListOp, Value};
        let reference = Reference { asset_path: "long relative path with spaces/model.usda".into(),
            prim_path: openusd::sdf::path("/Original").unwrap(), layer_offset: LayerOffset::new(10., 2.),
            custom_data: [("label".into(), Value::String("retained".into()))].into_iter().collect() };
        let mut opinion = usd_bevy::editor::ReferenceOpinion { layer: "source.usda".into(),
            prim: openusd::sdf::path("/Source{choice=a}").unwrap(), offset: LayerOffset::new(20., 3.),
            operation: ReferenceListOp::deleted([reference]) };
        let lines = super::reference_opinion_lines(&opinion, 0);
        assert!(lines.iter().all(|line| line.chars().count() <= 40));
        let text = lines.join("");
        for expected in ["source.usda", "/Source{choice=a}", "Delete", "long relative path with spaces/model.usda", "/Original", "offset 20, scale 3", "offset 10, scale 2", "label", "retained"] {
            assert!(text.contains(expected), "{expected}: {text}");
        }
        opinion.operation = ReferenceListOp::explicit([]);
        assert!(super::reference_opinion_lines(&opinion, 0).join("").contains("blocks weaker references"));
    }

    #[test]
    fn variant_composition_changes_conflict_with_relationship_drafts() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/draft_conflict.usda");
        let source = usd_bevy::UsdSource::new(path, std::fs::read(path).unwrap()).unwrap();
        let mut editor = usd_bevy::editor::EditorSession::new(source.open_stage().unwrap());
        editor.select(Some("/Root".into())).unwrap();
        let targets = |snapshot: usd_bevy::editor::EditorSnapshot| snapshot.relationships.into_iter()
            .find(|(name,_)| name == "links").unwrap().1.join(" ");
        let original = targets(editor.snapshot().unwrap());
        assert_eq!(original, "/A");
        let mut draft = (original, "/A/Child".into(), String::new());
        editor.edit(usd_bevy::editor::EditorEdit::Variant { prim: "/Root".into(), set: "choice".into(), selection: "b".into() }).unwrap();
        let current = targets(editor.snapshot().unwrap());
        assert_eq!(current, "/B");
        assert!(super::draft_conflicts(&mut draft, &current));
        assert_eq!(draft.1, "/A/Child");
        editor.undo().unwrap();
        assert!(!super::draft_conflicts(&mut draft, &targets(editor.snapshot().unwrap())));
        assert_eq!(draft.1, "/A/Child");
        editor.redo().unwrap();
        draft.0 = targets(editor.snapshot().unwrap());
        editor.edit(usd_bevy::editor::EditorEdit::RelationshipTargets {
            prim: "/Root".into(), name: "links".into(), targets: vec![openusd::sdf::path("/A/Child").unwrap()],
        }).unwrap();
        assert_eq!(targets(editor.snapshot().unwrap()), "/A/Child");
        assert!(!super::draft_conflicts(&mut draft, &targets(editor.snapshot().unwrap())));
        editor.edit(usd_bevy::editor::EditorEdit::Variant { prim: "/Root".into(), set: "choice".into(), selection: "a".into() }).unwrap();
        assert_eq!(targets(editor.snapshot().unwrap()), "/A/Child");
        editor.undo().unwrap();
        assert_eq!(targets(editor.snapshot().unwrap()), "/A/Child");
        editor.undo().unwrap();
        assert_eq!(targets(editor.snapshot().unwrap()), "/B");
        assert_eq!(std::fs::read(path).unwrap(), include_bytes!("../assets/draft_conflict.usda"));
    }

    #[test]
    fn changed_sources_preserve_dirty_drafts_and_acknowledge_applied_values() {
        let mut pristine = ("old".into(), "old".into(), "error".into());
        assert!(!super::draft_conflicts(&mut pristine, "new"));
        assert_eq!(pristine, ("new".into(), "new".into(), String::new()));
        for (old, draft, current) in [("1", "2", "3"), ("/A", "/B /C", "/D"),
            ("1 0\n0 1", "1 2\n3 4", "5 6\n7 8")] {
            let mut value = (old.into(), draft.into(), "parse error".into());
            let original = value.clone();
            assert!(super::draft_conflicts(&mut value, current));
            assert_eq!(value, original);
            assert!(super::draft_conflicts(&mut value, current));
            assert_eq!(value, original);
            value.0 = current.into();
            assert!(!super::draft_conflicts(&mut value, current));
            assert_eq!(value.1, draft);
        }
        let mut applied = ("old".into(), "authored".into(), "error".into());
        assert!(!super::draft_conflicts(&mut applied, "authored"));
        assert_eq!(applied, ("authored".into(), "authored".into(), String::new()));
    }

    #[test]
    fn drafts_reset_on_document_or_target_not_revision_or_selection() {
        let drafts = super::Drafts::default();
        let mut snapshot = usd_bevy::editor::EditorSnapshot {
            document_id: 1, edit_target: Some(openusd::usd::EditTarget::for_layer("root.usda")), ..Default::default()
        };
        for transition in 0..4 {
            match transition {
                1 => snapshot.document_id = 2,
                2 => snapshot.edit_target = Some(openusd::usd::EditTarget::for_layer("other.usda")),
                3 => snapshot.edit_target = Some(openusd::usd::EditTarget::for_local_direct_variant("other.usda", "/Root{v=a}").unwrap()),
                _ => {},
            }
            drafts.synchronize_context(&snapshot);
            assert!(drafts.0.lock().unwrap().is_empty());
            assert!(drafts.1.lock().unwrap().is_empty());
            assert!(drafts.2.lock().unwrap().is_empty());
            drafts.0.lock().unwrap().insert("field".into(), ("original".into(), "unsaved".into(), "error".into()));
            drafts.1.lock().unwrap().insert("field".into(), ("3".into(), "error".into()));
            drafts.2.lock().unwrap().insert("field".into(), true);
            snapshot.revision += 1;
            snapshot.selected = Some(format!("/Prim{transition}"));
            drafts.synchronize_context(&snapshot);
            assert_eq!(drafts.0.lock().unwrap()["field"].1, "unsaved");
            assert_eq!(drafts.1.lock().unwrap()["field"].0, "3");
            assert!(drafts.2.lock().unwrap()["field"]);
        }
    }

    #[test]
    fn inspector_edit_sender_preserves_context_and_rejects_stale_actions() {
        use bevy::prelude::*;
        use usd_bevy::editor::{EditorBridge, EditorCommand, EditorEdit, EditorPlugin, EditorSession};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, usd_bevy::live::LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge.send(EditorCommand::Open(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/payload_authoring.usda").into())).unwrap();
        app.update();
        let snapshot = bridge.view().unwrap().document;
        super::send_edit(&bridge, &snapshot, EditorEdit::Attribute {
            prim: "/Root".into(), name: "value".into(), type_name: "double".into(), value: openusd::sdf::Value::Double(3.),
        });
        app.update();
        let stage = app.world().non_send::<EditorSession>().stage().clone();
        assert_eq!(stage.attribute("/Root.value").unwrap().get::<f64>().unwrap(), Some(3.));
        super::send_edit(&bridge, &snapshot, EditorEdit::Rename { path: "/Root".into(), name: "Stale".into() });
        app.update();
        assert!(bridge.view().unwrap().status.starts_with("Failed:"));
        assert!(stage.prim("/Root").unwrap().is_valid().unwrap());
        assert!(!stage.prim("/Stale").unwrap().is_valid().unwrap());
        let snapshot = bridge.view().unwrap().document;
        super::send_edit(&bridge, &snapshot, EditorEdit::Rename { path: "/Root".into(), name: "Renamed".into() });
        app.update();
        assert!(stage.prim("/Renamed").unwrap().is_valid().unwrap());
        assert!(app.world().resource::<usd_bevy::live::PrimEntities>().entity("/Renamed").is_some());
        bridge.send(EditorCommand::Undo).unwrap();
        app.update();
        assert!(stage.prim("/Root").unwrap().is_valid().unwrap());
    }

    #[test]
    fn matrix_rows_preserve_double_precision_and_reject_invalid_input() {
        use openusd::sdf::Value;
        let values = [1.000000000000001,0.0,0.5,0.0, 0.75,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 3.25,-4.5,5.75,1.0];
        let value = Value::Matrix4d(openusd::gf::Matrix4d(values));
        let text = super::editable_text(&value).unwrap();
        assert_eq!(text.lines().count(), 4);
        assert_eq!(text.lines().last().unwrap(), "3.25 -4.5 5.75 1");
        assert_eq!(super::parse_value(&value, &text).unwrap(), value);
        for bad in ["", "1 2 3", "0 ".repeat(17).as_str(), text.replace("3.25", "NaN").as_str(), text.replace("3.25", "inf").as_str()] {
            assert!(super::parse_value(&value, bad).is_err());
        }
    }

    #[test]
    fn parsed_matrix_sample_keeps_stack_and_undo_restores_authored_data() {
        use usd_bevy::editor::{EditorEdit, EditorSession};
        let stage = usd_bevy::UsdSource::new("inspector-matrix.usda", br#"#usda 1.0
def Xform "M" {
    matrix4d xformOp:transform = ((1,0,0,0),(0,1,0,0),(0,0,1,0),(0,0,0,1))
    double3 xformOp:translate = (1,2,3)
    uniform token[] xformOpOrder = ["!resetXformStack!", "xformOp:translate", "xformOp:transform"]
}
"#.as_slice()).unwrap().open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let template = super::value_template("matrix4d").unwrap();
        let value = super::parse_value(&template, "1 0 0.5 0\n0.75 1 0 0\n0 0 1 0\n3 4 5 1").unwrap();
        let order = stage.attribute("/M.xformOpOrder").unwrap().get::<openusd::sdf::Value>().unwrap();
        editor.edit(EditorEdit::AttributeSample { prim: "/M".into(), name: "xformOp:transform".into(),
            type_name: "matrix4d".into(), value: value.clone(), time: 10.0 }).unwrap();
        let attribute = stage.attribute("/M.xformOp:transform").unwrap();
        assert_eq!(attribute.get::<openusd::sdf::Value>().unwrap(), Some(template));
        assert_eq!(attribute.get_at::<openusd::sdf::Value>(Some(openusd::usd::TimeCode::new(10.0))).unwrap(), Some(value));
        assert_eq!(stage.attribute("/M.xformOpOrder").unwrap().get::<openusd::sdf::Value>().unwrap(), order);
        assert!(editor.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn sample_summary_bounds_the_visible_time_list() {
        assert_eq!(super::sample_summary(&[]), "Scene samples: none");
        assert_eq!(super::sample_summary(&[0.0, 14.0]), "Scene samples: 0, 14");
        assert_eq!(super::sample_summary(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), "Scene samples: 0, 1, 2, 3, 4, 5 (7 total)");
    }

    #[test]
    fn sample_time_accepts_finite_scene_codes_only() {
        for input in ["", "NaN", "inf", "-inf", "1e999", "default", "1 2"] {
            assert!(super::parse_sample_time(input).is_err(), "{input}");
        }
        for (input, expected) in [("0", 0.0), (" -2.5 ", -2.5), ("1e3", 1000.0)] {
            assert_eq!(super::parse_sample_time(input).unwrap(), expected);
        }
    }

    #[test]
    fn absent_value_templates_roundtrip_declared_types_without_authoring() {
        for name in ["bool", "int", "int64", "uint", "uint64", "float", "double", "string", "token", "asset",
            "float3", "point3f", "vector3f", "normal3f", "color3f", "double3", "point3d", "vector3d", "normal3d", "color3d", "matrix4d"] {
            let template = super::value_template(name).unwrap();
            let text = super::editable_text(&template).unwrap();
            assert_eq!(super::parse_value(&template, &text).unwrap(), template, "{name}");
        }
        for name in ["", "float[]", "unknown"] { assert!(super::value_template(name).is_none()); }
        let template = super::value_template("asset").unwrap();
        let path = "../textures/材質 with  spaces.exr";
        let value = super::parse_value(&template, path).unwrap();
        assert_eq!(super::editable_text(&value).unwrap(), path);
    }

    #[test]
    fn wrapped_paths_preserve_unicode_spaces_and_long_identifiers() {
        let path = format!("/home/user/project with  spaces/{}/scene.usda", "材質é".repeat(30));
        let lines = super::path_lines(&path);
        assert_eq!(lines.concat(), path);
        assert!(lines.iter().all(|line| line.chars().count() <= 40));
        assert_eq!(super::path_lines("first\n\nthird"), ["first", "", "third"]);
        assert_eq!(super::path_lines(""), [""]);
    }

    use super::*;
    #[test]
    fn typed_input_rejects_overflow_and_preserves_text() {
        assert!(parse_value(&Value::Float(0.0), "inf").is_err());
        assert!(parse_value(&Value::Uint(0), "-1").is_err());
        assert_eq!(parse_value(&Value::String(String::new()), " a \" b ").unwrap(), Value::String(" a \" b ".into()));
        let value = Value::Vec3d([1.0, 2.0, 3.0].into());
        assert_eq!(parse_value(&value, &editable_text(&value).unwrap()).unwrap(), value);
        assert!(parse_value(&value, "1 2").is_err());
    }
}
