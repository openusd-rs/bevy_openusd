use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use mara::ui::modules::bevy as mara_bevy;
use usd_bevy::live::PrimEntities;
use bevy::dev_tools::infinite_grid::InfiniteGridSettings;
use crate::environment::ViewerGrid;

pub fn configure(app: &mut App) {
    app.add_systems(Last, frame_opened_document);
}

fn frame_opened_document(
    prims: Res<PrimEntities>,
    meshes: Query<(Entity, &Aabb, &GlobalTransform, &InheritedVisibility), With<Mesh3d>>,
    parents: Query<&ChildOf>,
    mut cameras: Query<(&mut mara_bevy::ChaseCamera, &mut Transform, &mut Projection), (With<Camera3d>, Without<ViewerGrid>)>,
    mut grids: Query<(&mut Transform, &mut InfiniteGridSettings), (With<ViewerGrid>, Without<Camera3d>)>,
    mut framed: Local<Option<Entity>>,
) {
    let Some(root) = prims.entity("/") else { return };
    if *framed == Some(root) { return; }
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for (entity, bounds, transform, visibility) in &meshes {
        if !belongs_to_document(entity, root, &parents) { continue; }
        if !visibility.get() { continue; }
        for corner in corners(bounds, transform) {
            if corner.is_finite() {
                low = low.min(corner);
                high = high.max(corner);
            }
        }
    }
    if !low.is_finite() || !high.is_finite() { return; }
    let mut applied = false;
    for (mut rig, mut transform, mut projection) in &mut cameras {
        let Projection::Perspective(perspective) = &mut *projection else { continue };
        let Some((focus, radius, distance)) = fit(low, high, perspective.fov, perspective.aspect_ratio) else { continue };
        rig.focus = focus;
        rig.min_distance = (radius * 0.01).max(0.0001);
        rig.max_distance = (distance * 1000.0).max(rig.max_distance);
        rig.distance = distance;
        perspective.near = (radius * 0.001).max(0.00001);
        perspective.far = (distance + radius * 100.0).max(1000.0);
        mara_bevy::apply_rig(&rig, &mut transform);
        applied = true;
    }
    if applied {
        for (mut transform, mut settings) in &mut grids {
            if let Some((height, scale, fade)) = crate::environment::fit_grid(low, high) {
                transform.translation.y = height;
                settings.scale = scale;
                settings.fadeout_distance = fade;
            }
        }
        *framed = Some(root);
    }
}

fn belongs_to_document(mut entity: Entity, root: Entity, parents: &Query<&ChildOf>) -> bool {
    for _ in 0..256 {
        if entity == root { return true; }
        let Ok(parent) = parents.get(entity) else { return false };
        entity = parent.parent();
    }
    false
}

fn corners(bounds: &Aabb, transform: &GlobalTransform) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        let sign = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 });
        transform.transform_point(Vec3::from(bounds.center) + Vec3::from(bounds.half_extents) * sign)
    })
}

fn fit(low: Vec3, high: Vec3, fov: f32, aspect: f32) -> Option<(Vec3, f32, f32)> {
    if !low.is_finite() || !high.is_finite() || !fov.is_finite() || !aspect.is_finite()
        || aspect <= 0.0 || fov <= 0.0 || fov >= std::f32::consts::PI || low.cmpgt(high).any() { return None; }
    let focus = low * 0.5 + high * 0.5;
    let radius = ((high - low).length() * 0.5).max(0.001);
    let half_angle = (fov * 0.5).min(((fov * 0.5).tan() * aspect).atan());
    let distance = radius * 1.15 / half_angle.sin();
    (focus.is_finite() && distance.is_finite()).then_some((focus, radius, distance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_children_contribute_bounds_but_unrelated_meshes_do_not() {
        let mut app = App::new();
        app.init_resource::<PrimEntities>();
        configure(&mut app);
        let root = app.world_mut().spawn_empty().id();
        let prim = app.world_mut().spawn(ChildOf(root)).id();
        app.world_mut().resource_mut::<PrimEntities>().insert("/", root);
        app.world_mut().resource_mut::<PrimEntities>().insert("/Mesh", prim);
        let child = app.world_mut().spawn((Mesh3d::default(), ChildOf(prim),
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::ONE),
            GlobalTransform::from_translation(Vec3::new(0.0, -5.0, 0.0)), InheritedVisibility::VISIBLE)).id();
        app.world_mut().spawn((Mesh3d::default(), Aabb::from_min_max(Vec3::splat(-1000.0), Vec3::splat(1000.0)),
            GlobalTransform::default(), InheritedVisibility::VISIBLE));
        let grid = app.world_mut().spawn((ViewerGrid, Transform::default(), InfiniteGridSettings::default())).id();
        let camera = app.world_mut().spawn((Camera3d::default(), mara_bevy::ChaseCamera::default(),
            Transform::default(), Projection::Perspective(PerspectiveProjection::default()))).id();
        app.update();
        assert_eq!(app.world().get::<mara_bevy::ChaseCamera>(camera).unwrap().focus, Vec3::new(0.0, -5.0, 0.0));
        let floor = app.world().get::<Transform>(grid).unwrap().translation.y;
        assert!(floor < -6.0 && floor > -6.01);
        assert_eq!(app.world().get::<GlobalTransform>(child).unwrap().translation(), Vec3::new(0.0, -5.0, 0.0));
    }

    #[test]
    fn opening_frames_once_and_preserves_subsequent_camera_input() {
        let mut app = App::new();
        app.init_resource::<PrimEntities>();
        configure(&mut app);
        let root = app.world_mut().spawn_empty().id();
        let mesh = app.world_mut().spawn((Mesh3d::default(),
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::ONE),
            GlobalTransform::from_translation(Vec3::new(20.0, 0.0, 0.0)),
            InheritedVisibility::VISIBLE, ChildOf(root))).id();
        app.world_mut().resource_mut::<PrimEntities>().insert("/", root);
        app.world_mut().resource_mut::<PrimEntities>().insert("/Mesh", mesh);
        let camera = app.world_mut().spawn((Camera3d::default(), mara_bevy::ChaseCamera::default(),
            Transform::default(), Projection::Perspective(PerspectiveProjection::default()))).id();
        app.update();
        assert_eq!(app.world().get::<mara_bevy::ChaseCamera>(camera).unwrap().focus, Vec3::new(20.0, 0.0, 0.0));
        app.world_mut().get_mut::<mara_bevy::ChaseCamera>(camera).unwrap().distance = 123.0;
        app.update();
        assert_eq!(app.world().get::<mara_bevy::ChaseCamera>(camera).unwrap().distance, 123.0);
        let replacement = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(mesh).insert(ChildOf(replacement));
        app.world_mut().resource_mut::<PrimEntities>().insert("/", replacement);
        app.update();
        assert_ne!(app.world().get::<mara_bevy::ChaseCamera>(camera).unwrap().distance, 123.0);
    }

    #[test]
    fn fitting_respects_narrow_viewports_and_scene_scale() {
        let wide = fit(Vec3::splat(-1.0), Vec3::ONE, 1.0, 2.0).unwrap();
        let narrow = fit(Vec3::splat(-1.0), Vec3::ONE, 1.0, 0.5).unwrap();
        assert_eq!(wide.0, Vec3::ZERO);
        assert!(narrow.2 > wide.2);
        let large = fit(Vec3::splat(-100.0), Vec3::splat(100.0), 1.0, 2.0).unwrap();
        assert!((large.2 / wide.2 - 100.0).abs() < 0.001);
        assert!(fit(Vec3::ZERO, Vec3::ONE, 1.0, 0.0).is_none());
        assert!(fit(Vec3::splat(f32::NAN), Vec3::ONE, 1.0, 1.0).is_none());
    }

    #[test]
    fn world_bounds_include_rotation_scale_and_translation() {
        let bounds = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::ONE);
        let transform = GlobalTransform::from(Transform::from_xyz(10.0, 0.0, 0.0)
            .with_scale(Vec3::new(2.0, 1.0, 3.0)).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)));
        let points = corners(&bounds, &transform);
        let low = points.into_iter().fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let high = points.into_iter().fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        assert!(low.abs_diff_eq(Vec3::new(7.0, -1.0, -2.0), 0.0001));
        assert!(high.abs_diff_eq(Vec3::new(13.0, 1.0, 2.0), 0.0001));
    }
}
