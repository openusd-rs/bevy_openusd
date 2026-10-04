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

/// A skeleton posed in world space: joint positions and each joint's parent.
#[derive(Debug, Clone)]
pub struct PosedSkeleton {
    pub path: String,
    pub joints: Vec<Vec3>,
    pub parents: Vec<Option<usize>>,
}

/// Draws every visible projected skeleton at its root's current time.
pub struct UsdSkeletonOverlayPlugin;

impl Plugin for UsdSkeletonOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UsdSkeletonOverlay>()
            .init_gizmo_group::<UsdSkeletonGizmos>()
            .add_systems(Startup, draw_on_top)
            .add_systems(
                PostUpdate,
                visible_skeletons
                    .pipe(draw)
                    .run_if(|overlay: Res<UsdSkeletonOverlay>| overlay.visible)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}

fn draw_on_top(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<UsdSkeletonGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 3.0;
}

/// Poses every projected skeleton whose prim is visible, at its root's time.
pub fn visible_skeletons(
    instances: Option<NonSend<UsdInstances>>,
    times: Query<&UsdInstanceTime>,
    transforms: Query<&GlobalTransform>,
    visibility: Query<&InheritedVisibility>,
) -> Vec<PosedSkeleton> {
    let Some(instances) = instances else {
        return Vec::new();
    };
    let mut skeletons = Vec::new();
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
            if !skeleton || visibility.get(entity).is_ok_and(|visible| !visible.get()) {
                continue;
            }
            let (Ok(world), Ok(Some(pose))) = (
                transforms.get(entity),
                crate::read::skel::skeleton_pose(stage, &prim_path, time),
            ) else {
                continue;
            };
            let world = world.to_matrix();
            skeletons.push(PosedSkeleton {
                path: path.to_string(),
                joints: pose
                    .joints
                    .iter()
                    .map(|joint| world.transform_point3(joint.w_axis.truncate()))
                    .collect(),
                parents: pose.parents,
            });
        }
    }
    skeletons
}

fn draw(In(skeletons): In<Vec<PosedSkeleton>>, mut gizmos: Gizmos<UsdSkeletonGizmos>) {
    for skeleton in skeletons {
        let points = &skeleton.joints;
        let bones: Vec<f32> = points
            .iter()
            .zip(&skeleton.parents)
            .filter_map(|(point, parent)| parent.map(|parent| point.distance(points[parent])))
            .filter(|length| *length > 0.0)
            .collect();
        let radius = 0.08 * bones.iter().sum::<f32>() / bones.len().max(1) as f32;
        for (point, parent) in points.iter().zip(&skeleton.parents) {
            if let Some(parent) = parent {
                gizmos.line(points[*parent], *point, CYAN_400);
            }
            gizmos.sphere(Isometry3d::from_translation(*point), radius, LIME_400);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;

    #[test]
    fn hidden_skeletons_are_skipped_and_shown_ones_follow_their_clock() {
        use crate::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};
        let skeleton = |name: &str, hidden: &str| {
            format!(
                r#"def Xform "{name}"
{{
    {hidden}
    def SkelRoot "Rig"
    {{
        def Skeleton "Skel"
        {{
            uniform token[] joints = ["Root", "Root/Tip"]
            uniform matrix4d[] bindTransforms = [((1,0,0,0),(0,1,0,0),(0,0,1,0),(0,0,0,1)), ((1,0,0,0),(0,1,0,0),(0,0,1,0),(0,1,0,1))]
            uniform matrix4d[] restTransforms = [((1,0,0,0),(0,1,0,0),(0,0,1,0),(0,0,0,1)), ((1,0,0,0),(0,1,0,0),(0,0,1,0),(0,1,0,1))]
            rel skel:animationSource = </{name}/Rig/Skel/Anim>
            def SkelAnimation "Anim"
            {{
                uniform token[] joints = ["Root", "Root/Tip"]
                float3[] translations.timeSamples = {{ 0: [(0,0,0), (0,1,0)], 10: [(0,0,0), (0,2,0)] }}
                quatf[] rotations = [(1,0,0,0), (1,0,0,0)]
                half3[] scales = [(1,1,1), (1,1,1)]
            }}
        }}
    }}
}}
"#
            )
        };
        let text = format!(
            "#usda 1.0\n{}{}",
            skeleton("Shown", ""),
            skeleton("Hidden", "token visibility = \"invisible\"")
        );
        let source = UsdSource::snapshot("rigs.usda", text.into_bytes()).unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
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
        let root = app.world_mut().spawn(UsdSceneRoot(scene)).id();
        for (time, tip) in [(0.0, 1.0), (10.0, 2.0)] {
            app.world_mut()
                .get_mut::<crate::instance::UsdInstanceTime>(root)
                .unwrap()
                .current = time;
            app.update();
            let skeletons = app
                .world_mut()
                .run_system_once(super::visible_skeletons)
                .unwrap();
            assert_eq!(skeletons.len(), 1, "{skeletons:?}");
            assert_eq!(skeletons[0].path, "/Shown/Rig/Skel");
            assert_eq!(skeletons[0].parents, [None, Some(0)]);
            assert!(
                (skeletons[0].joints[1].y - tip).abs() < 1e-5,
                "{skeletons:?} at {time}"
            );
        }
    }
}
