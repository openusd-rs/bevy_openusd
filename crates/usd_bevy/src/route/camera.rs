//! Camera route (PLAN P4): `UsdGeomCamera` → a [`Projection`] plus a
//! [`UsdCamera`] marker.
//!
//! Deliberately does **not** attach `Camera3d` / rendering components: a live
//! stage often has several cameras, and activating them would fight the app's
//! own viewport camera. The route projects the camera *parameters* (as a
//! `Projection`, positioned by the prim's `GlobalTransform`); the app decides
//! which — if any — to make active by querying [`UsdCamera`].

use bevy::prelude::*;
use bevy::camera::{CameraProjection, SubCameraView};
use bevy::math::Vec3A;
use openusd_schemas::geom::CameraSchema;
use std::f32::consts::PI;

use openusd_schemas::geom::Camera;
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// Marker on entities projected from a `UsdGeomCamera`. Carries no data — pair
/// it with the entity's [`Projection`] and `GlobalTransform`.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdCamera;

/// Maps `UsdGeomCamera` prims to a [`Projection`] + [`UsdCamera`] marker.
pub struct CameraRoute;

/// Off-axis perspective with vertical-FOV-preserving viewport resizing.
#[derive(Clone, Debug)]
pub struct UsdPerspectiveProjection {
    pub perspective: PerspectiveProjection,
    /// Aperture offsets divided by focal length, in view-space ray slopes.
    pub aperture_offset_over_focal: Vec2,
}

impl UsdPerspectiveProjection {
    fn shifted(&self, mut matrix: Mat4) -> Mat4 {
        matrix.z_axis.x += matrix.x_axis.x * self.aperture_offset_over_focal.x;
        matrix.z_axis.y += matrix.y_axis.y * self.aperture_offset_over_focal.y;
        matrix
    }
}

impl CameraProjection for UsdPerspectiveProjection {
    fn get_clip_from_view(&self) -> Mat4 { self.shifted(self.perspective.get_clip_from_view()) }
    fn get_clip_from_view_for_sub(&self, view: &SubCameraView) -> Mat4 {
        self.shifted(self.perspective.get_clip_from_view_for_sub(view))
    }
    fn update(&mut self, width: f32, height: f32) { self.perspective.update(width, height); }
    fn far(&self) -> f32 { self.perspective.far }
    fn get_frustum_corners(&self, near: f32, far: f32) -> [Vec3A;8] {
        self.perspective.get_frustum_corners(near, far).map(|mut corner| {
            corner.x += corner.z.abs() * self.aperture_offset_over_focal.x;
            corner.y += corner.z.abs() * self.aperture_offset_over_focal.y;
            corner
        })
    }
}

fn projection_of(ctx: &RouteCtx) -> Projection {
    let Ok(Some(cam)) = Camera::get(ctx.stage, ctx.path.clone()) else {
        return Projection::default();
    };
    // USD aperture + focal length share units (tenths of a scene unit / mm);
    // only their ratio sets the field of view, so the units cancel.
    let time = ctx.time.map(openusd::usd::TimeCode::new);
    let scalar = |attribute: openusd::usd::Attribute, fallback| attribute.get_at::<f32>(time).ok().flatten()
        .filter(|value| value.is_finite()).unwrap_or(fallback);
    let focal = scalar(cam.focal_length_attr(), 50.0).max(1e-3);
    let v_aperture = scalar(cam.vertical_aperture_attr(), 15.2908).max(1e-3);
    let h_aperture = scalar(cam.horizontal_aperture_attr(), 20.955).max(1e-3);
    let mut clip = match cam.clipping_range_attr().get_at::<Value>(time) {
        Ok(Some(Value::Vec2f(c))) => [c.x, c.y],
        Ok(Some(Value::Vec2d(c))) => [c.x as f32, c.y as f32],
        _ => [0.1, 1_000_000.0],
    };
    if !clip[0].is_finite() || !clip[1].is_finite() || clip[1] <= clip[0].max(1e-4) {
        clip = [0.1, 1_000_000.0];
    }
    let is_ortho = matches!(
        cam.projection_attr().get_at::<Value>(time),
        Ok(Some(Value::Token(t))) if t.as_str() == "orthographic"
    );

    if is_ortho {
        let width = h_aperture * 0.1;
        let height = v_aperture * 0.1;
        let offset = Vec2::new(scalar(cam.horizontal_aperture_offset_attr(), 0.0),
            scalar(cam.vertical_aperture_offset_attr(), 0.0)) * 0.1;
        let half = Vec2::new(width, height) * 0.5;
        Projection::Orthographic(OrthographicProjection {
            near: clip[0],
            far: clip[1],
            scaling_mode: bevy::camera::ScalingMode::Fixed { width, height },
            viewport_origin: Vec2::splat(0.5) - offset / Vec2::new(width, height),
            area: Rect::from_corners(offset - half, offset + half),
            ..OrthographicProjection::default_3d()
        })
    } else {
        // Vertical FOV from the aperture / focal-length ratio.
        let fov = 2.0 * (v_aperture / (2.0 * focal.max(1e-3))).atan();
        let perspective = PerspectiveProjection {
            fov: fov.clamp(1e-3, PI - 1e-3),
            near: clip[0].max(1e-4),
            far: clip[1],
            aspect_ratio: h_aperture / v_aperture,
            ..default()
        };
        let offset = Vec2::new(scalar(cam.horizontal_aperture_offset_attr(), 0.0),
            scalar(cam.vertical_aperture_offset_attr(), 0.0)) / focal;
        if offset == Vec2::ZERO || !offset.is_finite() { Projection::Perspective(perspective) }
        else { Projection::custom(UsdPerspectiveProjection { perspective, aperture_offset_over_focal: offset }) }
    }
}

impl PrimRoute for CameraRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get::<UsdCamera>(entity).is_some() {
            world.entity_mut(entity).remove::<(UsdCamera, Projection)>();
        }
    }

    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Camera")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let projection = projection_of(ctx);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert((UsdCamera, projection));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::sdf::Value;
    use openusd::usd::Stage;

    #[test]
    fn perspective_offsets_preserve_frustum_and_subview_geometry() {
        let mut projection = UsdPerspectiveProjection {
            perspective: PerspectiveProjection { fov: 1.0, aspect_ratio: 2.0, near: 0.5, far: 100.0, ..default() },
            aperture_offset_over_focal: Vec2::new(0.2,-0.1),
        };
        for size in [Vec2::new(1600.0,800.0), Vec2::new(800.0,1600.0)] {
            projection.update(size.x, size.y);
            let matrix = projection.get_clip_from_view();
            let corners = projection.get_frustum_corners(-0.5,-100.0);
            for (index, corner) in corners.into_iter().enumerate() {
                let projected = matrix.project_point3(corner.into());
                let expected = [Vec2::new(1.0,-1.0), Vec2::ONE, Vec2::new(-1.0,1.0), Vec2::NEG_ONE][index % 4];
                assert!(projected.truncate().abs_diff_eq(expected, 1e-5));
            }
            let center_ray = Vec3::new(0.8,-0.4,-4.0);
            assert!(matrix.project_point3(center_ray).truncate().abs_diff_eq(Vec2::ZERO, 1e-5));
            let sub = SubCameraView { full_size: size.as_uvec2(), offset: size * 0.25, size: (size * 0.5).as_uvec2() };
            let point = Vec3::new(0.3,0.2,-3.0);
            let full = matrix.project_point3(point);
            let cropped = projection.get_clip_from_view_for_sub(&sub).project_point3(point);
            assert!(cropped.truncate().abs_diff_eq(full.truncate() * 2.0, 1e-5));
            assert!((full.z-cropped.z).abs() < 1e-5);
        }
    }

    #[test]
    fn sampled_camera_lenses_preserve_orthographic_filmback() {
        use bevy::camera::CameraProjection;
        let source = crate::UsdSource::new("camera-samples.usda", &br#"#usda 1.0
def Camera "Cam" {
    token projection.timeSamples = { 0: "perspective", 10: "orthographic" }
    float focalLength.timeSamples = { 0: 50, 10: 150 }
    float horizontalAperture.timeSamples = { 0: 40, 10: 80 }
    float verticalAperture.timeSamples = { 0: 20, 10: 40 }
    float horizontalApertureOffset = 10
    float verticalApertureOffset = -5
    float2 clippingRange.timeSamples = { 0: (1,100), 10: (2,200) }
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Cam").unwrap();
        let Projection::Custom(custom) = projection_of(&RouteCtx::at(&stage, &path, Some(5.0))) else { panic!("perspective") };
        let shifted = custom.get::<UsdPerspectiveProjection>().unwrap();
        let perspective = &shifted.perspective;
        assert_eq!(shifted.aperture_offset_over_focal, Vec2::new(0.1,-0.05));
        assert!((perspective.fov - 2.0 * (30.0_f32 / 200.0).atan()).abs() < 1e-6);
        assert_eq!(perspective.aspect_ratio, 2.0);
        assert_eq!((perspective.near, perspective.far), (1.5, 150.0));
        let Projection::Orthographic(mut ortho) = projection_of(&RouteCtx::at(&stage, &path, Some(10.0))) else { panic!("orthographic") };
        assert_eq!((ortho.near, ortho.far), (2.0, 200.0));
        let area = Rect::from_corners(Vec2::new(-3.0,-2.5), Vec2::new(5.0,1.5));
        assert_eq!(ortho.area, area);
        ortho.update(1920.0, 1080.0);
        assert_eq!(ortho.area, area);
        ortho.update(500.0, 1000.0);
        assert_eq!(ortho.area, area);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::live::LiveStagePlugin));
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();
        let entity = app.world().resource::<PrimEntities>().entity("/Cam").unwrap();
        assert!(matches!(app.world().get::<Projection>(entity), Some(Projection::Custom(_))));
        app.world_mut().resource_mut::<crate::route::StageTime>().current = 10.0;
        app.update();
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Cam"), Some(entity));
        assert!(matches!(app.world().get::<Projection>(entity), Some(Projection::Orthographic(_))));
        assert!(app.world().get::<Camera3d>(entity).is_none());
    }

    #[test]
    fn camera_projects_perspective() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cam.usda").unwrap();
        stage
            .define_prim("/Cam")
            .unwrap()
            .set_type_name("Camera")
            .unwrap();
        stage
            .create_attribute("/Cam.focalLength", "float")
            .unwrap()
            .set(Value::Float(50.0))
            .unwrap();
        stage
            .create_attribute("/Cam.verticalAperture", "float")
            .unwrap()
            .set(Value::Float(24.0))
            .unwrap();
        stage
            .create_attribute("/Cam.clippingRange", "float2")
            .unwrap()
            .set(Value::Vec2f([0.5, 500.0].into()))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Cam").unwrap();
        assert!(world.get::<UsdCamera>(e).is_some(), "camera marker present");
        match world.get::<Projection>(e).expect("projection") {
            Projection::Perspective(p) => {
                let expected = 2.0 * (24.0f32 / (2.0 * 50.0)).atan();
                assert!((p.fov - expected).abs() < 1e-4, "fov from aperture/focal");
                assert!((p.near - 0.5).abs() < 1e-4);
                assert!((p.far - 500.0).abs() < 1e-2);
            }
            _ => panic!("expected a perspective projection"),
        }
    }

    #[test]
    fn does_not_attach_camera3d() {
        // The route must not create an active render camera.
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cam2.usda").unwrap();
        stage
            .define_prim("/Cam")
            .unwrap()
            .set_type_name("Camera")
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Cam").unwrap();
        assert!(world.get::<Camera3d>(e).is_none(), "no Camera3d attached");
    }
}
