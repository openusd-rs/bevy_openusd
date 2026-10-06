//! Renders an animated USD asset to a PNG sequence in one process.
//!
//! ```sh
//! cargo run --example gif_frames -- ASSET FRAMES.txt [--stream]
//! ```
//!
//! Each line of `FRAMES.txt` is one frame:
//! `TIME EYE_X EYE_Y EYE_Z FOCUS_X FOCUS_Y FOCUS_Z OVERLAYS(0|1) OUTPUT.png`,
//! where OVERLAYS shows the skeleton and physics joint drawings. With
//! `--stream` the scene streams its payloads and LOD variants around the
//! camera, and each frame waits briefly for streaming to catch up.

use std::{path::PathBuf, time::Duration};

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    asset::RenderAssetUsages,
    camera::RenderTarget,
    core_pipeline::{prepass::DepthPrepass, tonemapping::Tonemapping},
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use usd_bevy::{
    UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::UsdInstanceTime,
    physics_overlay::{UsdPhysicsOverlay, UsdPhysicsOverlayPlugin},
    skeleton_overlay::{UsdSkeletonOverlay, UsdSkeletonOverlayPlugin},
    streaming::{UsdStreaming, UsdStreamingStatus},
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[allow(dead_code)]
#[path = "../src/environment.rs"]
mod environment;

/// Frames drawn before the first capture, so pipelines and textures settle.
const WARMUP: u32 = 30;
/// Frames drawn after a pose change before it is captured.
const SETTLE: u32 = 3;
/// Frames a capture waits at most for streaming to catch up, so the frames
/// show loading as the camera moves.
const STREAM_WAIT: u32 = 8;

struct Frame {
    time: f64,
    eye: Vec3,
    focus: Vec3,
    overlays: bool,
    output: PathBuf,
}

#[derive(Resource)]
struct Recorder {
    frames: Vec<Frame>,
    next: usize,
    wait: u32,
    posed: bool,
    pending: bool,
    streamed: u32,
    /// Frames drawn after each pose change before it is captured.
    settle: u32,
}

/// The render target, and how many target pixels each saved pixel averages
/// along each axis.
#[derive(Resource)]
struct Target(Handle<Image>, u32);

#[derive(Resource)]
struct Asset(String, bool);

fn parse(text: &str) -> Result<Vec<Frame>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let [time, ex, ey, ez, fx, fy, fz, overlays, output] = words[..] else {
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
                overlays: overlays == "1",
                output: PathBuf::from(output),
            })
        })
        .collect()
}

fn main() -> AppExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (asset, list, streaming) = match &args[..] {
        [asset, list] => (asset, list, false),
        [asset, list, flag] if flag == "--stream" => (asset, list, true),
        _ => {
            eprintln!("usage: gif_frames ASSET FRAMES.txt [--stream]");
            return AppExit::error();
        }
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
    let mut app = App::new();
    app.add_plugins(
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
        UsdPhysicsOverlayPlugin,
        usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin,
        environment::ViewerEnvironmentPlugin,
    ))
    .insert_resource(usd_bevy::UsdProjectionBudget(Duration::from_secs(5)))
    .insert_resource(bevy::render::render_asset::RenderAssetBytesPerFrame::new(
        256 << 20,
    ))
    .insert_resource(Asset(name, streaming))
    .insert_resource(Recorder {
        frames,
        next: 0,
        wait: WARMUP,
        posed: false,
        pending: false,
        streamed: 0,
        settle: std::env::var("USD_FRAME_SETTLE")
            .ok()
            .and_then(|frames| frames.parse().ok())
            .unwrap_or(SETTLE),
    })
    .add_systems(Startup, setup)
    .add_systems(Last, (fit_grid, record).chain())
    .add_systems(Update, scene_lighting)
    .add_plugins(usd_bevy::route::dome_environment::UsdDomeEnvironmentPlugin);
    if let Some(steps) = std::env::var("USD_CURVE_STEPS")
        .ok()
        .and_then(|steps| steps.parse().ok())
    {
        app.insert_resource(
            usd_bevy::route::curves::UsdCurveSettings::new(steps).expect("USD_CURVE_STEPS"),
        );
    }
    if std::env::var_os("USD_SUBDIVISION_LIMIT").is_some() {
        app.init_resource::<usd_bevy::route::subdivision::UsdSubdivisionLimit>();
    }
    if std::env::var_os("USD_PTEX_ATLAS").is_some() {
        app.init_resource::<usd_bevy::route::ptex_atlas::UsdPtexAtlas>();
    }
    if std::env::var_os("USD_CPU_INSTANCING").is_none() {
        app.insert_resource(usd_bevy::route::gpu_instancing::UsdGpuInstancing {
            shadows: std::env::var_os("USD_INSTANCED_SHADOWS").is_some(),
            ..default()
        })
        .add_plugins(usd_bevy::route::gpu_instancing::UsdGpuInstancingPlugin);
    }
    app.run()
}

/// Sizes the floor grid and the camera clip planes to the loaded scene once.
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
    mut projections: Query<&mut Projection, With<Camera3d>>,
    mut commands: Commands,
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
        let span = (high - low).max_element();
        for mut projection in &mut projections {
            if let Projection::Perspective(perspective) = projection.as_mut() {
                perspective.far = perspective.far.max(span * 4.0);
                perspective.near = perspective.near.max(span * 1e-6);
            }
        }
        commands.insert_resource(SceneSpan(span, (low + high) * 0.5));
        *fitted = true;
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
    asset: Res<Asset>,
) {
    let (width, height) = std::env::var("USD_FRAME_SIZE")
        .ok()
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .unwrap_or((960, 540));
    let fov = std::env::var("USD_FRAME_FOV")
        .ok()
        .and_then(|fov| fov.parse::<f32>().ok())
        .map_or(std::f32::consts::FRAC_PI_4, f32::to_radians);
    let exposure = std::env::var("USD_FRAME_EV100")
        .ok()
        .and_then(|ev100| ev100.parse().ok())
        .map_or_else(bevy::camera::Exposure::default, |ev100| {
            bevy::camera::Exposure { ev100 }
        });
    let tonemapping = match std::env::var("USD_FRAME_TONEMAP").as_deref() {
        Ok("aces") => Tonemapping::AcesFitted,
        Ok("agx") => Tonemapping::AgX,
        Ok("filmic") => Tonemapping::BlenderFilmic,
        Ok("reinhard") => Tonemapping::ReinhardLuminance,
        Ok("none") => Tonemapping::None,
        _ => Tonemapping::TonyMcMapface,
    };
    let supersample = std::env::var("USD_FRAME_SUPERSAMPLE")
        .ok()
        .and_then(|factor| factor.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    // Linear captures keep the exposed scene light, untoned, for grading
    // elsewhere.
    let linear = std::env::var_os("USD_FRAME_LINEAR").is_some();
    let mut image = Image::new_target_texture(
        width * supersample,
        height * supersample,
        if linear {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        },
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(image);
    let tonemapping = if linear {
        Tonemapping::None
    } else {
        tonemapping
    };
    let mut camera = commands.spawn((
        Camera3d::default(),
        exposure,
        tonemapping,
        // Media such as water measure the depth behind them from the prepass.
        DepthPrepass,
        Projection::Perspective(PerspectiveProjection { fov, ..default() }),
        Msaa::Sample4,
        RenderTarget::from(target.clone()),
        Transform::default(),
        AmbientLight {
            color: Color::srgb(0.78, 0.85, 1.0),
            brightness: 160.0,
            ..default()
        },
    ));
    if linear {
        camera.insert(bevy::camera::Hdr);
    }
    commands.insert_resource(Target(target, supersample));
    let scene: Handle<UsdScene> = server.load(asset.0.clone());
    let root = commands
        .spawn((UsdSceneRoot(scene), UsdInstanceTime::default()))
        .id();
    if asset.1 {
        commands.entity(root).insert(UsdStreaming {
            interval: Duration::ZERO,
            budget: Duration::from_millis(500),
            lod_full: Some(2160.0),
            ..default()
        });
    }
}

fn record(
    mut commands: Commands,
    mut recorder: ResMut<Recorder>,
    target: Res<Target>,
    states: Query<&UsdSceneState>,
    mut roots: Query<&mut UsdInstanceTime, With<UsdSceneRoot>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut skeletons: ResMut<UsdSkeletonOverlay>,
    mut joints: ResMut<UsdPhysicsOverlay>,
    streaming: Query<&UsdStreamingStatus>,
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
        skeletons.visible = frame.overlays;
        joints.visible = frame.overlays;
        recorder.posed = true;
        recorder.wait = recorder.wait.max(recorder.settle);
    }
    if recorder.wait > 0 {
        recorder.wait -= 1;
        return;
    }
    if streaming.iter().any(|status| status.pending > 0) && recorder.streamed < STREAM_WAIT {
        recorder.streamed += 1;
        return;
    }
    recorder.streamed = 0;
    recorder.pending = true;
    let supersample = target.1;
    commands
        .spawn(Screenshot(RenderTarget::from(target.0.clone())))
        .observe(
            move |event: On<ScreenshotCaptured>,
                  mut recorder: ResMut<Recorder>,
                  mut exit: MessageWriter<AppExit>| {
                let output = recorder.frames[recorder.next].output.clone();
                let saved = if event.image.texture_descriptor.format == TextureFormat::Rgba16Float {
                    save_linear(&event.image, supersample, &output)
                        .map_err(|error| error.to_string())
                } else {
                    downsample(&event.image, supersample)
                        .try_into_dynamic()
                        .map_err(|error| error.to_string())
                        .and_then(|image| {
                            image
                                .to_rgba8()
                                .save(&output)
                                .map_err(|error| error.to_string())
                        })
                };
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

/// The fitted scene's largest extent and its center.
#[derive(Resource)]
struct SceneSpan(f32, Vec3);

#[derive(Component)]
struct SunFitted;

/// A directional sun standing in for a far-away scene light.
#[derive(Component)]
struct FrameSun;

/// Lights the scene with the dome named by `USD_FRAME_DOME` instead of the
/// studio rig, with its sun as a shadowed light when `USD_FRAME_SUN` is set,
/// and draws the dome named by `USD_FRAME_SKY` behind it. The scene light
/// named by `USD_FRAME_SUN_LIGHT`, too far away to light the scene as a
/// point, becomes a shadowed directional sun of the same irradiance.
fn scene_lighting(
    mut commands: Commands,
    mut done: Local<(bool, bool, bool)>,
    span: Option<Res<SceneSpan>>,
    suns: Query<
        Entity,
        (
            Or<(
                With<usd_bevy::route::dome_environment::UsdDomeSun>,
                With<FrameSun>,
            )>,
            Without<SunFitted>,
        ),
    >,
    lights: Query<(
        &usd_bevy::UsdPrimRef,
        &usd_bevy::route::light::UsdLightEmission,
        Option<&usd_bevy::route::light::UsdAreaLight>,
        &GlobalTransform,
    )>,
    domes: Query<(
        Entity,
        &usd_bevy::UsdPrimRef,
        &usd_bevy::route::dome::UsdDomeLight,
        Option<&usd_bevy::route::dome::UsdDomeTexture>,
        &GlobalTransform,
        Option<&usd_bevy::route::dome::UsdDomePoleRotation>,
    )>,
    camera: Single<Entity, With<Camera3d>>,
    mut studio: Query<&mut DirectionalLight, With<environment::StudioLight>>,
    mut images: ResMut<Assets<Image>>,
) {
    let find = |name: &str| {
        let path = std::env::var(name).ok()?;
        domes.iter().find(|dome| dome.1.path == path)
    };
    if !done.0
        && let Some((dome, ..)) = find("USD_FRAME_DOME")
    {
        let source = usd_bevy::route::dome_environment::UsdDomeEnvironmentSource::new(dome);
        let source = if std::env::var_os("USD_FRAME_SUN").is_some() {
            source.with_sun()
        } else {
            source
        };
        commands.entity(*camera).insert((
            source,
            AmbientLight {
                brightness: 0.0,
                ..default()
            },
        ));
        for mut light in &mut studio {
            light.illuminance = 0.0;
        }
        done.0 = true;
    }
    if !done.1
        && let Some((_, _, light, Some(texture), transform, pole)) = find("USD_FRAME_SKY")
        && let Some(image) = images.get(&texture.0)
    {
        let cube = usd_bevy::route::environment_map::latlong_cubemap(image, 1024, light.color)
            .expect("sky cubemap");
        let rotation = transform.to_scale_rotation_translation().1
            * pole.map_or(Quat::IDENTITY, |pole| pole.0);
        commands
            .entity(*camera)
            .insert(bevy::core_pipeline::Skybox {
                image: Some(images.add(cube)),
                brightness: light.intensity,
                rotation,
            });
        done.1 = true;
    }
    if !done.2
        && let Some(span) = &span
        && let Ok(path) = std::env::var("USD_FRAME_SUN_LIGHT")
        && let Some((_, emission, area, transform)) =
            lights.iter().find(|light| light.0.path == path)
    {
        let (scale, rotation, position) = transform.to_scale_rotation_translation();
        let toward = (position - span.1).normalize();
        let area = match area {
            Some(usd_bevy::route::light::UsdAreaLight::Rect { width, height }) => {
                width * height * scale.x * scale.y
            }
            Some(usd_bevy::route::light::UsdAreaLight::Cylinder { length, radius }) => {
                2.0 * radius * length * scale.x * scale.y
            }
            None => 1.0,
        };
        // A rect emits along its -Z, so a tilted sun delivers less.
        let facing = (rotation * Vec3::NEG_Z).dot(-toward).max(0.0);
        let illuminance = emission.intensity * area * facing / position.distance_squared(span.1);
        commands.spawn((
            FrameSun,
            DirectionalLight {
                color: emission.color,
                illuminance,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, -toward)),
        ));
        for mut light in &mut studio {
            light.illuminance = 0.0;
        }
        done.2 = true;
    }
    if let Some(span) = span {
        for sun in &suns {
            commands.entity(sun).insert((
                bevy::light::CascadeShadowConfigBuilder {
                    first_cascade_far_bound: span.0 * 0.002,
                    maximum_distance: span.0 * 0.2,
                    ..default()
                }
                .build(),
                SunFitted,
            ));
        }
    }
}

/// Averages each `factor`×`factor` block of an sRGB RGBA8 capture in linear
/// light, as a film's pixels gather many samples.
fn downsample(image: &Image, factor: u32) -> Image {
    let Some(source) = image.data.as_ref().filter(|_| factor > 1) else {
        return image.clone();
    };
    let (width, height) = (image.width() / factor, image.height() / factor);
    let linear: Vec<f32> = (0..=255u8)
        .map(|value| Srgba::gamma_function(f32::from(value) / 255.0))
        .collect();
    let samples = (factor * factor) as f32;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for row in y * factor..(y + 1) * factor {
                for column in x * factor..(x + 1) * factor {
                    let at = ((row * image.width() + column) * 4) as usize;
                    for channel in 0..3 {
                        sum[channel] += linear[usize::from(source[at + channel])];
                    }
                    sum[3] += f32::from(source[at + 3]) / 255.0;
                }
            }
            for (channel, total) in sum.iter().enumerate() {
                let mean = total / samples;
                let encoded = if channel < 3 {
                    Srgba::gamma_function_inverse(mean)
                } else {
                    mean
                };
                pixels.push((encoded * 255.0).round() as u8);
            }
        }
    }
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Writes a half-float RGBA capture as a PFM file of linear RGB, each pixel
/// the mean of a `factor`×`factor` block.
fn save_linear(image: &Image, factor: u32, output: &std::path::Path) -> std::io::Result<()> {
    let source = image
        .data
        .as_ref()
        .ok_or_else(|| std::io::Error::other("capture holds no data"))?;
    let half = |at: usize| {
        let bits = u16::from_le_bytes([source[at], source[at + 1]]);
        let magnitude = match (bits >> 10) & 31 {
            0 => f32::from(bits & 1023) * 2f32.powi(-24),
            31 => f32::INFINITY,
            exponent => {
                (1.0 + f32::from(bits & 1023) / 1024.0) * 2f32.powi(i32::from(exponent) - 15)
            }
        };
        if bits & 0x8000 == 0 {
            magnitude
        } else {
            -magnitude
        }
    };
    let (width, height) = (image.width() / factor, image.height() / factor);
    let mut pfm = format!("PF\n{width} {height}\n-1.0\n").into_bytes();
    // PFM rows run from the bottom of the image up.
    for y in (0..height).rev() {
        for x in 0..width {
            for channel in 0..3 {
                let mut sum = 0.0;
                for row in y * factor..(y + 1) * factor {
                    for column in x * factor..(x + 1) * factor {
                        sum += half((((row * image.width() + column) * 4 + channel) * 2) as usize);
                    }
                }
                pfm.extend((sum / (factor * factor) as f32).to_le_bytes());
            }
        }
    }
    std::fs::write(output, pfm)
}
