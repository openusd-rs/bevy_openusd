//! Per-draw posed centers for depth-sorted USD deformation.

use super::{deformation::CullingOverride, gpu_skin::DeformedSortTiming};
use bevy::{
    asset::AssetId,
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    mesh::{
        morph::MeshMorphWeights,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    pbr::Transmissive3d,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        render_phase::ViewSortedRenderPhases,
        sync_world::{MainEntity, MainEntityHashMap},
        view::RetainedViewEntity,
    },
};

#[derive(Component)]
struct CachedCenter {
    center: Option<Vec3>,
    skinned: bool,
    morphed: bool,
}

#[derive(Resource, Default)]
struct Centers(MainEntityHashMap<Vec3>);

type Original = (
    RetainedViewEntity,
    (Entity, MainEntity),
    TransparentSortingInfo3d,
);

#[derive(Resource, Default)]
struct Originals {
    transparent: Vec<Original>,
    transmissive: Vec<Original>,
}

#[derive(Resource)]
struct Configured;

pub(super) fn configure(app: &mut App) {
    if app.get_sub_app(RenderApp).is_none() || app.world().contains_resource::<Configured>() {
        return;
    }
    app.insert_resource(Configured)
        .add_systems(Last, update_centers);
    app.sub_app_mut(RenderApp)
        .init_resource::<Centers>()
        .init_resource::<Originals>()
        .add_systems(ExtractSchedule, extract_centers)
        .add_systems(
            Render,
            (
                restore_centers
                    .after(RenderSystems::PrepareViews)
                    .before(RenderSystems::Queue),
                apply_centers
                    .after(RenderSystems::Queue)
                    .before(RenderSystems::PhaseSort),
            ),
        );
}

pub(super) fn clear(world: &mut World, entity: Entity) {
    world.entity_mut(entity).remove::<CachedCenter>();
}

type Draws<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        Ref<'static, Mesh3d>,
        Ref<'static, GlobalTransform>,
        Option<Ref<'static, SkinnedMesh>>,
        Option<Ref<'static, MeshMorphWeights>>,
        Option<&'static mut CachedCenter>,
    ),
    With<CullingOverride>,
>;

fn update_centers(
    mut commands: Commands,
    mut draws: Draws,
    bounds: crate::mesh::bounds::MeshBounds,
    meshes: Res<Assets<Mesh>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut inverse_events: MessageReader<AssetEvent<SkinnedMeshInverseBindposes>>,
    timing: Option<ResMut<DeformedSortTiming>>,
) {
    let started = timing
        .as_ref()
        .map(|_| bevy::platform::time::Instant::now());
    let dirty_meshes: bevy::platform::collections::HashSet<_> =
        mesh_events.read().map(asset_id).collect();
    let dirty_inverse: bevy::platform::collections::HashSet<_> =
        inverse_events.read().map(asset_id).collect();
    let (mut rebuilt, mut vertices) = (0, 0);
    for (entity, mesh, transform, skin, morph, cached) in &mut draws {
        let skin_value = skin.as_deref();
        let morph_value = morph.as_deref();
        let changed = cached.as_ref().is_none_or(|cached| {
            cached.skinned != skin.is_some() || cached.morphed != morph.is_some()
        }) || mesh.is_changed()
            || transform.is_changed()
            || skin.as_ref().is_some_and(|skin| {
                skin.is_changed() || dirty_inverse.contains(&skin.inverse_bindposes.id())
            })
            || morph.as_ref().is_some_and(|morph| morph.is_changed())
            || dirty_meshes.contains(&mesh.0.id())
            || bounds.pose_changed(skin_value, morph_value);
        if !changed {
            continue;
        }
        let center = if skin.is_some() || morph.is_some() {
            bounds.sort_center(&mesh, &transform, skin_value, morph_value)
        } else {
            None
        };
        rebuilt += 1;
        vertices += meshes.get(&mesh.0).map_or(0, Mesh::count_vertices) as u64;
        let value = CachedCenter {
            center,
            skinned: skin.is_some(),
            morphed: morph.is_some(),
        };
        if let Some(mut cached) = cached {
            *cached = value;
        } else {
            commands.entity(entity).insert(value);
        }
    }
    if let (Some(started), Some(mut timing)) = (started, timing) {
        timing.updates += 1;
        timing.rebuilt += rebuilt;
        timing.vertices += vertices;
        timing.elapsed += started.elapsed();
    }
}

fn asset_id<A: Asset>(event: &AssetEvent<A>) -> AssetId<A> {
    match *event {
        AssetEvent::Added { id }
        | AssetEvent::Modified { id }
        | AssetEvent::Removed { id }
        | AssetEvent::Unused { id }
        | AssetEvent::LoadedWithDependencies { id } => id,
    }
}

fn extract_centers(
    mut centers: ResMut<Centers>,
    draws: Extract<Query<(Entity, &CachedCenter), With<CullingOverride>>>,
) {
    centers.0.clear();
    centers.0.extend(
        draws
            .iter()
            .filter_map(|(entity, cached)| cached.center.map(|center| (entity.into(), center))),
    );
    let len = centers.0.len();
    if len < centers.0.capacity() / 4 {
        centers.0.shrink_to(len.saturating_mul(2));
    }
}

fn set_center(sorting: &mut TransparentSortingInfo3d, center: Vec3) {
    if let TransparentSortingInfo3d::Sorted { mesh_center, .. } = sorting {
        *mesh_center = center;
    }
}

fn apply_centers(
    centers: Res<Centers>,
    mut originals: ResMut<Originals>,
    transparent: Option<ResMut<ViewSortedRenderPhases<Transparent3d>>>,
    transmissive: Option<ResMut<ViewSortedRenderPhases<Transmissive3d>>>,
) {
    if let Some(mut phases) = transparent {
        for (&view, phase) in &mut phases.0 {
            for item in phase.items.values_mut() {
                if let Some(&center) = centers.0.get(&item.entity.1) {
                    originals
                        .transparent
                        .push((view, item.entity, item.sorting_info));
                    set_center(&mut item.sorting_info, center);
                }
            }
        }
    }
    if let Some(mut phases) = transmissive {
        for (&view, phase) in &mut phases.0 {
            for item in phase.items.values_mut() {
                if let Some(&center) = centers.0.get(&item.entity.1) {
                    originals
                        .transmissive
                        .push((view, item.entity, item.sorting_info));
                    set_center(&mut item.sorting_info, center);
                }
            }
        }
    }
    let originals = &mut *originals;
    for values in [&mut originals.transparent, &mut originals.transmissive] {
        if values.len() < values.capacity() / 4 {
            values.shrink_to(values.len().saturating_mul(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce,
        mesh::{
            PrimitiveTopology,
            morph::{MorphAttributes, MorphWeights},
        },
    };

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<SkinnedMeshInverseBindposes>()
            .init_resource::<DeformedSortTiming>()
            .add_systems(Last, update_centers);
        app
    }

    fn mesh() -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.set_morph_targets(vec![
            MorphAttributes::new(
                Vec3::X * 2.0,
                Vec3::ZERO,
                Vec3::ZERO
            );
            3
        ]);
        mesh
    }

    fn center(world: &World, entity: Entity) -> Option<Vec3> {
        world.get::<CachedCenter>(entity).unwrap().center
    }

    #[test]
    fn cached_centers_track_shared_assets_weights_and_owner_lifetimes() {
        let mut app = app();
        let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh());
        let weights = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.5], None).unwrap())
            .id();
        let a = app
            .world_mut()
            .spawn((
                Mesh3d(handle.clone()),
                GlobalTransform::IDENTITY,
                MeshMorphWeights::Reference(weights),
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                Mesh3d(handle.clone()),
                GlobalTransform::from_translation(Vec3::X * 10.0),
                MeshMorphWeights::Value { weights: vec![1.0] },
            ))
            .id();
        for entity in [a, b] {
            super::super::deformation::acquire(
                app.world_mut(),
                entity,
                super::super::deformation::MORPH,
            );
        }
        app.update();
        assert_eq!(center(app.world(), a), Some(Vec3::new(3.0, 0.5, 0.0)));
        assert_eq!(center(app.world(), b), Some(Vec3::new(14.0, 0.5, 0.0)));
        let rebuilt = app.world().resource::<DeformedSortTiming>().rebuilt;
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<DeformedSortTiming>().rebuilt,
            rebuilt
        );
        app.world_mut()
            .get_mut::<MorphWeights>(weights)
            .unwrap()
            .weights_mut()[0] = 0.75;
        app.update();
        assert_eq!(center(app.world(), a), Some(Vec3::new(3.5, 0.5, 0.0)));
        assert_eq!(center(app.world(), b), Some(Vec3::new(14.0, 0.5, 0.0)));
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .get_mut(&handle)
            .unwrap()
            .insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [8.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            );
        app.update();
        assert_eq!(center(app.world(), a), Some(Vec3::new(5.5, 0.5, 0.0)));
        assert_eq!(center(app.world(), b), Some(Vec3::new(16.0, 0.5, 0.0)));
        app.world_mut().entity_mut(b).remove::<MeshMorphWeights>();
        app.update();
        assert_eq!(center(app.world(), b), None);
        super::super::deformation::release(app.world_mut(), a, super::super::deformation::MORPH);
        assert!(app.world().get::<CachedCenter>(a).is_none());
        app.world_mut().despawn(a);
        app.world_mut().despawn(b);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&CachedCenter>()
                .iter(app.world())
                .count(),
            0
        );
        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&handle)
                .unwrap()
                .final_aabb
                .is_none()
        );
    }

    #[test]
    fn posed_centers_use_local_aabbs_and_invalidate_joint_and_bind_changes() {
        use bevy::{
            camera::primitives::{Aabb, MeshAabb},
            mesh::VertexAttributeValues,
        };
        let mut app = app();
        let mut mesh = mesh();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(vec![[0; 4]; 3]),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0, 0.0, 0.0, 0.0]; 3]);
        let original = mesh.clone();
        let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 0.5, 1.0),
            Quat::from_rotation_z(0.7),
            Vec3::new(4.0, -2.0, 1.0),
        );
        let local_pose = Mat4::from_translation(Vec3::X * 3.0);
        let joint = app
            .world_mut()
            .spawn(GlobalTransform::from(transform * local_pose))
            .id();
        let inverse = app
            .world_mut()
            .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
            .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY]));
        let entity = app
            .world_mut()
            .spawn((
                Mesh3d(handle.clone()),
                GlobalTransform::from(transform),
                SkinnedMesh {
                    inverse_bindposes: inverse.clone(),
                    joints: vec![joint],
                },
            ))
            .id();
        super::super::deformation::acquire(
            app.world_mut(),
            entity,
            super::super::deformation::SKIN,
        );
        app.update();
        let expected = transform.transform_point3(Vec3::new(5.0, 0.5, 0.0));
        assert!(
            center(app.world(), entity)
                .unwrap()
                .abs_diff_eq(expected, 1e-4)
        );
        let positions = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let world_aabb =
            Aabb::enclosing(positions.map(|p| (transform * local_pose).transform_point3(p.into())))
                .unwrap();
        assert!(!Vec3::from(world_aabb.center).abs_diff_eq(expected, 0.01));
        let rebuilt = app.world().resource::<DeformedSortTiming>().rebuilt;
        app.update();
        assert_eq!(
            app.world().resource::<DeformedSortTiming>().rebuilt,
            rebuilt
        );
        *app.world_mut().get_mut::<GlobalTransform>(joint).unwrap() =
            GlobalTransform::from(transform * Mat4::from_translation(Vec3::Y));
        app.update();
        assert!(
            center(app.world(), entity)
                .unwrap()
                .abs_diff_eq(transform.transform_point3(Vec3::new(2.0, 1.5, 0.0)), 1e-4)
        );
        *app.world_mut()
            .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
            .get_mut(&inverse)
            .unwrap() = SkinnedMeshInverseBindposes::from(vec![Mat4::from_translation(Vec3::Z)]);
        app.update();
        assert!(
            center(app.world(), entity)
                .unwrap()
                .abs_diff_eq(transform.transform_point3(Vec3::new(2.0, 1.5, 1.0)), 1e-4)
        );
        assert_eq!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&handle)
                .unwrap()
                .get_aabb(),
            original.get_aabb()
        );
        app.world_mut().despawn(joint);
        app.update();
        assert_eq!(center(app.world(), entity), None);
    }

    #[test]
    fn phase_overrides_restore_retained_keys_and_preserve_other_draws() {
        use bevy::render::{
            render_phase::{DrawFunctionId, PhaseItemExtraIndex, SortedRenderPhase},
            render_resource::CachedRenderPipelineId,
        };
        let mut world = World::new();
        let main = world.spawn_empty().id();
        let grid = world.spawn_empty().id();
        let view = RetainedViewEntity {
            main_entity: main.into(),
            auxiliary_entity: None,
            subview_index: 0,
        };
        let sorting = || TransparentSortingInfo3d::Sorted {
            mesh_center: Vec3::X,
            depth_bias: 0.25,
        };
        let make = |entity| Transparent3d {
            sorting_info: sorting(),
            distance: 0.0,
            pipeline: CachedRenderPipelineId::INVALID,
            entity: (Entity::PLACEHOLDER, entity),
            draw_function: DrawFunctionId(0),
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        };
        let mut transparent = ViewSortedRenderPhases::<Transparent3d>::default();
        let mut phase = SortedRenderPhase::default();
        phase.add_retained(make(main.into()));
        phase.add_retained(make(grid.into()));
        transparent.0.insert(view, phase);
        let mut transmissive = ViewSortedRenderPhases::<Transmissive3d>::default();
        let mut phase = SortedRenderPhase::default();
        phase.add_retained(Transmissive3d {
            sorting_info: sorting(),
            distance: 0.0,
            pipeline: CachedRenderPipelineId::INVALID,
            entity: (Entity::PLACEHOLDER, main.into()),
            draw_function: DrawFunctionId(0),
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        });
        transmissive.0.insert(view, phase);
        world.insert_resource(transparent);
        world.insert_resource(transmissive);
        world.init_resource::<Originals>();
        world.insert_resource(Centers([(main.into(), Vec3::Y)].into_iter().collect()));
        world.run_system_once(apply_centers).unwrap();
        let assert_key = |info, center| match info {
            TransparentSortingInfo3d::Sorted {
                mesh_center,
                depth_bias,
            } => {
                assert_eq!(mesh_center, center);
                assert_eq!(depth_bias, 0.25);
            }
            _ => panic!("sorting mode changed"),
        };
        for item in world.resource::<ViewSortedRenderPhases<Transparent3d>>().0[&view]
            .items
            .values()
        {
            assert_key(
                item.sorting_info,
                if item.entity.1 == main.into() {
                    Vec3::Y
                } else {
                    Vec3::X
                },
            );
        }
        assert_key(
            world.resource::<ViewSortedRenderPhases<Transmissive3d>>().0[&view]
                .items
                .values()
                .next()
                .unwrap()
                .sorting_info,
            Vec3::Y,
        );
        world.resource_mut::<Centers>().0.clear();
        world.run_system_once(restore_centers).unwrap();
        world.run_system_once(apply_centers).unwrap();
        for item in world.resource::<ViewSortedRenderPhases<Transparent3d>>().0[&view]
            .items
            .values()
        {
            assert_key(item.sorting_info, Vec3::X);
        }
        assert_key(
            world.resource::<ViewSortedRenderPhases<Transmissive3d>>().0[&view]
                .items
                .values()
                .next()
                .unwrap()
                .sorting_info,
            Vec3::X,
        );
        assert!(world.resource::<Originals>().transparent.is_empty());
        let mut top = TransparentSortingInfo3d::AlwaysOnTop;
        set_center(&mut top, Vec3::Y);
        assert!(matches!(top, TransparentSortingInfo3d::AlwaysOnTop));
    }
}

fn restore_centers(
    mut originals: ResMut<Originals>,
    transparent: Option<ResMut<ViewSortedRenderPhases<Transparent3d>>>,
    transmissive: Option<ResMut<ViewSortedRenderPhases<Transmissive3d>>>,
) {
    if let Some(mut phases) = transparent {
        for (view, entity, sorting) in originals.transparent.drain(..) {
            if let Some(item) = phases
                .0
                .get_mut(&view)
                .and_then(|phase| phase.items.get_mut(&entity))
            {
                item.sorting_info = sorting;
            }
        }
    } else {
        originals.transparent.clear();
    }
    if let Some(mut phases) = transmissive {
        for (view, entity, sorting) in originals.transmissive.drain(..) {
            if let Some(item) = phases
                .0
                .get_mut(&view)
                .and_then(|phase| phase.items.get_mut(&entity))
            {
                item.sorting_info = sorting;
            }
        }
    } else {
        originals.transmissive.clear();
    }
}
