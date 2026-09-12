//! Physics markers route (SCHEMA_INTEGRATION Phase D): UsdPhysics prims/APIs →
//! typed marker components.
//!
//! Bevy has no built-in physics, so this doesn't simulate anything — it
//! projects **markers** (read through openusd's `physics` schemas) that an app
//! can query to wire up its own backend (avian, rapier). This is the coverage
//! hook the plan calls for; a full physics integration is out of scope.

use bevy::prelude::*;
use openusd_schemas::physics::RevoluteJointSchema;
use openusd_schemas::physics::PrismaticJointSchema;

use openusd_schemas::physics::{
    CollisionAPI, DriveAPI, LimitAPI, MassAPI, PrismaticJoint, RevoluteJoint, RigidBodyAPI,
};
use openusd::sdf::Value;
use openusd::usd::Attribute;

use super::{PrimRoute, RouteCtx};
use crate::read::util::targets_at;

/// The prim has `UsdPhysicsRigidBodyAPI` applied.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdRigidBody;

/// The prim has `UsdPhysicsCollisionAPI` applied (a collider).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdCollider;

/// The prim is a `UsdPhysicsJoint` (or a typed subclass).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdPhysicsJoint;

/// Mass properties from `UsdPhysicsMassAPI` (Phase E). Data only — a backend
/// (avian/rapier) reads these to configure a body; usd_bevy simulates nothing.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct UsdMass {
    pub mass: Option<f32>,
    pub density: Option<f32>,
    pub center_of_mass: Option<Vec3>,
    pub diagonal_inertia: Option<Vec3>,
}

/// A typed physics joint's authored parameters (Phase E).
#[derive(Component, Debug, Clone, Default, PartialEq)]
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
}

/// One `UsdPhysicsDriveAPI` instance (a driven DOF).
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

/// All `UsdPhysicsDriveAPI` instances applied to a joint.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdDrives(pub Vec<UsdDrive>);

/// One `UsdPhysicsLimitAPI` instance (a limited DOF).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsdLimit {
    pub dof: String,
    pub low: Option<f32>,
    pub high: Option<f32>,
}

/// All `UsdPhysicsLimitAPI` instances applied to a joint.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdLimits(pub Vec<UsdLimit>);

fn f32_of(attr: Attribute) -> Option<f32> {
    attr.get::<f32>().ok().flatten()
}

fn vec3_of(attr: Attribute) -> Option<Vec3> {
    match attr.get::<Value>().ok().flatten()? {
        Value::Vec3f(v) => Some(Vec3::new(v.x, v.y, v.z)),
        Value::Vec3d(v) => Some(Vec3::new(v.x as f32, v.y as f32, v.z as f32)),
        _ => None,
    }
}

fn token_of(attr: Attribute) -> Option<String> {
    match attr.get::<Value>().ok().flatten()? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }
}

const JOINT_TYPES: &[&str] = &[
    "PhysicsJoint",
    "PhysicsFixedJoint",
    "PhysicsRevoluteJoint",
    "PhysicsPrismaticJoint",
    "PhysicsSphericalJoint",
    "PhysicsDistanceJoint",
];

/// Projects UsdPhysics schemas as marker components.
pub struct PhysicsRoute;

impl PrimRoute for PhysicsRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<(UsdRigidBody, UsdCollider, UsdPhysicsJoint, UsdMass, UsdJoint, UsdDrives, UsdLimits)>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        // Joints are typed; rigid-body/collision are applied API schemas.
        JOINT_TYPES.contains(&ctx.type_name.as_deref().unwrap_or_default())
            || matches!(RigidBodyAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)))
            || matches!(CollisionAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)))
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let type_name = ctx.type_name.as_deref().unwrap_or_default();
        let is_joint = JOINT_TYPES.contains(&type_name);
        let is_body = matches!(RigidBodyAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)));
        let is_collider = matches!(CollisionAPI::get(ctx.stage, ctx.path.clone()), Ok(Some(_)));

        // Enriched data (Phase E): mass on bodies; axis/limits/bodies/drives/
        // limits on joints.
        let mass = is_body.then(|| read_mass(ctx)).flatten();
        let joint = is_joint.then(|| read_joint(ctx, type_name));
        let drives = is_joint.then(|| read_drives(ctx)).filter(|d| !d.0.is_empty());
        let limits = is_joint.then(|| read_limits(ctx)).filter(|l| !l.0.is_empty());

        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.remove::<(UsdRigidBody, UsdCollider, UsdPhysicsJoint, UsdMass, UsdJoint, UsdDrives, UsdLimits)>();
            if is_body {
                e.insert(UsdRigidBody);
            }
            if is_collider {
                e.insert(UsdCollider);
            }
            if is_joint {
                e.insert(UsdPhysicsJoint);
            }
            if let Some(m) = mass {
                e.insert(m);
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
}

fn read_mass(ctx: &RouteCtx) -> Option<UsdMass> {
    let m = MassAPI::get(ctx.stage, ctx.path.clone()).ok().flatten()?;
    let mass = UsdMass {
        mass: f32_of(m.mass_attr()),
        density: f32_of(m.density_attr()),
        center_of_mass: vec3_of(m.center_of_mass_attr()),
        diagonal_inertia: vec3_of(m.diagonal_inertia_attr()),
    };
    // Only emit if something was actually authored.
    (mass != UsdMass::default()).then_some(mass)
}

fn read_joint(ctx: &RouteCtx, type_name: &str) -> UsdJoint {
    let kind = type_name
        .strip_prefix("Physics")
        .unwrap_or(type_name)
        .strip_suffix("Joint")
        .map(|s| if s.is_empty() { "joint" } else { s })
        .unwrap_or("joint")
        .to_lowercase();

    let (axis, lower, upper) = match type_name {
        "PhysicsRevoluteJoint" => match RevoluteJoint::get(ctx.stage, ctx.path.clone()) {
            Ok(Some(j)) => (
                token_of(j.axis_attr()),
                f32_of(j.lower_limit_attr()),
                f32_of(j.upper_limit_attr()),
            ),
            _ => (None, None, None),
        },
        "PhysicsPrismaticJoint" => match PrismaticJoint::get(ctx.stage, ctx.path.clone()) {
            Ok(Some(j)) => (
                token_of(j.axis_attr()),
                f32_of(j.lower_limit_attr()),
                f32_of(j.upper_limit_attr()),
            ),
            _ => (None, None, None),
        },
        _ => (None, None, None),
    };

    let body = |rel: &str| -> Option<String> {
        let path = ctx.path.append_property(rel).ok()?;
        targets_at(ctx.stage, &path)
            .ok()?
            .into_iter()
            .next()
            .map(|p| p.as_str().to_string())
    };

    UsdJoint {
        kind,
        axis,
        lower,
        upper,
        body0: body("physics:body0"),
        body1: body("physics:body1"),
    }
}

fn read_drives(ctx: &RouteCtx) -> UsdDrives {
    let drives = DriveAPI::get_all(&ctx.stage.prim(ctx.path.clone()).expect("validated USD path"))
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
    let limits = LimitAPI::get_all(&ctx.stage.prim(ctx.path.clone()).expect("validated USD path"))
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

    #[test]
    fn removed_physics_apis_clear_only_their_projected_state() {
        use crate::live::{LiveStage, PrimEntities, project_stage, apply_changes};
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("physics-removal.usda").unwrap();
        stage.define_prim("/Body").unwrap().set_type_name("Xform").unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI").unwrap()
            .add_applied_schema("PhysicsCollisionAPI").unwrap()
            .add_applied_schema("PhysicsMassAPI").unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Body").unwrap();
        assert!(world.get::<UsdRigidBody>(entity).is_some());
        assert!(world.get::<UsdCollider>(entity).is_some());
        assert!(world.get::<UsdMass>(entity).is_some());
        live.stage.prim("/Body").unwrap().set_metadata("apiSchemas", Value::TokenListOp(openusd::sdf::ListOp::explicit(vec!["PhysicsCollisionAPI".into()]))).unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdRigidBody>(entity).is_none());
        assert!(world.get::<UsdMass>(entity).is_none());
        assert!(world.get::<UsdCollider>(entity).is_some());
        live.stage.prim("/Body").unwrap().set_metadata("apiSchemas", Value::TokenListOp(openusd::sdf::ListOp::explicit(vec![]))).unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdCollider>(entity).is_none());
        live.stage.prim("/Body").unwrap().set_type_name("PhysicsFixedJoint").unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdPhysicsJoint>(entity).is_some());
        assert!(world.get::<UsdJoint>(entity).is_some());
        live.stage.prim("/Body").unwrap().set_type_name("Xform").unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdPhysicsJoint>(entity).is_none());
        assert!(world.get::<UsdJoint>(entity).is_none());
        assert_eq!(map.entity("/Body"), Some(entity));
        assert!(world.get::<Transform>(entity).is_some());
    }
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;

    #[test]
    fn physics_schemas_project_markers() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("phys.usda").unwrap();
        // A rigid body + collider.
        stage
            .define_prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI")
            .unwrap()
            .add_applied_schema("PhysicsCollisionAPI")
            .unwrap();
        // A joint.
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
        assert!(world.get::<UsdRigidBody>(body).is_some(), "rigid body marker");
        assert!(world.get::<UsdCollider>(body).is_some(), "collider marker");
        let joint = map.entity("/Joint").unwrap();
        assert!(
            world.get::<UsdPhysicsJoint>(joint).is_some(),
            "joint marker"
        );
    }

    #[test]
    fn joint_mass_and_drive_data_enriched() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("phys2.usda").unwrap();
        // Rigid body carrying mass properties.
        stage
            .define_prim("/Body")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .add_applied_schema("PhysicsRigidBodyAPI")
            .unwrap();
        let mass = MassAPI::apply(&stage.prim("/Body").unwrap()).unwrap();
        mass.create_mass_attr().unwrap().set(Value::Float(2.5)).unwrap();
        mass.create_center_of_mass_attr()
            .unwrap()
            .set(Value::Vec3f([0.0, 1.0, 0.0].into()))
            .unwrap();

        // Revolute joint with axis, limits, bodies, a drive + a limit.
        let joint = RevoluteJoint::define(&stage, "/Joint").unwrap();
        joint.create_axis_attr().unwrap().set(Value::Token("X".into())).unwrap();
        joint.create_lower_limit_attr().unwrap().set(Value::Float(-45.0)).unwrap();
        joint.create_upper_limit_attr().unwrap().set(Value::Float(45.0)).unwrap();
        stage
            .create_relationship("/Joint.physics:body0")
            .unwrap()
            .add_target(openusd::sdf::path("/Body").unwrap())
            .unwrap();
        let drive = DriveAPI::apply(&stage.prim("/Joint").unwrap(), "angular").unwrap();
        drive.create_target_position_attr().unwrap().set(Value::Float(10.0)).unwrap();
        drive.create_stiffness_attr().unwrap().set(Value::Float(100.0)).unwrap();
        let limit = LimitAPI::apply(&stage.prim("/Joint").unwrap(), "angular").unwrap();
        limit.create_low_attr().unwrap().set(Value::Float(-90.0)).unwrap();
        limit.create_high_attr().unwrap().set(Value::Float(90.0)).unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let body = map.entity("/Body").unwrap();
        let m = world.get::<UsdMass>(body).expect("mass marker");
        assert_eq!(m.mass, Some(2.5));
        assert_eq!(m.center_of_mass, Some(Vec3::new(0.0, 1.0, 0.0)));

        let j = map.entity("/Joint").unwrap();
        let jd = world.get::<UsdJoint>(j).expect("joint data");
        assert_eq!(jd.kind, "revolute");
        assert_eq!(jd.axis.as_deref(), Some("X"));
        assert_eq!(jd.lower, Some(-45.0));
        assert_eq!(jd.upper, Some(45.0));
        assert_eq!(jd.body0.as_deref(), Some("/Body"));

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
