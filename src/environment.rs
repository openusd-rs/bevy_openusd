use bevy::dev_tools::infinite_grid::{InfiniteGrid, InfiniteGridPlugin, InfiniteGridSettings};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

pub struct ViewerEnvironmentPlugin;

#[derive(Component)]
pub struct ViewerGrid;

#[derive(Component)]
pub struct StudioLight(pub f32);

pub fn fit_grid(low: Vec3, high: Vec3) -> Option<(f32, f32, f32)> {
    if !low.is_finite() || !high.is_finite() || low.cmpgt(high).any() { return None; }
    let span = (high - low).max_element().max(0.001);
    let height = low.y - span * 0.001;
    let target_spacing = span / 8.0;
    let decade = 10.0_f32.powf(target_spacing.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].into_iter().find(|step| decade * step >= target_spacing).unwrap_or(10.0);
    let scale = (decade * step).recip();
    let fade = (span * 12.0).max(1.0);
    (height.is_finite() && scale.is_finite() && fade.is_finite()).then_some((height, scale, fade))
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
    fn grid_density_tracks_scene_scale() {
        for span in [0.001, 0.01, 0.1, 0.9, 1.0, 4.0, 10.0, 1000.0, 1_000_000.0] {
            let (_, scale, _) = fit_grid(Vec3::ZERO, Vec3::splat(span)).unwrap();
            let cells = span * scale;
            assert!((3.0..=8.001).contains(&cells), "span={span} cells={cells}");
        }
    }
}

impl Plugin for ViewerEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InfiniteGridPlugin)
            .insert_resource(ClearColor(Color::srgb_u8(32, 39, 49)))
            .add_systems(Startup, setup_environment);
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
