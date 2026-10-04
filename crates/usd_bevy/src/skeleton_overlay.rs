//! Bone lines over UsdSkel skeletons, drawn while [`UsdSkeletonOverlay`] is visible.

use bevy::color::palettes::tailwind::{CYAN_400, LIME_400};
use bevy::prelude::*;

use crate::instance::{UsdInstanceTime, UsdInstances};

/// Whether skeleton bones are drawn.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct UsdSkeletonOverlay {
    pub visible: bool,
}

/// Gizmo group for skeleton bones, drawn over geometry.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct UsdSkeletonGizmos;

/// Draws every projected skeleton at its root's current time.
pub struct UsdSkeletonOverlayPlugin;

impl Plugin for UsdSkeletonOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UsdSkeletonOverlay>()
            .init_gizmo_group::<UsdSkeletonGizmos>()
            .add_systems(Startup, draw_on_top)
            .add_systems(
                PostUpdate,
                draw.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn draw_on_top(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<UsdSkeletonGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 3.0;
}

fn draw(
    overlay: Res<UsdSkeletonOverlay>,
    instances: Option<NonSend<UsdInstances>>,
    times: Query<&UsdInstanceTime>,
    transforms: Query<&GlobalTransform>,
    mut gizmos: Gizmos<UsdSkeletonGizmos>,
) {
    let (true, Some(instances)) = (overlay.visible, instances) else {
        return;
    };
    for root in instances.roots() {
        let Some(stage) = instances.stage(root) else {
            continue;
        };
        let time = times.get(root).ok().map(|time| time.current);
        for (path, entity) in instances.prims(root) {
            let Ok(prim_path) = openusd::sdf::path(path) else {
                continue;
            };
            let skeleton = stage
                .prim(prim_path.clone())
                .ok()
                .and_then(|prim| prim.type_name().ok().flatten())
                .is_some_and(|kind| kind.as_str() == "Skeleton");
            if !skeleton {
                continue;
            }
            let (Ok(world), Ok(Some(pose))) = (
                transforms.get(entity),
                crate::read::skel::skeleton_pose(stage, &prim_path, time),
            ) else {
                continue;
            };
            let world = world.to_matrix();
            let points: Vec<Vec3> = pose
                .joints
                .iter()
                .map(|joint| world.transform_point3(joint.w_axis.truncate()))
                .collect();
            let bones: Vec<f32> = points
                .iter()
                .zip(&pose.parents)
                .filter_map(|(point, parent)| parent.map(|parent| point.distance(points[parent])))
                .filter(|length| *length > 0.0)
                .collect();
            let radius = 0.08 * bones.iter().sum::<f32>() / bones.len().max(1) as f32;
            for (point, parent) in points.iter().zip(&pose.parents) {
                if let Some(parent) = parent {
                    gizmos.line(points[*parent], *point, CYAN_400);
                }
                gizmos.sphere(Isometry3d::from_translation(*point), radius, LIME_400);
            }
        }
    }
}
