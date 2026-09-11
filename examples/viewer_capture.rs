//! Fixed-camera offscreen viewer capture with GPU readback and explicit exit status.

use std::{path::PathBuf, time::{Duration, Instant}};
use bevy::{app::{AppExit, ScheduleRunnerPlugin}, camera::RenderTarget, prelude::*};
use bevy::render::{render_resource::{TextureFormat, TextureUsages}, view::screenshot::{Screenshot, ScreenshotCaptured}};
use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot};

#[derive(Resource, Clone, Default)]
struct PipelineProgress(std::sync::Arc<std::sync::Mutex<Option<(usize, Vec<String>)>>>);

fn pipeline_progress(cache: Res<bevy::render::render_resource::PipelineCache>, progress: Res<PipelineProgress>) {
    use bevy::render::render_resource::CachedPipelineState;
    let mut waiting = 0;
    let mut errors = Vec::new();
    for pipeline in cache.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => (),
            CachedPipelineState::Err(error) => errors.push(error.to_string()),
            _ => waiting += 1,
        }
    }
    *progress.0.lock().unwrap() = Some((waiting, errors));
}

#[path = "../src/environment.rs"]
mod environment;
#[path = "../src/curve_quality.rs"]
mod curve_quality;
#[path = "../src/capture_metadata.rs"]
mod capture_metadata;

#[derive(Resource)]
struct Capture {
    camera_path: Option<String>,
    camera_ready: bool,
    renderer: CaptureRenderer,
    shadow_maps: bool,
    subdivision_levels: Option<u32>,
    curve_steps: usize,
    curve_surface_sides: Option<usize>,
    asset: PathBuf,
    output: PathBuf,
    time: f64,
    instance_times: Vec<f64>,
    instance_spacing: f32,
    swap_clocks: bool,
    clocks_swapped: bool,
    eye: Vec3,
    focus: Vec3,
    started: Instant,
    ready_frames: u32,
    requested: bool,
    mesh_report: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CaptureRenderer {
    #[default]
    Forward,
    Prepass,
    Deferred,
}

impl CaptureRenderer {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "forward" => Ok(Self::Forward),
            "prepass" => Ok(Self::Prepass),
            "deferred" => Ok(Self::Deferred),
            _ => Err("USD_CAPTURE_RENDERER must be forward, prepass or deferred".into()),
        }
    }
}

impl Capture {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.len() != 3 && args.len() != 9 {
            return Err("usage: viewer_capture ASSET OUTPUT.png TIME [EYE_X EYE_Y EYE_Z TARGET_X TARGET_Y TARGET_Z]".into());
        }
        let time: f64 = args[2].parse().map_err(|_| "invalid time code")?;
        let mut eye = Vec3::new(6.0, 4.0, 8.0);
        let mut focus = Vec3::new(0.0, 1.0, 0.0);
        if args.len() == 9 {
            let values = args[3..].iter().map(|arg| arg.parse::<f32>().map_err(|_| "invalid camera coordinate"))
                .collect::<Result<Vec<_>, _>>()?;
            eye = Vec3::new(values[0], values[1], values[2]);
            focus = Vec3::new(values[3], values[4], values[5]);
        }
        if !time.is_finite() || !eye.is_finite() || !focus.is_finite() || !(focus-eye).is_finite()
            || (focus-eye).cross(Vec3::Y).length_squared() < 1e-8 {
            return Err("time/camera must be finite and view direction must not be parallel to up".into());
        }
        let output = PathBuf::from(&args[1]);
        if output.extension().and_then(|ext| ext.to_str()) != Some("png") { return Err("output must end in .png".into()); }
        Ok(Self { camera_path: None, camera_ready: false, renderer: CaptureRenderer::Forward, shadow_maps: true, subdivision_levels: None, curve_steps: 8, curve_surface_sides: None, asset: PathBuf::from(&args[0]), output, time, eye, focus,
            instance_times: vec![time], instance_spacing: 2.5, swap_clocks: false, clocks_swapped: false,
            started: Instant::now(), ready_frames: 0, requested: false, mesh_report: String::new() })
    }

    fn set_instance_times(&mut self, value: &str) -> Result<(), String> {
        let times = value.split(',').map(|part| part.trim().parse::<f64>()
            .map_err(|_| "USD_CAPTURE_INSTANCE_TIMES requires comma-separated time codes".to_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        if times.len() > 16 || times.iter().any(|time| !time.is_finite()) {
            return Err("USD_CAPTURE_INSTANCE_TIMES requires 1 to 16 finite time codes".into());
        }
        self.instance_times = times;
        Ok(())
    }

    fn set_instance_spacing(&mut self, value: &str) -> Result<(), String> {
        let spacing = value.parse::<f32>().map_err(|_| "invalid USD_CAPTURE_INSTANCE_SPACING")?;
        if !spacing.is_finite() || spacing <= 0.0 || !(spacing * 16.0).is_finite() {
            return Err("USD_CAPTURE_INSTANCE_SPACING must be positive, finite and safe for 16 instances".into());
        }
        self.instance_spacing = spacing;
        Ok(())
    }
}

#[derive(Component)]
struct CaptureCamera;

#[derive(Component)]
struct CaptureInstance(usize);

fn reverse_capture_clocks(mut capture: ResMut<Capture>,
    mut roots: Query<(&CaptureInstance, &mut usd_bevy::instance::UsdInstanceTime)>) {
    if !capture.swap_clocks || capture.clocks_swapped || capture.ready_frames < 30 { return; }
    if roots.iter().count() != capture.instance_times.len() { return; }
    capture.instance_times.reverse();
    for (instance, mut time) in &mut roots {
        time.current = capture.instance_times[instance.0];
    }
    capture.clocks_swapped = true;
    capture.ready_frames = 0;
    eprintln!("CAPTURE_CLOCKS_REVERSED {:?}", capture.instance_times);
}

fn main() -> AppExit {
    let curve_settings = match curve_quality::from_env() {
        Ok(settings) => settings,
        Err(error) => { eprintln!("{error}"); return AppExit::error(); }
    };
    let mut capture = match Capture::parse(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(capture) => capture,
        Err(error) => { eprintln!("{error}"); return AppExit::error(); }
    };
    capture.renderer = match CaptureRenderer::parse(&std::env::var("USD_CAPTURE_RENDERER").unwrap_or_else(|_| "forward".into())) {
        Ok(renderer) => renderer,
        Err(error) => { eprintln!("{error}"); return AppExit::error(); }
    };
    capture.asset = match capture.asset.canonicalize() {
        Ok(path) => path,
        Err(error) => { eprintln!("asset: {error}"); return AppExit::error(); }
    };
    capture.camera_path = std::env::var("USD_CAPTURE_CAMERA").ok();
    if let Ok(times) = std::env::var("USD_CAPTURE_INSTANCE_TIMES") {
        if let Err(error) = capture.set_instance_times(&times) {
            eprintln!("{error}"); return AppExit::error();
        }
    }
    match std::env::var("USD_CAPTURE_INSTANCE_SPACING") {
        Ok(value) => if let Err(error) = capture.set_instance_spacing(&value) {
            eprintln!("{error}"); return AppExit::error();
        },
        Err(std::env::VarError::NotPresent) => {},
        Err(error) => { eprintln!("USD_CAPTURE_INSTANCE_SPACING: {error}"); return AppExit::error(); },
    }
    if capture.instance_times.len() > 1 && capture.camera_path.is_some() {
        eprintln!("multi-instance capture requires a fixed camera"); return AppExit::error();
    }
    capture.swap_clocks = match std::env::var("USD_CAPTURE_SWAP_CLOCKS").as_deref().unwrap_or("0") {
        "0" => false,
        "1" if capture.instance_times.len() > 1 => true,
        _ => { eprintln!("USD_CAPTURE_SWAP_CLOCKS must be 0, or 1 with multiple instances"); return AppExit::error(); }
    };
    capture.shadow_maps = match std::env::var("USD_CAPTURE_SHADOWS").as_deref().unwrap_or("scene") {
        "scene" => true,
        "off" => false,
        _ => { eprintln!("USD_CAPTURE_SHADOWS must be scene or off"); return AppExit::error(); }
    };
    let asset_dir = capture.asset.parent().unwrap().to_string_lossy().into_owned();
    let mut app = App::new();
    capture.curve_steps = curve_settings.cubic_steps();
    capture.curve_surface_sides = curve_settings.surface_sides();
    app.insert_resource(curve_settings);
    if capture.renderer == CaptureRenderer::Deferred {
        app.insert_resource(bevy::pbr::DefaultOpaqueRendererMethod::deferred());
    }
    let progress = PipelineProgress::default();
    app.insert_resource(progress.clone());
    app.insert_resource(capture).add_plugins(DefaultPlugins
        .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
        .set(AssetPlugin { file_path: asset_dir, ..default() })
        .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
        .disable::<bevy::winit::WinitPlugin>())
        .add_plugins((ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)),
            UsdPlugin, UsdAssetPlugin, environment::ViewerEnvironmentPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, (select_authored_camera, reverse_capture_clocks).chain())
        .add_systems(PostUpdate, configure_shadow_maps)
        .add_systems(Last, (fit_capture_grid, capture_frame).chain());
    if std::env::var_os("USD_CAPTURE_DOME").is_some() {
        app.add_plugins(usd_bevy::route::dome_environment::UsdDomeEnvironmentPlugin)
            .add_systems(Update, select_dome);
    }
    app.sub_app_mut(bevy::render::RenderApp).insert_resource(progress)
        .add_systems(bevy::render::Render, pipeline_progress.after(bevy::render::RenderSystems::Render));
    if std::env::var_os("USD_CPU_SKINNING").is_none() {
        app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin);
    }
    if let Ok(levels) = std::env::var("USD_SUBDIVISION_LEVELS") {
        let settings = usd_bevy::route::subdivision::UsdSubdivisionSettings::new(
            levels.parse().expect("USD_SUBDIVISION_LEVELS must be an integer")).expect("invalid subdivision levels");
        app.world_mut().resource_mut::<Capture>().subdivision_levels = Some(settings.levels());
        app.insert_resource(settings);
    }
    app.run()
}

fn configure_shadow_maps(capture: Res<Capture>, mut directional: Query<&mut DirectionalLight>,
    mut points: Query<&mut PointLight>, mut spots: Query<&mut SpotLight>) {
    if capture.shadow_maps { return; }
    for mut light in &mut directional { light.shadow_maps_enabled = false; }
    for mut light in &mut points { light.shadow_maps_enabled = false; }
    for mut light in &mut spots { light.shadow_maps_enabled = false; }
}

fn select_dome(mut commands: Commands,
    domes: Query<(Entity, &usd_bevy::UsdPrimRef), With<usd_bevy::route::dome::UsdDomeLight>>,
    cameras: Query<Entity, (With<CaptureCamera>, Without<usd_bevy::route::dome_environment::UsdDomeEnvironmentSource>)>,
    mut lights: Query<&mut DirectionalLight>) {
    let Ok(path) = std::env::var("USD_CAPTURE_DOME") else { return };
    let Some((dome, _)) = domes.iter().find(|(_, prim)| prim.path == path) else { return };
    for camera in &cameras {
        commands.entity(camera).insert((usd_bevy::route::dome_environment::UsdDomeEnvironmentSource::new(dome),
            AmbientLight { brightness: 0.0, ..default() }));
    }
    for mut light in &mut lights { light.illuminance = 0.0; }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, server: Res<AssetServer>, capture: Res<Capture>) {
    let mut image = Image::new_target_texture(1280, 720, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(image);
    let mut camera = commands.spawn((Camera3d::default(), CaptureCamera, RenderTarget::from(target),
        Transform::from_translation(capture.eye).looking_at(capture.focus, Vec3::Y),
        AmbientLight { color: Color::srgb(0.78,0.85,1.0), brightness: 160.0, ..default() }));
    if capture.renderer != CaptureRenderer::Forward {
        camera.insert((bevy::core_pipeline::prepass::DepthPrepass,
            bevy::core_pipeline::prepass::NormalPrepass,
            bevy::core_pipeline::prepass::MotionVectorPrepass, Msaa::Off));
    }
    if capture.renderer == CaptureRenderer::Deferred {
        camera.insert(bevy::core_pipeline::prepass::DeferredPrepass);
    }
    let scene: Handle<UsdScene> = server.load(capture.asset.file_name().unwrap().to_string_lossy().into_owned());
    for (index, current) in capture.instance_times.iter().copied().enumerate() {
        let x = (index as f32 - (capture.instance_times.len() - 1) as f32 * 0.5) * capture.instance_spacing;
        commands.spawn((UsdSceneRoot(scene.clone()), CaptureInstance(index), usd_bevy::instance::UsdInstanceTime { current },
            Transform::from_xyz(x, 0.0, 0.0)));
    }
}

fn fit_capture_grid(
    capture: Res<Capture>,
    meshes: Query<(&Mesh3d, Option<&bevy::camera::primitives::Aabb>, &GlobalTransform, &InheritedVisibility,
        Option<&bevy::mesh::skinning::SkinnedMesh>, Option<&bevy::mesh::morph::MeshMorphWeights>)>,
    mesh_bounds: usd_bevy::mesh::bounds::MeshBounds,
    mut grids: Query<(&mut Transform, &mut bevy::dev_tools::infinite_grid::InfiniteGridSettings), With<environment::ViewerGrid>>,
) {
    if capture.requested { return; }
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for (mesh, bounds, transform, visibility, skin, morph) in &meshes {
        if !visibility.get() { continue; }
        if let Some((min, max)) = mesh_bounds.get(mesh, transform, bounds, skin, morph) {
            low = low.min(min); high = high.max(max);
        }
    }
    if let Some((height, scale, fade)) = environment::fit_grid(low, high) {
        for (mut transform, mut settings) in &mut grids {
            transform.translation.y = height;
            settings.scale = scale;
            settings.fadeout_distance = fade;
        }
    }
}

fn select_authored_camera(mut capture: ResMut<Capture>,
    sources: Query<(&usd_bevy::UsdPrimRef, &GlobalTransform, &Projection), (With<usd_bevy::route::camera::UsdCamera>, Without<CaptureCamera>)>,
    mut targets: Query<(&mut Transform, &mut Projection), With<CaptureCamera>>) {
    let Some(path) = &capture.camera_path else { capture.camera_ready = true; return };
    let Some((_, transform, projection)) = sources.iter().find(|(prim, _, _)| &prim.path == path) else {
        capture.camera_ready = false;
        capture.ready_frames = 0;
        return;
    };
    for (mut target_transform, mut target_projection) in &mut targets {
        *target_transform = Transform::from_matrix(transform.to_matrix());
        *target_projection = projection.clone();
    }
    capture.camera_ready = true;
}

fn capture_frame(mut commands: Commands, mut capture: ResMut<Capture>,
    progress: Res<PipelineProgress>,
    states: Query<&usd_bevy::asset::UsdSceneState, With<UsdSceneRoot>>,
    meshes: Query<(Entity, &InheritedVisibility, Option<&bevy::mesh::skinning::SkinnedMesh>, Option<&usd_bevy::route::gpu_skin::UsdCpuSkinFallback>, Option<&usd_bevy::route::gpu_morph::UsdGpuMorph>, Option<&MeshMaterial3d<usd_bevy::route::flat_material::FlatMaterial>>), With<Mesh3d>>,
    camera: Query<(&RenderTarget, &GlobalTransform, &Camera), With<CaptureCamera>>,
    environments: Query<&usd_bevy::route::dome_environment::UsdDomeEnvironmentState, With<CaptureCamera>>,
    environment_lights: Query<&bevy::light::EnvironmentMapLight, With<CaptureCamera>>,
    generators: Query<(), With<bevy::light::GeneratedEnvironmentMapLight>>,
    dome_diagnostics: Option<Res<usd_bevy::route::dome_environment::UsdDomeEnvironmentDiagnostics>>,
    studio_lights: Query<&environment::StudioLight>,
    deformation_errors: Query<(&usd_bevy::UsdPrimRef, &usd_bevy::route::skel::UsdDeformationError)>,
    subdivision_errors: Query<(&usd_bevy::UsdPrimRef, &usd_bevy::route::subdivision::UsdSubdivisionError)>,
    instancer_errors: Query<(&usd_bevy::UsdPrimRef, &usd_bevy::route::instancer::UsdInstancerWarning)>,
    geometry_errors: Query<(&usd_bevy::UsdPrimRef, Option<&usd_bevy::route::shapes::UsdShapeError>, Option<&usd_bevy::route::curves::UsdCurveError>, Option<&usd_bevy::route::xform::UsdTransformError>)>,
    mut exit: MessageWriter<AppExit>) {
    if capture.started.elapsed() > Duration::from_secs(60) {
        eprintln!("capture failed: timed out waiting for scene/render readback; camera={:?} ready={}; pipeline status: {:?}", capture.camera_path, capture.camera_ready, progress.0.lock().unwrap());
        exit.write(AppExit::error());
        return;
    }
    for state in &states {
        match state {
            usd_bevy::asset::UsdSceneState::Failed(error) => {
                eprintln!("capture failed: {error}"); exit.write(AppExit::error()); return;
            }
            usd_bevy::asset::UsdSceneState::Ready => (),
            _ => return,
        }
    }
    if states.iter().count() != capture.instance_times.len() || capture.requested { return; }
    if !capture.camera_ready { return; }
    if let Some((prim, error)) = geometry_errors.iter().find_map(|(prim, _, _, transform)| transform.map(|error| (prim, error))) {
        eprintln!("capture failed: transform {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if let Some((prim, error)) = geometry_errors.iter().find_map(|(prim, _, curve, _)| curve.map(|error| (prim, error))) {
        eprintln!("capture failed: curve {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if let Some((prim, error)) = geometry_errors.iter().find_map(|(prim, shape, _, _)| shape.map(|error| (prim, error))) {
        eprintln!("capture failed: shape {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if let Some((prim, error)) = instancer_errors.iter().next() {
        eprintln!("capture failed: point instancer {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if let Some((prim, error)) = subdivision_errors.iter().next() {
        eprintln!("capture failed: subdivision {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if let Some((prim, error)) = deformation_errors.iter().next() {
        eprintln!("capture failed: deformation {}: {}", prim.path, error.0);
        exit.write(AppExit::error());
        return;
    }
    if std::env::var_os("USD_CAPTURE_DOME").is_some() {
        use usd_bevy::route::dome_environment::UsdDomeEnvironmentState;
        match environments.single() {
            Ok(UsdDomeEnvironmentState::Attached) => (),
            Ok(UsdDomeEnvironmentState::Unavailable(error)) => {
                eprintln!("capture failed: dome environment: {error}"); exit.write(AppExit::error()); return;
            }
            _ => { capture.ready_frames = 0; return; }
        }
        if !generators.is_empty() || dome_diagnostics.as_ref().is_none_or(|d| d.recorded_generations == 0) {
            capture.ready_frames = 0;
            return;
        }
    }
    if !progress.0.lock().unwrap().as_ref().is_some_and(|(waiting, errors)| *waiting == 0 && errors.is_empty()) {
        capture.ready_frames = 0;
        return;
    }
    capture.ready_frames += 1;
    if capture.ready_frames < 60 { return; }
    let Ok((target, camera_transform, camera)) = camera.single() else { return };
    let visible = meshes.iter().filter(|(_, visibility, _, _, _, _)| visibility.get()).count();
    let gpu = meshes.iter().filter(|(_, visibility, skin, _, _, _)| visibility.get() && skin.is_some()).count();
    let morph = meshes.iter().filter(|(_, visibility, _, _, morph, _)| visibility.get() && morph.is_some()).count();
    let flat_handles: Vec<_> = meshes.iter().filter(|(_, visibility, _, _, _, _)| visibility.get())
        .filter_map(|(_, _, _, _, _, material)| material.map(|material| material.0.id())).collect();
    let flat_unique: std::collections::HashSet<_> = flat_handles.iter().copied().collect();
    capture.mesh_report = format!("hierarchy_visible_meshes={visible}\nhierarchy_visible_gpu_meshes={gpu}\nhierarchy_visible_gpu_morph_meshes={morph}\n");
    capture.mesh_report.push_str(&capture_metadata::camera_report(camera_transform, camera.clip_from_view()));
    capture.mesh_report.push_str(&format!("hierarchy_visible_flat_material_entities={}\nhierarchy_visible_unique_flat_materials={}\n", flat_handles.len(), flat_unique.len()));
    capture.mesh_report.push_str(&format!("studio_baseline_lux={:?}\n", studio_lights.iter().map(|light| light.0).collect::<Vec<_>>()));
    if let Ok(dome) = std::env::var("USD_CAPTURE_DOME") {
        capture.mesh_report.push_str(&format!("dome={dome}\ndome_maps_attached=true\ndirectional_lights_disabled=true\ncamera_ambient_disabled=true\n"));
        if let Ok(light) = environment_lights.single() {
            capture.mesh_report.push_str(&format!("dome_intensity={}\ndome_rotation={:?}\n", light.intensity, light.rotation));
        }
        capture.mesh_report.push_str(&format!("active_environment_generators={}\nrecorded_environment_generations={}\n",
            generators.iter().count(), dome_diagnostics.as_ref().map_or(0, |d| d.recorded_generations)));
    }
    for (entity, visibility, _, fallback, _, _) in &meshes {
        if visibility.get() && let Some(fallback) = fallback {
            capture.mesh_report.push_str(&format!("cpu_fallback[{entity:?}]={}\n", fallback.0));
        }
    }
    let shadows = if capture.shadow_maps { "shadow_maps=scene\n" } else { "shadow_maps=off\n" };
    capture.mesh_report.push_str(shadows);
    capture.requested = true;
    commands.spawn(Screenshot(target.clone())).observe(
        |event: On<ScreenshotCaptured>, capture: Res<Capture>, mut exit: MessageWriter<AppExit>| {
            match save(&event.image, &capture) {
                Ok(()) => { println!("CAPTURE_OK {output}", output = capture.output.display()); exit.write(AppExit::Success); }
                Err(error) => { eprintln!("CAPTURE_FAILED {error}"); exit.write(AppExit::error()); }
            }
        });
}

fn save(image: &Image, capture: &Capture) -> Result<(), String> {
    let dynamic = image.clone().try_into_dynamic().map_err(|error| format!("readback format: {error}"))?;
    let rgba = dynamic.to_rgba8();
    if rgba.dimensions() != (1280,720) { return Err(format!("unexpected readback dimensions: {:?}", rgba.dimensions())); }
    let temporary = capture.output.with_extension("partial.png");
    dynamic.to_rgb8().save(&temporary).map_err(|error| format!("PNG write {output}: {error}", output = temporary.display()))?;
    std::fs::write(capture.output.with_extension("rgba"), rgba.as_raw()).map_err(|error| format!("RGBA write: {error}"))?;
    let report = format!("asset={asset}\ntime={time}\nrequested_eye={eye:?}\nrequested_target={target:?}\nwidth=1280\nheight=720\nformat=rgba8-srgb\nrow_bytes=5120\nbytes={bytes}\nready_frames={frames}\ncpu_skinning={cpu}\n",
        asset = capture.asset.display(), time = capture.time, eye = capture.eye, target = capture.focus,
        bytes = rgba.len(), frames = capture.ready_frames, cpu = std::env::var_os("USD_CPU_SKINNING").is_some());
    let report = report + &format!("instance_times={:?}\ninstance_spacing={}\n", capture.instance_times, capture.instance_spacing)
        + &format!("clocks_reversed_after_ready_frames={}\n", if capture.clocks_swapped { 30 } else { 0 })
        + &format!("camera_source={}\n", capture.camera_path.as_deref().unwrap_or("fixed-arguments")) + &format!("renderer={:?}\nsubdivision_levels={}\n",
        capture.renderer, capture.subdivision_levels.unwrap_or(0)) + &format!("curve_steps={}\ncurve_surface_sides={:?}\n", capture.curve_steps, capture.curve_surface_sides) + &capture.mesh_report;
    std::fs::write(capture.output.with_extension("capture.txt"), report).map_err(|error| format!("metadata write: {error}"))?;
    std::fs::rename(&temporary, &capture.output).map_err(|error| format!("PNG publish: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_clock_reversal_waits_and_mutates_existing_roots_once() {
        use super::*;
        let mut capture = Capture::parse(&["a.usda".into(), "a.png".into(), "0".into()]).unwrap();
        capture.set_instance_times("0,10").unwrap();
        capture.swap_clocks = true;
        capture.ready_frames = 29;
        let mut app = App::new();
        app.insert_resource(capture).add_systems(Update, reverse_capture_clocks);
        let first = app.world_mut().spawn((CaptureInstance(0), usd_bevy::instance::UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((CaptureInstance(1), usd_bevy::instance::UsdInstanceTime { current: 10.0 })).id();
        app.update();
        assert!(!app.world().resource::<Capture>().clocks_swapped);
        app.world_mut().resource_mut::<Capture>().ready_frames = 30;
        app.update();
        assert_eq!(app.world().get::<usd_bevy::instance::UsdInstanceTime>(first).unwrap().current, 10.0);
        assert_eq!(app.world().get::<usd_bevy::instance::UsdInstanceTime>(second).unwrap().current, 0.0);
        assert_eq!(app.world().resource::<Capture>().ready_frames, 0);
        assert!(app.world().resource::<Capture>().clocks_swapped);
        app.world_mut().resource_mut::<Capture>().ready_frames = 60;
        app.update();
        assert_eq!(app.world().resource::<Capture>().instance_times, [10.0, 0.0]);
    }

    use super::*;
    #[test]
    fn instance_spacing_is_finite_positive_and_atomic() {
        let mut capture = Capture::parse(&["a.usda".into(), "a.png".into(), "0".into()]).unwrap();
        assert_eq!(capture.instance_spacing, 2.5);
        capture.set_instance_spacing("14").unwrap();
        for invalid in ["", "NaN", "inf", "0", "-1", "3e38", "bad"] {
            assert!(capture.set_instance_spacing(invalid).is_err());
            assert_eq!(capture.instance_spacing, 14.0);
        }
    }

    #[test]
    fn instance_time_lists_are_finite_bounded_and_atomic() {
        let mut capture = Capture::parse(&["a.usda".into(), "a.png".into(), "3".into()]).unwrap();
        assert_eq!(capture.instance_times, [3.0]);
        capture.set_instance_times("0, 10, -2.5").unwrap();
        assert_eq!(capture.instance_times, [0.0, 10.0, -2.5]);
        for invalid in ["", "0,", "NaN", "inf", "1e999", "0,nope"] {
            assert!(capture.set_instance_times(invalid).is_err());
            assert_eq!(capture.instance_times, [0.0, 10.0, -2.5]);
        }
        assert!(capture.set_instance_times(&vec!["0"; 17].join(",")).is_err());
        capture.set_instance_times(&vec!["0"; 16].join(",")).unwrap();
        assert_eq!(capture.instance_times.len(), 16);
    }
    #[test]
    fn authored_camera_selection_waits_for_the_requested_prim() {
        let mut capture = Capture::parse(&["a.usda".into(), "a.png".into(), "0".into()]).unwrap();
        capture.camera_path = Some("/Camera".into());
        capture.ready_frames = 5;
        let mut app = App::new();
        app.insert_resource(capture).add_systems(Update, select_authored_camera);
        let target = app.world_mut().spawn((CaptureCamera, Transform::default(), Projection::default())).id();
        app.world_mut().run_schedule(Update);
        assert!(!app.world().resource::<Capture>().camera_ready);
        assert_eq!(app.world().resource::<Capture>().ready_frames, 0);
        app.world_mut().spawn((usd_bevy::UsdPrimRef { path: "/Camera".into() }, usd_bevy::route::camera::UsdCamera,
            GlobalTransform::from_translation(Vec3::new(1.0,2.0,3.0)), Projection::custom(usd_bevy::route::camera::UsdPerspectiveProjection {
                perspective: default(), aperture_offset_over_focal: Vec2::new(0.2,0.1),
            })));
        app.world_mut().run_schedule(Update);
        assert!(app.world().resource::<Capture>().camera_ready);
        assert_eq!(app.world().get::<Transform>(target).unwrap().translation, Vec3::new(1.0,2.0,3.0));
        assert!(matches!(app.world().get::<Projection>(target), Some(Projection::Custom(_))));
    }

    #[test]
    fn shadow_diagnostic_preserves_scene_defaults_and_disables_all_light_types() {
        let capture = Capture::parse(&["a.usda".into(), "a.png".into(), "0".into()]).unwrap();
        let mut app = App::new();
        app.insert_resource(capture).add_systems(Update, configure_shadow_maps);
        let directional = app.world_mut().spawn(DirectionalLight { shadow_maps_enabled: true, ..default() }).id();
        let point = app.world_mut().spawn(PointLight { shadow_maps_enabled: true, ..default() }).id();
        let spot = app.world_mut().spawn(SpotLight { shadow_maps_enabled: true, ..default() }).id();
        let disabled = app.world_mut().spawn(DirectionalLight { shadow_maps_enabled: false, ..default() }).id();
        app.world_mut().run_schedule(Update);
        assert!(app.world().get::<DirectionalLight>(directional).unwrap().shadow_maps_enabled);
        assert!(app.world().get::<PointLight>(point).unwrap().shadow_maps_enabled);
        assert!(app.world().get::<SpotLight>(spot).unwrap().shadow_maps_enabled);
        assert!(!app.world().get::<DirectionalLight>(disabled).unwrap().shadow_maps_enabled);
        app.world_mut().resource_mut::<Capture>().shadow_maps = false;
        app.world_mut().run_schedule(Update);
        assert!(!app.world().get::<DirectionalLight>(directional).unwrap().shadow_maps_enabled);
        assert!(!app.world().get::<PointLight>(point).unwrap().shadow_maps_enabled);
        assert!(!app.world().get::<SpotLight>(spot).unwrap().shadow_maps_enabled);
        assert!(!app.world().get::<DirectionalLight>(disabled).unwrap().shadow_maps_enabled);
    }

    #[test]
    fn renderer_selection_is_explicit() {
        assert_eq!(CaptureRenderer::parse("forward").unwrap(), CaptureRenderer::Forward);
        assert_eq!(CaptureRenderer::parse("prepass").unwrap(), CaptureRenderer::Prepass);
        assert_eq!(CaptureRenderer::parse("deferred").unwrap(), CaptureRenderer::Deferred);
        assert!(CaptureRenderer::parse("typo").is_err());
    }
    #[test]
    fn readback_records_subdivision_and_tight_rgba() {
        let directory = std::env::temp_dir().join(format!("usd-capture-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&directory).unwrap();
        let output = directory.join("frame.png");
        let mut capture = Capture::parse(&["a.usda".into(), output.to_string_lossy().into_owned(), "3".into()]).unwrap();
        let image = Image::new_target_texture(1280, 720, TextureFormat::Rgba8UnormSrgb, None);
        for levels in [None, Some(2)] {
            capture.subdivision_levels = levels;
            capture.curve_steps = if levels.is_some() { 32 } else { 8 };
            save(&image, &capture).unwrap();
            let report = std::fs::read_to_string(output.with_extension("capture.txt")).unwrap();
            assert!(report.contains(&format!("subdivision_levels={}\n", levels.unwrap_or(0))));
            assert!(report.contains(&format!("curve_steps={}\n", capture.curve_steps)));
            assert!(report.contains("requested_eye=Vec3(6.0, 4.0, 8.0)\n"));
            assert!(!report.contains("\neye="));
            assert_eq!(std::fs::metadata(output.with_extension("rgba")).unwrap().len(), 1280 * 720 * 4);
            assert!(output.is_file());
            assert!(!output.with_extension("partial.png").exists());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn invalid_dimensions_and_write_failures_are_errors() {
        let capture = Capture::parse(&["a.usda".into(), "/nonexistent-usd-capture-dir/a.png".into(), "0".into()]).unwrap();
        let wrong_size = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        assert!(save(&wrong_size, &capture).unwrap_err().contains("dimensions"));
        let image = Image::new_target_texture(1280, 720, TextureFormat::Rgba8UnormSrgb, None);
        assert!(save(&image, &capture).unwrap_err().contains("PNG write"));
    }
    #[test]
    fn rejects_invalid_camera_and_time() {
        for args in ["a.usda a.png NaN", "a.usda a.png 0 0 1 0 0 0 0", "a.usda a.jpg 0"] {
            assert!(Capture::parse(&args.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
        assert!(Capture::parse(&["a.usda".into(), "a.png".into(), "30".into()]).is_ok());
    }
}
