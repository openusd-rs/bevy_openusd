//! UsdPhysics joint drawings, shown while [`UsdPhysicsOverlay`] is visible:
//! joint frame triads, joint axes, body connections and limit arcs.

use bevy::color::palettes::tailwind::{
    AMBER_400, BLUE_500, FUCHSIA_500, GREEN_500, ORANGE_500, RED_500, SKY_400, ZINC_400,
};
use bevy::prelude::*;

use crate::instance::UsdInstances;
use crate::route::physics::UsdJoint;

/// Whether physics joints are drawn.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct UsdPhysicsOverlay {
    pub visible: bool,
}

/// Gizmo group for physics joints, drawn over geometry.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct UsdPhysicsGizmos;

/// A joint posed in world space.
#[derive(Debug, Clone)]
pub struct PosedJoint {
    pub path: String,
    pub kind: String,
    /// The joint frame on body0.
    pub frame: Transform,
    /// World direction of the joint axis, for axis joints.
    pub axis: Option<Vec3>,
    /// Authored lower/upper limits: degrees for revolute, distance for prismatic.
    pub limits: Option<(f32, f32)>,
    pub body0: Option<Vec3>,
    pub body1: Option<Vec3>,
}

/// Draws every projected joint whose bodies are visible.
pub struct UsdPhysicsOverlayPlugin;

impl Plugin for UsdPhysicsOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UsdPhysicsOverlay>()
            .init_gizmo_group::<UsdPhysicsGizmos>()
            .add_systems(Startup, draw_on_top)
            .add_systems(
                PostUpdate,
                posed_joints
                    .pipe(draw)
                    .run_if(|overlay: Res<UsdPhysicsOverlay>| overlay.visible)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}

fn draw_on_top(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<UsdPhysicsGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 3.0;
}

/// Poses every projected joint in world space from its bodies' transforms.
pub fn posed_joints(
    instances: Option<NonSend<UsdInstances>>,
    joints: Query<(Entity, &UsdJoint)>,
    transforms: Query<&GlobalTransform>,
    visibility: Query<&InheritedVisibility>,
) -> Vec<PosedJoint> {
    let Some(instances) = instances else {
        return Vec::new();
    };
    let mut posed = Vec::new();
    for (entity, joint) in &joints {
        let Some((root, path)) = instances.roots().find_map(|root| {
            instances
                .path(root, entity)
                .map(|path| (root, path.to_string()))
        }) else {
            continue;
        };
        let body = |target: &Option<String>| {
            target
                .as_deref()
                .and_then(|target| instances.entity(root, target))
                .and_then(|body| transforms.get(body).ok().map(|world| (body, *world)))
        };
        let (body0, body1) = (body(&joint.body0), body(&joint.body1));
        let hidden = |body: Option<(Entity, GlobalTransform)>| {
            body.is_some_and(|(body, _)| visibility.get(body).is_ok_and(|visible| !visible.get()))
        };
        if hidden(body0) || hidden(body1) {
            continue;
        }
        let local = Transform::from_translation(joint.local_pos0).with_rotation(joint.local_rot0);
        let world = body0.map_or(GlobalTransform::IDENTITY, |(_, world)| world);
        let frame = world.mul_transform(local).compute_transform();
        let axis = match joint.axis.as_deref() {
            Some("X") => Some(Vec3::X),
            Some("Y") => Some(Vec3::Y),
            Some("Z") => Some(Vec3::Z),
            _ => None,
        };
        posed.push(PosedJoint {
            path,
            kind: joint.kind.clone(),
            frame,
            axis: axis.map(|axis| frame.rotation * axis),
            limits: joint.lower.zip(joint.upper),
            body0: body0.map(|(_, world)| world.translation()),
            body1: body1.map(|(_, world)| world.translation()),
        });
    }
    posed
}

fn draw(In(joints): In<Vec<PosedJoint>>, mut gizmos: Gizmos<UsdPhysicsGizmos>) {
    let links: Vec<f32> = joints
        .iter()
        .filter_map(|joint| Some(joint.body0?.distance(joint.body1?)))
        .filter(|length| *length > 0.0)
        .collect();
    let size = if links.is_empty() {
        0.1
    } else {
        0.3 * links.iter().sum::<f32>() / links.len() as f32
    };
    for joint in &joints {
        let anchor = joint.frame.translation;
        let rotation = joint.frame.rotation;
        for (axis, color) in [
            (Vec3::X, RED_500),
            (Vec3::Y, GREEN_500),
            (Vec3::Z, BLUE_500),
        ] {
            gizmos.line(anchor, anchor + rotation * axis * size, color);
        }
        let link = match joint.kind.as_str() {
            "revolute" => AMBER_400,
            "prismatic" => SKY_400,
            _ => ZINC_400,
        };
        for body in [joint.body0, joint.body1].into_iter().flatten() {
            gizmos.line(body, anchor, link);
        }
        let Some(axis) = joint.axis else {
            continue;
        };
        gizmos.arrow(anchor, anchor + axis * size * 1.5, FUCHSIA_500);
        match (joint.kind.as_str(), joint.limits) {
            ("revolute", Some((lower, upper))) => {
                let reference = rotation * perpendicular(rotation.inverse() * axis);
                let side = axis.cross(reference);
                let (lower, upper) = if upper - lower >= 360.0 {
                    (0.0, 360.0)
                } else {
                    (lower, upper)
                };
                let steps = ((upper - lower).abs() / 6.0).ceil().max(2.0) as usize;
                let radius = size * 0.7;
                let points: Vec<Vec3> = (0..=steps)
                    .map(|step| {
                        let angle =
                            (lower + (upper - lower) * step as f32 / steps as f32).to_radians();
                        anchor + (reference * angle.cos() + side * angle.sin()) * radius
                    })
                    .collect();
                if upper - lower < 360.0 {
                    gizmos.line(anchor, points[0], ORANGE_500);
                    gizmos.line(anchor, points[steps], ORANGE_500);
                }
                gizmos.linestrip(points, ORANGE_500);
            }
            ("prismatic", Some((lower, upper))) => {
                gizmos.line(anchor + axis * lower, anchor + axis * upper, ORANGE_500);
            }
            _ => {}
        }
    }
}

/// A unit vector perpendicular to a frame-local unit axis.
fn perpendicular(axis: Vec3) -> Vec3 {
    if axis.x.abs() > 0.9 { Vec3::Y } else { Vec3::X }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;

    #[test]
    fn joints_pose_in_world_space_and_hidden_bodies_are_skipped() {
        use crate::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};
        let text = r#"#usda 1.0
def Xform "Base" { double3 xformOp:translate = (2, 0, 0) uniform token[] xformOpOrder = ["xformOp:translate"] }
def Xform "Link" { double3 xformOp:translate = (2, 1, 0) uniform token[] xformOpOrder = ["xformOp:translate"] }
def Xform "Hidden" { token visibility = "invisible" }
def PhysicsRevoluteJoint "Shown"
{
    rel physics:body0 = </Base>
    rel physics:body1 = </Link>
    point3f physics:localPos0 = (0, 1, 0)
    quatf physics:localRot0 = (0.70710677, 0, 0, 0.70710677)
    uniform token physics:axis = "X"
    float physics:lowerLimit = -45
    float physics:upperLimit = 90
}
def PhysicsRevoluteJoint "Gone"
{
    rel physics:body0 = </Base>
    rel physics:body1 = </Hidden>
    uniform token physics:axis = "X"
}
"#;
        let source = UsdSource::snapshot("joints.usda", text.as_bytes()).unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::transform::TransformPlugin,
            bevy::camera::visibility::VisibilityPlugin,
            UsdPlugin,
            UsdAssetPlugin,
        ));
        app.init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<UsdScene>>()
            .add(UsdScene {
                source,
                textures: default(),
            });
        app.world_mut().spawn(UsdSceneRoot(scene));
        app.update();
        let joints = app
            .world_mut()
            .run_system_once(super::posed_joints)
            .unwrap();
        assert_eq!(joints.len(), 1, "{joints:?}");
        let joint = &joints[0];
        assert_eq!(joint.path, "/Shown");
        assert_eq!(joint.kind, "revolute");
        assert!(
            joint.frame.translation.distance(Vec3::new(2.0, 1.0, 0.0)) < 1e-5,
            "{joint:?}"
        );
        assert!(joint.axis.unwrap().distance(Vec3::Y) < 1e-5, "{joint:?}");
        assert_eq!(joint.limits, Some((-45.0, 90.0)));
        assert!(joint.body0.unwrap().distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-5);
        assert!(joint.body1.unwrap().distance(Vec3::new(2.0, 1.0, 0.0)) < 1e-5);
    }
}
