//! Physics decode interface between `usd_bevy` (which reads the stage into
//! these records) and the rapier builders below. These structs used to live
//! in openusd's `physics` module; openusd #103 reorganized physics into typed
//! views, so the interchange types now live here in the consumer crate.

pub use openusd::schemas::physics::{CollisionApprox, Dof, DriveType};

/// USD physics joint kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JointKind {
    Fixed,
    Revolute,
    Prismatic,
    Spherical,
    Distance,
    Generic,
}

/// One authored `PhysicsLimitAPI` entry on a joint.
#[derive(Debug, Clone, Copy)]
pub struct ReadLimit {
    pub dof: Dof,
    pub low: f32,
    pub high: f32,
}

/// One authored `PhysicsDriveAPI` entry on a joint.
#[derive(Debug, Clone, Copy)]
pub struct ReadDrive {
    pub dof: Dof,
    pub drive_type: DriveType,
    pub target_position: Option<f32>,
    pub target_velocity: Option<f32>,
    pub stiffness: f32,
    pub damping: f32,
    pub max_force: Option<f32>,
}

/// A decoded physics joint, ready for the rapier builders.
#[derive(Debug, Clone)]
pub struct ReadJoint {
    pub path: String,
    pub kind: JointKind,
    pub body0: Option<String>,
    pub body1: Option<String>,
    pub local_pos0: [f32; 3],
    pub local_rot0: [f32; 4],
    pub local_pos1: [f32; 3],
    pub local_rot1: [f32; 4],
    pub axis: Option<String>,
    pub lower_limit: Option<f32>,
    pub upper_limit: Option<f32>,
    pub collision_enabled: bool,
    pub joint_enabled: bool,
    pub exclude_from_articulation: bool,
    pub break_force: Option<f32>,
    pub break_torque: Option<f32>,
    pub min_distance: Option<f32>,
    pub max_distance: Option<f32>,
    pub cone_angle_0: Option<f32>,
    pub cone_angle_1: Option<f32>,
    pub limits: Vec<ReadLimit>,
    pub drives: Vec<ReadDrive>,
}

// ── Stage physics reader ────────────────────────────────────────────────────
//
// Re-creates the reader openusd #103 removed, on top of the new typed physics
// views. Per-DOF `PhysicsLimitAPI`/`PhysicsDriveAPI` and collision-group
// membership need multi-apply / collection introspection and are deferred
// (see the `TODO`s); everything the viewer needs for basic joints/colliders
// is read here.

use openusd::gf;
use openusd::schemas::physics::{
    DriveAPI, LimitAPI,
    CollisionAPI, CollisionGroup, DistanceJoint, FilteredPairsAPI, FixedJoint, Joint, JointBase, MassAPI, MaterialAPI,
    MeshCollisionAPI, PrismaticJoint, RevoluteJoint, Scene, SphericalJoint,
};
use openusd::sdf::Path;
use openusd::usd::{Attribute, PrimPredicate, Relationship, Stage};

#[derive(Debug, Clone, Default)]
pub struct ReadScene {
    pub gravity_direction: Option<[f32; 3]>,
    pub gravity_magnitude: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadMass {
    pub mass: Option<f32>,
    pub density: Option<f32>,
    pub center_of_mass: Option<[f32; 3]>,
    pub diagonal_inertia: Option<[f32; 3]>,
    pub principal_axes: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadCollisionShape {
    pub approximation: Option<CollisionApprox>,
    pub physics_material_path: Option<String>,
    pub collision_enabled: bool,
    pub simulation_owner: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadPhysicsMaterial {
    pub static_friction: Option<f32>,
    pub dynamic_friction: Option<f32>,
    pub restitution: Option<f32>,
    pub density: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadFilteredPairs {
    pub filtered: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadCollisionGroup {
    pub merge_group: Option<String>,
    pub invert_filtered_groups: bool,
    pub members: Vec<String>,
    pub filtered_groups: Vec<String>,
}

/// Physics prim paths found on a stage, bucketed by role.
#[derive(Debug, Clone, Default)]
pub struct PhysicsPrims {
    pub joints: Vec<String>,
    pub rigid_bodies: Vec<String>,
    pub scenes: Vec<String>,
    pub articulation_roots: Vec<String>,
    pub materials: Vec<String>,
    pub collision_groups: Vec<String>,
    pub filtered_pairs: Vec<String>,
    pub colliders: Vec<String>,
}

fn af32(a: Attribute) -> Option<f32> {
    if let Ok(Some(v)) = a.get::<f32>() {
        return Some(v);
    }
    a.get::<f64>().ok().flatten().map(|d| d as f32)
}
fn abool(a: Attribute) -> Option<bool> {
    a.get::<bool>().ok().flatten()
}
fn atoken(a: Attribute) -> Option<String> {
    a.get::<String>().ok().flatten()
}
fn avec3(a: Attribute) -> Option<[f32; 3]> {
    a.get::<gf::Vec3f>().ok().flatten().map(|v| [v.x, v.y, v.z])
}
fn aquat(a: Attribute) -> Option<[f32; 4]> {
    a.get::<gf::Quatf>().ok().flatten().map(|q| [q.w, q.x, q.y, q.z])
}
fn rel_first(r: Relationship) -> Option<String> {
    r.targets().ok().and_then(|v| v.into_iter().next()).map(|p| p.as_str().to_string())
}
fn rel_all(r: Relationship) -> Vec<String> {
    r.targets().unwrap_or_default().into_iter().map(|p| p.as_str().to_string()).collect()
}

fn approx_from_token(t: &str) -> Option<CollisionApprox> {
    Some(match t {
        "none" => CollisionApprox::None,
        "convexHull" => CollisionApprox::ConvexHull,
        "convexDecomposition" => CollisionApprox::ConvexDecomposition,
        "boundingSphere" => CollisionApprox::BoundingSphere,
        "boundingCube" => CollisionApprox::BoundingCube,
        "meshSimplification" => CollisionApprox::MeshSimplification,
        _ => return None,
    })
}

pub fn read_physics_scene(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadScene>> {
    let Some(v) = Scene::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(Some(ReadScene {
        gravity_direction: avec3(v.gravity_direction_attr()),
        gravity_magnitude: af32(v.gravity_magnitude_attr()),
    }))
}

pub fn read_mass(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadMass>> {
    let Some(v) = MassAPI::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(Some(ReadMass {
        mass: af32(v.mass_attr()),
        density: af32(v.density_attr()),
        center_of_mass: avec3(v.center_of_mass_attr()),
        diagonal_inertia: avec3(v.diagonal_inertia_attr()),
        principal_axes: aquat(v.principal_axes_attr()),
    }))
}

pub fn read_collision_shape(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadCollisionShape>> {
    let Some(v) = CollisionAPI::get(stage, path.clone())? else {
        return Ok(None);
    };
    let approximation = MeshCollisionAPI::get(stage, path.clone())?
        .and_then(|m| atoken(m.approximation_attr()))
        .and_then(|t| approx_from_token(&t));
    let physics_material_path = rel_first(stage.prim(path.clone()).relationship("material:binding:physics"))
        .or_else(|| rel_first(stage.prim(path.clone()).relationship("material:binding")));
    Ok(Some(ReadCollisionShape {
        approximation,
        physics_material_path,
        collision_enabled: abool(v.collision_enabled_attr()).unwrap_or(true),
        // TODO: simulationOwner is a multi-target rel; introspection deferred.
        simulation_owner: None,
    }))
}

pub fn read_physics_material(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadPhysicsMaterial>> {
    let Some(v) = MaterialAPI::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(Some(ReadPhysicsMaterial {
        static_friction: af32(v.static_friction_attr()),
        dynamic_friction: af32(v.dynamic_friction_attr()),
        restitution: af32(v.restitution_attr()),
        density: af32(v.density_attr()),
    }))
}

pub fn read_filtered_pairs(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadFilteredPairs>> {
    let Some(v) = FilteredPairsAPI::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(Some(ReadFilteredPairs { filtered: rel_all(v.filtered_pairs_rel()) }))
}

pub fn read_collision_group(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadCollisionGroup>> {
    let Some(v) = CollisionGroup::get(stage, path.clone())? else {
        return Ok(None);
    };
    Ok(Some(ReadCollisionGroup {
        merge_group: atoken(v.merge_group_attr()),
        invert_filtered_groups: abool(v.invert_filtered_groups_attr()).unwrap_or(false),
        // TODO: group membership is a CollectionAPI; introspection deferred.
        members: Vec::new(),
        filtered_groups: rel_all(v.filtered_groups_rel()),
    }))
}

fn joint_common<J: JointBase>(v: &J, path: &Path, kind: JointKind) -> ReadJoint {
    ReadJoint {
        path: path.as_str().to_string(),
        kind,
        body0: rel_first(v.body0_rel()),
        body1: rel_first(v.body1_rel()),
        local_pos0: avec3(v.local_pos0_attr()).unwrap_or([0.0; 3]),
        local_rot0: aquat(v.local_rot0_attr()).unwrap_or([1.0, 0.0, 0.0, 0.0]),
        local_pos1: avec3(v.local_pos1_attr()).unwrap_or([0.0; 3]),
        local_rot1: aquat(v.local_rot1_attr()).unwrap_or([1.0, 0.0, 0.0, 0.0]),
        axis: None,
        lower_limit: None,
        upper_limit: None,
        collision_enabled: abool(v.collision_enabled_attr()).unwrap_or(false),
        joint_enabled: abool(v.joint_enabled_attr()).unwrap_or(true),
        exclude_from_articulation: abool(v.exclude_from_articulation_attr()).unwrap_or(false),
        break_force: af32(v.break_force_attr()),
        break_torque: af32(v.break_torque_attr()),
        min_distance: None,
        max_distance: None,
        cone_angle_0: None,
        cone_angle_1: None,
        // TODO: per-DOF PhysicsLimitAPI / PhysicsDriveAPI need multi-apply
        // introspection; deferred (built-in lower/upper limits still apply).
        limits: Vec::new(),
        drives: Vec::new(),
    }
}

pub fn read_joint(stage: &Stage, path: &Path) -> anyhow::Result<Option<ReadJoint>> {
    let ty = stage.prim(path.clone()).type_name()?.unwrap_or_default();
    let mut joint = match ty.as_str() {
        "PhysicsRevoluteJoint" => RevoluteJoint::get(stage, path.clone())?.map(|v| {
            let mut j = joint_common(&v, path, JointKind::Revolute);
            j.axis = atoken(v.axis_attr());
            j.lower_limit = af32(v.lower_limit_attr());
            j.upper_limit = af32(v.upper_limit_attr());
            j
        }),
        "PhysicsPrismaticJoint" => PrismaticJoint::get(stage, path.clone())?.map(|v| {
            let mut j = joint_common(&v, path, JointKind::Prismatic);
            j.axis = atoken(v.axis_attr());
            j.lower_limit = af32(v.lower_limit_attr());
            j.upper_limit = af32(v.upper_limit_attr());
            j
        }),
        "PhysicsSphericalJoint" => SphericalJoint::get(stage, path.clone())?.map(|v| {
            let mut j = joint_common(&v, path, JointKind::Spherical);
            j.axis = atoken(v.axis_attr());
            j.cone_angle_0 = af32(v.cone_angle0_limit_attr());
            j.cone_angle_1 = af32(v.cone_angle1_limit_attr());
            j
        }),
        "PhysicsDistanceJoint" => DistanceJoint::get(stage, path.clone())?.map(|v| {
            let mut j = joint_common(&v, path, JointKind::Distance);
            j.min_distance = af32(v.min_distance_attr());
            j.max_distance = af32(v.max_distance_attr());
            j
        }),
        "PhysicsFixedJoint" => FixedJoint::get(stage, path.clone())?.map(|v| joint_common(&v, path, JointKind::Fixed)),
        "PhysicsJoint" => Joint::get(stage, path.clone())?.map(|v| joint_common(&v, path, JointKind::Generic)),
        _ => return Ok(None),
    };
    if let Some(j) = joint.as_mut() {
        j.limits = read_joint_limits(stage, path);
        j.drives = read_joint_drives(stage, path);
    }
    Ok(joint)
}

/// Per-DOF `PhysicsLimitAPI` entries applied to a joint (multi-apply).
fn read_joint_limits(stage: &Stage, path: &Path) -> Vec<ReadLimit> {
    LimitAPI::get_all(stage, path.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|l| {
            let dof = Dof::from_token(l.name())?;
            Some(ReadLimit {
                dof,
                low: af32(l.low_attr()).unwrap_or(f32::NEG_INFINITY),
                high: af32(l.high_attr()).unwrap_or(f32::INFINITY),
            })
        })
        .collect()
}

/// Per-DOF `PhysicsDriveAPI` entries applied to a joint (multi-apply).
fn read_joint_drives(stage: &Stage, path: &Path) -> Vec<ReadDrive> {
    DriveAPI::get_all(stage, path.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|d| {
            let dof = Dof::from_token(d.name())?;
            let drive_type = atoken(d.type_attr())
                .and_then(|t| DriveType::from_token(&t))
                .unwrap_or(DriveType::Force);
            Some(ReadDrive {
                dof,
                drive_type,
                target_position: af32(d.target_position_attr()),
                target_velocity: af32(d.target_velocity_attr()),
                stiffness: af32(d.stiffness_attr()).unwrap_or(0.0),
                damping: af32(d.damping_attr()).unwrap_or(0.0),
                max_force: af32(d.max_force_attr()),
            })
        })
        .collect()
}

/// Walk the stage and bucket every physics prim by role (typed prims by
/// `typeName`, applied-API roles by `apiSchemas`).
pub fn find_physics_prims(stage: &Stage) -> anyhow::Result<PhysicsPrims> {
    use std::cell::RefCell;
    let out = RefCell::new(PhysicsPrims::default());
    stage.traverse(PrimPredicate::default(), |path| {
        let prim = stage.prim(path.clone());
        let ty = prim.type_name().ok().flatten().unwrap_or_default();
        let apis = prim.api_schemas().unwrap_or_default();
        let s = path.as_str().to_string();
        let mut o = out.borrow_mut();
        match ty.as_str() {
            "PhysicsScene" => o.scenes.push(s.clone()),
            "PhysicsCollisionGroup" => o.collision_groups.push(s.clone()),
            t if t.starts_with("Physics") && t.ends_with("Joint") => o.joints.push(s.clone()),
            _ => {}
        }
        let has = |name: &str| apis.iter().any(|a| a == name);
        if has("PhysicsRigidBodyAPI") {
            o.rigid_bodies.push(s.clone());
        }
        if has("PhysicsArticulationRootAPI") {
            o.articulation_roots.push(s.clone());
        }
        if has("PhysicsMaterialAPI") {
            o.materials.push(s.clone());
        }
        if has("PhysicsCollisionAPI") {
            o.colliders.push(s.clone());
        }
        if has("PhysicsFilteredPairsAPI") {
            o.filtered_pairs.push(s.clone());
        }
    })?;
    Ok(out.into_inner())
}
