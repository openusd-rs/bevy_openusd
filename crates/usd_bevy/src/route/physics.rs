//! Physics route: UsdPhysics prims and APIs → data components.
//!
//! Bevy has no built-in physics, so this simulates nothing. It projects
//! everything a backend (avian, rapier) needs to build a scene: bodies,
//! mass, colliders with their shape, materials, joints with their local
//! frames, drives, limits, filtered pairs, collision groups, articulation
//! roots and the scene's gravity. Values stay in the stage's authored units
//! (`metersPerUnit`, `kilogramsPerUnit`, degrees); the backend converts.
//! Only authored opinions count: UsdPhysics schema fallbacks such as a zero
//! `principalAxes` or an infinite `centerOfMass` are placeholders, not
//! values, and read as `None`.

use bevy::prelude::*;
use openusd::sdf::Value;
use openusd::usd::{Attribute, ResolveInfoSource};
use openusd_schemas::physics::{CollisionAPI, DriveAPI, LimitAPI, MassAPI, RigidBodyAPI};

use super::{PrimRoute, RouteCtx};
use crate::read::util::targets_at;

/// A `PhysicsScene` prim: gravity for the simulation it owns.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdPhysicsScene {
    /// `physics:gravityDirection`, in stage axes.
    pub gravity_direction: Option<Vec3>,
    /// `physics:gravityMagnitude`, in stage units per second squared.
    pub gravity_magnitude: Option<f32>,
}

/// The prim has `PhysicsRigidBodyAPI` applied.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct UsdRigidBody {
    pub enabled: bool,
    pub kinematic: bool,
    pub starts_asleep: bool,
    pub velocity: Option<Vec3>,
    pub angular_velocity: Option<Vec3>,
    /// `physics:simulationOwner` target, a `PhysicsScene` prim path.
    pub simulation_owner: Option<String>,
}

impl Default for UsdRigidBody {
    fn default() -> Self {
        Self {
            enabled: true,
            kinematic: false,
            starts_asleep: false,
            velocity: None,
            angular_velocity: None,
            simulation_owner: None,
        }
    }
}

/// The geometry a collider prim describes, from its type and shape
/// attributes. A `Mesh` collider's vertices are the prim's projected mesh.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum UsdColliderShape {
    Cube {
        size: f32,
    },
    Sphere {
        radius: f32,
    },
    Cylinder {
        radius: f32,
        height: f32,
        axis: String,
    },
    Capsule {
        radius: f32,
        height: f32,
        axis: String,
    },
    Cone {
        radius: f32,
        height: f32,
        axis: String,
    },
    Plane,
    Mesh,
    /// Any other prim type (an `Xform` whose meshes sit below it, say).
    #[default]
    Other,
}

/// The prim has `PhysicsCollisionAPI` applied.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct UsdCollider {
    pub enabled: bool,
    pub shape: UsdColliderShape,
    /// `physics:approximation` when `PhysicsMeshCollisionAPI` is applied.
    pub approximation: Option<String>,
    /// The bound physics material prim (`material:binding:physics`, else
    /// `material:binding`).
    pub material: Option<String>,
    pub simulation_owner: Option<String>,
}

impl Default for UsdCollider {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: UsdColliderShape::Other,
            approximation: None,
            material: None,
            simulation_owner: None,
        }
    }
}

/// The prim has `PhysicsMaterialAPI` applied.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdPhysicsMaterial {
    pub static_friction: Option<f32>,
    pub dynamic_friction: Option<f32>,
    pub restitution: Option<f32>,
    pub density: Option<f32>,
}

/// The prim has `PhysicsArticulationRootAPI` applied.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdArticulationRoot;

/// The prim has `PhysicsFilteredPairsAPI` applied: contacts with the
/// listed prims are dropped.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdCollisionFilter {
    pub filtered: Vec<String>,
}

/// A `PhysicsCollisionGroup` prim.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdCollisionGroup {
    /// `collection:colliders:includes` targets.
    pub members: Vec<String>,
    /// `physics:filteredGroups` targets.
    pub filtered_groups: Vec<String>,
    pub merge_group: Option<String>,
    pub invert_filtered_groups: bool,
}

/// The prim is a `PhysicsJoint` (or a typed subclass).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdPhysicsJoint;

/// Mass properties from `PhysicsMassAPI`, authored fields only.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct UsdMass {
    pub mass: Option<f32>,
    pub density: Option<f32>,
    pub center_of_mass: Option<Vec3>,
    pub diagonal_inertia: Option<Vec3>,
    /// The inertia frame, when a real rotation is authored.
    pub principal_axes: Option<Quat>,
}

/// A joint's authored parameters.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct UsdJoint {
    /// Short kind: `"revolute"`, `"prismatic"`, `"fixed"`, `"spherical"`,
    /// `"distance"`, or `"joint"` for the base type.
    pub kind: String,
    /// Rotation/translation axis (`"X"`/`"Y"`/`"Z"`) for revolute/prismatic.
    pub axis: Option<String>,
    pub lower: Option<f32>,
    pub upper: Option<f32>,
    /// The two bodies the joint connects (`physics:body0`/`body1` targets).
    pub body0: Option<String>,
    pub body1: Option<String>,
    /// Joint frame in each body's frame (`physics:localPos0/1`,
    /// `physics:localRot0/1`).
    pub local_pos0: Vec3,
    pub local_rot0: Quat,
    pub local_pos1: Vec3,
    pub local_rot1: Quat,
    pub enabled: bool,
    pub collision_enabled: bool,
    pub exclude_from_articulation: bool,
    pub break_force: Option<f32>,
    pub break_torque: Option<f32>,
    /// Spherical joints: `physics:coneAngle0Limit`/`coneAngle1Limit`.
    pub cone_angle0: Option<f32>,
    pub cone_angle1: Option<f32>,
    /// Distance joints: `physics:minDistance`/`maxDistance`.
    pub min_distance: Option<f32>,
    pub max_distance: Option<f32>,
}

impl Default for UsdJoint {
    fn default() -> Self {
        Self {
            kind: "joint".to_string(),
            axis: None,
            lower: None,
            upper: None,
            body0: None,
            body1: None,
            local_pos0: Vec3::ZERO,
            local_rot0: Quat::IDENTITY,
            local_pos1: Vec3::ZERO,
            local_rot1: Quat::IDENTITY,
            enabled: true,
            collision_enabled: false,
            exclude_from_articulation: false,
            break_force: None,
            break_torque: None,
            cone_angle0: None,
            cone_angle1: None,
            min_distance: None,
            max_distance: None,
        }
    }
}

/// One `PhysicsDriveAPI` instance (a driven DOF).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsdDrive {
    /// The driven DOF name (`"linear"`, `"angular"`, `"transX"`, …).
    pub dof: String,
    pub drive_type: Option<String>,
    pub target_position: Option<f32>,
    pub target_velocity: Option<f32>,
    pub stiffness: Option<f32>,
    pub damping: Option<f32>,
    pub max_force: Option<f32>,
}

/// All `PhysicsDriveAPI` instances applied to a joint.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdDrives(pub Vec<UsdDrive>);

/// One `PhysicsLimitAPI` instance (a limited DOF).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsdLimit {
    pub dof: String,
    pub low: Option<f32>,
    pub high: Option<f32>,
}

/// All `PhysicsLimitAPI` instances applied to a joint.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdLimits(pub Vec<UsdLimit>);

type Everything = (
    UsdPhysicsScene,
    UsdRigidBody,
    UsdCollider,
    UsdPhysicsMaterial,
    UsdArticulationRoot,
    UsdCollisionFilter,
    UsdCollisionGroup,
    UsdPhysicsJoint,
    UsdMass,
    UsdJoint,
    UsdDrives,
    UsdLimits,
);

// ── authored-only reads ────────────────────────────────────────────────

fn authored(attr: Attribute) -> Option<Value> {
    match attr.resolve_info().ok()?.source() {
        ResolveInfoSource::Default
        | ResolveInfoSource::TimeSamples
        | ResolveInfoSource::ValueClips => attr.get::<Value>().ok().flatten(),
        _ => None,
    }
}

fn f32_of(attr: Attribute) -> Option<f32> {
    match authored(attr)? {
        Value::Float(f) => Some(f),
        Value::Double(d) => Some(d as f32),
        Value::Int(i) => Some(i as f32),
        _ => None,
    }
}

fn bool_of(attr: Attribute) -> Option<bool> {
    match authored(attr)? {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

fn vec3_of(attr: Attribute) -> Option<Vec3> {
    match authored(attr)? {
        Value::Vec3f(v) => Some(Vec3::new(v.x, v.y, v.z)),
        Value::Vec3d(v) => Some(Vec3::new(v.x as f32, v.y as f32, v.z as f32)),
        _ => None,
    }
}

/// Quaternions author as `(w, x, y, z)`; a zero quaternion is no rotation.
fn quat_of(attr: Attribute) -> Option<Quat> {
    let q = match authored(attr)? {
        Value::Quatf(q) => Quat::from_xyzw(q.x, q.y, q.z, q.w),
        Value::Quatd(q) => Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32),
        _ => return None,
    };
    (q.length() > 1e-3).then(|| q.normalize())
}

fn token_of(attr: Attribute) -> Option<String> {
    match authored(attr)? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn attr(ctx: &RouteCtx, name: &str) -> Attribute {
    ctx.stage
        .prim(ctx.path.clone())
        .expect("validated USD path")
        .attribute(name)
}

fn rel_targets(ctx: &RouteCtx, name: &str) -> Vec<String> {
    ctx.path
        .append_property(name)
        .ok()
        .and_then(|path| targets_at(ctx.stage, &path).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

fn rel_first(ctx: &RouteCtx, name: &str) -> Option<String> {
    rel_targets(ctx, name).into_iter().next()
}

fn api_schemas(ctx: &RouteCtx) -> Vec<String> {
    ctx.stage
        .prim(ctx.path.clone())
        .ok()
        .and_then(|prim| prim.api_schemas().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.as_str().to_string())
        .collect()
}

const JOINT_TYPES: &[&str] = &[
    "PhysicsJoint",
    "PhysicsFixedJoint",
    "PhysicsRevoluteJoint",
    "PhysicsPrismaticJoint",
    "PhysicsSphericalJoint",
    "PhysicsDistanceJoint",
];

/// Projects UsdPhysics schemas as data components.
pub struct PhysicsRoute;

impl PrimRoute for PhysicsRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<Everything>();
    }

    fn matches(&self, ctx: &RouteCtx) -> bool {
        let type_name = ctx.type_name.as_deref().unwrap_or_default();
        JOINT_TYPES.contains(&type_name)
            || matches!(type_name, "PhysicsScene" | "PhysicsCollisionGroup")
            || api_schemas(ctx).iter().any(|s| {
                matches!(
                    s.as_str(),
                    "PhysicsRigidBodyAPI"
                        | "PhysicsCollisionAPI"
                        | "PhysicsMassAPI"
                        | "PhysicsMaterialAPI"
                        | "PhysicsArticulationRootAPI"
                        | "PhysicsFilteredPairsAPI"
                )
            })
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let type_name = ctx.type_name.as_deref().unwrap_or_default().to_string();
        let schemas = api_schemas(ctx);
        let has = |name: &str| schemas.iter().any(|s| s == name);
        let is_joint = JOINT_TYPES.contains(&type_name.as_str());
        let is_body = matches!(RigidBodyAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)));
        let is_collider = matches!(CollisionAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)));

        let scene = (type_name == "PhysicsScene").then(|| UsdPhysicsScene {
            gravity_direction: vec3_of(attr(ctx, "physics:gravityDirection")),
            gravity_magnitude: f32_of(attr(ctx, "physics:gravityMagnitude")),
        });
        let body = is_body.then(|| read_body(ctx));
        let mass = has("PhysicsMassAPI").then(|| read_mass(ctx)).flatten();
        let collider =
            is_collider.then(|| read_collider(ctx, &type_name, has("PhysicsMeshCollisionAPI")));
        let material = has("PhysicsMaterialAPI").then(|| UsdPhysicsMaterial {
            static_friction: f32_of(attr(ctx, "physics:staticFriction")),
            dynamic_friction: f32_of(attr(ctx, "physics:dynamicFriction")),
            restitution: f32_of(attr(ctx, "physics:restitution")),
            density: f32_of(attr(ctx, "physics:density")),
        });
        let filter = has("PhysicsFilteredPairsAPI").then(|| UsdCollisionFilter {
            filtered: rel_targets(ctx, "physics:filteredPairs"),
        });
        let group = (type_name == "PhysicsCollisionGroup").then(|| UsdCollisionGroup {
            members: rel_targets(ctx, "collection:colliders:includes"),
            filtered_groups: rel_targets(ctx, "physics:filteredGroups"),
            merge_group: token_of(attr(ctx, "physics:mergeGroup")),
            invert_filtered_groups: bool_of(attr(ctx, "physics:invertFilteredGroups"))
                .unwrap_or(false),
        });
        let joint = is_joint.then(|| read_joint(ctx, &type_name));
        let drives = is_joint
            .then(|| read_drives(ctx))
            .filter(|d| !d.0.is_empty());
        let limits = is_joint
            .then(|| read_limits(ctx))
            .filter(|l| !l.0.is_empty());

        let Ok(mut e) = world.get_entity_mut(entity) else {
            return;
        };
        e.remove::<Everything>();
        if let Some(s) = scene {
            e.insert(s);
        }
        if let Some(b) = body {
            e.insert(b);
        }
        if let Some(c) = collider {
            e.insert(c);
        }
        if let Some(m) = material {
            e.insert(m);
        }
        if has("PhysicsArticulationRootAPI") {
            e.insert(UsdArticulationRoot);
        }
        if let Some(f) = filter {
            e.insert(f);
        }
        if let Some(g) = group {
            e.insert(g);
        }
        if let Some(m) = mass {
            e.insert(m);
        }
        if is_joint {
            e.insert(UsdPhysicsJoint);
        }
        if let Some(j) = joint {
            e.insert(j);
        }
        if let Some(d) = drives {
            e.insert(d);
        }
        if let Some(l) = limits {
            e.insert(l);
        }
    }
}

fn read_body(ctx: &RouteCtx) -> UsdRigidBody {
    UsdRigidBody {
        enabled: bool_of(attr(ctx, "physics:rigidBodyEnabled")).unwrap_or(true),
        kinematic: bool_of(attr(ctx, "physics:kinematicEnabled")).unwrap_or(false),
        starts_asleep: bool_of(attr(ctx, "physics:startsAsleep")).unwrap_or(false),
        velocity: vec3_of(attr(ctx, "physics:velocity")),
        angular_velocity: vec3_of(attr(ctx, "physics:angularVelocity")),
        simulation_owner: rel_first(ctx, "physics:simulationOwner"),
    }
}

/// `Some` whenever `PhysicsMassAPI` is applied, fields only when authored.
fn read_mass(ctx: &RouteCtx) -> Option<UsdMass> {
    let m = MassAPI::get(ctx.stage, ctx.path.clone()).ok().flatten()?;
    Some(UsdMass {
        mass: f32_of(m.mass_attr()),
        density: f32_of(m.density_attr()),
        center_of_mass: vec3_of(m.center_of_mass_attr()),
        diagonal_inertia: vec3_of(m.diagonal_inertia_attr()),
        principal_axes: quat_of(m.principal_axes_attr()),
    })
}

fn read_collider(ctx: &RouteCtx, type_name: &str, mesh_api: bool) -> UsdCollider {
    let radius = || f32_of(attr(ctx, "radius")).unwrap_or(1.0);
    let height = || f32_of(attr(ctx, "height")).unwrap_or(2.0);
    let axis = || token_of(attr(ctx, "axis")).unwrap_or_else(|| "Z".to_string());
    let shape = match type_name {
        "Cube" => UsdColliderShape::Cube {
            size: f32_of(attr(ctx, "size")).unwrap_or(2.0),
        },
        "Sphere" => UsdColliderShape::Sphere { radius: radius() },
        "Cylinder" => UsdColliderShape::Cylinder {
            radius: radius(),
            height: height(),
            axis: axis(),
        },
        "Capsule" => UsdColliderShape::Capsule {
            radius: radius(),
            height: height(),
            axis: axis(),
        },
        "Cone" => UsdColliderShape::Cone {
            radius: radius(),
            height: height(),
            axis: axis(),
        },
        "Plane" => UsdColliderShape::Plane,
        "Mesh" => UsdColliderShape::Mesh,
        _ => UsdColliderShape::Other,
    };
    UsdCollider {
        enabled: bool_of(attr(ctx, "physics:collisionEnabled")).unwrap_or(true),
        shape,
        approximation: mesh_api
            .then(|| token_of(attr(ctx, "physics:approximation")))
            .flatten(),
        material: rel_first(ctx, "material:binding:physics")
            .or_else(|| rel_first(ctx, "material:binding")),
        simulation_owner: rel_first(ctx, "physics:simulationOwner"),
    }
}

fn read_joint(ctx: &RouteCtx, type_name: &str) -> UsdJoint {
    let kind = type_name
        .strip_prefix("Physics")
        .unwrap_or(type_name)
        .strip_suffix("Joint")
        .map(|s| if s.is_empty() { "joint" } else { s })
        .unwrap_or("joint")
        .to_lowercase();
    let axis_joint = matches!(type_name, "PhysicsRevoluteJoint" | "PhysicsPrismaticJoint");
    UsdJoint {
        kind,
        axis: axis_joint
            .then(|| token_of(attr(ctx, "physics:axis")))
            .flatten(),
        lower: axis_joint
            .then(|| f32_of(attr(ctx, "physics:lowerLimit")))
            .flatten(),
        upper: axis_joint
            .then(|| f32_of(attr(ctx, "physics:upperLimit")))
            .flatten(),
        body0: rel_first(ctx, "physics:body0"),
        body1: rel_first(ctx, "physics:body1"),
        local_pos0: vec3_of(attr(ctx, "physics:localPos0")).unwrap_or(Vec3::ZERO),
        local_rot0: quat_of(attr(ctx, "physics:localRot0")).unwrap_or(Quat::IDENTITY),
        local_pos1: vec3_of(attr(ctx, "physics:localPos1")).unwrap_or(Vec3::ZERO),
        local_rot1: quat_of(attr(ctx, "physics:localRot1")).unwrap_or(Quat::IDENTITY),
        enabled: bool_of(attr(ctx, "physics:jointEnabled")).unwrap_or(true),
        collision_enabled: bool_of(attr(ctx, "physics:collisionEnabled")).unwrap_or(false),
        exclude_from_articulation: bool_of(attr(ctx, "physics:excludeFromArticulation"))
            .unwrap_or(false),
        break_force: f32_of(attr(ctx, "physics:breakForce")),
        break_torque: f32_of(attr(ctx, "physics:breakTorque")),
        cone_angle0: f32_of(attr(ctx, "physics:coneAngle0Limit")),
        cone_angle1: f32_of(attr(ctx, "physics:coneAngle1Limit")),
        min_distance: f32_of(attr(ctx, "physics:minDistance")),
        max_distance: f32_of(attr(ctx, "physics:maxDistance")),
    }
}

fn read_drives(ctx: &RouteCtx) -> UsdDrives {
    let drives = DriveAPI::get_all(
        &ctx.stage
            .prim(ctx.path.clone())
            .expect("validated USD path"),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|d| UsdDrive {
        dof: d.name().to_string(),
        drive_type: token_of(d.type_attr()),
        target_position: f32_of(d.target_position_attr()),
        target_velocity: f32_of(d.target_velocity_attr()),
        stiffness: f32_of(d.stiffness_attr()),
        damping: f32_of(d.damping_attr()),
        max_force: f32_of(d.max_force_attr()),
    })
    .collect();
    UsdDrives(drives)
}

fn read_limits(ctx: &RouteCtx) -> UsdLimits {
    let limits = LimitAPI::get_all(
        &ctx.stage
            .prim(ctx.path.clone())
            .expect("validated USD path"),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|l| UsdLimit {
        dof: l.name().to_string(),
        low: f32_of(l.low_attr()),
        high: f32_of(l.high_attr()),
    })
    .collect();
    UsdLimits(limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, apply_changes, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;
    use openusd_schemas::physics::{RevoluteJoint, RevoluteJointSchema};

    fn physics_stage(name: &str) -> Stage {
        Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .in_memory(name)
            .unwrap()
    }

    #[test]
    fn removed_physics_apis_clear_only_their_projected_state() {
        let stage = physics_stage("physics-removal.usda");
        stage
            .define_prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI")
            .unwrap()
            .add_applied_schema("PhysicsCollisionAPI")
            .unwrap()
            .add_applied_schema("PhysicsMassAPI")
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Body").unwrap();
        assert!(world.get::<UsdRigidBody>(entity).is_some());
        assert!(world.get::<UsdCollider>(entity).is_some());
        assert!(world.get::<UsdMass>(entity).is_some());
        live.stage
            .prim("/Body")
            .unwrap()
            .set_metadata(
                "apiSchemas",
                Value::TokenListOp(openusd::sdf::ListOp::explicit(vec![
                    "PhysicsCollisionAPI".into(),
                ])),
            )
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdRigidBody>(entity).is_none());
        assert!(world.get::<UsdMass>(entity).is_none());
        assert!(world.get::<UsdCollider>(entity).is_some());
        live.stage
            .prim("/Body")
            .unwrap()
            .set_metadata(
                "apiSchemas",
                Value::TokenListOp(openusd::sdf::ListOp::explicit(vec![])),
            )
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdCollider>(entity).is_none());
        live.stage
            .prim("/Body")
            .unwrap()
            .set_type_name("PhysicsFixedJoint")
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdPhysicsJoint>(entity).is_some());
        assert!(world.get::<UsdJoint>(entity).is_some());
        live.stage
            .prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdPhysicsJoint>(entity).is_none());
        assert!(world.get::<UsdJoint>(entity).is_none());
        assert_eq!(map.entity("/Body"), Some(entity));
        assert!(world.get::<Transform>(entity).is_some());
    }

    #[test]
    fn physics_schemas_project_markers() {
        let stage = physics_stage("phys.usda");
        stage
            .define_prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI")
            .unwrap()
            .add_applied_schema("PhysicsCollisionAPI")
            .unwrap();
        stage
            .define_prim("/Joint")
            .unwrap()
            .set_type_name("PhysicsRevoluteJoint")
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let body = map.entity("/Body").unwrap();
        let rb = world.get::<UsdRigidBody>(body).expect("rigid body marker");
        assert!(rb.enabled && !rb.kinematic);
        let collider = world.get::<UsdCollider>(body).expect("collider marker");
        assert_eq!(collider.shape, UsdColliderShape::Other);
        let joint = map.entity("/Joint").unwrap();
        assert!(
            world.get::<UsdPhysicsJoint>(joint).is_some(),
            "joint marker"
        );
    }

    #[test]
    fn joint_mass_and_drive_data_enriched() {
        let stage = physics_stage("phys2.usda");
        stage
            .define_prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI")
            .unwrap();
        let mass = MassAPI::apply(&stage.prim("/Body").unwrap()).unwrap();
        mass.create_mass_attr()
            .unwrap()
            .set(Value::Float(2.5))
            .unwrap();
        mass.create_center_of_mass_attr()
            .unwrap()
            .set(Value::Vec3f([0.0, 1.0, 0.0].into()))
            .unwrap();

        let joint = RevoluteJoint::define(&stage, "/Joint").unwrap();
        joint
            .create_axis_attr()
            .unwrap()
            .set(Value::Token("X".into()))
            .unwrap();
        joint
            .create_lower_limit_attr()
            .unwrap()
            .set(Value::Float(-45.0))
            .unwrap();
        joint
            .create_upper_limit_attr()
            .unwrap()
            .set(Value::Float(45.0))
            .unwrap();
        stage
            .create_relationship("/Joint.physics:body0")
            .unwrap()
            .add_target(openusd::sdf::path("/Body").unwrap())
            .unwrap();
        stage
            .prim("/Joint")
            .unwrap()
            .create_attribute("physics:localPos0", "point3f")
            .unwrap()
            .set(Value::Vec3f([1.0, 2.0, 3.0].into()))
            .unwrap();
        let drive = DriveAPI::apply(&stage.prim("/Joint").unwrap(), "angular").unwrap();
        drive
            .create_target_position_attr()
            .unwrap()
            .set(Value::Float(10.0))
            .unwrap();
        drive
            .create_stiffness_attr()
            .unwrap()
            .set(Value::Float(100.0))
            .unwrap();
        let limit = LimitAPI::apply(&stage.prim("/Joint").unwrap(), "angular").unwrap();
        limit
            .create_low_attr()
            .unwrap()
            .set(Value::Float(-90.0))
            .unwrap();
        limit
            .create_high_attr()
            .unwrap()
            .set(Value::Float(90.0))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let body = map.entity("/Body").unwrap();
        let m = world.get::<UsdMass>(body).expect("mass marker");
        assert_eq!(m.mass, Some(2.5));
        assert_eq!(m.center_of_mass, Some(Vec3::new(0.0, 1.0, 0.0)));
        // Schema fallbacks (zero principalAxes, unset inertia) are not values.
        assert_eq!(m.principal_axes, None);
        assert_eq!(m.diagonal_inertia, None);

        let j = map.entity("/Joint").unwrap();
        let jd = world.get::<UsdJoint>(j).expect("joint data");
        assert_eq!(jd.kind, "revolute");
        assert_eq!(jd.axis.as_deref(), Some("X"));
        assert_eq!(jd.lower, Some(-45.0));
        assert_eq!(jd.upper, Some(45.0));
        assert_eq!(jd.body0.as_deref(), Some("/Body"));
        assert_eq!(jd.local_pos0, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(jd.local_rot0, Quat::IDENTITY);
        assert!(jd.enabled && !jd.exclude_from_articulation);

        let drives = world.get::<UsdDrives>(j).expect("drives");
        assert_eq!(drives.0.len(), 1);
        assert_eq!(drives.0[0].dof, "angular");
        assert_eq!(drives.0[0].target_position, Some(10.0));
        assert_eq!(drives.0[0].stiffness, Some(100.0));

        let limits = world.get::<UsdLimits>(j).expect("limits");
        assert_eq!(limits.0.len(), 1);
        assert_eq!(limits.0[0].low, Some(-90.0));
        assert_eq!(limits.0[0].high, Some(90.0));
    }
}
