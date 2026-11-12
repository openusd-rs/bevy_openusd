use bevy::camera::CameraUpdateSystems;
use bevy::dev_tools::infinite_grid::{InfiniteGrid, InfiniteGridPlugin, InfiniteGridSettings};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, SimulationLightSystems};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

pub struct ViewerEnvironmentPlugin;

#[derive(Component)]
pub struct ViewerGrid;

#[derive(Component)]
pub struct StudioLight(pub f32);

/// World-space bounds of the displayed scene; shadow cascades span them.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneBounds(pub Option<(Vec3, Vec3)>);

/// Nearest and farthest distance from `eye` to the box `low..high`.
pub fn depth_range(eye: Vec3, low: Vec3, high: Vec3) -> Option<(f32, f32)> {
    if !eye.is_finite() || !low.is_finite() || !high.is_finite() || low.cmpgt(high).any() {
        return None;
    }
    let nearest = eye.distance(eye.clamp(low, high));
    let farthest = (eye - low).abs().max((eye - high).abs()).length();
    (nearest.is_finite() && farthest.is_finite()).then_some((nearest, farthest))
}

/// Four cascades from `near` to just past `farthest`, the first ending a
/// twentieth of the way from `nearest`.
pub fn shadow_cascades(nearest: f32, farthest: f32, near: f32) -> Option<CascadeShadowConfig> {
    if !nearest.is_finite() || !farthest.is_finite() || !near.is_finite() || farthest <= 0.0 {
        return None;
    }
    let maximum = farthest * 1.05;
    let nearest = nearest.clamp(0.0, farthest);
    let first = nearest + (maximum - nearest) * 0.05;
    let minimum = near.clamp(0.0, first * 0.5);
    (minimum < first && first < maximum && maximum.is_finite()).then(|| {
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: minimum,
            maximum_distance: maximum,
            first_cascade_far_bound: first,
            overlap_proportion: 0.2,
        }
        .build()
    })
}

fn same_cascades(a: &CascadeShadowConfig, b: &CascadeShadowConfig) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() <= x.abs().max(y.abs()) * 0.005;
    a.bounds.len() == b.bounds.len()
        && a.bounds.iter().zip(&b.bounds).all(|(x, y)| close(*x, *y))
        && close(a.minimum_distance, b.minimum_distance)
        && a.overlap_proportion == b.overlap_proportion
}

fn fit_shadows(
    bounds: Res<SceneBounds>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut lights: Query<(&DirectionalLight, &mut CascadeShadowConfig)>,
) {
    let Some((low, high)) = bounds.0 else {
        return;
    };
    let mut range: Option<(f32, f32, f32)> = None;
    for (camera, transform, projection) in &cameras {
        if !camera.is_active {
            continue;
        }
        let Some((nearest, farthest)) = depth_range(transform.translation(), low, high) else {
            continue;
        };
        let near = match projection {
            Projection::Perspective(perspective) => perspective.near,
            _ => 0.0,
        };
        range = Some(match range {
            Some((a, b, c)) => (a.min(nearest), b.max(farthest), c.min(near)),
            None => (nearest, farthest, near),
        });
    }
    let Some(config) =
        range.and_then(|(nearest, farthest, near)| shadow_cascades(nearest, farthest, near))
    else {
        return;
    };
    for (light, mut current) in &mut lights {
        if light.shadow_maps_enabled && !same_cascades(&current, &config) {
            *current = config.clone();
        }
    }
}

pub fn fit_grid(low: Vec3, high: Vec3) -> Option<(f32, f32, f32)> {
    if !low.is_finite() || !high.is_finite() || low.cmpgt(high).any() {
        return None;
    }
    let span = (high - low).max_element().max(0.001);
    let height = low.y - span * 0.001;
    let target_spacing = span / 8.0;
    let decade = 10.0_f32.powf(target_spacing.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .find(|step| decade * step >= target_spacing)
        .unwrap_or(10.0);
    let scale = (decade * step).recip();
    let fade = (span * 12.0).max(1.0);
    (height.is_finite() && scale.is_finite() && fade.is_finite()).then_some((height, scale, fade))
}

impl Plugin for ViewerEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InfiniteGridPlugin)
            .insert_resource(ClearColor(Color::srgb_u8(32, 39, 49)))
            .init_resource::<SceneBounds>()
            .add_systems(Startup, setup_environment)
            .add_systems(
                PostUpdate,
                fit_shadows
                    .after(TransformSystems::Propagate)
                    .after(CameraUpdateSystems)
                    .before(SimulationLightSystems::UpdateDirectionalLightCascades),
            );
    }
}

fn setup_environment(mut commands: Commands) {
    commands.spawn((
        Name::new("Viewer grid"),
        InfiniteGrid,
        ViewerGrid,
        InfiniteGridSettings {
            x_axis_color: Color::srgb_u8(161, 81, 89),
            z_axis_color: Color::srgb_u8(74, 127, 168),
            minor_line_color: Color::srgba(0.45, 0.51, 0.60, 0.14),
            major_line_color: Color::srgba(0.58, 0.65, 0.75, 0.30),
            fadeout_distance: 100.0,
            dot_fadeout_strength: 0.9,
            scale: 1.0,
            major_line_interval: 10,
        },
    ));

    for (name, color, illuminance, position, shadows) in [
        (
            "Studio key",
            Color::srgb(1.0, 0.92, 0.82),
            7_500.0,
            Vec3::new(4.0, 8.0, 5.0),
            true,
        ),
        (
            "Studio fill",
            Color::srgb(0.65, 0.78, 1.0),
            2_500.0,
            Vec3::new(-6.0, 3.0, 2.0),
            false,
        ),
        (
            "Studio rim",
            Color::srgb(0.82, 0.9, 1.0),
            4_000.0,
            Vec3::new(1.0, 5.0, -6.0),
            false,
        ),
    ] {
        commands.spawn((
            Name::new(name),
            StudioLight(illuminance),
            DirectionalLight {
                color,
                illuminance,
                shadow_maps_enabled: shadows,
                ..default()
            },
            CascadeShadowConfigBuilder {
                maximum_distance: 100.0,
                ..default()
            }
            .build(),
            Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fits_below_negative_bounds() {
        let (height, scale, fade) = fit_grid(Vec3::splat(-2.0), Vec3::splat(2.0)).unwrap();
        assert!(height < -2.0);
        assert_eq!(scale, 2.0);
        assert_eq!(fade, 48.0);
        assert!(fit_grid(Vec3::ONE, Vec3::ZERO).is_none());
        assert!(fit_grid(Vec3::NAN, Vec3::ONE).is_none());
        assert!(fit_grid(Vec3::splat(-f32::MAX), Vec3::splat(f32::MAX)).is_none());
    }

    #[test]
    fn depth_range_measures_box_from_eye() {
        let (near, far) =
            depth_range(Vec3::new(0.0, 0.0, 10.0), Vec3::splat(-1.0), Vec3::ONE).unwrap();
        assert_eq!(near, 9.0);
        assert!((far - Vec3::new(1.0, 1.0, 11.0).length()).abs() < 1e-4);
        let (near, far) = depth_range(Vec3::ZERO, Vec3::splat(-1.0), Vec3::ONE).unwrap();
        assert_eq!(near, 0.0);
        assert!((far - 3.0_f32.sqrt()).abs() < 1e-4);
        assert!(depth_range(Vec3::ZERO, Vec3::ONE, Vec3::ZERO).is_none());
    }

    #[test]
    fn cascades_span_the_scene_at_any_scale() {
        for scale in [0.01, 1.0, 100.0, 100_000.0] {
            let (near, far) = depth_range(
                Vec3::splat(8.0 * scale),
                Vec3::splat(-scale),
                Vec3::splat(scale),
            )
            .unwrap();
            let config = shadow_cascades(near, far, scale * 0.001).unwrap();
            assert_eq!(config.bounds.len(), 4);
            assert!(config.bounds[0] > near, "scale={scale}");
            assert!(config.bounds[3] >= far, "scale={scale}");
            assert!(config.minimum_distance < config.bounds[0]);
        }
        let inside = shadow_cascades(0.0, 600.0, 0.6).unwrap();
        assert!(inside.bounds[0] <= 35.0);
        assert!(shadow_cascades(0.0, 0.0, 0.1).is_none());
        assert!(shadow_cascades(f32::NAN, 1.0, 0.1).is_none());
    }

    #[test]
    fn shadowed_lights_follow_the_camera_and_scene() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(SceneBounds(Some((Vec3::splat(-500.0), Vec3::splat(500.0)))));
        world.spawn((
            Camera3d::default(),
            Camera::default(),
            Projection::Perspective(PerspectiveProjection {
                near: 0.5,
                ..default()
            }),
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 3000.0)),
        ));
        let config = CascadeShadowConfigBuilder {
            maximum_distance: 100.0,
            ..default()
        }
        .build();
        let lit = world
            .spawn((
                DirectionalLight {
                    shadow_maps_enabled: true,
                    ..default()
                },
                config.clone(),
            ))
            .id();
        let unlit = world
            .spawn((DirectionalLight::default(), config.clone()))
            .id();
        world.run_system_once(fit_shadows).unwrap();
        let fitted = world.get::<CascadeShadowConfig>(lit).unwrap().clone();
        assert!(fitted.bounds[0] > 2500.0);
        assert!(*fitted.bounds.last().unwrap() >= 3500.0);
        assert_eq!(
            world.get::<CascadeShadowConfig>(unlit).unwrap().bounds,
            config.bounds
        );
        world.clear_trackers();
        world.run_system_once(fit_shadows).unwrap();
        let ticks = world
            .entity(lit)
            .get_change_ticks::<CascadeShadowConfig>()
            .unwrap();
        assert!(!ticks.is_changed(world.last_change_tick(), world.read_change_tick()));
    }

    #[test]
    fn grid_density_tracks_scene_scale() {
        for span in [0.001, 0.01, 0.1, 0.9, 1.0, 4.0, 10.0, 1000.0, 1_000_000.0] {
            let (_, scale, _) = fit_grid(Vec3::ZERO, Vec3::splat(span)).unwrap();
            let cells = span * scale;
            assert!((3.0..=8.001).contains(&cells), "span={span} cells={cells}");
        }
    }
}
