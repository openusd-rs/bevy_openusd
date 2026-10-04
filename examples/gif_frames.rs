//! Renders an animated USD asset to a PNG sequence in one process.
//!
//! ```sh
//! cargo run --example gif_frames -- ASSET FRAMES.txt
//! ```
//!
//! Each line of `FRAMES.txt` is one frame:
//! `TIME EYE_X EYE_Y EYE_Z FOCUS_X FOCUS_Y FOCUS_Z SKELETON(0|1) OUTPUT.png`.

use std::{path::PathBuf, time::Duration};

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::RenderTarget,
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use usd_bevy::{
    UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::UsdInstanceTime,
    skeleton_overlay::{UsdSkeletonOverlay, UsdSkeletonOverlayPlugin},
};

#[allow(dead_code)]
#[path = "../src/environment.rs"]
mod environment;

/// Frames drawn before the first capture, so pipelines and textures settle.
const WARMUP: u32 = 30;
/// Frames drawn after a pose change before it is captured.
const SETTLE: u32 = 3;

struct Frame {
    time: f64,
    eye: Vec3,
    focus: Vec3,
    skeleton: bool,
    output: PathBuf,
}

#[derive(Resource)]
struct Recorder {
    frames: Vec<Frame>,
    next: usize,
    wait: u32,
    posed: bool,
    pending: bool,
}

#[derive(Resource)]
struct Target(Handle<Image>);

#[derive(Resource)]
struct Asset(String);

fn parse(text: &str) -> Result<Vec<Frame>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let [time, ex, ey, ez, fx, fy, fz, skeleton, output] = words[..] else {
                return Err(format!("expected 9 fields: {line}"));
            };
            let number = |word: &str| {
                word.parse::<f32>()
                    .map_err(|_| format!("bad number {word}"))
            };
            Ok(Frame {
                time: time.parse().map_err(|_| format!("bad time {time}"))?,
                eye: Vec3::new(number(ex)?, number(ey)?, number(ez)?),
                focus: Vec3::new(number(fx)?, number(fy)?, number(fz)?),
                skeleton: skeleton == "1",
                output: PathBuf::from(output),
            })
        })
        .collect()
}

fn main() -> AppExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [asset, list] = &args[..] else {
        eprintln!("usage: gif_frames ASSET FRAMES.txt");
        return AppExit::error();
    };
    let frames = match std::fs::read_to_string(list)
        .map_err(|error| error.to_string())
        .and_then(|text| parse(&text))
    {
        Ok(frames) if !frames.is_empty() => frames,
        Ok(_) => {
            eprintln!("no frames in {list}");
            return AppExit::error();
        }
        Err(error) => {
            eprintln!("{list}: {error}");
            return AppExit::error();
        }
    };
    let asset = std::path::absolute(asset).expect("asset path");
    let directory = asset.parent().unwrap().to_string_lossy().into_owned();
    let name = asset.file_name().unwrap().to_string_lossy().into_owned();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: directory,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins((
            ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)),
            UsdPlugin,
            UsdAssetPlugin,
            UsdSkeletonOverlayPlugin,
            usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin,
            environment::ViewerEnvironmentPlugin,
        ))
        .insert_resource(usd_bevy::UsdProjectionBudget(Duration::MAX))
        .insert_resource(Asset(name))
        .insert_resource(Recorder {
            frames,
            next: 0,
            wait: WARMUP,
            posed: false,
            pending: false,
        })
        .add_systems(Startup, setup)
        .add_systems(Last, (fit_grid, record).chain())
        .run()
}

/// Sizes the floor grid to the loaded scene once.
fn fit_grid(
    mut fitted: Local<bool>,
    states: Query<&UsdSceneState>,
    meshes: Query<(
        &Mesh3d,
        Option<&bevy::camera::primitives::Aabb>,
        &GlobalTransform,
        &InheritedVisibility,
        Option<&bevy::mesh::skinning::SkinnedMesh>,
        Option<&bevy::mesh::morph::MeshMorphWeights>,
    )>,
    bounds: usd_bevy::mesh::bounds::MeshBounds,
    mut grids: Query<
        (
            &mut Transform,
            &mut bevy::dev_tools::infinite_grid::InfiniteGridSettings,
        ),
        With<environment::ViewerGrid>,
    >,
) {
    if *fitted || !matches!(states.iter().next(), Some(UsdSceneState::Ready)) {
        return;
    }
    let (mut low, mut high) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for (mesh, aabb, transform, visibility, skin, morph) in &meshes {
        if visibility.get()
            && let Some((min, max)) = bounds.get(mesh, transform, aabb, skin, morph)
        {
            low = low.min(min);
            high = high.max(max);
        }
    }
    if let Some((height, scale, fade)) = environment::fit_grid(low, high) {
        for (mut transform, mut settings) in &mut grids {
            transform.translation.y = height;
            settings.scale = scale;
            settings.fadeout_distance = fade;
        }
        *fitted = true;
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
    asset: Res<Asset>,
) {
    let mut image = Image::new_target_texture(960, 540, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(image);
    commands.spawn((
        Camera3d::default(),
        Msaa::Sample4,
        RenderTarget::from(target.clone()),
        Transform::default(),
        AmbientLight {
            color: Color::srgb(0.78, 0.85, 1.0),
            brightness: 160.0,
            ..default()
        },
    ));
    commands.insert_resource(Target(target));
    let scene: Handle<UsdScene> = server.load(asset.0.clone());
    commands.spawn((UsdSceneRoot(scene), UsdInstanceTime::default()));
}

fn record(
    mut commands: Commands,
    mut recorder: ResMut<Recorder>,
    target: Res<Target>,
    states: Query<&UsdSceneState>,
    mut roots: Query<&mut UsdInstanceTime, With<UsdSceneRoot>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut overlay: ResMut<UsdSkeletonOverlay>,
    mut exit: MessageWriter<AppExit>,
) {
    match states.iter().next() {
        Some(UsdSceneState::Ready) => (),
        Some(UsdSceneState::Failed(error)) => {
            eprintln!("load failed: {error}");
            exit.write(AppExit::error());
            return;
        }
        _ => return,
    }
    if recorder.pending {
        return;
    }
    if recorder.next == recorder.frames.len() {
        exit.write(AppExit::Success);
        return;
    }
    if !recorder.posed {
        let frame = &recorder.frames[recorder.next];
        for mut time in &mut roots {
            time.current = frame.time;
        }
        for mut transform in &mut cameras {
            *transform = Transform::from_translation(frame.eye).looking_at(frame.focus, Vec3::Y);
        }
        overlay.visible = frame.skeleton;
        recorder.posed = true;
        recorder.wait = recorder.wait.max(SETTLE);
    }
    if recorder.wait > 0 {
        recorder.wait -= 1;
        return;
    }
    recorder.pending = true;
    commands
        .spawn(Screenshot(RenderTarget::from(target.0.clone())))
        .observe(
            |event: On<ScreenshotCaptured>,
             mut recorder: ResMut<Recorder>,
             mut exit: MessageWriter<AppExit>| {
                let output = recorder.frames[recorder.next].output.clone();
                let saved = event
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|error| error.to_string())
                    .and_then(|image| {
                        image
                            .to_rgba8()
                            .save(&output)
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = saved {
                    eprintln!("{}: {error}", output.display());
                    exit.write(AppExit::error());
                    return;
                }
                println!("FRAME {} {}", recorder.next, output.display());
                recorder.next += 1;
                recorder.posed = false;
                recorder.pending = false;
            },
        );
}
