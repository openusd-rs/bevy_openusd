//! Reflect route (PLAN P1.3) — the BSN-parity core.
//!
//! Makes *any* `#[derive(Component, Reflect)] #[reflect(Component)]` type
//! authorable from USD with **no per-type Rust code**, using Bevy's type
//! registry. This is what turns `usd_bevy` from an importer into a scene
//! system: arbitrary gameplay components, authored in `.usda`, with USD
//! composition doing the field merging.
//!
//! ## Authoring convention
//!
//! A component field is authored as a custom attribute named
//! `bevy:<ShortType>:<field-path>`, where `<field-path>` uses `:` to descend
//! into nested struct fields:
//!
//! ```usda
//! def Xform "Enemy"
//! {
//!     custom double bevy:Health:current = 50
//!     custom double bevy:Health:max     = 100
//!     custom bool   bevy:Boss:enraged   = true
//! }
//! ```
//!
//! ## Sparse semantics (= BSN `TemplatePatch`, composed by USD)
//!
//! * **project**: for every `bevy:` attr on the prim, get-or-default the
//!   component and set exactly the authored fields.
//! * **patch**: set exactly the fields whose attrs changed. A USD `over` on one
//!   attribute patches one field, leaving the rest of the component intact —
//!   and because USD already composed the opinion stack, the value that lands
//!   is the LIVERPS-resolved one. When an opinion is *cleared* the field
//!   reverts to the type's `ReflectDefault`; clearing the last `bevy:` attr of
//!   a type removes the component.

use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant, VariantInfo};
use bevy::reflect::structs::DynamicStruct;
use bevy::reflect::tuple::DynamicTuple;
use bevy::reflect::{
    GetPath, PartialReflect, ReflectRef, TypeInfo, std_traits::ReflectDefault,
};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// The attribute namespace that marks a reflect-routed component field.
const NS: &str = "bevy:";

/// `bevy:` attributes grouped by short type path, each field paired with its
/// composed value (`None` = no effective opinion / blocked).
type Groups = Vec<(String, Vec<(String, Option<Value>)>)>;

/// Maps `bevy:`-namespaced attributes onto arbitrary registered components.
pub struct ReflectRoute;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReflectIssueKind {
    MissingRegistry,
    UnregisteredType,
    MissingComponentReflection,
    MissingDefault,
    UnknownField,
    UnsupportedValue,
    ConflictingAliases,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflectIssue {
    /// Canonical registered Rust path, or the unresolved authored type segment.
    pub type_segment: String,
    pub field: Option<String>,
    pub kind: ReflectIssueKind,
}

/// Current reflected-component projection failures on this prim.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct UsdReflectIssues(pub Vec<ReflectIssue>);

/// Parse a property name `bevy:<Type>:<field>:<sub>…` into the type segment and
/// a `.`-joined reflect field path. Non-`bevy:` names return `None`;
/// bare `bevy:Type` returns an empty field path for component presence.
///
/// The type segment is the first `:`-delimited component; it is normally a
/// short type path (`Health`). To disambiguate a short-name collision, author
/// the *full* path with `__` standing in for `::` (`my_game__Health`) — USD
/// rejects a literal `::` in a property name (empty namespace component), so
/// `__` is the path-legal encoding, mapped back in [`resolve`].
fn parse_attr(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(NS)?;
    let mut segs = rest.split(':');
    let ty = segs.next().filter(|s| !s.is_empty())?.to_string();
    let field: Vec<String> = segs
        .filter(|s| !s.is_empty())
        .map(decode_index)
        .collect();
    if field.is_empty() {
        return Some((ty, String::new()));
    }
    Some((ty, field.join(".")))
}

/// Decode a tuple-index segment: USD identifiers can't start with a digit, so a
/// tuple field `.0` is authored as `_0` and decoded back here (`_0` → `0`).
/// Named fields (including `_foo`) pass through unchanged.
fn decode_index(seg: &str) -> String {
    if let Some(rest) = seg.strip_prefix('_')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        rest.to_string()
    } else {
        seg.to_string()
    }
}

/// Resolve a type segment to its registration: short type path first (the
/// common case), then the full type path, then the full path with `__` decoded
/// to `::` (the path-legal way to author a full path for disambiguation).
pub(crate) fn resolve<'a>(
    registry: &'a bevy::reflect::TypeRegistry,
    ty: &str,
) -> Option<&'a bevy::reflect::TypeRegistration> {
    registry
        .get_with_short_type_path(ty)
        .or_else(|| registry.get_with_type_path(ty))
        .or_else(|| registry.get_with_type_path(&ty.replace("__", "::")))
}

/// Every authored `bevy:` attribute on the prim, grouped by short type path,
/// each as `(reflect_field_path, composed_value_or_none)`. `None` value means
/// no layer currently authors an opinion (a cleared field).
fn collect(ctx: &RouteCtx) -> Groups {
    let prim = ctx.stage.prim(ctx.path.clone()).expect("validated USD path");
    let Ok(names) = prim.property_names() else {
        return Vec::new();
    };
    let tc = ctx.time.map(openusd::usd::TimeCode::new);
    let mut groups: Groups = Vec::new();
    for name in names {
        let name = name.as_str();
        let Some((ty, field)) = parse_attr(name) else {
            continue;
        };
        let value = prim.attribute(name).get_at::<Value>(tc).ok().flatten();
        match groups.iter_mut().find(|(t, _)| *t == ty) {
            Some((_, fields)) => fields.push((field, value)),
            None => groups.push((ty, vec![(field, value)])),
        }
    }
    groups
}

/// Coerce a USD value into a reflected field, matching on the field's concrete
/// Rust type (numeric widths coerce; glam vectors/quat map component-wise).
/// Returns whether the field was set.
fn set_field(field: &mut dyn PartialReflect, v: &Value) -> bool {
    // Scalars ------------------------------------------------------------
    if let Some(f) = field.try_downcast_mut::<f32>()
        && let Some(n) = as_f64(v)
    {
        *f = n as f32;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<f64>()
        && let Some(n) = as_f64(v)
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<i32>()
        && let Some(n) = as_integer(v).and_then(|n| i32::try_from(n).ok())
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<u32>()
        && let Some(n) = as_integer(v).and_then(|n| u32::try_from(n).ok())
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<i64>()
        && let Some(n) = as_integer(v).and_then(|n| i64::try_from(n).ok())
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<u64>()
        && let Some(n) = as_integer(v).and_then(|n| u64::try_from(n).ok())
    {
        *f = n;
        return true;
    }
    if let Some(f) = field.try_downcast_mut::<usize>()
        && let Some(n) = as_integer(v).and_then(|n| usize::try_from(n).ok())
    {
        *f = n;
        return true;
    }
    if let Some(b) = field.try_downcast_mut::<bool>()
        && let Value::Bool(x) = v
    {
        *b = *x;
        return true;
    }
    if let Some(s) = field.try_downcast_mut::<String>() {
        match v {
            Value::String(x) => {
                *s = x.clone();
                return true;
            }
            Value::Token(x) => {
                *s = x.as_str().to_string();
                return true;
            }
            _ => {}
        }
    }
    // Vectors / quaternion ----------------------------------------------
    if let Some(out) = field.try_downcast_mut::<Vec2>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec2::new(a[0], a[1]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec3>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec3::new(a[0], a[1], a[2]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec4>()
        && let Some(a) = as_vec(v)
    {
        *out = Vec4::new(a[0], a[1], a[2], a[3]);
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Quat>()
        && let Some(a) = as_vec(v)
    {
        *out = Quat::from_xyzw(a[0], a[1], a[2], a[3]);
        return true;
    }
    // Color: `color3f`/`float3` → opaque, `color4f`/`float4` → rgba. USD colors
    // are linear, so map straight into `LinearRgba` (no sRGB transfer).
    if let Some(out) = field.try_downcast_mut::<Color>() {
        match v {
            Value::Vec3f(_) | Value::Vec3d(_) => {
                if let Some(a) = as_vec(v) {
                    *out = Color::linear_rgb(a[0], a[1], a[2]);
                    return true;
                }
            }
            Value::Vec4f(_) | Value::Vec4d(_) => {
                if let Some(a) = as_vec(v) {
                    *out = Color::linear_rgba(a[0], a[1], a[2], a[3]);
                    return true;
                }
            }
            _ => {}
        }
    }
    // Array fields ↔ USD array values (element types we already handle).
    if let Some(out) = field.try_downcast_mut::<Vec<f32>>() {
        match v {
            Value::FloatVec(a) => {
                *out = a.clone();
                return true;
            }
            Value::DoubleVec(a) => {
                *out = a.iter().map(|x| *x as f32).collect();
                return true;
            }
            _ => {}
        }
    }
    if let Some(out) = field.try_downcast_mut::<Vec<i32>>()
        && let Value::IntVec(a) = v
    {
        *out = a.clone();
        return true;
    }
    if let Some(out) = field.try_downcast_mut::<Vec<String>>() {
        match v {
            Value::StringVec(a) => {
                *out = a.clone();
                return true;
            }
            Value::TokenVec(a) => {
                *out = a.iter().map(|t| t.as_str().to_string()).collect();
                return true;
            }
            _ => {}
        }
    }
    if let Some(out) = field.try_downcast_mut::<Vec<Vec3>>() {
        match v {
            Value::Vec3fVec(a) => {
                *out = a.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
                return true;
            }
            Value::Vec3dVec(a) => {
                *out = a
                    .iter()
                    .map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32))
                    .collect();
                return true;
            }
            _ => {}
        }
    }
    // Option<T>: a present opinion sets `Some`; a cleared attribute reverts the
    // field to `Option::default()` (`None`) via the route's revert mechanism.
    if let Some(o) = field.try_downcast_mut::<Option<f32>>()
        && let Some(n) = as_f64(v)
    {
        *o = Some(n as f32);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<f64>>()
        && let Some(n) = as_f64(v)
    {
        *o = Some(n);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<i32>>()
        && let Some(n) = as_integer(v).and_then(|n| i32::try_from(n).ok())
    {
        *o = Some(n);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<bool>>()
        && let Value::Bool(b) = v
    {
        *o = Some(*b);
        return true;
    }
    if let Some(o) = field.try_downcast_mut::<Option<String>>() {
        match v {
            Value::String(s) => {
                *o = Some(s.clone());
                return true;
            }
            Value::Token(t) => {
                *o = Some(t.as_str().to_string());
                return true;
            }
            _ => {}
        }
    }
    if let Some(o) = field.try_downcast_mut::<Option<Vec3>>()
        && let Some(a) = as_vec(v)
    {
        *o = Some(Vec3::new(a[0], a[1], a[2]));
        return true;
    }
    // Enums: a token/string names the variant. Unit variants switch directly;
    // tuple/struct (data) variants are constructed with synthesized-default
    // payloads (PLAN 4c) — the payload fields are then filled by the sibling
    // `bevy:T:field:_0` / `bevy:T:field:name` attributes, applied after this.
    if matches!(field.reflect_ref(), ReflectRef::Enum(_)) {
        let name = match v {
            Value::Token(t) => Some(t.as_str().to_string()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        };
        if let Some(name) = name {
            return set_enum_variant(field, &name);
        }
    }
    false
}

/// Switch `field` (an enum) to the named variant. For data variants the payload
/// is initialised to synthesized defaults; the caller's per-field pass then
/// overwrites those from the authored payload attributes. Returns whether the
/// variant switch applied.
fn set_enum_variant(field: &mut dyn PartialReflect, variant: &str) -> bool {
    let Some(TypeInfo::Enum(info)) = field.get_represented_type_info() else {
        // No static type info (a dynamic enum): assume a unit variant.
        let dynamic = DynamicEnum::new(variant.to_string(), DynamicVariant::Unit);
        return field.try_apply(&dynamic).is_ok();
    };
    let Some(vinfo) = info.variant(variant) else {
        return false;
    };
    let dv = match vinfo {
        VariantInfo::Unit(_) => DynamicVariant::Unit,
        VariantInfo::Tuple(t) => {
            let mut dt = DynamicTuple::default();
            for i in 0..t.field_len() {
                let Some(def) = synthesize_default(t.field_at(i).unwrap().type_path()) else {
                    return false;
                };
                dt.insert_boxed(def);
            }
            DynamicVariant::Tuple(dt)
        }
        VariantInfo::Struct(s) => {
            let mut ds = DynamicStruct::default();
            for i in 0..s.field_len() {
                let f = s.field_at(i).unwrap();
                let Some(def) = synthesize_default(f.type_path()) else {
                    return false;
                };
                ds.insert_boxed(f.name(), def);
            }
            DynamicVariant::Struct(ds)
        }
    };
    let dynamic = DynamicEnum::new(variant.to_string(), dv);
    field.try_apply(&dynamic).is_ok()
}

/// A zero/empty default value for a variant payload field, by type path. Covers
/// the scalar/string/glam types the reflect route already coerces USD values
/// into; an unsupported payload type makes the whole variant switch fail (and
/// warn), rather than constructing a half-built variant.
fn synthesize_default(type_path: &str) -> Option<Box<dyn PartialReflect>> {
    let v: Box<dyn PartialReflect> = match type_path {
        "f32" => Box::new(0f32),
        "f64" => Box::new(0f64),
        "i32" => Box::new(0i32),
        "u32" => Box::new(0u32),
        "i64" => Box::new(0i64),
        "u64" => Box::new(0u64),
        "usize" => Box::new(0usize),
        "bool" => Box::new(false),
        "alloc::string::String" => Box::new(String::new()),
        "glam::Vec2" => Box::new(Vec2::ZERO),
        "glam::Vec3" => Box::new(Vec3::ZERO),
        "glam::Vec4" => Box::new(Vec4::ZERO),
        _ => return None,
    };
    Some(v)
}

/// Resolve an `asset`/`string` value into a `Handle<T>` field via the
/// `AssetServer` (PLAN 4a). Covers the common asset types; other `Handle<T>`
/// fields fall through to the normal coercion (which won't match, and warns).
/// Path resolution is currently Bevy-asset-root-relative; layer-relative
/// resolution is the openusd-blocked part.
fn try_set_handle(
    field: &mut dyn PartialReflect,
    v: &Value,
    assets: Option<&AssetServer>,
) -> bool {
    let Some(server) = assets else {
        return false;
    };
    let path = match v {
        Value::AssetPath(a) => a.as_str().to_string(),
        Value::String(s) => s.clone(),
        Value::Token(t) => t.as_str().to_string(),
        _ => return false,
    };
    if let Some(h) = field.try_downcast_mut::<Handle<Image>>() {
        *h = server.load(path);
        return true;
    }
    if let Some(h) = field.try_downcast_mut::<Handle<Mesh>>() {
        *h = server.load(path);
        return true;
    }
    if let Some(h) = field.try_downcast_mut::<Handle<StandardMaterial>>() {
        *h = server.load(path);
        return true;
    }
    false
}

fn as_f64(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Float(x) => *x as f64,
        Value::Double(x) => *x,
        Value::Half(x) => f32::from(*x) as f64,
        Value::Int(x) => *x as f64,
        Value::Int64(x) => *x as f64,
        Value::Uint(x) => *x as f64,
        Value::Uint64(x) => *x as f64,
        Value::Uchar(x) => *x as f64,
        _ => return None,
    })
}

fn as_integer(v: &Value) -> Option<i128> {
    Some(match v {
        Value::Int(x) => i128::from(*x),
        Value::Int64(x) => i128::from(*x),
        Value::Uint(x) => i128::from(*x),
        Value::Uint64(x) => i128::from(*x),
        Value::Uchar(x) => i128::from(*x),
        Value::Bool(x) => i128::from(*x),
        Value::Float(_) | Value::Double(_) | Value::Half(_) => {
            let n = as_f64(v)?;
            if !n.is_finite() || n.fract() != 0.0 || n < i128::MIN as f64 || n >= -(i128::MIN as f64) {
                return None;
            }
            n as i128
        }
        _ => return None,
    })
}

/// A USD vector/quat value as up to 4 `f32` lanes (missing lanes are 0).
fn as_vec(v: &Value) -> Option<[f32; 4]> {
    Some(match v {
        Value::Vec2f(a) => [a.x, a.y, 0.0, 0.0],
        Value::Vec2d(a) => [a.x as f32, a.y as f32, 0.0, 0.0],
        Value::Vec3f(a) => [a.x, a.y, a.z, 0.0],
        Value::Vec3d(a) => [a.x as f32, a.y as f32, a.z as f32, 0.0],
        Value::Vec4f(a) => [a.x, a.y, a.z, a.w],
        Value::Vec4d(a) => [a.x as f32, a.y as f32, a.z as f32, a.w as f32],
        Value::Quatf(q) => [q.x, q.y, q.z, q.w],
        Value::Quatd(q) => [q.x as f32, q.y as f32, q.z as f32, q.w as f32],
        _ => return None,
    })
}


/// Per-entity bookkeeping: the `(short_type, field_paths)` the reflect route
/// has authored onto this entity. A later reconcile uses it to revert exactly
/// the fields (and components) *it* set when their USD opinions vanish — never
/// touching state a gameplay system added.
/// `(short_type, authored_field_paths)` pairs the reflect route has set.
type Record = Vec<(String, Vec<String>)>;

#[derive(Component, Clone, Default)]
struct ReflectAuthored(Record);

impl ReflectRoute {
    /// Reconcile the entity's reflect-routed components against the prim's
    /// currently-authored `bevy:` opinions — the single code path for both
    /// project and patch. It:
    /// * sets each authored field to its composed (LIVERPS-resolved) value,
    /// * reverts fields the route previously set but that are no longer
    ///   authored, back to the type default,
    /// * removes components whose last authored opinion is gone.
    ///
    /// USD has already done the opinion merging; this only projects the result.
    fn reconcile_prim(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let mut now = collect(ctx);
        let mut prev = world
            .get::<ReflectAuthored>(entity)
            .cloned()
            .unwrap_or_default();

        // Cheap exit: a prim with no effective `bevy:` opinion that the route
        // never touched (the overwhelmingly common case on a large stage).
        let nothing_now = now.iter().all(|(_, fs)| fs.iter().all(|(_, v)| v.is_none()));
        if nothing_now && prev.0.is_empty() {
            if let Ok(mut ent) = world.get_entity_mut(entity) { ent.remove::<UsdReflectIssues>(); }
            return;
        }

        // No type registry → nothing the reflect route can do. Warn once (only
        // when there actually *are* `bevy:` opinions to project) and bail
        // instead of panicking on a bare `World`.
        let Some(app_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
            if let Ok(mut ent) = world.get_entity_mut(entity) {
                let issues: Vec<_> = now.iter().filter(|(_, fields)| fields.iter().any(|(_, value)| value.is_some()))
                    .map(|(ty, _)| ReflectIssue { type_segment: ty.clone(), field: None, kind: ReflectIssueKind::MissingRegistry }).collect();
                if issues.is_empty() { ent.remove::<UsdReflectIssues>(); }
                else { ent.insert(UsdReflectIssues(issues)); }
            }
            if !nothing_now {
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "{}: has bevy: opinions but no AppTypeRegistry — add UsdPlugin / register types",
                    ctx.prim_str()
                );
            }
            return;
        };
        let registry = app_registry.read();
        let canonical = |ty: &str| resolve(&registry, ty)
            .map_or_else(|| ty.to_string(), |registration| registration.type_info().type_path().to_string());
        let mut grouped: Groups = Vec::new();
        for (ty, fields) in now {
            let ty = canonical(&ty);
            if let Some((_, existing)) = grouped.iter_mut().find(|(name, _)| *name == ty) {
                existing.extend(fields);
            } else { grouped.push((ty, fields)); }
        }
        now = grouped;
        let mut previous: Record = Vec::new();
        for (ty, fields) in prev.0 {
            let ty = canonical(&ty);
            if let Some((_, existing)) = previous.iter_mut().find(|(name, _)| *name == ty) {
                existing.extend(fields);
            } else { previous.push((ty, fields)); }
        }
        prev.0 = previous;
        // Read the AssetServer (if any) before borrowing the entity, so
        // `Handle<T>` fields can resolve asset-path values at project time.
        let assets = world.get_resource::<AssetServer>().cloned();
        let Ok(mut ent) = world.get_entity_mut(entity) else {
            return;
        };

        let mut new_record: Record = Vec::new();
        let mut issues = Vec::new();

        for (ty, fields) in &now {
            // Fields with an actual authored opinion.
            let mut effective: Vec<(&String, &Value)> = fields
                .iter()
                .filter_map(|(f, v)| v.as_ref().map(|v| (f, v)))
                .collect();
            if effective.is_empty() {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            let conflicts: Vec<_> = effective.iter().filter(|(field, _)| !seen.insert(field.as_str()))
                .map(|(field, _)| (*field).clone()).collect();
            if !conflicts.is_empty() {
                for field in conflicts {
                    issues.push(ReflectIssue { type_segment: ty.clone(), field: Some(field), kind: ReflectIssueKind::ConflictingAliases });
                }
                if let Some(record) = prev.0.iter().find(|(name, _)| name == ty) { new_record.push(record.clone()); }
                continue;
            }
            if let Some((_, presence)) = effective.iter().find(|(field, _)| field.is_empty()) {
                match presence {
                    Value::Bool(false) => continue,
                    Value::Bool(true) => (),
                    _ => {
                        issues.push(ReflectIssue { type_segment: ty.clone(), field: None, kind: ReflectIssueKind::UnsupportedValue });
                        if let Some(record) = prev.0.iter().find(|(name, _)| name == ty) { new_record.push(record.clone()); }
                        continue;
                    }
                }
            }
            // Apply shallower paths first so an enum variant selector (`state`)
            // lands before its payload fields (`state.0`) that descend into it.
            effective.sort_by(|a, b| a.0.cmp(b.0));
            let Some(registration) = resolve(&registry, ty) else {
                issues.push(ReflectIssue { type_segment: ty.clone(), field: None, kind: ReflectIssueKind::UnregisteredType });
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "bevy:{ty}: not in the type registry — skipping (register the type + #[reflect(Component)])"
                );
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                issues.push(ReflectIssue { type_segment: ty.clone(), field: None, kind: ReflectIssueKind::MissingComponentReflection });
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "bevy:{ty}: registered but missing ReflectComponent (add #[reflect(Component)])"
                );
                continue;
            };

            // Ensure the component exists, constructing a default when absent.
            if reflect_component.reflect(&ent).is_none() {
                let Some(default) = registration.data::<ReflectDefault>() else {
                    issues.push(ReflectIssue { type_segment: ty.clone(), field: None, kind: ReflectIssueKind::MissingDefault });
                    bevy::log::warn!(
                        target: "usd_bevy::route::reflect",
                        "bevy:{ty}: no ReflectDefault and not present — cannot construct (add #[reflect(Default)])"
                    );
                    continue;
                };
                let value = default.default();
                reflect_component.insert(&mut ent, value.as_partial_reflect(), &registry);
            }
            let default_instance = registration.data::<ReflectDefault>().map(|d| d.default());

            // Set each authored field.
            let mut field_names: Vec<String> = Vec::new();
            for (field_path, v) in &effective {
                if field_path.is_empty() { field_names.push(String::new()); continue; }
                let path = format!(".{field_path}");
                if let Some(mut comp) = reflect_component.reflect_mut(&mut ent) {
                    match comp.reflect_path_mut(path.as_str()) {
                        Ok(target) => {
                            if !try_set_handle(target, v, assets.as_ref()) && !set_field(target, v)
                            {
                                issues.push(ReflectIssue { type_segment: ty.clone(), field: Some((*field_path).clone()), kind: ReflectIssueKind::UnsupportedValue });
                                bevy::log::warn!(
                                    target: "usd_bevy::route::reflect",
                                    "bevy:{ty}:{field_path}: unsupported value/type pairing — skipped"
                                );
                            }
                        }
                        Err(e) => {
                            issues.push(ReflectIssue { type_segment: ty.clone(), field: Some((*field_path).clone()), kind: ReflectIssueKind::UnknownField });
                            bevy::log::warn!(
                            target: "usd_bevy::route::reflect",
                            "bevy:{ty}:{field_path}: no such field ({e})"
                            );
                        }
                    }
                }
                field_names.push((*field_path).clone());
            }

            // Revert fields the route set previously but that are no longer
            // authored, back to the type default.
            if let Some((_, prev_fields)) = prev.0.iter().find(|(t, _)| t == ty) {
                for pf in prev_fields {
                    if field_names.iter().any(|f| f == pf) {
                        continue;
                    }
                    let path = format!(".{pf}");
                    let def_val = default_instance
                        .as_ref()
                        .and_then(|d| d.reflect_path(path.as_str()).ok())
                        .map(|f| f.to_dynamic());
                    if let Some(def_val) = def_val
                        && let Some(mut comp) = reflect_component.reflect_mut(&mut ent)
                        && let Ok(target) = comp.reflect_path_mut(path.as_str())
                    {
                        let _ = target.try_apply(&*def_val);
                    }
                }
            }

            new_record.push((ty.clone(), field_names));
        }

        // Remove components the route authored before but whose type now has no
        // effective opinion (the "cleared last field → remove component" rule).
        for (ty, _) in &prev.0 {
            if new_record.iter().any(|(t, _)| t == ty) {
                continue;
            }
            if let Some(reflect_component) =
                resolve(&registry, ty).and_then(|r| r.data::<ReflectComponent>())
            {
                reflect_component.remove(&mut ent);
            }
        }

        ent.insert(ReflectAuthored(new_record));
        if issues.is_empty() { ent.remove::<UsdReflectIssues>(); }
        else { ent.insert(UsdReflectIssues(issues)); }
    }
}

impl PrimRoute for ReflectRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        // Runs on every prim. The route must also see a prim whose *last*
        // `bevy:` opinion was just cleared — so it can remove the component it
        // authored. `reconcile_prim` early-outs when there is nothing to do.
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        self.reconcile_prim(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, _changed: &[&str]) {
        self.reconcile_prim(ctx, world, entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openusd::tf::Token;

    #[test]
    fn parse_attr_forms() {
        let s = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
        assert_eq!(parse_attr("bevy:Health:max"), s("Health", "max"));
        assert_eq!(
            parse_attr("bevy:Transform:translation:x"),
            s("Transform", "translation.x")
        );
        // Full type path (encoded with `__`) disambiguates a short-name
        // collision; the type segment is kept verbatim and decoded in `resolve`.
        assert_eq!(
            parse_attr("bevy:my_game__Health:max"),
            s("my_game__Health", "max")
        );
        // Tuple-index encoding `_0` → reflect path `0`.
        assert_eq!(parse_attr("bevy:Level:_0"), s("Level", "0"));
        assert_eq!(
            parse_attr("bevy:Pair:_1:x"),
            s("Pair", "1.x"),
            "nested tuple index then struct field"
        );
        // Not reflect-routed.
        assert_eq!(parse_attr("bevy:Health"), s("Health", ""), "component presence");
        assert_eq!(parse_attr("bevy:"), None, "no type or field");
        assert_eq!(parse_attr("xformOp:translate"), None, "not bevy-namespaced");
        assert_eq!(parse_attr("visibility"), None);
    }

    #[test]
    fn scalar_coercions() {
        let mut f = 0f32;
        assert!(set_field(&mut f, &Value::Double(1.5)));
        assert_eq!(f, 1.5);
        assert!(set_field(&mut f, &Value::Int(3)));
        assert_eq!(f, 3.0, "int coerces to f32");

        let mut d = 0f64;
        assert!(set_field(&mut d, &Value::Float(2.5)));
        assert_eq!(d, 2.5);

        let mut i = 0i32;
        assert!(set_field(&mut i, &Value::Int64(42)));
        assert_eq!(i, 42);

        let mut b = false;
        assert!(set_field(&mut b, &Value::Bool(true)));
        assert!(b);

        let mut s = String::new();
        assert!(set_field(&mut s, &Value::Token(Token::new("hi"))));
        assert_eq!(s, "hi", "token coerces to String");
        assert!(set_field(&mut s, &Value::String("bye".into())));
        assert_eq!(s, "bye");
    }

    #[test]
    fn integer_coercions_reject_loss_without_mutation() {
        let mut signed = 7i32;
        let mut unsigned = 7u32;
        let mut optional = Some(7i32);
        for value in [Value::Int64(i64::MAX), Value::Uint64(u64::MAX),
            Value::Double(f64::NAN), Value::Double(f64::INFINITY), Value::Double(1.5),
            Value::Double(2.0_f64.powi(127)), Value::Double(-2.0_f64.powi(127))] {
            assert!(!set_field(&mut signed, &value));
            assert!(!set_field(&mut unsigned, &value));
            assert!(!set_field(&mut optional, &value));
        }
        assert_eq!((signed, unsigned, optional), (7, 7, Some(7)));
        let mut wide_signed = 9i64;
        let mut wide_unsigned = 9u64;
        let mut size = 9usize;
        assert!(!set_field(&mut wide_signed, &Value::Uint64(u64::MAX)));
        assert!(!set_field(&mut wide_signed, &Value::Double(2.0_f64.powi(63))));
        assert!(!set_field(&mut wide_unsigned, &Value::Int(-1)));
        assert!(!set_field(&mut wide_unsigned, &Value::Double(2.0_f64.powi(64))));
        assert!(!set_field(&mut size, &Value::Int(-1)));
        assert_eq!((wide_signed, wide_unsigned, size), (9, 9, 9));
        assert!(set_field(&mut wide_signed, &Value::Int64(i64::MIN)));
        assert_eq!(wide_signed, i64::MIN);
        assert!(set_field(&mut wide_unsigned, &Value::Uint64(u64::MAX)));
        assert_eq!(wide_unsigned, u64::MAX);
        assert!(set_field(&mut signed, &Value::Double(42.0)));
        assert_eq!(signed, 42);
        assert!(set_field(&mut size, &Value::Uint64(usize::MAX as u64)));
        assert_eq!(size, usize::MAX);
    }

    #[test]
    fn vector_coercions() {
        let mut v2 = Vec2::ZERO;
        assert!(set_field(&mut v2, &Value::Vec2f([1.0, 2.0].into())));
        assert_eq!(v2, Vec2::new(1.0, 2.0));

        let mut v3 = Vec3::ZERO;
        assert!(set_field(&mut v3, &Value::Vec3d(openusd::gf::Vec3d::from([1.0, 2.0, 3.0]))));
        assert_eq!(v3, Vec3::new(1.0, 2.0, 3.0), "double-precision vec coerces");

        let mut v4 = Vec4::ZERO;
        assert!(set_field(&mut v4, &Value::Vec4f([1.0, 2.0, 3.0, 4.0].into())));
        assert_eq!(v4, Vec4::new(1.0, 2.0, 3.0, 4.0));

        // gf Quatf fields are (w, x, y, z); as_vec must map to bevy xyzw.
        let gq = openusd::gf::Quatf {
            w: 0.7,
            x: 0.1,
            y: 0.2,
            z: 0.3,
        };
        let mut q = Quat::IDENTITY;
        assert!(set_field(&mut q, &Value::Quatf(gq)));
        assert_eq!(
            q,
            Quat::from_xyzw(0.1, 0.2, 0.3, 0.7),
            "quat maps imaginary→xyz, real→w"
        );
    }

    #[test]
    fn mismatched_value_rejected() {
        let mut f = 7f32;
        // A string into a float field is unsupported → false, field untouched.
        assert!(!set_field(&mut f, &Value::String("x".into())));
        assert_eq!(f, 7.0);
        let mut b = true;
        assert!(!set_field(&mut b, &Value::Double(1.0)));
        assert!(b, "double into bool rejected");
    }

    #[derive(Reflect, Default, Debug, PartialEq)]
    enum Mode {
        #[default]
        Off,
        On,
        Sleep,
    }

    #[test]
    fn enum_unit_variant_by_token() {
        let mut m = Mode::Off;
        assert!(set_field(&mut m, &Value::Token(Token::new("On"))));
        assert_eq!(m, Mode::On, "token selects the variant");
        assert!(set_field(&mut m, &Value::String("Sleep".into())));
        assert_eq!(m, Mode::Sleep, "string selects the variant");
        // Unknown variant is rejected, leaving the value intact.
        assert!(!set_field(&mut m, &Value::Token(Token::new("Nope"))));
        assert_eq!(m, Mode::Sleep);
    }
}
