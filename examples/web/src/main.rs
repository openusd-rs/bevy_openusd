//! usd_bevy in the browser, two ways: the AssetServer fetches a package over
//! HTTP, and `UsdSource::from_memory` opens layers already held as bytes.
//!
//! ```sh
//! cd examples/web && trunk serve --open
//! ```

use bevy::asset::AssetMetaCheck;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState, UsdSource};

/// The flagship showcase and every file it composes, keyed by relative path.
const SHOWCASE: [(&str, &[u8]); 6] = [
    (
        "flagship_showcase.usda",
        include_bytes!("../../../assets/flagship_showcase.usda"),
    ),
    (
        "animation_showcase.usda",
        include_bytes!("../../../assets/animation_showcase.usda"),
    ),
    (
        "morph_animation.usda",
        include_bytes!("../../../assets/morph_animation.usda"),
    ),
    (
        "blendshape_test.usda",
        include_bytes!("../../../assets/blendshape_test.usda"),
    ),
    (
        "skel_test_simple.usda",
        include_bytes!("../../../assets/skel_test_simple.usda"),
    ),
    (
        "dome_directional.hdr",
        include_bytes!("../../../assets/dome_directional.hdr"),
    ),
];

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "usd_bevy web".into(),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((UsdPlugin, UsdAssetPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, (report, orbit))
        .run();
}

/// Drag to orbit around `focus`, scroll to zoom.
#[derive(Component)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    focus: Vec3,
}

fn orbit(
    mut cameras: Query<(&mut Orbit, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
    };
    for (mut orbit, mut transform) in &mut cameras {
        if buttons.pressed(MouseButton::Left) {
            orbit.yaw -= motion.delta.x * 0.005;
            orbit.pitch = (orbit.pitch - motion.delta.y * 0.005).clamp(-1.5, 0.1);
        }
        orbit.distance = (orbit.distance * (1.0 - notches * 0.1)).clamp(1.0, 200.0);
        let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
        let eye = orbit.focus + rotation * Vec3::new(0.0, 0.0, orbit.distance);
        *transform = Transform::from_translation(eye).looking_at(orbit.focus, Vec3::Y);
    }
}

fn report(roots: Query<(Entity, &UsdSceneState), Changed<UsdSceneState>>) {
    for (root, state) in &roots {
        match state {
            UsdSceneState::Failed(error) => error!("{root}: {error}"),
            state => info!("{root}: {state:?}"),
        }
    }
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut scenes: ResMut<Assets<UsdScene>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        Orbit {
            yaw: 0.0,
            pitch: -0.45,
            distance: 14.0,
            focus: Vec3::new(0.0, 1.0, 0.0),
        },
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Over HTTP: the loader reads the package through Bevy's web asset reader.
    // Kitchen_set is authored in centimetres.
    commands.spawn((
        UsdSceneRoot(assets.load("Kitchen_set.usdz")),
        Transform::from_xyz(-4.0, 0.0, 0.0).with_scale(Vec3::splat(0.01)),
    ));

    // From memory: no filesystem and no network.
    let scene = UsdSource::from_memory("flagship_showcase.usda", SHOWCASE)
        .and_then(|source| UsdScene::from_source(source, &mut images));
    match scene {
        Ok(scene) => {
            commands.spawn((
                UsdSceneRoot(scenes.add(scene)),
                Transform::from_xyz(4.0, 0.0, 0.0).with_scale(Vec3::splat(0.5)),
            ));
        }
        Err(error) => error!("in-memory showcase: {error}"),
    }
}
