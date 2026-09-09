//! Author-back (PLAN P2): ECS component → USD opinions.
//!
//! The write direction of the reflect route. Where [`crate::route::reflect`]
//! projects `bevy:` attributes onto components, this reads a component back
//! and authors its fields as `bevy:<Type>:<field>` attributes into the stage's
//! current edit target. This is the round-trip BSN structurally can't do:
//! spawned entities remain views of the stage, and edits flow back into
//! layers that persist and export.
//!
//! USD is still the source of truth. After authoring, the commit fires the
//! change sink and the reflect route re-projects — writing the same value it
//! just read, so the round-trip is idempotent (see the echo guard on
//! [`crate::live::LiveStage`]).

use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy::prelude::*;
use bevy::reflect::enums::VariantType;
use bevy::reflect::{PartialReflect, ReflectRef, TypeRegistry};
use openusd::sdf::Value;
use openusd::usd::Stage;

type Result<T> = anyhow::Result<T>;

/// A reflected leaf value as a USD `(value, typeName)` pair, or `None` for a
/// field type we don't author (mirrors the forward route's coverage).
fn value_of(field: &dyn PartialReflect) -> Option<(Value, &'static str)> {
    if let Some(x) = field.try_downcast_ref::<f64>() {
        return Some((Value::Double(*x), "double"));
    }
    if let Some(x) = field.try_downcast_ref::<f32>() {
        return Some((Value::Float(*x), "float"));
    }
    if let Some(x) = field.try_downcast_ref::<i32>() {
        return Some((Value::Int(*x), "int"));
    }
    if let Some(x) = field.try_downcast_ref::<i64>() {
        return Some((Value::Int64(*x), "int64"));
    }
    if let Some(x) = field.try_downcast_ref::<u32>() {
        return Some((Value::Uint(*x), "uint"));
    }
    if let Some(x) = field.try_downcast_ref::<u64>() {
        return Some((Value::Uint64(*x), "uint64"));
    }
    if let Some(x) = field.try_downcast_ref::<usize>() {
        return Some((Value::Uint64(*x as u64), "uint64"));
    }
    if let Some(x) = field.try_downcast_ref::<bool>() {
        return Some((Value::Bool(*x), "bool"));
    }
    if let Some(x) = field.try_downcast_ref::<String>() {
        return Some((Value::String(x.clone()), "string"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec2>() {
        return Some((Value::Vec2f([x.x, x.y].into()), "float2"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec3>() {
        return Some((Value::Vec3f([x.x, x.y, x.z].into()), "float3"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec4>() {
        return Some((Value::Vec4f([x.x, x.y, x.z, x.w].into()), "float4"));
    }
    if let Some(x) = field.try_downcast_ref::<Quat>() {
        // gf Quatf fields are (w, x, y, z); build them explicitly rather than
        // via `From<[f32;4]>` (which is real-first and would scramble xyzw).
        return Some((
            Value::Quatf(openusd::gf::Quatf {
                w: x.w,
                x: x.x,
                y: x.y,
                z: x.z,
            }),
            "quatf",
        ));
    }
    if let Some(c) = field.try_downcast_ref::<Color>() {
        // Author as linear `color4f` (round-trips with the forward route's
        // linear mapping, preserving alpha).
        let l = c.to_linear();
        return Some((
            Value::Vec4f([l.red, l.green, l.blue, l.alpha].into()),
            "color4f",
        ));
    }
    // Array fields.
    if let Some(x) = field.try_downcast_ref::<Vec<f32>>() {
        return Some((Value::FloatVec(x.clone()), "float[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<i32>>() {
        return Some((Value::IntVec(x.clone()), "int[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<String>>() {
        return Some((Value::StringVec(x.clone()), "string[]"));
    }
    if let Some(x) = field.try_downcast_ref::<Vec<Vec3>>() {
        let v = x.iter().map(|p| [p.x, p.y, p.z].into()).collect();
        return Some((Value::Vec3fVec(v), "float3[]"));
    }
    // Option<T>: `Some` authors the inner value; `None` authors nothing.
    if let Some(o) = field.try_downcast_ref::<Option<f32>>() {
        return (*o).map(|x| (Value::Float(x), "float"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<f64>>() {
        return (*o).map(|x| (Value::Double(x), "double"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<i32>>() {
        return (*o).map(|x| (Value::Int(x), "int"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<bool>>() {
        return (*o).map(|x| (Value::Bool(x), "bool"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<String>>() {
        return o.as_ref().map(|x| (Value::String(x.clone()), "string"));
    }
    if let Some(o) = field.try_downcast_ref::<Option<Vec3>>() {
        return o.map(|v| (Value::Vec3f([v.x, v.y, v.z].into()), "float3"));
    }
    // Unit-variant enums author as a token naming the variant.
    if let ReflectRef::Enum(e) = field.reflect_ref()
        && e.variant_type() == VariantType::Unit
    {
        return Some((Value::Token(e.variant_name().into()), "token"));
    }
    None
}

/// A reflected field that cannot produce a USD opinion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentFieldIssue {
    pub path: String,
    pub type_path: String,
    pub reason: ComponentFieldIssueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentFieldIssueKind { Unsupported, AbsentOption }

/// Reports unsupported fields and intentionally omitted absent options.
pub fn component_field_issues(component: &dyn PartialReflect) -> Vec<ComponentFieldIssue> {
    let mut issues = Vec::new();
    walk(component, "", &mut Vec::new(), &mut issues);
    issues
}

fn supported_option(field: &dyn PartialReflect) -> bool {
    field.try_downcast_ref::<Option<f32>>().is_some() || field.try_downcast_ref::<Option<f64>>().is_some()
        || field.try_downcast_ref::<Option<i32>>().is_some() || field.try_downcast_ref::<Option<bool>>().is_some()
        || field.try_downcast_ref::<Option<String>>().is_some() || field.try_downcast_ref::<Option<Vec3>>().is_some()
}

fn walk(field: &dyn PartialReflect, prefix: &str, out: &mut Vec<(String, Value, &'static str)>, issues: &mut Vec<ComponentFieldIssue>) {
    let issue = |reason| ComponentFieldIssue { path: if prefix.is_empty() { "<component>".into() } else { prefix.to_string() }, type_path: field.reflect_type_path().to_string(), reason };
    if field.reflect_type_path().starts_with("core::option::Option") {
        if !supported_option(field) { issues.push(issue(ComponentFieldIssueKind::Unsupported)); return; }
        if value_of(field).is_none() { issues.push(issue(ComponentFieldIssueKind::AbsentOption)); return; }
    }
    if let Some((v, ty)) = value_of(field) {
        if prefix.is_empty() { issues.push(issue(ComponentFieldIssueKind::Unsupported)); return; }
        out.push((prefix.to_string(), v, ty));
        return;
    }
    match field.reflect_ref() {
        ReflectRef::Struct(s) => {
            if s.field_len() == 0 {
                if prefix.is_empty() { out.push((String::new(), Value::Bool(true), "bool")); }
                else { issues.push(issue(ComponentFieldIssueKind::Unsupported)); }
            }
            for i in 0..s.field_len() {
                let Some(name) = s.name_at(i) else { continue };
                let Some(child) = s.field_at(i) else { continue };
                let path = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}:{name}")
                };
                walk(child, &path, out, issues);
            }
        }
        // Tuple fields use identifier-safe numeric segments.
        ReflectRef::TupleStruct(ts) => {
            if ts.field_len() == 0 {
                if prefix.is_empty() { out.push((String::new(), Value::Bool(true), "bool")); }
                else { issues.push(issue(ComponentFieldIssueKind::Unsupported)); }
            }
            for i in 0..ts.field_len() {
                let Some(child) = ts.field(i) else { continue };
                let path = if prefix.is_empty() {
                    format!("_{i}")
                } else {
                    format!("{prefix}:_{i}")
                };
                walk(child, &path, out, issues);
            }
        }
        // Data variants store their tag and active payload fields.
        ReflectRef::Enum(e) => {
            if prefix.is_empty() { issues.push(issue(ComponentFieldIssueKind::Unsupported)); return; }
            out.push((prefix.to_string(), Value::Token(e.variant_name().into()), "token"));
            match e.variant_type() {
                VariantType::Tuple => {
                    for i in 0..e.field_len() {
                        let Some(child) = e.field_at(i) else { continue };
                        let path = if prefix.is_empty() {
                            format!("_{i}")
                        } else {
                            format!("{prefix}:_{i}")
                        };
                        walk(child, &path, out, issues);
                    }
                }
                VariantType::Struct => {
                    for i in 0..e.field_len() {
                        let Some(name) = e.name_at(i) else { continue };
                        let Some(child) = e.field_at(i) else { continue };
                        let path = if prefix.is_empty() {
                            name.to_string()
                        } else {
                            format!("{prefix}:{name}")
                        };
                        walk(child, &path, out, issues);
                    }
                }
                VariantType::Unit => {}
            }
        }
        _ => issues.push(issue(ComponentFieldIssueKind::Unsupported)),
    }
}

/// Author every encodable field of the reflect component `short_type` on
/// `entity` as `bevy:<short_type>:<field>` attributes on `prim_path`, into the
/// stage's current edit target. Returns the authored attribute names.
///
/// This is the inverse of the reflect route. Pair it with the stage's edit
/// target (session layer for a scratch override, a stronger layer to persist)
/// and, in a live session, the echo guard so the re-projection is swallowed.
pub fn author_component(
    world: &World,
    registry: &TypeRegistry,
    stage: &Stage,
    entity: Entity,
    prim_path: &str,
    short_type: &str,
) -> Result<Vec<String>> {
    let registration = crate::route::reflect::resolve(registry, short_type)
        .ok_or_else(|| anyhow::anyhow!("type `{short_type}` is not registered"))?;
    let reflect_component = registration
        .data::<ReflectComponent>()
        .ok_or_else(|| anyhow::anyhow!("type `{short_type}` has no ReflectComponent"))?;
    let entity_ref = world
        .get_entity(entity)
        .map_err(|_| anyhow::anyhow!("entity {entity:?} does not exist"))?;
    let component = reflect_component
        .reflect(entity_ref)
        .ok_or_else(|| anyhow::anyhow!("entity {entity:?} has no `{short_type}` component"))?;

    let name = component_type_segment(registry, registration)?;
    write_component_fields(stage, prim_path, &name, component.as_partial_reflect(), false)
}

/// Authors encodable fields from a registered Rust component value without spawning an entity.
/// All fields commit atomically into the current USD edit target. Unsupported
/// fields reject the whole patch before authoring. Supported absent options
/// remain unauthored; this does not replace existing component opinions.
pub fn author_component_value<T: Component + Reflect + TypePath>(
    registry: &TypeRegistry,
    stage: &Stage,
    prim_path: &str,
    component: &T,
) -> Result<Vec<String>> {
    let registration = registry.get(std::any::TypeId::of::<T>())
        .ok_or_else(|| anyhow::anyhow!("type `{}` is not registered", T::type_path()))?;
    anyhow::ensure!(registration.data::<ReflectComponent>().is_some(), "type `{}` has no ReflectComponent", T::type_path());
    let name = component_type_segment(registry, registration)?;
    write_component_fields(stage, prim_path, &name, component.as_partial_reflect(), true)
}

fn component_type_segment(registry: &TypeRegistry, registration: &bevy::reflect::TypeRegistration) -> Result<String> {
    let names = registration.type_info().type_path_table();
    let short = names.short_path();
    if crate::route::reflect::resolve(registry, short).is_some_and(|candidate| candidate.type_id() == registration.type_id()) {
        return Ok(short.to_string());
    }
    let qualified = names.path().replace("::", "__");
    anyhow::ensure!(crate::route::reflect::resolve(registry, &qualified).is_some_and(|candidate| candidate.type_id() == registration.type_id()),
        "component type `{}` has no unambiguous USD type segment", names.path());
    openusd::sdf::path("/Type")?.append_property(&format!("bevy:{qualified}:field"))?;
    Ok(qualified)
}

/// Authors explicit component presence in the current edit target without an ECS entity.
/// False suppresses USD-owned projection without clearing the component's field opinions.
pub fn author_component_presence<T: Component + Reflect + TypePath>(
    registry: &TypeRegistry,
    stage: &Stage,
    prim_path: &str,
    present: bool,
) -> Result<Vec<String>> {
    let registration = registry.get(std::any::TypeId::of::<T>())
        .ok_or_else(|| anyhow::anyhow!("type `{}` is not registered", T::type_path()))?;
    anyhow::ensure!(registration.data::<ReflectComponent>().is_some(), "type `{}` has no ReflectComponent", T::type_path());
    let name = component_type_segment(registry, registration)?;
    write_component_opinions(stage, prim_path, &name, vec![(String::new(), Value::Bool(present), "bool")])
}

fn write_component_fields(stage: &Stage, prim_path: &str, short_type: &str, component: &dyn PartialReflect, strict: bool) -> Result<Vec<String>> {
    write_component_opinions(stage, prim_path, short_type, component_fields(component, strict)?)
}

fn component_fields(component: &dyn PartialReflect, strict: bool) -> Result<Vec<(String, Value, &'static str)>> {
    let mut leaves = Vec::new();
    let mut issues = Vec::new();
    walk(component, "", &mut leaves, &mut issues);
    if strict {
        let unsupported: Vec<_> = issues.iter().filter(|issue| issue.reason == ComponentFieldIssueKind::Unsupported)
            .map(|issue| format!("{} ({})", issue.path, issue.type_path)).collect();
        anyhow::ensure!(unsupported.is_empty(), "unsupported component fields: {}", unsupported.join(", "));
    }
    Ok(leaves)
}

pub(crate) fn component_value_overrides<T: Component + Reflect + TypePath>(
    registry: &TypeRegistry, prim_path: &str, component: &T,
) -> Result<Vec<crate::instance::UsdAttributeOverride>> {
    let prim = openusd::sdf::path(prim_path)?;
    anyhow::ensure!(prim.is_prim_path() && prim.as_str() != "/", "component owner must be a non-root prim path");
    let registration = registry.get(std::any::TypeId::of::<T>())
        .ok_or_else(|| anyhow::anyhow!("type `{}` is not registered", T::type_path()))?;
    anyhow::ensure!(registration.data::<ReflectComponent>().is_some(), "type `{}` has no ReflectComponent", T::type_path());
    let segment = component_type_segment(registry, registration)?;
    component_fields(component.as_partial_reflect(), true)?.into_iter().map(|(field, value, type_name)| {
        let name = if field.is_empty() { format!("bevy:{segment}") } else { format!("bevy:{segment}:{field}") };
        prim.append_property(&name)?;
        Ok(crate::instance::UsdAttributeOverride { prim: prim_path.to_string(), name, type_name: type_name.to_string(), value })
    }).collect()
}

fn write_component_opinions(stage: &Stage, prim_path: &str, short_type: &str, leaves: Vec<(String, Value, &'static str)>) -> Result<Vec<String>> {
    let prim = openusd::sdf::path(prim_path)?;
    anyhow::ensure!(prim.is_prim_path() && prim.as_str() != "/" && stage.prim(&prim)?.is_valid()?, "component owner must be an existing prim");
    let owner = stage.prim(&prim)?;
    anyhow::ensure!(!owner.is_instance_proxy()? && !owner.is_in_prototype()?, "component owner is a read-only instance proxy or prototype");
    let target = stage.edit_target();
    let mut fields = Vec::with_capacity(leaves.len());
    for (path, value, type_name) in leaves {
        let name = if path.is_empty() { format!("bevy:{short_type}") } else { format!("bevy:{short_type}:{path}") };
        let scene_path = prim.append_property(&name)?;
        let spec_path = target.map_to_spec_path(&scene_path).ok_or_else(|| anyhow::anyhow!("component property is outside the edit target: {scene_path}"))?;
        fields.push((name, spec_path, value, type_name));
    }
    if fields.is_empty() { return Ok(Vec::new()); }
    stage.batch_edit(&[target.layer_identifier()], |layers| {
        let layer = &mut layers[0];
        for (_, path, value, type_name) in &fields {
            if let Some(mut attr) = layer.attribute_mut(path)? { attr.set_default(value.clone())?; }
            else {
                openusd::sdf::AttributeSpec::new(layer.data_mut(), path, *type_name, openusd::sdf::Variability::Varying, true)?
                    .set_default(value.clone())?;
            }
        }
        Ok(())
    })?;
    Ok(fields.into_iter().map(|(name, _, _, _)| name).collect())
}

/// Convenience for reading the `AppTypeRegistry` out of the world and authoring
/// `short_type` on `entity` in one call. The registry read-lock is held only
/// for the duration of the author.
pub fn author_component_from_world(
    world: &World,
    stage: &Stage,
    entity: Entity,
    prim_path: &str,
    short_type: &str,
) -> Result<Vec<String>> {
    let app_registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| anyhow::anyhow!("no AppTypeRegistry in world"))?
        .clone();
    let registry = app_registry.read();
    author_component(world, &registry, stage, entity, prim_path, short_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Health {
        current: f64,
        max: f64,
    }

    #[derive(Reflect, Default)]
    struct UnsupportedPayload { bytes: Vec<u8>, optional: Option<u64> }

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct DiagnosticComponent { value: f32, nested: UnsupportedPayload, absent: Option<i32> }

    #[test]
    fn typed_field_diagnostics_reject_unsupported_payloads_before_writing() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("issues.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<DiagnosticComponent>();
        for optional in [None, Some(9)] {
            let value = DiagnosticComponent { value: 7.0, nested: UnsupportedPayload { bytes: vec![1,2], optional }, absent: None };
            let issues = component_field_issues(&value);
            assert_eq!(issues.iter().map(|issue| (issue.path.as_str(), issue.reason)).collect::<Vec<_>>(), vec![
                ("nested:bytes", ComponentFieldIssueKind::Unsupported),
                ("nested:optional", ComponentFieldIssueKind::Unsupported),
                ("absent", ComponentFieldIssueKind::AbsentOption),
            ]);
            assert!(issues.iter().all(|issue| !issue.type_path.is_empty()));
            let error = author_component_value(&registry, &stage, "/Prim", &value).unwrap_err().to_string();
            assert!(error.contains("nested:bytes") && error.contains("nested:optional"));
            assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        }
        let mut world = World::new();
        let entity = world.spawn(DiagnosticComponent::default()).id();
        let authored = author_component(&world, &registry, &stage, entity, "/Prim", "DiagnosticComponent").unwrap();
        assert_eq!(authored, vec!["bevy:DiagnosticComponent:value"]);
        assert!(!stage.root_layer().export_to_string().unwrap().contains("nested:optional"));
    }

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct OptionalPatch { value: Option<f64> }

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct EmptyMarker;

    #[test]
    fn marker_component_presence_roundtrips_and_live_false_removes_it() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("marker.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let mut world = world_with(|registry| registry.register::<EmptyMarker>());
        let registry = world.resource::<AppTypeRegistry>().clone();
        assert!(component_field_issues(&EmptyMarker).is_empty());
        assert_eq!(author_component_value(&registry.read(), &stage, "/Prim", &EmptyMarker).unwrap(), vec!["bevy:EmptyMarker"]);
        let stage = crate::UsdSource::new("marker-reopened.usda", stage.root_layer().export_to_string().unwrap().into_bytes())
            .unwrap().open_stage().unwrap();
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        assert!(world.get::<EmptyMarker>(entity).is_some());
        crate::authoring::set_attribute(&live.stage, "/Prim", "bevy:EmptyMarker", "bool", Value::Bool(false)).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<EmptyMarker>(entity).is_none());
        author_component_value(&registry.read(), &live.stage, "/Prim", &EmptyMarker).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<EmptyMarker>(entity).is_some());
        crate::authoring::clear_attribute(&live.stage, "/Prim", "bevy:EmptyMarker").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<EmptyMarker>(entity).is_none());
    }

    #[test]
    fn absent_supported_options_are_reported_and_preserve_existing_opinions() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("optional-patch.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<OptionalPatch>();
        author_component_value(&registry, &stage, "/Prim", &OptionalPatch { value: Some(7.0) }).unwrap();
        let absent = OptionalPatch::default();
        assert_eq!(component_field_issues(&absent)[0].reason, ComponentFieldIssueKind::AbsentOption);
        assert!(author_component_value(&registry, &stage, "/Prim", &absent).unwrap().is_empty());
        assert_eq!(stage.prim("/Prim").unwrap().attribute("bevy:OptionalPatch:value").get::<f64>().unwrap(), Some(7.0));
    }

    #[test]
    fn explicit_presence_disables_authored_fields_but_preserves_runtime_only_components() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("presence.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let mut world = world_with(|registry| { registry.register::<Health>(); registry.register::<EmptyMarker>(); });
        let registry = world.resource::<AppTypeRegistry>().clone();
        let value = Health { current: 7.0, max: 12.0 };
        author_component_value(&registry.read(), &stage, "/Prim", &value).unwrap();
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        world.entity_mut(entity).insert(EmptyMarker);
        for ty in ["Health", "EmptyMarker"] {
            crate::authoring::set_attribute(&live.stage, "/Prim", &format!("bevy:{ty}"), "bool", Value::Bool(false)).unwrap();
        }
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<Health>(entity).is_none());
        assert!(world.get::<EmptyMarker>(entity).is_some());
        crate::authoring::clear_attribute(&live.stage, "/Prim", "bevy:Health").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(entity), Some(&value));
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Placement {
        offset: Vec3,
    }

    #[derive(Component, Reflect, Default, Debug, PartialEq)]
    #[reflect(Component, Default)]
    struct IntegerLimits { signed: i64, unsigned: u64, size: usize }

    #[test]
    fn projection_issues_track_invalid_fields_and_clear_after_live_correction() {
        use crate::route::reflect::{ReflectIssueKind, UsdReflectIssues};
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("projection-issues.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        for (name, value) in [("unsigned", -1), ("missing", 7)] {
            crate::authoring::set_attribute(&stage, "/Prim", &format!("bevy:IntegerLimits:{name}"), "int", Value::Int(value)).unwrap();
        }
        let mut world = world_with(|registry| registry.register::<IntegerLimits>());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        let issues = &world.get::<UsdReflectIssues>(entity).unwrap().0;
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|issue| issue.field.as_deref() == Some("unsigned") && issue.kind == ReflectIssueKind::UnsupportedValue));
        assert!(issues.iter().any(|issue| issue.field.as_deref() == Some("missing") && issue.kind == ReflectIssueKind::UnknownField));
        assert_eq!(world.get::<IntegerLimits>(entity).unwrap().unsigned, 0);
        crate::authoring::set_attribute(&live.stage, "/Prim", "bevy:IntegerLimits:unsigned", "int", Value::Int(23)).unwrap();
        crate::authoring::clear_attribute(&live.stage, "/Prim", "bevy:IntegerLimits:missing").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(map.entity("/Prim"), Some(entity));
        assert!(world.get::<UsdReflectIssues>(entity).is_none());
        assert_eq!(world.get::<IntegerLimits>(entity).unwrap().unsigned, 23);
    }

    #[test]
    fn integer_limits_survive_typed_authoring_and_projection() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("integers.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let mut world = world_with(|registry| registry.register::<IntegerLimits>());
        let registry = world.resource::<AppTypeRegistry>().clone();
        let value = IntegerLimits { signed: i64::MIN, unsigned: u64::MAX, size: usize::MAX };
        author_component_value(&registry.read(), &stage, "/Prim", &value).unwrap();
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        assert_eq!(world.get::<IntegerLimits>(map.entity("/Prim").unwrap()), Some(&value));
    }

    fn world_with(register: impl FnOnce(&mut TypeRegistry)) -> World {
        let mut world = World::new();
        let type_registry = AppTypeRegistry::default();
        register(&mut type_registry.write());
        world.insert_resource(type_registry);
        world.insert_resource(SchemaRegistry::builtin());
        world
    }

    #[test]
    fn typed_component_values_roundtrip_without_source_entities() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("typed.usda").unwrap();
        stage.define_prim("/Enemy").unwrap().set_type_name("Xform").unwrap();
        let mut world = world_with(|registry| { registry.register::<Health>(); });
        let value = Health { current: 7.0, max: 12.0 };
        let registry = world.resource::<AppTypeRegistry>().clone();
        let entities_before = world.entities().len();
        let names = author_component_value(&registry.read(), &stage, "/Enemy", &value).unwrap();
        assert_eq!(names, vec!["bevy:Health:current", "bevy:Health:max"]);
        assert_eq!(world.entities().len(), entities_before);
        let live = crate::live::LiveStage::new(stage);
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(map.entity("/Enemy").unwrap()), Some(&value));
    }

    #[test]
    fn component_aliases_merge_fields_and_reconcile_clears_by_type() {
        use crate::route::reflect::{ReflectIssueKind, UsdReflectIssues};
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("aliases.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let qualified = Health::type_path().replace("::", "__");
        let max = format!("bevy:{qualified}:max");
        let current = format!("bevy:{qualified}:current");
        crate::authoring::set_attribute(&stage, "/Prim", "bevy:Health:current", "double", Value::Double(7.0)).unwrap();
        crate::authoring::set_attribute(&stage, "/Prim", &max, "double", Value::Double(12.0)).unwrap();
        stage.define_prim("/Conflict").unwrap().set_type_name("Xform").unwrap();
        crate::authoring::set_attribute(&stage, "/Conflict", "bevy:Health:current", "double", Value::Double(7.0)).unwrap();
        crate::authoring::set_attribute(&stage, "/Conflict", &current, "double", Value::Double(23.0)).unwrap();
        let mut world = world_with(|registry| registry.register::<Health>());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 7.0, max: 12.0 }));
        let conflicting = map.entity("/Conflict").unwrap();
        assert!(world.get::<Health>(conflicting).is_none());
        assert_eq!(world.get::<UsdReflectIssues>(conflicting).unwrap().0[0].kind, ReflectIssueKind::ConflictingAliases);
        crate::authoring::set_attribute(&live.stage, "/Prim", &current, "double", Value::Double(23.0)).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 7.0, max: 12.0 }));
        assert_eq!(world.get::<UsdReflectIssues>(entity).unwrap().0[0].kind, ReflectIssueKind::ConflictingAliases);
        crate::authoring::clear_attribute(&live.stage, "/Prim", "bevy:Health:current").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 23.0, max: 12.0 }));
        assert!(world.get::<UsdReflectIssues>(entity).is_none());
        crate::authoring::clear_attribute(&live.stage, "/Prim", &current).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 0.0, max: 12.0 }));
        crate::authoring::clear_attribute(&live.stage, "/Prim", &max).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<Health>(entity).is_none());
    }

    mod first {
        use super::*;
        #[derive(Component, Reflect, Default, Debug, PartialEq)]
        #[reflect(Component, Default)]
        pub struct Shared { pub value: f64 }
    }

    mod second {
        use super::*;
        #[derive(Component, Reflect, Default, Debug, PartialEq)]
        #[reflect(Component, Default)]
        pub struct Shared { pub value: f64 }
    }

    mod unreversible {
        use super::*;
        #[derive(Component, Reflect, Default)]
        #[reflect(Component, Default)]
        #[type_path = "ambiguous__module"]
        pub struct Shared { pub value: f64 }
    }

    #[test]
    fn lossy_qualified_component_names_reject_before_authoring() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("lossy.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<first::Shared>();
        registry.register::<unreversible::Shared>();
        let error = author_component_value(&registry, &stage, "/Prim", &unreversible::Shared { value: 9.0 }).unwrap_err();
        assert!(error.to_string().contains("no unambiguous USD type segment"));
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn colliding_component_names_author_and_project_independently() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("qualified.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        let mut world = world_with(|registry| {
            registry.register::<first::Shared>();
            registry.register::<second::Shared>();
        });
        let registry = world.resource::<AppTypeRegistry>().clone();
        let first_name = first::Shared::type_path().replace("::", "__");
        let second_name = second::Shared::type_path().replace("::", "__");
        assert_ne!(first_name, second_name);
        assert_eq!(author_component_value(&registry.read(), &stage, "/Prim", &first::Shared { value: 7.0 }).unwrap(),
            vec![format!("bevy:{first_name}:value")]);
        assert_eq!(author_component_value(&registry.read(), &stage, "/Prim", &second::Shared { value: 12.0 }).unwrap(),
            vec![format!("bevy:{second_name}:value")]);
        let source = world.spawn((first::Shared { value: 23.0 }, second::Shared { value: 42.0 })).id();
        let before = stage.root_layer().export_to_string().unwrap();
        assert!(author_component(&world, &registry.read(), &stage, source, "/Prim", "Shared").is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        author_component(&world, &registry.read(), &stage, source, "/Prim", first::Shared::type_path()).unwrap();
        author_component(&world, &registry.read(), &stage, source, "/Prim", &second_name).unwrap();
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let projected = map.entity("/Prim").unwrap();
        assert_eq!(world.get::<first::Shared>(projected), Some(&first::Shared { value: 23.0 }));
        assert_eq!(world.get::<second::Shared>(projected), Some(&second::Shared { value: 42.0 }));
    }

    #[test]
    fn component_write_rolls_back_all_fields_on_type_conflict() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("atomic.usda").unwrap();
        stage.define_prim("/Enemy").unwrap().set_type_name("Xform").unwrap();
        crate::authoring::set_attribute(&stage, "/Enemy", "bevy:Health:current", "double", Value::Double(1.0)).unwrap();
        crate::authoring::set_attribute(&stage, "/Enemy", "bevy:Health:max", "string", Value::String("conflict".into())).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        assert!(author_component_value(&registry, &stage, "/Enemy", &Health { current: 7.0, max: 12.0 }).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn typed_component_values_reject_read_only_instance_proxies() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Xform "Model" { def Xform "Child" {} }
def Xform "Instance" (instanceable = true references = </Model>) {}
"#).open_stage().unwrap();
        assert!(stage.prim("/Instance/Child").unwrap().is_instance_proxy().unwrap());
        let before = stage.root_layer().export_to_string().unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        assert!(author_component_value(&registry, &stage, "/Instance/Child", &Health::default()).is_err());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn typed_component_values_respect_non_root_edit_targets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("weak.usda"), "#usda 1.0\ndef Xform \"Enemy\" {}\n").unwrap();
        let source = crate::UsdSource::new(dir.path().join("root.usda"), b"#usda 1.0\n(subLayers = [@weak.usda@])\n".as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let layer = stage.layer_identifiers().into_iter().find(|path| path.ends_with("weak.usda")).unwrap();
        stage.set_edit_target(openusd::usd::EditTarget::for_layer(layer.clone())).unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        author_component_value(&registry, &stage, "/Enemy", &Health { current: 7.0, max: 12.0 }).unwrap();
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert!(stage.layer(&layer).unwrap().export_to_string().unwrap().contains("bevy:Health:current"));
        assert_eq!(stage.prim("/Enemy").unwrap().attribute("bevy:Health:current").get::<f64>().unwrap(), Some(7.0));
    }

    #[test]
    fn typed_presence_overrides_weaker_fields_without_rewriting_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("weak.usda"), "#usda 1.0\ndef Xform \"Enemy\" { custom double bevy:Health:current = 7\n custom double bevy:Health:max = 12\n}\n").unwrap();
        std::fs::write(dir.path().join("override.usda"), "#usda 1.0\n").unwrap();
        let source = crate::UsdSource::new(dir.path().join("root.usda"), b"#usda 1.0\n(subLayers = [@override.usda@, @weak.usda@])\n".as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let root_before = stage.root_layer().export_to_string().unwrap();
        let weak = stage.layer_identifiers().into_iter().find(|path| path.ends_with("weak.usda")).unwrap();
        let weak_before = stage.layer(&weak).unwrap().export_to_string().unwrap();
        let target = stage.layer_identifiers().into_iter().find(|path| path.ends_with("override.usda")).unwrap();
        stage.set_edit_target(openusd::usd::EditTarget::for_layer(target.clone())).unwrap();
        let mut world = world_with(|registry| registry.register::<Health>());
        let registry = world.resource::<AppTypeRegistry>().clone();
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Enemy").unwrap();
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 7.0, max: 12.0 }));
        let names = author_component_presence::<Health>(&registry.read(), &live.stage, "/Enemy", false).unwrap();
        assert_eq!(names, vec!["bevy:Health"]);
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<Health>(entity).is_none());
        assert!(live.stage.layer(&target).unwrap().export_to_string().unwrap().contains("bevy:Health"));
        assert_eq!(live.stage.root_layer().export_to_string().unwrap(), root_before);
        assert_eq!(live.stage.layer(&weak).unwrap().export_to_string().unwrap(), weak_before);
        crate::authoring::clear_attribute(&live.stage, "/Enemy", &names[0]).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Health>(entity), Some(&Health { current: 7.0, max: 12.0 }));
        for invalid in ["/", "/Missing"] {
            assert!(author_component_presence::<Health>(&registry.read(), &live.stage, invalid, true).is_err());
        }
    }

    /// A component authored back into the stage re-reads (via the reflect
    /// route) into an identical component — a full ECS → USD → ECS round-trip,
    /// the thing BSN cannot do. Also proves the opinions land in an exportable
    /// layer.
    #[test]
    fn author_component_roundtrips_through_usd() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("author.usda").unwrap();
        stage
            .define_prim("/Enemy")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();

        // Source world holds the component; author it onto the stage.
        let mut src = world_with(|r| {
            r.register::<Health>();
            r.register::<Placement>();
        });
        let e = src
            .spawn((
                crate::UsdPrimRef::new("/Enemy"),
                Health {
                    current: 30.0,
                    max: 120.0,
                },
                Placement {
                    offset: Vec3::new(1.0, 2.0, 3.0),
                },
            ))
            .id();
        author_component_from_world(&src, &stage, e, "/Enemy", "Health").unwrap();
        author_component_from_world(&src, &stage, e, "/Enemy", "Placement").unwrap();

        // The opinions are really in the layer.
        let text = crate::authoring::export_stage_string(&stage).unwrap();
        assert!(
            text.contains("bevy:Health:max"),
            "authored attr present in exported layer:\n{text}"
        );

        // Re-project the stage into a fresh world → identical components.
        let live = LiveStage::new(stage);
        let mut dst = world_with(|r| {
            r.register::<Health>();
            r.register::<Placement>();
        });
        let mut map = PrimEntities::default();
        project_stage(&mut dst, &live, &mut map);
        let re = map.entity("/Enemy").unwrap();
        assert_eq!(
            dst.get::<Health>(re),
            Some(&Health {
                current: 30.0,
                max: 120.0
            }),
            "Health round-trips ECS → USD → ECS"
        );
        assert_eq!(
            dst.get::<Placement>(re),
            Some(&Placement {
                offset: Vec3::new(1.0, 2.0, 3.0)
            }),
            "nested Vec3 field round-trips as float3"
        );
    }

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    enum Mode {
        #[default]
        Idle,
        Run,
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Mixed {
        flag: bool,
        count: i32,
        label: String,
        mode: Mode,
        spin: Quat,
    }

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct Inner {
        a: f32,
        b: f32,
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Outer {
        inner: Inner,
        tag: i32,
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Tint {
        color: Color,
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Opts {
        maybe_hp: Option<f32>,
        maybe_name: Option<String>,
        count: u64,
        idx: usize,
    }

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    enum Motion {
        #[default]
        Idle,
        Moving(f32),
        Warp {
            x: i32,
            y: i32,
        },
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Mover {
        motion: Motion,
    }

    /// `Option<T>` (Some), `u64`, and `usize` fields round-trip; a `None` option
    /// authors nothing and re-reads as the default.
    #[test]
    fn option_and_wide_int_roundtrip() {
        let value = Opts {
            maybe_hp: Some(12.5),
            maybe_name: Some("boss".into()),
            count: 9_000_000_000,
            idx: 7,
        };
        let back = roundtrip(|r| r.register::<Opts>(), value.clone());
        assert_eq!(back, value, "Some options + u64/usize round-trip");

        // A None option authors nothing → re-reads as the default (None).
        let none = Opts {
            maybe_hp: None,
            maybe_name: None,
            count: 1,
            idx: 2,
        };
        let back = roundtrip(|r| r.register::<Opts>(), none.clone());
        assert_eq!(back, none, "None options stay None through the round-trip");
    }

    #[test]
    fn data_enum_roundtrip() {
        // Unit variant.
        let idle = Mover { motion: Motion::Idle };
        assert_eq!(
            roundtrip(|r| r.register::<Mover>(), idle.clone()),
            idle,
            "unit variant round-trips as a bare token"
        );
        // Tuple variant carries its payload.
        let moving = Mover {
            motion: Motion::Moving(4.5),
        };
        assert_eq!(
            roundtrip(|r| r.register::<Mover>(), moving.clone()),
            moving,
            "tuple-variant payload round-trips"
        );
        // Struct variant carries its named fields.
        let warp = Mover {
            motion: Motion::Warp { x: 3, y: 7 },
        };
        assert_eq!(
            roundtrip(|r| r.register::<Mover>(), warp.clone()),
            warp,
            "struct-variant payload round-trips"
        );
    }

    /// A `Color` field round-trips as linear `color4f` (alpha preserved).
    #[test]
    fn color_roundtrip() {
        let value = Tint {
            color: Color::linear_rgba(0.1, 0.2, 0.3, 0.4),
        };
        let back = roundtrip(|r| r.register::<Tint>(), value.clone());
        assert_eq!(back, value, "Color survives ECS → USD → ECS in linear space");
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Level(u32);

    /// A newtype / tuple-struct component authors its field by index (`:0`).
    #[test]
    fn newtype_roundtrip() {
        let back = roundtrip(|r| r.register::<Level>(), Level(42));
        assert_eq!(back, Level(42), "tuple-struct field `.0` round-trips");
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Path {
        waypoints: Vec<Vec3>,
        weights: Vec<f32>,
        labels: Vec<String>,
    }

    /// Array fields round-trip through USD array values.
    #[test]
    fn array_fields_roundtrip() {
        let value = Path {
            waypoints: vec![Vec3::new(0.0, 1.0, 2.0), Vec3::new(3.0, 4.0, 5.0)],
            weights: vec![0.5, 1.5, 2.5],
            labels: vec!["a".into(), "b".into()],
        };
        let back = roundtrip(|r| r.register::<Path>(), value.clone());
        assert_eq!(back, value, "Vec<Vec3>/Vec<f32>/Vec<String> round-trip");
    }

    fn roundtrip<C: Component + PartialReflect + Clone + PartialEq + std::fmt::Debug>(
        register: impl Fn(&mut TypeRegistry) + Copy,
        value: C,
    ) -> C {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("rt.usda").unwrap();
        stage.define_prim("/P").unwrap();
        let short = std::any::type_name::<C>().rsplit("::").next().unwrap();

        let mut src = world_with(register);
        let e = src.spawn((crate::UsdPrimRef::new("/P"), value)).id();
        author_component_from_world(&src, &stage, e, "/P", short).unwrap();

        let live = LiveStage::new(stage);
        let mut dst = world_with(register);
        let mut map = PrimEntities::default();
        project_stage(&mut dst, &live, &mut map);
        let re = map.entity("/P").unwrap();
        dst.get::<C>(re).cloned().expect("component re-projected")
    }

    /// bool / int / String / unit-enum / quaternion all survive ECS → USD →
    /// ECS. The quaternion case guards the gf real-first ordering bug.
    #[test]
    fn scalar_enum_quat_roundtrip() {
        let value = Mixed {
            flag: true,
            count: -7,
            label: "boss".into(),
            mode: Mode::Run,
            spin: Quat::from_xyzw(0.1, 0.2, 0.3, 0.7),
        };
        let back = roundtrip(
            |r| {
                r.register::<Mixed>();
                r.register::<Mode>();
            },
            value.clone(),
        );
        assert_eq!(back, value, "all field kinds round-trip, quat unscrambled");
    }

    /// Nested struct fields round-trip through `:`-namespaced attribute paths in
    /// both directions.
    #[test]
    fn nested_struct_roundtrip() {
        let value = Outer {
            inner: Inner { a: 1.5, b: -2.5 },
            tag: 9,
        };
        let back = roundtrip(
            |r| {
                r.register::<Outer>();
                r.register::<Inner>();
            },
            value.clone(),
        );
        assert_eq!(back, value, "nested `inner:a`/`inner:b` round-trip");
    }

    #[test]
    fn author_errors_are_graceful() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("err.usda").unwrap();
        stage.define_prim("/P").unwrap();
        let mut world = world_with(|r| r.register::<Health>());
        let e = world.spawn(crate::UsdPrimRef::new("/P")).id();

        // Unregistered type.
        assert!(
            author_component_from_world(&world, &stage, e, "/P", "Nope").is_err(),
            "unregistered type errors, not panics"
        );
        // Registered but entity lacks the component.
        assert!(
            author_component_from_world(&world, &stage, e, "/P", "Health").is_err(),
            "missing component errors"
        );
    }

    /// The reflect route must not panic when a stage carries `bevy:` opinions
    /// but the world has no `AppTypeRegistry` (a bare headless world).
    #[test]
    fn no_type_registry_is_graceful() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("noreg.usda").unwrap();
        stage.define_prim("/P").unwrap();
        stage
            .create_attribute("/P.bevy:Health:max", "double")
            .unwrap()
            .set(Value::Double(1.0))
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new(); // no AppTypeRegistry, no SchemaRegistry
        let mut map = PrimEntities::default();
        // Should log a warning and carry on, not panic.
        project_stage(&mut world, &live, &mut map);
        assert!(map.entity("/P").is_some(), "prim still projected");
    }
}
