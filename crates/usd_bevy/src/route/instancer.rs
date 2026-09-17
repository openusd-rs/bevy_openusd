//! PointInstancer route (PLAN P4, in-repo): `UsdGeomPointInstancer` → one
//! child entity per instance, sharing the prototype's baked mesh.
//!
//! This is *point* instancing (an explicit position/orientation/scale table),
//! distinct from USD native scenegraph instancing.
//! Instances are spawned as children of the instancer entity, each carrying a
//! [`UsdInstance`] marker and stable ID for reconciliation. All
//! instances of one prototype share a single `Mesh`/`StandardMaterial` handle
//! (baked once per project), so N instances cost one mesh in memory.

use bevy::prelude::*;
use openusd_schemas::geom::PointInstancerSchema;
use bevy::platform::collections::HashMap;

use super::{PrimRoute, RouteCtx};
use crate::read::geom::{ReadPointInstancer, read_point_instancer_at};
use openusd_schemas::geom::PointInstancer;
use openusd::sdf::Value;

/// Instance IDs masked by composed metadata or sampled visibility.
fn masked_ids(ctx: &RouteCtx) -> Result<bevy::platform::collections::HashSet<i64>, String> {
    let mut set = bevy::platform::collections::HashSet::default();
    let prim = ctx.stage.prim(ctx.path.clone()).map_err(|error| error.to_string())?;
    match prim.get_metadata::<Value>("inactiveIds").map_err(|error| error.to_string())? {
        Some(Value::Int64ListOp(op)) if op.explicit => set.extend(op.explicit_items),
        None => {}
        _ => return Err("invalid composed inactiveIds metadata".into()),
    }
    if let Ok(Some(pi)) = PointInstancer::get(ctx.stage, ctx.path.clone()) {
        let attribute = pi.invisible_ids_attr();
        match ctx.time.map_or_else(|| attribute.get::<Value>(), |time| attribute.get_at::<Value>(openusd::usd::TimeCode::new(time))) {
            Ok(Some(Value::Int64Vec(v))) => set.extend(v),
            Ok(Some(Value::IntVec(v))) => set.extend(v.into_iter().map(i64::from)),
            Ok(None) => {}
            Ok(Some(_)) => return Err("invalid invisibleIds array".into()),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(set)
}

/// A baked prototype's shared render handles.
type ProtoHandles = (Handle<Mesh>, Handle<StandardMaterial>, Vec<String>, super::subset::PreparedSubsets);

#[derive(Clone)]
enum Prototype { Mesh(ProtoHandles), Hierarchy(Vec<PrototypePart>) }

#[derive(Clone)]
struct PrototypePart {
    path: String,
    parent: Option<String>,
    transform: Transform,
    visibility: Visibility,
    handles: Option<ProtoHandles>,
}

/// USD prototype path represented by a generated hierarchy node.
#[derive(Component, Debug, Clone)]
pub struct UsdPrototypePart(pub String);

#[derive(Component, Default)]
struct PrototypeEntities(HashMap<String, Entity>);

/// Marker on entities spawned for a PointInstancer instance.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdInstance;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsdInstanceId(pub i64);

#[derive(Component, Debug)]
pub struct UsdInstancerWarning(pub String);

/// Maps a `PointInstancer` prim to per-instance child entities.
pub struct PointInstancerRoute;

fn validate_instances(read: &ReadPointInstancer) -> Result<(), String> {
    let count = read.positions.len();
    if read.proto_indices.len() != count
        || (!read.orientations.is_empty() && read.orientations.len() != count)
        || (!read.scales.is_empty() && read.scales.len() != count) {
        return Err("prototype indices and authored transform arrays must match positions".into());
    }
    if read.proto_indices.iter().any(|&index| index < 0 || (!read.prototypes.is_empty() && index as usize >= read.prototypes.len())) {
        return Err("prototype index is out of range".into());
    }
    if read.positions.iter().chain(&read.scales).flatten().any(|value| !value.is_finite()) {
        return Err("instance positions and scales must be finite".into());
    }
    if read.orientations.iter().any(|value| {
        let norm = value.iter().map(|component| component * component).sum::<f32>();
        !norm.is_finite() || norm <= f32::MIN_POSITIVE
    }) {
        return Err("instance orientations must be finite nonzero quaternions".into());
    }
    Ok(())
}

fn instance_transform(read: &ReadPointInstancer, i: usize) -> Transform {
    let t = read.positions[i];
    let mut xf = Transform::from_translation(Vec3::from_array(t));
    if let Some(o) = read.orientations.get(i) {
        // read_quat_array yields [w, x, y, z]; bevy is xyzw.
        xf.rotation = Quat::from_xyzw(o[1], o[2], o[3], o[0]).normalize();
    }
    if let Some(s) = read.scales.get(i) {
        xf.scale = Vec3::from_array(*s);
    }
    xf
}

fn valid_instance_ids(ids: Option<&[i64]>, count: usize) -> bool {
    let Some(ids) = ids else { return true };
    if ids.len() != count { return false; }
    if ids.windows(2).all(|pair| pair[0] < pair[1]) { return true; }
    let mut unique = bevy::platform::collections::HashSet::with_capacity(ids.len());
    ids.iter().all(|id| unique.insert(*id))
}

impl PointInstancerRoute {
    /// Despawn instance children this route spawned on a previous project, so a
    /// reproject doesn't stack duplicate batches.
    fn clear_instances(world: &mut World, entity: Entity) {
        let existing: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        for child in existing {
            if world.get::<UsdInstance>(child).is_some() {
                world.entity_mut(child).despawn();
            }
        }
    }
}

impl PrimRoute for PointInstancerRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        Self::clear_instances(world, entity);
        world.entity_mut(entity).remove::<UsdInstancerWarning>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("PointInstancer")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let read = match read_point_instancer_at(ctx.stage, ctx.path, ctx.time) {
            Ok(Some(read)) => read,
            Ok(None) => {
                Self::clear_instances(world, entity);
                world.entity_mut(entity).remove::<UsdInstancerWarning>();
                return;
            }
            Err(error) => {
                world.entity_mut(entity).insert(UsdInstancerWarning(error.to_string()));
                return;
            }
        };
        if let Err(error) = validate_instances(&read) {
            world.entity_mut(entity).insert(UsdInstancerWarning(error));
            return;
        }
        if ctx.trace_memory {
            eprintln!("point_instancer_memory path={:?} phase=arrays-read instances={} prototypes={} positions_bytes={} orientations_bytes={} scales_bytes={} indices_bytes={}", ctx.path,
                read.positions.len(), read.prototypes.len(), std::mem::size_of_val(read.positions.as_slice()),
                std::mem::size_of_val(read.orientations.as_slice()), std::mem::size_of_val(read.scales.as_slice()), std::mem::size_of_val(read.proto_indices.as_slice()));
            super::profiling::memory_event("instancer-arrays-read", ctx.path, self.name(), None);
        }
        let ids = ctx.stage.prim(ctx.path.clone()).ok()
            .and_then(|prim| {
                let attribute = prim.attribute("ids");
                ctx.time.map_or_else(|| attribute.get::<Value>(), |time| attribute.get_at::<Value>(openusd::usd::TimeCode::new(time))).ok().flatten()
            });
        let ids = match ids {
            Some(Value::Int64Vec(ids)) => Some(ids),
            Some(Value::IntVec(ids)) => Some(ids.into_iter().map(i64::from).collect()),
            None => None,
            _ => {
                world.entity_mut(entity).insert(UsdInstancerWarning("invalid instance IDs".into()));
                return;
            }
        };
        if !valid_instance_ids(ids.as_deref(), read.positions.len()) {
            world.entity_mut(entity).insert(UsdInstancerWarning("instance IDs must be unique and match positions".into()));
            return;
        }
        if ctx.trace_memory {
            eprintln!("point_instancer_memory path={:?} phase=ids-validated authored_ids={}", ctx.path, ids.is_some());
            super::profiling::memory_event("instancer-ids-validated", ctx.path, self.name(), None);
        }
        let invisible = match masked_ids(ctx) {
            Ok(ids) => ids,
            Err(error) => {
                world.entity_mut(entity).insert(UsdInstancerWarning(error));
                return;
            }
        };
        world.entity_mut(entity).remove::<UsdInstancerWarning>();
        let mut existing: HashMap<i64, Entity> = world.get::<Children>(entity).into_iter()
            .flat_map(|children| children.iter()).filter_map(|child| {
                world.get::<UsdInstance>(child)?;
                Some((world.get::<UsdInstanceId>(child)?.0, child))
            }).collect();

        let have_assets = world.get_resource::<Assets<Mesh>>().is_some()
            && world.get_resource::<Assets<StandardMaterial>>().is_some();

        // Bake each referenced prototype's mesh once; share the handles.
        let mut proto_cache: HashMap<usize, Option<Prototype>> =
            HashMap::default();

        if ctx.trace_memory { super::profiling::memory_event("instancer-spawn-begin", ctx.path, self.name(), None); }
        for i in 0..read.positions.len() {
            let id = ids.as_ref().map_or(i as i64, |ids| ids[i]);
            let xf = instance_transform(&read, i);
            let proto_idx = read.proto_indices[i] as usize;

            let handles = if have_assets {
                proto_cache
                    .entry(proto_idx)
                    .or_insert_with(|| bake_prototype(ctx, world, &read, proto_idx))
                    .as_ref()
            } else {
                None
            };

            let child = existing.remove(&id).unwrap_or_else(|| world.spawn_empty().id());
            let mut e = world.entity_mut(child);
            let visibility = if invisible.contains(&id) { Visibility::Hidden } else { Visibility::default() };
            e.insert((UsdInstance, UsdInstanceId(id), xf, visibility, ChildOf(entity)));
            match handles {
                Some(Prototype::Hierarchy(parts)) => {
                    apply_handles(ctx, world, child, None);
                    apply_hierarchy(ctx, world, child, parts);
                }
                Some(Prototype::Mesh(handles)) => {
                    clear_hierarchy(world, child);
                    apply_handles(ctx, world, child, Some(handles));
                }
                None => {
                    let parts: Vec<_> = world.get::<PrototypeEntities>(child).into_iter()
                        .flat_map(|parts| parts.0.values().copied()).collect();
                    for part in parts {
                        if world.get_entity(part).is_ok() { apply_handles(ctx, world, part, None); }
                    }
                    apply_handles(ctx, world, child, None);
                }
            }
            if ctx.trace_memory && (i == 0 || (i+1) % 100_000 == 0) {
                eprintln!("point_instancer_memory path={:?} phase=spawn-progress completed={} total={} allocated_entity_indices={}", ctx.path, i+1, read.positions.len(), world.entities().len());
                super::profiling::memory_event("instancer-spawn-progress", ctx.path, self.name(), None);
            }
        }
        for child in existing.into_values() { world.despawn(child); }
        let mut failed: Vec<_> = proto_cache.iter().filter(|(_, handles)| handles.is_none())
            .map(|(index, _)| format!("prototype {index} could not be evaluated; generated geometry omitted"))
            .collect();
        if !failed.is_empty() {
            failed.sort();
            world.entity_mut(entity).insert(UsdInstancerWarning(failed.join("; ")));
        }
    }
}

/// Prepares shared geometry and local transforms for a referenced prototype.
fn bake_prototype(
    ctx: &RouteCtx,
    world: &mut World,
    read: &ReadPointInstancer,
    proto_idx: usize,
) -> Option<Prototype> {
    let proto_path = read.prototypes.get(proto_idx)?;
    let prim = ctx.stage.prim(proto_path.clone()).ok()?;
    let has_scene_children = has_prototype_geometry_children(ctx.stage, proto_path)?;
    if prim.type_name().ok().flatten().as_deref() == Some("Mesh") && !has_scene_children {
        return bake_mesh(ctx, world, proto_path, true).map(Prototype::Mesh);
    }
    let mut parts = Vec::new();
    collect_parts(ctx, world, proto_path, None, 0, &mut parts)?;
    Some(Prototype::Hierarchy(parts))
}

fn prototype_metadata(kind: &str) -> bool {
    matches!(kind, "Material" | "Shader" | "NodeGraph" | "GeomSubset" | "Skeleton" | "SkelAnimation" | "BlendShape")
}

fn has_prototype_geometry_children(stage: &openusd::usd::Stage, root: &openusd::sdf::Path) -> Option<bool> {
    let mut pending = vec![root.clone()];
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 4096 { return Some(true); }
        let prim = stage.prim(path.clone()).ok()?;
        if !prim.is_active().ok()? || !prim.is_defined().ok()? || prim.is_abstract().ok()? { continue; }
        if path != *root {
            let kind = prim.type_name().ok().flatten().unwrap_or_default();
            if prototype_metadata(&kind) { continue; }
            if !matches!(kind.as_str(), "" | "Xform" | "Scope" | "SkelRoot") { return Some(true); }
        }
        for name in prim.child_names().ok()? { pending.push(path.append_path(name.as_str()).ok()?); }
    }
    Some(false)
}

fn collect_parts(ctx: &RouteCtx, world: &mut World, path: &openusd::sdf::Path,
    parent: Option<String>, depth: usize, parts: &mut Vec<PrototypePart>) -> Option<()> {
    if parts.len() >= 4096 || depth >= 256 { return None; }
    let prim = ctx.stage.prim(path.clone()).ok()?;
    if !prim.is_active().ok()? || !prim.is_defined().ok()? || prim.is_abstract().ok()? { return Some(()); }
    let kind = prim.type_name().ok().flatten().unwrap_or_default();
    if prototype_metadata(&kind) { return parent.as_ref().map(|_| ()); }
    let shape = matches!(kind.as_str(), "Cube" | "Sphere" | "Cylinder" | "Capsule" | "Cone" | "Plane");
    if !shape && !matches!(kind.as_str(), "" | "Xform" | "Scope" | "SkelRoot" | "Mesh") { return None; }
    let order = prim.attribute("xformOpOrder").get_at::<Value>(ctx.time.map(openusd::usd::TimeCode::new)).ok()?;
    let reset = match order {
        Some(Value::TokenVec(values)) => values.iter().any(|value| value.as_str() == "!resetXformStack!"),
        Some(Value::StringVec(values)) => values.iter().any(|value| value == "!resetXformStack!"),
        Some(Value::TokenListOp(values)) => values.flatten().iter().any(|value| value.as_str() == "!resetXformStack!"),
        None => false,
        _ => return None,
    };
    if reset { return None; }
    let matrix = crate::read::xform::read_transform_matrix_at(ctx.stage, path, ctx.time).ok()?
        .map(|values| Mat4::from_cols_array(&values)).unwrap_or(Mat4::IDENTITY);
    let transform = hierarchy_transform(matrix)?;
    if !transform.translation.is_finite() || !transform.rotation.is_finite()
        || !transform.scale.is_finite() || transform.scale.abs().min_element() == 0.0 { return None; }
    let mut visibility = match prim.attribute("visibility").get_at::<Value>(ctx.time.map(openusd::usd::TimeCode::new)).ok()? {
        Some(Value::Token(value)) if value.as_str() == "invisible" => Visibility::Hidden,
        _ => Visibility::Inherited,
    };
    let purpose = crate::read::geom::read_effective_purpose(ctx.stage, path).ok()?;
    if !world.get_resource::<super::DisplayPurposes>().copied().unwrap_or_default().shows(&purpose) {
        visibility = Visibility::Hidden;
    }
    let handles = if kind == "Mesh" { Some(bake_mesh(ctx, world, path, false)?) }
        else if shape { Some(bake_shape(ctx, world, path)?) } else { None };
    parts.push(PrototypePart { path: path.as_str().into(), parent, transform, visibility, handles });
    for name in prim.child_names().ok()? {
        collect_parts(ctx, world, &path.append_path(name.as_str()).ok()?, Some(path.as_str().into()), depth + 1, parts)?;
    }
    Some(())
}

fn hierarchy_transform(matrix: Mat4) -> Option<Transform> {
    if !matrix.is_finite() || matrix.row(3) != Vec4::W { return None; }
    let axes = [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|axis| [f64::from(axis.x), f64::from(axis.y), f64::from(axis.z)]);
    let dot = |a: [f64;3], b: [f64;3]| a.into_iter().zip(b).map(|(a,b)| a*b).sum::<f64>();
    let lengths = axes.map(|axis| dot(axis,axis).sqrt());
    if lengths.contains(&0.0) { return None; }
    for (a,b) in [(0,1),(0,2),(1,2)] {
        if dot(axes[a],axes[b]).abs() > 1e-5 * lengths[a] * lengths[b] { return None; }
    }
    Some(Transform::from_matrix(matrix))
}

fn clear_hierarchy(world: &mut World, entity: Entity) {
    if let Some(parts) = world.entity_mut(entity).take::<PrototypeEntities>() {
        for child in parts.0.into_values() {
            if world.get_entity(child).is_ok() { world.despawn(child); }
        }
    }
}

fn apply_handles(ctx: &RouteCtx, world: &mut World, entity: Entity, handles: Option<&ProtoHandles>) {
    let mut e = world.entity_mut(entity);
    e.remove::<bevy::camera::primitives::Aabb>();
    if let Some((mesh, material, warnings, subsets)) = handles {
        e.insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
        if warnings.is_empty() { e.remove::<super::material::UsdMaterialWarning>(); }
        else { e.insert(super::material::UsdMaterialWarning(warnings.join("; "))); }
        super::subset::apply(world, entity, subsets);
    } else {
        e.remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>, super::material::UsdMaterialWarning)>();
        super::subset::SubsetRoute.remove(ctx, world, entity);
    }
}

fn apply_hierarchy(ctx: &RouteCtx, world: &mut World, instance: Entity, parts: &[PrototypePart]) {
    let mut previous = world.entity_mut(instance).take::<PrototypeEntities>().unwrap_or_default().0;
    let mut next = HashMap::default();
    for part in parts {
        let entity = previous.remove(&part.path).filter(|entity| world.get_entity(*entity).is_ok())
            .unwrap_or_else(|| world.spawn_empty().id());
        let parent = part.parent.as_ref().and_then(|path| next.get(path)).copied().unwrap_or(instance);
        world.entity_mut(entity).insert((UsdPrototypePart(part.path.clone()), part.transform, part.visibility, ChildOf(parent)));
        apply_handles(ctx, world, entity, part.handles.as_ref());
        next.insert(part.path.clone(), entity);
    }
    for entity in previous.into_values() {
        if world.get_entity(entity).is_ok() { world.despawn(entity); }
    }
    world.entity_mut(instance).insert(PrototypeEntities(next));
}

fn prototype_material(ctx: &RouteCtx, world: &mut World, fallback: impl FnOnce() -> StandardMaterial) -> (Handle<StandardMaterial>, Vec<String>) {
    match super::material::resolve_material(ctx, world) {
        Ok(Some(material)) => material,
        result => {
            let warnings = result.err().map(|error| vec![error.to_string()]).unwrap_or_default();
            (super::cache::intern_material(world, fallback()), warnings)
        }
    }
}

fn bake_shape(ctx: &RouteCtx, world: &mut World, path: &openusd::sdf::Path) -> Option<ProtoHandles> {
    let ctx = RouteCtx::at(ctx.stage, path, ctx.time);
    let shape = super::shapes::shape_mesh(&ctx)?;
    let (material, mut warnings) = prototype_material(&ctx, world,
        || super::material::default_material_with_opacity(&ctx, shape.opacity.as_ref()));
    if let Some(material) = world.resource::<Assets<StandardMaterial>>().get(&material) {
        super::material::warn_geometry_inputs(&shape.mesh, material, &mut warnings);
    }
    Some((super::cache::intern_mesh(world, shape.mesh), material, warnings, default()))
}

fn bake_mesh(ctx: &RouteCtx, world: &mut World, proto_path: &openusd::sdf::Path, bake_transform: bool) -> Option<ProtoHandles> {
    let proto_ctx = RouteCtx::at(ctx.stage, proto_path, ctx.time);
    let mesh_read = if super::subdivision::enabled(&proto_ctx, world) {
        super::subdivision::refined_mesh(&proto_ctx,
            world.resource::<super::subdivision::UsdSubdivisionSettings>().levels()).ok()?
    } else if crate::read::skel::is_skinned(ctx.stage, proto_path)
        || crate::read::skel::has_blend_shapes(ctx.stage, proto_path) {
        super::skel::deformed_mesh(&proto_ctx).ok().flatten()?
    } else {
        crate::read::geom::read_mesh_at(ctx.stage, proto_path, ctx.time).ok().flatten()?
    };
    let mut mesh = crate::mesh::mesh_from_usd(&mesh_read);
    if bake_transform && let Some(matrix) = crate::read::xform::read_transform_matrix_at(ctx.stage, proto_path, ctx.time).ok()? {
        crate::mesh::affine::bake(&mut mesh, Mat4::from_cols_array(&matrix))?;
    }
    let (material, mut warnings) = prototype_material(&proto_ctx, world, || super::material::default_material(&proto_ctx));
    if let Some(material) = world.resource::<Assets<StandardMaterial>>().get(&material) {
        super::material::warn_geometry_inputs(&mesh, material, &mut warnings);
    }
    let mesh_handle = super::cache::intern_mesh(world, mesh);
    let subsets = super::subset::prepare(&proto_ctx, world, &mesh_read, &mesh_handle, &material)?;
    Some((mesh_handle, material, warnings, subsets))
}

#[cfg(test)]
mod tests {
    #[test]
    fn instance_id_validation_preserves_implicit_and_explicit_identity() {
        assert!(super::valid_instance_ids(None, 10_000_000));
        for ids in [&[][..], &[-5, 0, 10], &[10, -5, 0]] {
            assert!(super::valid_instance_ids(Some(ids), ids.len()));
        }
        for (ids, count) in [(&[1, 1][..], 2), (&[1, 3, 1][..], 3), (&[1][..], 2), (&[][..], 1)] {
            assert!(!super::valid_instance_ids(Some(ids), count));
        }
    }

    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::sdf::Value;
    use openusd::usd::Stage;

    #[test]
    fn affine_mesh_prototypes_match_explicit_points_and_share_subsets() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/point_affine.usda")).unwrap();
        let reference = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/point_affine_reference.usda")).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let path = openusd::sdf::path("/Panels").unwrap();
        let mut expected = crate::read::geom::read_mesh_at(&reference, &path, Some(0.0)).unwrap().unwrap();
        expected.triangulation_points = Some(crate::read::geom::read_mesh_at(&stage, &path, Some(0.0)).unwrap().unwrap().points);
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let entity = world.spawn_empty().id();
        let instancer = openusd::sdf::path("/Instances").unwrap();
        PointInstancerRoute.project(&RouteCtx::at(&stage, &instancer, Some(0.0)), &mut world, entity);
        assert!(world.get::<UsdInstancerWarning>(entity).is_none());
        let copies: Vec<_> = world.get::<Children>(entity).unwrap().iter().collect();
        assert_eq!(copies.len(), 3);
        let mut handles = Vec::new();
        for copy in copies {
            let mut parts = Vec::new();
            for child in world.get::<Children>(copy).unwrap().iter() {
                let subset = world.get::<super::super::subset::UsdSubset>(child).unwrap();
                let faces = &expected.subsets.iter().find(|part| part.name == subset.0).unwrap().indices;
                let reference = crate::mesh::mesh_from_usd_subset(&expected, Some(faces));
                let handle = &world.get::<Mesh3d>(child).unwrap().0;
                let actual = world.resource::<Assets<Mesh>>().get(handle).unwrap();
                for attribute in [Mesh::ATTRIBUTE_POSITION, Mesh::ATTRIBUTE_NORMAL] {
                    let bevy::mesh::VertexAttributeValues::Float32x3(a) = actual.attribute(attribute).unwrap() else { panic!() };
                    let bevy::mesh::VertexAttributeValues::Float32x3(b) = reference.attribute(attribute).unwrap() else { panic!() };
                    for (ia, ib) in actual.indices().unwrap().iter().zip(reference.indices().unwrap().iter()) {
                        let (mut a, mut b) = (Vec3::from(a[ia]), Vec3::from(b[ib]));
                        if attribute == Mesh::ATTRIBUTE_NORMAL { a = a.normalize(); b = b.normalize(); }
                        assert!(a.abs_diff_eq(b, 1e-6), "{}: {a:?} != {b:?}", subset.0);
                    }
                    assert_eq!(actual.indices().unwrap().len(), reference.indices().unwrap().len());
                }
                parts.push((subset.0.clone(), handle.clone()));
            }
            parts.sort_by(|a,b| a.0.cmp(&b.0));
            assert_eq!(parts.len(), 2);
            handles.push(parts);
        }
        assert_eq!(handles[0], handles[1]);
        assert_eq!(handles[1], handles[2]);
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn hierarchy_local_transforms_reject_shear_perspective_and_singular_axes() {
        for scale in [Vec3::new(2.0,3.0,0.5), Vec3::new(-2.0,3.0,0.5)] {
            let transform = Transform::from_xyz(4.0,2.0,-1.0).with_rotation(Quat::from_rotation_z(0.7)).with_scale(scale);
            let matrix = transform.to_matrix();
            assert!(hierarchy_transform(matrix).unwrap().to_matrix().abs_diff_eq(matrix, 1e-5));
        }
        assert!(hierarchy_transform(Mat4::from_cols(Vec4::X, Vec4::new(0.2,1.0,0.0,0.0), Vec4::Z, Vec4::W)).is_none());
        assert!(hierarchy_transform(Mat4::from_scale(Vec3::new(0.0,1.0,1.0))).is_none());
        assert!(hierarchy_transform(Mat4::perspective_rh(1.0,1.0,0.1,10.0)).is_none());
        assert!(hierarchy_transform(Mat4::from_cols_array(&[f32::NAN;16])).is_none());
    }

    #[test]
    fn shape_prototypes_share_ordinary_geometry_at_independent_times() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("shapes.usda", include_bytes!("../../../../assets/point_shapes.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let parents = roots.map(|root| app.world().get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap());
        let copies = parents.map(|parent| app.world().get::<Children>(parent).unwrap().iter()
            .filter(|entity| app.world().get::<UsdInstance>(*entity).is_some()).collect::<Vec<_>>());
        let cube_path = "/Library/Assembly/Cube";
        let cube = app.world().get::<PrototypeEntities>(copies[0][0]).unwrap().0[cube_path];
        let runtime = app.world_mut().spawn(ChildOf(cube)).id();
        for times in [[0.0,10.0], [10.0,0.0], [5.0,10.0]] {
            for (root, time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
            app.update();
            for (index, root) in roots.into_iter().enumerate() {
                assert!(app.world().get::<UsdInstancerWarning>(parents[index]).is_none());
                assert_eq!(copies[index].len(), 2);
                for kind in ["Cube", "Sphere", "Cylinder", "Capsule", "Cone", "Plane"] {
                    let path = format!("/Library/Assembly/{kind}");
                    let ordinary = app.world().get_non_send::<UsdInstances>().unwrap().entity(root, &path).unwrap();
                    let expected = &app.world().get::<Mesh3d>(ordinary).unwrap().0;
                    if kind == "Cube" {
                        let mesh = app.world().resource::<Assets<Mesh>>().get(expected).unwrap();
                        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("cube color") };
                        assert!(colors.iter().all(|color| *color == [0.8,0.2,0.1,1.0]));
                    }
                    for &copy in &copies[index] {
                        let node = app.world().get::<PrototypeEntities>(copy).unwrap().0[&path];
                        assert_eq!(&app.world().get::<Mesh3d>(node).unwrap().0, expected, "{kind} at {}", times[index]);
                        assert_eq!(app.world().get::<Visibility>(node), Some(&Visibility::Inherited));
                        assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(node), app.world().get::<MeshMaterial3d<StandardMaterial>>(ordinary));
                    }
                }
            }
            assert_eq!(app.world().get::<PrototypeEntities>(copies[0][0]).unwrap().0[cube_path], cube);
            assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), cube);
        }
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(roots[0]).unwrap().clone();
        let size = stage.attribute("/Library/Assembly/Cube.size").unwrap();
        size.clone().set_at(Value::Double(-1.0), openusd::usd::TimeCode::new(5.0)).unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(cube).is_none());
        assert!(app.world().get::<UsdInstancerWarning>(parents[0]).is_some());
        assert!(app.world().get::<UsdInstancerWarning>(parents[1]).is_none());
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), cube);
        size.set_at(Value::Double(0.6), openusd::usd::TimeCode::new(5.0)).unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(cube).is_some());
        assert!(app.world().get::<UsdInstancerWarning>(parents[0]).is_none());
        stage.prim(cube_path).unwrap().set_type_name("Xform").unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(cube).is_none());
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), cube);
        stage.prim(cube_path).unwrap().set_type_name("Sphere").unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(cube).is_some());
        stage.relationship("/PI.prototypes").unwrap().set_targets([openusd::sdf::path(cube_path).unwrap()]).unwrap();
        app.update();
        assert!(app.world().get::<UsdInstancerWarning>(parents[0]).is_none());
        assert_eq!(app.world().get::<PrototypeEntities>(copies[0][0]).unwrap().0.len(), 1);
        assert_eq!(app.world().get::<PrototypeEntities>(copies[0][0]).unwrap().0[cube_path], cube);
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), cube);
    }

    #[test]
    fn hierarchy_prototypes_share_meshes_and_keep_independent_animation() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("hierarchy.usda", include_bytes!("../../../../assets/point_hierarchy.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        if !app.is_plugin_added::<bevy::transform::TransformPlugin>() { app.add_plugins(bevy::transform::TransformPlugin); }
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let parents = roots.map(|root| app.world().get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap());
        let instances = parents.map(|parent| app.world().get::<Children>(parent).unwrap().iter()
            .filter(|entity| app.world().get::<UsdInstance>(*entity).is_some()).collect::<Vec<_>>());
        assert!(instances.iter().all(|instances| instances.len() == 2));
        let part = |world: &World, instance, path: &str| world.get::<PrototypeEntities>(instance).unwrap().0[path];
        let b_path = "/Library/Assembly/Nested/B";
        let b = part(app.world(), instances[0][0], b_path);
        let runtime = app.world_mut().spawn((Transform::default(), ChildOf(b))).id();
        for times in [[0.0,10.0], [10.0,0.0], [5.0,10.0]] {
            for (root, time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
            app.update();
            for (index, group) in instances.iter().enumerate() {
                assert!(app.world().get::<UsdInstancerWarning>(parents[index]).is_none());
                let mut handles = Vec::new();
                for &instance in group {
                    assert!(app.world().get::<Mesh3d>(instance).is_none());
                    let node = part(app.world(), instance, b_path);
                    assert_eq!(app.world().get::<UsdPrototypePart>(node).unwrap().0, b_path);
                    let translation = app.world().get::<GlobalTransform>(node).unwrap().translation();
                    let placement = app.world().get::<Transform>(instance).unwrap().translation;
                    assert!((translation - (placement + Vec3::new(0.2,1.0 + times[index] as f32 / 10.0,0.0))).length() < 1e-5);
                    let subset = app.world().get::<Children>(node).unwrap().iter()
                        .find(|entity| app.world().get::<super::super::subset::UsdSubset>(*entity).is_some()).unwrap();
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(subset).unwrap().0).unwrap();
                    assert_eq!(mesh.indices().unwrap().len(), 6);
                    handles.push(app.world().get::<Mesh3d>(node).unwrap().0.clone());
                }
                assert_eq!(handles[0], handles[1]);
            }
            assert_eq!(part(app.world(), instances[0][0], b_path), b);
            assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), b);
        }
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(roots[0]).unwrap().clone();
        let order = "/Library/Assembly/Nested.xformOpOrder";
        stage.attribute(order).unwrap().set(Value::TokenVec(vec!["!resetXformStack!".into(), "xformOp:translate".into()])).unwrap();
        app.update();
        assert!(app.world().get::<UsdInstancerWarning>(parents[0]).is_some());
        assert!(app.world().get::<Mesh3d>(b).is_none());
        assert!(app.world().get::<UsdInstancerWarning>(parents[1]).is_none());
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), b);
        stage.attribute(order).unwrap().set(Value::TokenVec(vec!["xformOp:translate".into()])).unwrap();
        app.update();
        assert!(app.world().get::<UsdInstancerWarning>(parents[0]).is_none());
        assert!(app.world().get::<Mesh3d>(b).is_some());
        assert_eq!(part(app.world(), instances[0][0], b_path), b);
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), b);
    }

    #[test]
    fn deformed_prototypes_match_ordinary_meshes_at_independent_times() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        for (fixture, path, end) in [
            (include_str!("../../../../assets/blendshape_test.usda"), "/Test/Face", 10.0),
            (include_str!("../../../../assets/skel_test_simple.usda"), "/Test/Bar", 30.0),
            (include_str!("../../../../assets/skel_influences.usda"), "/Test/Bar", 10.0),
        ] {
            let mut text = fixture.replace("float[] blendShapeWeights = [1.0]", "float[] blendShapeWeights.timeSamples = { 0: [0.0], 10: [1.0] }");
            text = text.replace("point3f[] points = [", r#"def GeomSubset "Part" {
                uniform token familyName = "materialBind"
                uniform token elementType = "face"
                int[] indices = [0]
            }
            point3f[] points = ["#);
            text.push_str(&format!("\ndef PointInstancer \"PI\" {{\n point3f[] positions = [(4,0,0), (8,0,0)]\n int[] protoIndices = [0,0]\n rel prototypes = [<{path}>]\n}}\n"));
            let source = crate::UsdSource::new("deformed-prototype.usda", text.into_bytes()).unwrap();
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<Assets<StandardMaterial>>();
            let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
            let a = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
            let b = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: end })).id();
            app.update();
            let entities = |world: &World, root| {
                let instances = world.get_non_send::<UsdInstances>().unwrap();
                let ordinary = instances.entity(root, path).unwrap();
                let pi = instances.entity(root, "/PI").unwrap();
                let children = world.get::<Children>(pi).unwrap();
                [ordinary, children[0], children[1]]
            };
            let ea = entities(app.world(), a);
            let eb = entities(app.world(), b);
            let positions = |world: &World, entity| {
                let mut entities = vec![entity];
                entities.extend(world.get::<Children>(entity).into_iter().flat_map(|children| children.iter())
                    .filter(|child| world.get::<super::super::subset::UsdSubset>(*child).is_some()));
                entities.into_iter().flat_map(|entity| {
                    let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
                    let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
                    mesh.indices().unwrap().iter().map(|index| positions[index]).collect::<Vec<_>>()
                }).collect::<Vec<_>>()
            };
            let validate = |world: &World, entities: [Entity; 3]| {
                let expected = positions(world, entities[0]);
                for &entity in &entities[1..] {
                    assert_eq!(positions(world, entity), expected);
                    let subset = world.get::<Children>(entity).unwrap()[0];
                    assert!(world.get::<super::super::subset::UsdSubset>(subset).is_some());
                    let name = &world.get::<super::super::subset::UsdSubset>(subset).unwrap().0;
                    let ordinary_subset = world.get::<Children>(entities[0]).unwrap().iter()
                        .find(|child| world.get::<super::super::subset::UsdSubset>(*child).is_some_and(|subset| &subset.0 == name)).unwrap();
                    assert_eq!(positions(world, subset), positions(world, ordinary_subset));
                }
                assert_eq!(world.get::<Mesh3d>(entities[1]).unwrap().0, world.get::<Mesh3d>(entities[2]).unwrap().0);
            };
            validate(app.world(), ea);
            validate(app.world(), eb);
            assert_ne!(positions(app.world(), ea[1]), positions(app.world(), eb[1]));
            let before_b = positions(app.world(), eb[1]);
            app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = end * 0.5;
            app.update();
            assert_eq!(entities(app.world(), a), ea);
            assert_eq!(entities(app.world(), b), eb);
            validate(app.world(), ea);
            assert_eq!(positions(app.world(), eb[1]), before_b);
            let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(a).unwrap().clone();
            if path.ends_with("Face") {
                stage.prim("/Test/Face/smile").unwrap().attribute("offsets")
                    .set(Value::Vec3fVec(vec![openusd::gf::Vec3f { x: 0.0, y: 0.0, z: 4.0 }])).unwrap();
                let before = positions(app.world(), ea[1]);
                app.update();
                assert_ne!(positions(app.world(), ea[1]), before);
                validate(app.world(), ea);
                assert_eq!(positions(app.world(), eb[1]), before_b);
            } else {
                let weights = stage.prim(path).unwrap().attribute("primvars:skel:jointWeights");
                let time = openusd::usd::TimeCode::new(end * 0.5);
                let original = weights.get_at::<Value>(Some(time)).unwrap().unwrap();
                weights.clone().set_at(Value::FloatVec(vec![1.0]), time).unwrap();
                app.update();
                assert!(app.world().get::<Mesh3d>(ea[1]).is_none());
                let pi = app.world().get_non_send::<UsdInstances>().unwrap().entity(a, "/PI").unwrap();
                assert!(app.world().get::<UsdInstancerWarning>(pi).is_some());
                weights.set_at(original, time).unwrap();
                app.update();
                assert_eq!(entities(app.world(), a), ea);
                validate(app.world(), ea);
                assert!(app.world().get::<UsdInstancerWarning>(pi).is_none());
                assert_eq!(positions(app.world(), eb[1]), before_b);
            }
        }
    }

    #[test]
    fn subset_preparation_cost_does_not_scale_asset_allocations_with_instance_count() {
        let measure = |count: usize| {
            let mut text = include_str!("../../../../assets/material_subsets.usda").to_owned();
            let positions = vec!["(0,0,0)"; count].join(",");
            let indices = vec!["0"; count].join(",");
            text.push_str(&format!("\ndef PointInstancer \"PI\" {{\n point3f[] positions = [{positions}]\n int[] protoIndices = [{indices}]\n rel prototypes = [</Panels>]\n}}\n"));
            let stage = crate::snippet::UsdSnippet::new(text).open_stage().unwrap();
            let path = openusd::sdf::path("/PI").unwrap();
            let ctx = RouteCtx::at(&stage, &path, Some(0.0));
            let mut world = World::new();
            world.init_resource::<Assets<Mesh>>();
            world.init_resource::<Assets<StandardMaterial>>();
            let entity = world.spawn_empty().id();
            PointInstancerRoute.project(&ctx, &mut world, entity);
            let instances = world.get::<Children>(entity).unwrap();
            assert_eq!(instances.len(), count);
            let parts: Vec<_> = instances.iter().map(|instance| {
                let child = world.get::<Children>(instance).unwrap()[0];
                assert_eq!(world.get::<Children>(instance).unwrap().len(), 2);
                (world.get::<Mesh3d>(child).unwrap().0.clone(), world.get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0.clone())
            }).collect();
            assert!(parts.windows(2).all(|parts| parts[0] == parts[1]));
            (world.resource::<Assets<Mesh>>().len(), world.resource::<Assets<StandardMaterial>>().len())
        };
        let one = measure(1);
        let many = measure(64);
        assert_eq!(one, many);
        assert_eq!(one.0, 4);
    }

    #[test]
    fn prototype_subsets_share_assets_and_follow_independent_clocks() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let mut text = include_str!("../../../../assets/material_subsets.usda").replace(
            "def Mesh \"Panels\" {",
            "def Mesh \"Panels\" {\n double3 xformOp:translate = (3,0,0)\n uniform token[] xformOpOrder = [\"xformOp:translate\"]",
        );
        text.push_str(r#"
def PointInstancer "PI" {
    point3f[] positions = [(10,0,0), (20,0,0)]
    int[] protoIndices = [0,0]
    int64[] ids = [42,99]
    rel prototypes = [</Panels>]
}
"#);
        let source = crate::UsdSource::new("pi-subsets.usda", text.into_bytes()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let a = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let b = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let instances = |world: &World, root| {
            let pi = world.get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap();
            let mut children: Vec<_> = world.get::<Children>(pi).unwrap().iter()
                .filter(|&child| world.get::<UsdInstance>(child).is_some()).collect();
            children.sort_by_key(|&child| world.get::<UsdInstanceId>(child).unwrap().0);
            children
        };
        let parts = |world: &World, entity| {
            let mut parts: Vec<_> = world.get::<Children>(entity).unwrap().iter()
                .filter(|&child| world.get::<super::super::subset::UsdSubset>(child).is_some()).collect();
            parts.sort_by_key(|&child| world.get::<super::super::subset::UsdSubset>(child).unwrap().0.clone());
            parts
        };
        let ia = instances(app.world(), a);
        let ib = instances(app.world(), b);
        let pa = parts(app.world(), ia[0]);
        let pb = parts(app.world(), ib[0]);
        assert_eq!(pa.len(), 2);
        for (root_instances, root_parts) in [(&ia, &pa), (&ib, &pb)] {
            let other = parts(app.world(), root_instances[1]);
            for (&first, second) in root_parts.iter().zip(other) {
                assert_eq!(app.world().get::<Mesh3d>(first).unwrap().0, app.world().get::<Mesh3d>(second).unwrap().0);
                assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(first).unwrap().0,
                    app.world().get::<MeshMaterial3d<StandardMaterial>>(second).unwrap().0);
                assert_eq!(*app.world().get::<Transform>(first).unwrap(), Transform::default());
            }
        }
        assert_eq!(app.world().get::<Transform>(ia[0]).unwrap().translation.x, 10.0);
        let indices = |world: &World, part| {
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(part).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            let points: Vec<_> = mesh.indices().unwrap().iter().map(|index| positions[index]).collect();
            assert!(points.iter().all(|point| point[0] >= 1.0 && point[0] <= 3.0));
            points
        };
        let start = indices(app.world(), pa[0]);
        let end = indices(app.world(), pb[0]);
        assert_ne!(start, end);
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 10.0;
        app.update();
        assert_eq!(instances(app.world(), a), ia);
        assert_eq!(parts(app.world(), ia[0]), pa);
        assert_eq!(parts(app.world(), ib[0]), pb);
        assert_eq!(indices(app.world(), pa[0]), end);
        assert_eq!(app.world().get::<Mesh3d>(pa[0]).unwrap().0, app.world().get::<Mesh3d>(pb[0]).unwrap().0);
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 0.0;
        app.update();
        assert_eq!(indices(app.world(), pa[0]), start);
        assert_eq!(indices(app.world(), pb[0]), end);
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(a).unwrap().clone();
        stage.prim("/Panels/Blue").unwrap().set_type_name("Xform").unwrap();
        app.update();
        assert_eq!(parts(app.world(), ia[0]), vec![pa[1]]);
        assert!(app.world().get_entity(pa[0]).is_err());
        assert_eq!(parts(app.world(), ib[0]), pb);
        let runtime_child = app.world_mut().spawn(ChildOf(ia[0])).id();
        crate::authoring::set_relationship_targets(&stage, "/PI", "prototypes", &[openusd::sdf::path("/Missing").unwrap()]).unwrap();
        app.update();
        assert_eq!(instances(app.world(), a), ia);
        assert!(app.world().get::<Mesh3d>(ia[0]).is_none());
        assert!(parts(app.world(), ia[0]).is_empty());
        assert!(app.world().get_entity(runtime_child).is_ok());
        assert_eq!(parts(app.world(), ib[0]), pb);
    }

    #[test]
    fn malformed_instance_arrays_preserve_last_valid_children() {
        let stage = crate::UsdSource::new("invalid-instancer.usda", &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(1,2,3)]
    float3[] scales = [(1,1,1)]
    quath[] orientations = [(1,0,0,0)]
    int[] protoIndices = [0]
    rel prototypes = [</Proto>]
}
def Scope "Proto" {}
"#[..]).unwrap().open_stage().unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/PI").unwrap();
        let child = world.get::<Children>(parent).unwrap()[0];
        let before = *world.get::<Transform>(child).unwrap();
        for (name, bad) in [
            ("positions", Value::Vec3fVec(vec![[f32::INFINITY,0.0,0.0].into()])),
            ("scales", Value::Vec3fVec(vec![[1.0,1.0,1.0].into(); 2])),
            ("orientations", Value::QuathVec(vec![openusd::gf::Quath::default()])),
            ("protoIndices", Value::IntVec(vec![-1])),
            ("protoIndices", Value::IntVec(vec![1])),
            ("protoIndices", Value::IntVec(vec![])),
        ] {
            let attribute = live.stage.prim("/PI").unwrap().attribute(name);
            let original = attribute.get::<Value>().unwrap().unwrap();
            attribute.set(bad).unwrap();
            crate::live::apply_changes(&mut world, &live, &mut map);
            assert!(world.get::<UsdInstancerWarning>(parent).is_some(), "{name}");
            assert_eq!(world.get::<Children>(parent).unwrap()[0], child);
            assert_eq!(*world.get::<Transform>(child).unwrap(), before);
            live.stage.prim("/PI").unwrap().attribute(name).set(original).unwrap();
            crate::live::apply_changes(&mut world, &live, &mut map);
            assert!(world.get::<UsdInstancerWarning>(parent).is_none(), "{name}");
        }
        let mut rotation = openusd::gf::Quath::IDENTITY;
        rotation.w += rotation.w;
        live.stage.prim("/PI").unwrap().attribute("orientations")
            .set(Value::QuathVec(vec![rotation])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdInstancerWarning>(parent).is_none());
        assert_eq!(world.get::<Transform>(child).unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn bundled_animation_showcase_has_outward_prototypes() {
        let source = crate::UsdSource::new("showcase.usda", &include_bytes!("../../../../assets/animation_showcase.usda")[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Showcase/Prototypes/Tetrahedron").unwrap();
        for (time, height) in [(0.0, 1.0), (10.0, 2.5)] {
            let mesh = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            assert_eq!(mesh.points[3][1], height);
            let center = mesh.points.iter().map(|point| Vec3::from_array(*point)).sum::<Vec3>() / mesh.points.len() as f32;
            for triangle in mesh.face_vertex_indices.chunks_exact(3) {
                let a = Vec3::from_array(mesh.points[triangle[0] as usize]);
                let b = Vec3::from_array(mesh.points[triangle[1] as usize]);
                let c = Vec3::from_array(mesh.points[triangle[2] as usize]);
                assert!((b-a).cross(c-a).dot(a-center) > 0.0);
            }
        }
    }

    #[test]
    fn prototype_local_transforms_follow_independent_clocks() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("prototype-transform.usda", &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(10,0,0)]
    float3[] scales = [(2,2,2)]
    int[] protoIndices = [0]
    rel prototypes = [</Group/Proto>]
}
def Xform "Group" {
    double3 xformOp:translate = (100,0,0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Mesh "Proto" {
        point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        double3 xformOp:translate.timeSamples = { 0: (1,0,0), 10: (3,0,0) }
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let child = |world: &World, root| {
            let parent = world.get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap();
            world.get::<Children>(parent).unwrap().iter().find(|child| world.get::<UsdInstance>(*child).is_some()).unwrap()
        };
        let a = child(app.world(), first);
        let b = child(app.world(), second);
        let leftmost = |world: &World, entity| {
            let mesh = &world.get::<Mesh3d>(entity).unwrap().0;
            let positions = world.resource::<Assets<Mesh>>().get(mesh).unwrap().attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) = positions else { panic!("positions") };
            let transform = world.get::<Transform>(entity).unwrap();
            positions.iter().map(|point| transform.transform_point(Vec3::from_array(*point)).x).fold(f32::INFINITY, f32::min)
        };
        assert_eq!(leftmost(app.world(), a), 12.0);
        assert_eq!(leftmost(app.world(), b), 16.0);
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.update();
        assert_eq!(child(app.world(), first), a);
        assert_eq!(child(app.world(), second), b);
        assert_eq!(leftmost(app.world(), a), 14.0);
        assert_eq!(leftmost(app.world(), b), 16.0);
    }

    #[test]
    fn prototype_materials_share_update_and_report_failures() {
        let source = crate::UsdSource::new("prototype-material.usda", &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(0,0,0), (2,0,0)]
    int[] protoIndices = [0,0]
    rel prototypes = [</Group/Proto>]
}
def Xform "Group" {
    rel material:binding = </Mat>
    def Mesh "Proto" {
        point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        bool doubleSided = true
    }
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0.2,0.4,0.6)
        float inputs:metallic = 0.8
        color3f inputs:emissiveColor.connect = </Mat/Texture.outputs:rgb>
        token outputs:surface
    }
    def Shader "Texture" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @emission.png@
        float3 outputs:rgb
    }
}
"#[..]).unwrap();
        let live = LiveStage::new(source.open_stage().unwrap());
        let stage = &live.stage;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<super::super::cache::MaterialCache>();
        world.init_resource::<Assets<Image>>();
        let image = world.resource_mut::<Assets<Image>>().add(Image::default());
        let material_path = openusd::sdf::path("/Mat").unwrap();
        let read = crate::read::shade::read_preview_material(&stage, &material_path).unwrap().unwrap();
        let mut textures = crate::asset::SnapshotTextures::default();
        textures.0.insert((read.emissive_texture.unwrap(), true), image.clone());
        world.insert_resource(textures);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/PI").unwrap();
        let children: Vec<_> = world.get::<Children>(parent).unwrap().iter().collect();
        assert_eq!(children.len(), 2);
        #[derive(Component)]
        struct RuntimeOnly;
        world.entity_mut(children[0]).insert(RuntimeOnly);
        let handle = world.get::<MeshMaterial3d<StandardMaterial>>(children[0]).unwrap().0.clone();
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(children[1]).unwrap().0, handle);
        let material = world.resource::<Assets<StandardMaterial>>().get(&handle).unwrap();
        assert_eq!(material.base_color, Color::linear_rgb(0.2, 0.4, 0.6));
        assert_eq!(material.metallic, 0.8);
        assert_eq!(material.emissive_texture, Some(image));
        assert!(material.double_sided);
        assert!(material.cull_mode.is_none());
        crate::authoring::set_attribute(&stage, "/Mat/Surface", "inputs:metallic", "float", Value::Float(0.3)).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        let updated = world.get::<MeshMaterial3d<StandardMaterial>>(children[0]).unwrap().0.clone();
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(&updated).unwrap().metallic, 0.3);
        let prototype = map.entity("/Group/Proto").unwrap();
        let ordinary = &world.get::<MeshMaterial3d<StandardMaterial>>(prototype).unwrap().0;
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(ordinary).unwrap().metallic, 0.3);
        crate::authoring::set_attribute(&stage, "/Group/Proto", "points", "point3f[]", Value::Vec3fVec(vec![
            [0.0, 0.0, 0.0].into(), [4.0, 0.0, 0.0].into(), [0.0, 1.0, 0.0].into(),
        ])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        let mesh = &world.get::<Mesh3d>(children[0]).unwrap().0;
        let positions = world.resource::<Assets<Mesh>>().get(mesh).unwrap().attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(positions) = positions else { panic!("positions") };
        assert!(positions.iter().any(|position| position[0] == 4.0));
        crate::authoring::set_relationship_targets(&stage, "/Group", "material:binding", &[openusd::sdf::path("/Missing").unwrap()]).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        for child in &children {
            assert!(world.get::<crate::route::material::UsdMaterialWarning>(*child).unwrap().0.contains("unsupported material surface"));
        }
        crate::authoring::set_relationship_targets(&stage, "/Group", "material:binding", &[]).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<crate::route::material::UsdMaterialWarning>(children[0]).is_none());
        assert_eq!(world.get::<Children>(parent).unwrap().iter().collect::<Vec<_>>(), children);
        assert!(world.get::<RuntimeOnly>(children[0]).is_some());
    }

    #[test]
    fn inactive_ids_compose_and_preserve_masked_children() {
        let mut source = crate::UsdSource::snapshot(std::path::Path::new("/virtual/masks/root.usda"), br#"#usda 1.0
( subLayers = [@weak.usda@] )
over "PI" {}
"#.to_vec()).unwrap();
        source.insert_dependency("/virtual/masks/weak.usda".into(), br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(1,0,0), (2,0,0), (3,0,0)]
    int[] protoIndices = [0,0,0]
    int64[] ids = [10,20,30]
    int64[] invisibleIds = [30]
}
"#.to_vec());
        let live = LiveStage::new(source.open_stage().unwrap());
        let root = live.stage.edit_target().clone();
        live.stage.set_edit_target(openusd::usd::EditTarget::for_layer("/virtual/masks/weak.usda")).unwrap();
        live.stage.prim("/PI").unwrap().set_metadata("inactiveIds", Value::Int64ListOp(openusd::sdf::ListOp::explicit(vec![10]))).unwrap();
        live.stage.set_edit_target(root).unwrap();
        let mut operation = openusd::sdf::ListOp::prepended(vec![20]);
        operation.deleted_items = vec![10];
        live.stage.prim("/PI").unwrap().set_metadata("inactiveIds", Value::Int64ListOp(operation)).unwrap();
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/PI").unwrap();
        assert!(world.get::<UsdInstancerWarning>(parent).is_none());
        let children: Vec<_> = world.get::<Children>(parent).unwrap().iter().collect();
        assert_eq!(children.len(), 3);
        let child = |id| *children.iter().find(|child| world.get::<UsdInstanceId>(**child) == Some(&UsdInstanceId(id))).unwrap();
        let (ten, twenty, thirty) = (child(10), child(20), child(30));
        assert_ne!(world.get::<Visibility>(ten), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(twenty), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(thirty), Some(&Visibility::Hidden));
        live.stage.prim("/PI").unwrap().clear_metadata("inactiveIds").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Visibility>(ten), Some(&Visibility::Hidden));
        assert_ne!(world.get::<Visibility>(twenty), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(thirty), Some(&Visibility::Hidden));
        live.stage.prim("/PI").unwrap().set_metadata("inactiveIds", Value::Int64ListOp(openusd::sdf::ListOp::explicit(vec![]))).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_ne!(world.get::<Visibility>(ten), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(thirty), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Children>(parent).unwrap().iter().collect::<Vec<_>>(), children);
    }

    #[test]
    fn independent_clocks_sample_instancer_transforms_and_masks() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("sampled.usda", &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions.timeSamples = { 0: [(0,0,0)], 10: [(10,0,0)] }
    float3[] scales.timeSamples = { 0: [(1,1,1)], 10: [(3,3,3)] }
    quath[] orientations.timeSamples = { 0: [(1,0,0,0)], 10: [(0,0,0,1)] }
    int[] protoIndices = [0]
    int64[] ids = [10]
    int64[] invisibleIds.timeSamples = { 0: [], 10: [10] }
}
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let child = |world: &World, root| {
            let parent = world.get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap();
            world.get::<Children>(parent).unwrap().iter().find(|child| world.get::<UsdInstanceId>(*child) == Some(&UsdInstanceId(10))).unwrap()
        };
        let a = child(app.world(), first);
        let b = child(app.world(), second);
        assert_eq!(app.world().get::<Transform>(a).unwrap().translation, Vec3::ZERO);
        let transform = app.world().get::<Transform>(b).unwrap();
        assert_eq!(transform.translation, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(transform.scale, Vec3::splat(3.0));
        assert!(transform.rotation.dot(Quat::from_rotation_z(std::f32::consts::PI)).abs() > 0.9999);
        assert_eq!(app.world().get::<Visibility>(b), Some(&Visibility::Hidden));
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.update();
        assert_eq!(child(app.world(), first), a);
        assert_eq!(child(app.world(), second), b);
        assert_eq!(app.world().get::<Transform>(a).unwrap().translation, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(app.world().get::<Transform>(a).unwrap().scale, Vec3::splat(2.0));
        assert_eq!(app.world().get::<Transform>(b).unwrap().translation.x, 10.0);
    }

    #[test]
    fn authored_ids_preserve_identity_reorder_and_control_visibility() {
        #[derive(Component)]
        struct RuntimeOnly;
        let source = crate::UsdSource::new("ids.usda", &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(1,0,0), (2,0,0)]
    int[] protoIndices = [0,0]
    int64[] ids = [10,20]
}
"#[..]).unwrap();
        let live = LiveStage::new(source.open_stage().unwrap());
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/PI").unwrap();
        let child_for = |world: &World, id| world.get::<Children>(parent).unwrap().iter()
            .find(|child| world.get::<UsdInstanceId>(*child) == Some(&UsdInstanceId(id))).unwrap();
        let ten = child_for(&world, 10);
        let twenty = child_for(&world, 20);
        world.entity_mut(ten).insert(RuntimeOnly);
        let unrelated = world.spawn((RuntimeOnly, ChildOf(parent))).id();
        crate::authoring::set_attribute(&live.stage, "/PI", "ids", "int64[]", Value::Int64Vec(vec![20,10])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(child_for(&world, 10), ten);
        assert_eq!(child_for(&world, 20), twenty);
        assert_eq!(world.get::<Transform>(ten).unwrap().translation.x, 2.0);
        assert!(world.get::<RuntimeOnly>(ten).is_some());
        crate::authoring::set_attribute(&live.stage, "/PI", "ids", "int64[]", Value::Int64Vec(vec![10,10])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdInstancerWarning>(parent).is_some());
        assert_eq!(child_for(&world, 10), ten);
        crate::authoring::set_attribute(&live.stage, "/PI", "ids", "int64[]", Value::Int64Vec(vec![20,10])).unwrap();
        crate::authoring::set_attribute(&live.stage, "/PI", "invisibleIds", "int64[]", Value::Int64Vec(vec![20])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Visibility>(twenty), Some(&Visibility::Hidden));
        assert_eq!(child_for(&world, 10), ten);
        assert!(world.get::<UsdInstancerWarning>(parent).is_none());
        crate::authoring::set_attribute(&live.stage, "/PI", "invisibleIds", "int64[]", Value::Int64Vec(vec![])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(child_for(&world, 20), twenty);
        assert_ne!(world.get::<Visibility>(twenty), Some(&Visibility::Hidden));
        live.stage.prim("/PI").unwrap().set_type_name("Xform").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get_entity(ten).is_err());
        assert!(world.get::<RuntimeOnly>(unrelated).is_some());
    }

    #[test]
    fn point_instancer_spawns_instance_entities() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("pi.usda").unwrap();
        stage
            .define_prim("/PI")
            .unwrap()
            .set_type_name("PointInstancer")
            .unwrap();
        stage
            .create_attribute("/PI.positions", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [5.0, 0.0, 0.0].into(),
                [0.0, 5.0, 0.0].into(),
            ]))
            .unwrap();
        stage
            .create_attribute("/PI.protoIndices", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![0, 0, 0]))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let pi = map.entity("/PI").unwrap();
        let children: Vec<Entity> = world
            .get::<Children>(pi)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        let instances: Vec<Entity> = children
            .into_iter()
            .filter(|e| world.get::<UsdInstance>(*e).is_some())
            .collect();
        assert_eq!(instances.len(), 3, "one child entity per instance");

        // Second instance sits at (5,0,0).
        let at_5 = instances.iter().any(|e| {
            world
                .get::<Transform>(*e)
                .map(|t| (t.translation.x - 5.0).abs() < 1e-4)
                .unwrap_or(false)
        });
        assert!(at_5, "instance transform placed from positions");
    }

    #[test]
    fn reproject_clears_previous_instances() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("pi2.usda").unwrap();
        stage
            .define_prim("/PI")
            .unwrap()
            .set_type_name("PointInstancer")
            .unwrap();
        stage
            .create_attribute("/PI.positions", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![[0.0, 0.0, 0.0].into()]))
            .unwrap();
        stage
            .create_attribute("/PI.protoIndices", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![0]))
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let pi = map.entity("/PI").unwrap();

        // Re-run the route (as a resync would) and confirm no duplication.
        let registry = SchemaRegistry::builtin();
        let p = openusd::sdf::path("/PI").unwrap();
        registry.patch_prim(&live.stage, &p, &mut world, pi, &[]);

        let instances = world
            .get::<Children>(pi)
            .map(|c| {
                c.iter()
                    .filter(|e| world.get::<UsdInstance>(*e).is_some())
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(instances, 1, "reproject cleared and rebuilt, no stacking");
    }

    #[test]
    fn invisible_ids_are_culled() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("pi3.usda").unwrap();
        stage
            .define_prim("/PI")
            .unwrap()
            .set_type_name("PointInstancer")
            .unwrap();
        stage
            .create_attribute("/PI.positions", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 0.0, 0.0].into(),
                [2.0, 0.0, 0.0].into(),
            ]))
            .unwrap();
        stage
            .create_attribute("/PI.protoIndices", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![0, 0, 0]))
            .unwrap();
        // Hide instance index 1.
        stage
            .create_attribute("/PI.invisibleIds", "int64[]")
            .unwrap()
            .set(Value::Int64Vec(vec![1]))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let pi = map.entity("/PI").unwrap();
        let count = world
            .get::<Children>(pi)
            .map(|c| {
                c.iter()
                    .filter(|e| world.get::<UsdInstance>(*e).is_some())
                    .filter(|e| world.get::<Visibility>(*e) != Some(&Visibility::Hidden))
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(count, 2, "the invisible instance was culled (3 → 2)");
    }
}
