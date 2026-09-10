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
);

fn path_lines(path: &str) -> Vec<String> {
    path.split('\n').flat_map(|line| {
        let chars: Vec<_> = line.chars().collect();
        if chars.is_empty() { vec![String::new()] }
        else { chars.chunks(40).map(|chunk| chunk.iter().collect()).collect() }
    }).collect()
}

pub fn show(body: &mut PaneBody, snapshot: &EditorSnapshot, bridge: &EditorBridge, drafts: &Drafts, current_time: f64) {
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
                    super::send(&bridge, EditorCommand::Edit(edit));
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
    for (set, options) in &snapshot.variant_choices {
        let key = format!("{path}:variant:{set}");
        let selected = snapshot.variants.iter().find(|(name, _)| name == set)
            .map(|(_, value)| value.clone()).unwrap_or_default();
        let (path, set, options) = (path.clone(), set.clone(), options.clone());
        let bridge = bridge.clone();
        pods.push(Pod::new(Id::new(&key)).with_custom_units(options.len() + 1, move |ui| {
            ui.label(&format!("Variant: {set} = {selected}"));
            for selection in options {
                if ui.button(&selection).clicked && selection != selected {
                    super::send(&bridge, EditorCommand::Edit(EditorEdit::Variant {
                        prim: path.clone(), set: set.clone(), selection,
                    }));
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
        pods.push(Pod::new(Id::new(&key)).with_custom_units(5, move |ui| {
            ui.label(&format!("Relationship: {name}"));
            let Ok(mut drafts) = drafts.0.lock() else { return };
            let draft = drafts.entry(key).or_insert_with(|| (current.clone(), current.clone(), String::new()));
            if draft.0 != current { *draft = (current.clone(), current, String::new()); }
            ui.text_input(&mut draft.1, "Absolute targets, separated by spaces");
            if ui.button("Set targets (empty blocks weaker targets)").clicked {
                match draft.1.split_whitespace().map(openusd::sdf::path).collect::<Result<Vec<_>, _>>() {
                    Ok(targets) => {
                        draft.2.clear();
                        super::send(&bridge, EditorCommand::Edit(EditorEdit::RelationshipTargets { prim: path.clone(), name: name.clone(), targets }));
                    }
                    Err(error) => draft.2 = error.to_string(),
                }
            }
            if ui.button("Clear local target opinion").clicked {
                super::send(&bridge, EditorCommand::Edit(EditorEdit::ClearRelationshipTargets { prim: path, name }));
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
        let summary_lines = path_lines(&attribute.source_summary);
        let expanded = drafts.2.lock().ok().and_then(|states| states.get(&key).copied()).unwrap_or(false);
        let sample_lines = path_lines(&sample_summary(&attribute.sample_times));
        let units = 16 + summary_lines.len() + sample_lines.len() + if expanded { source_lines.len() } else { 0 } + if matrix_attribute { 6 } else { 0 };
        let destination = if matrix_attribute { &mut matrix_pods } else { &mut pods };
        destination.push(Pod::new(Id::new(&key)).with_custom_units(units, move |ui| {
            ui.label(&format!("{} ({})", attribute.name, attribute.type_name));
            if matrix_attribute {
                if attribute.value.is_none() { ui.label("No authored default; edits are a draft"); }
                let value = attribute.value.clone().or_else(|| value_template("matrix4d")).unwrap();
                let Some(current) = editable_text(&value) else { ui.label("Invalid matrix value"); return };
                let Ok(mut state) = drafts.0.lock() else { return };
                let draft = state.entry(key.clone()).or_insert_with(|| (current.clone(), current.clone(), String::new()));
                if draft.0 != current { *draft = (current.clone(), current, String::new()); }
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
                super::send(&bridge, EditorCommand::Edit(EditorEdit::ClearAttributeValues { prim: path.clone(), name: attribute.name.clone() }));
            }
            if ui.button("Block values and samples").clicked {
                super::send(&bridge, EditorCommand::Edit(EditorEdit::BlockAttributeValues { prim: path.clone(), name: attribute.name.clone() }));
            }
            let Ok(mut times) = drafts.1.lock() else { return };
            let time = times.entry(key.clone()).or_insert_with(|| (current_time.to_string(), String::new()));
            ui.text_input(&mut time.0, "Scene sample time");
            if ui.button("Use timeline time").clicked { time.0 = current_time.to_string(); time.1.clear(); }
            if ui.button("Clear sample at time").clicked {
                match parse_sample_time(&time.0) {
                    Ok(time_code) => {
                        time.1.clear();
                        super::send(&bridge, EditorCommand::Edit(EditorEdit::ClearAttributeSample {
                            prim: path.clone(), name: attribute.name.clone(), time: time_code,
                        }));
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
            if draft.0 != current { *draft = (current.clone(), current, String::new()); }
            if !matrix_attribute { ui.text_input(&mut draft.1, "Value"); }
            if ui.button("Apply sample at time").clicked {
                match parse_sample_time(&time.0).and_then(|time| parse_value(&value, &draft.1).map(|value| (time, value))) {
                    Ok((time_code, value)) => {
                        draft.2.clear();
                        time.1.clear();
                        super::send(&bridge, EditorCommand::Edit(EditorEdit::AttributeSample {
                            prim: path.clone(), name: attribute.name.clone(), type_name: attribute.type_name.clone(), value, time: time_code,
                        }));
                    }
                    Err(error) => draft.2 = error,
                }
            }
            if ui.button("Apply default value").clicked {
                match parse_value(&value, &draft.1) {
                    Ok(value) => {
                        draft.2.clear();
                        super::send(&bridge, EditorCommand::Edit(EditorEdit::Attribute {
                            prim: path, name: attribute.name, type_name: attribute.type_name, value,
                        }));
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
