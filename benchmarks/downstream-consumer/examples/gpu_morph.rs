use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use bevy::{
    app::PluginsState,
    camera::RenderTarget,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::morph::MeshMorphWeights,
    pbr::{
        MeshInputUniform, MeshUniform, MorphIndices, MorphUniforms, RenderMeshInstances,
        collect_meshes_for_gpu_building,
    },
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
        batching::gpu_preprocessing::BatchedInstanceBuffers,
        mesh::{RenderMesh, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_resource::{TextureFormat, TextureUsages},
        renderer::{RenderAdapterInfo, RenderDevice},
        sync_world::MainEntity,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use usd_bevy::{
    UsdAssetPlugin, UsdPlugin, UsdProjectionBudget, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::UsdInstanceTime, route::gpu_skin::UsdGpuSkinningPlugin,
};

#[path = "../../../examples/support/pipeline_probe.rs"]
mod pipeline_probe;

#[derive(Resource, Default)]
struct Phase(usize);

#[derive(Resource, Default)]
struct Sources {
    frame: u64,
    phase: usize,
    meshes: Vec<(MainEntity, AssetId<Mesh>)>,
}

struct Row {
    phase: usize,
    entity: u64,
    valid: bool,
    line: String,
}

#[derive(Clone, Resource, Default)]
struct Probe(Arc<Mutex<Vec<Row>>>);

type MorphMeshes<'w, 's> =
    Query<'w, 's, (Entity, &'static Mesh3d, &'static ViewVisibility), With<MeshMorphWeights>>;

fn extract_sources(
    mut sources: ResMut<Sources>,
    phase: Extract<Res<Phase>>,
    meshes: Extract<MorphMeshes>,
) {
    sources.frame += 1;
    sources.phase = phase.0;
    sources.meshes = meshes
        .iter()
        .filter(|(_, _, visibility)| visibility.get())
        .map(|(entity, mesh, _)| (entity.into(), mesh.id()))
        .collect();
}

#[derive(bevy::ecs::system::SystemParam)]
struct Inputs<'w> {
    sources: Res<'w, Sources>,
    indices: Res<'w, MorphIndices>,
    uniforms: Res<'w, MorphUniforms>,
    instances: Res<'w, RenderMeshInstances>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    allocator: Res<'w, MeshAllocator>,
    buffers: Option<Res<'w, BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
}

fn observe(inputs: Inputs, probe: Res<Probe>) {
    for &(entity, asset) in &inputs.sources.meshes {
        let Some(mesh) = inputs.meshes.get(asset) else {
            continue;
        };
        if inputs.allocator.mesh_morph_target_slice(&asset).is_none() {
            continue;
        }
        let RenderMeshInstances::GpuBuilding(instances) = &*inputs.instances else {
            panic!("downstream GPU test requires GPU mesh input collection");
        };
        let Some(instance) = instances.get(&entity) else {
            continue;
        };
        let input = inputs.buffers.as_ref().and_then(|buffers| {
            buffers
                .current_input_buffer
                .get(instance.gpu_specific.current_uniform_index())
        });
        let index = inputs.indices.morph_descriptor_index(entity);
        let descriptor =
            index.and_then(|index| inputs.uniforms.descriptors_buffer.as_ref()?.get(index.0));
        let valid = input
            .zip(index)
            .zip(descriptor)
            .is_some_and(|((input, index), descriptor)| {
                input.morph_descriptor_index == index.0
                    && index.0 != u32::MAX
                    && descriptor.vertex_count == mesh.vertex_count
                    && descriptor.weight_count == 1
                    && inputs
                        .uniforms
                        .current_buffer
                        .get(descriptor.current_weights_offset)
                        == Some(&1.0)
            });
        let line = format!(
            "phase={} frame={} entity={} valid={valid} descriptor={index:?} input={input:?}",
            inputs.sources.phase,
            inputs.sources.frame,
            entity.id().to_bits()
        );
        let mut rows = probe.0.lock().unwrap();
        assert!(
            rows.len() < 4096,
            "downstream publication probe capacity exceeded"
        );
        rows.push(Row {
            phase: inputs.sources.phase,
            entity: entity.id().to_bits(),
            valid,
            line,
        });
    }
}

fn device(info: Res<RenderAdapterInfo>, device: Res<RenderDevice>) {
    println!("CONSUMER_ADAPTER {:?} limits={:?}", **info, device.limits());
}

fn update(app: &mut App, deadline: Instant) {
    assert!(Instant::now() < deadline, "downstream GPU test timed out");
    app.update();
    std::thread::sleep(Duration::from_millis(2));
}

const VISIBLE_PHASES: [usize; 3] = [1, 3, 4];
const RGB_TOLERANCE: u8 = 1;
const MIN_REGION_PIXELS: usize = 1000;

struct Frames {
    target: Handle<Image>,
    output: PathBuf,
    reference: Option<PathBuf>,
    deadline: Instant,
    deferred: bool,
}

fn verify(phase: usize, rgba: &[u8], reference: Option<&Path>) {
    let pixels = rgba.as_chunks::<4>().0;
    let lit = pixels.iter().filter(|p| p[..3] != [0, 0, 0]).count();
    let subset = pixels
        .iter()
        .filter(|p| p[0] > p[1].saturating_add(40) && p[1] > p[2].saturating_add(40))
        .count();
    let parent = pixels
        .iter()
        .filter(|p| p[0] > 64 && p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) <= 8)
        .count();
    println!("CONSUMER_FRAME phase={phase} lit={lit} subset={subset} parent={parent}");
    if VISIBLE_PHASES.contains(&phase) {
        assert!(
            subset >= MIN_REGION_PIXELS && parent >= MIN_REGION_PIXELS,
            "phase {phase} lacks subset or parent shading"
        );
    } else {
        assert_eq!(lit, 0, "phase {phase} rendered a hidden root");
    }
    let Some(reference) = reference else {
        return;
    };
    let expected = std::fs::read(reference.join(format!("phase-{phase}.rgba"))).unwrap();
    assert_eq!(expected.len(), rgba.len(), "phase {phase} reference size");
    let (mut changed, mut maximum) = (0, 0);
    for (a, b) in pixels.iter().zip(expected.as_chunks::<4>().0) {
        let error = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
        changed += usize::from(error > RGB_TOLERANCE);
        maximum = maximum.max(error);
    }
    println!("CONSUMER_COMPARE phase={phase} changed={changed} max_rgb_error={maximum}");
    assert_eq!(changed, 0, "phase {phase} differs from the CPU reference");
}

fn capture(app: &mut App, root: Entity, frames: &Frames, phase: usize) {
    let deadline = frames.deadline;
    app.world_mut().resource_mut::<Phase>().0 = phase;
    let mut ready = 0;
    while ready < 10 {
        let pipelines = app.world().resource::<pipeline_probe::PipelineProbe>();
        assert!(
            Instant::now() < deadline,
            "phase {phase} not ready: scene={:?} pipelines={:?}",
            app.world().get::<UsdSceneState>(root),
            pipelines.snapshot()
        );
        update(app, deadline);
        let loaded = app.world().get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready);
        if loaded
            && app
                .world()
                .resource::<pipeline_probe::PipelineProbe>()
                .ready_count()
                .is_some()
        {
            ready += 1;
        } else {
            ready = 0;
        }
    }
    let result = Arc::new(Mutex::new(None));
    let received = result.clone();
    app.world_mut()
        .spawn(Screenshot(RenderTarget::from(frames.target.clone())))
        .observe(move |event: On<ScreenshotCaptured>| {
            *received.lock().unwrap() = Some(event.image.clone());
        });
    let image = loop {
        update(app, deadline);
        if let Some(image) = result.lock().unwrap().take() {
            break image;
        }
    };
    assert!(
        app.world()
            .resource::<pipeline_probe::PipelineProbe>()
            .ready_count()
            .is_some()
    );
    let rgba = image.try_into_dynamic().unwrap().to_rgba8();
    assert_eq!(rgba.dimensions(), (512, 512));
    let output = &frames.output;
    std::fs::write(output.join(format!("phase-{phase}.rgba")), rgba.as_raw()).unwrap();
    rgba.save(output.join(format!("phase-{phase}.png")))
        .unwrap();
    verify(phase, rgba.as_raw(), frames.reference.as_deref());
    let world = app.world_mut();
    let meshes = world.query::<&Mesh3d>().iter(world).count();
    let morphs = world.query::<&MeshMorphWeights>().iter(world).count();
    println!(
        "CONSUMER_CAPTURE phase={phase} meshes={meshes} morphs={morphs} bytes={}",
        rgba.len()
    );
    let never_shown = frames.deferred && phase == 0;
    assert_eq!(meshes, if never_shown { 0 } else { 2 });
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        matches!(args.len(), 2 | 3)
            && matches!(
                args[0].as_str(),
                "cpu" | "gpu" | "cpu-deferred" | "gpu-deferred"
            ),
        "usage: gpu_morph cpu|gpu[-deferred] NEW_OUTPUT_DIRECTORY [CPU_REFERENCE_DIRECTORY]"
    );
    let gpu = args[0].starts_with("gpu");
    let deferred = args[0].ends_with("-deferred");
    let output = PathBuf::from(&args[1]);
    std::fs::create_dir(&output).unwrap();
    let reference = args.get(2).map(PathBuf::from);
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .canonicalize()
        .unwrap();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            }),
    );
    app.add_plugins((UsdPlugin, UsdAssetPlugin));
    if gpu {
        app.add_plugins(UsdGpuSkinningPlugin);
    }
    if deferred {
        app.init_resource::<usd_bevy::route::residency::DeferHiddenMeshes>();
    }
    app.insert_resource(UsdProjectionBudget(Duration::MAX))
        .init_resource::<Phase>();
    pipeline_probe::configure(&mut app);
    let probe = Probe::default();
    app.sub_app_mut(RenderApp)
        .insert_resource(probe.clone())
        .init_resource::<Sources>()
        .add_systems(ExtractSchedule, extract_sources)
        .add_systems(RenderStartup, device)
        .add_systems(
            Render,
            observe
                .after(collect_meshes_for_gpu_building)
                .in_set(RenderSystems::PrepareMeshes),
        );
    let deadline = Instant::now() + Duration::from_secs(120);
    while app.plugins_state() == PluginsState::Adding {
        assert!(
            Instant::now() < deadline,
            "renderer initialization timed out"
        );
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    let mut image = Image::new_target_texture(512, 512, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().insert_resource(ClearColor(Color::BLACK));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Sample4,
        Transform::from_xyz(2.0, 1.5, 4.0).looking_at(Vec3::new(0.5, 0.5, 0.3), Vec3::Y),
        AmbientLight {
            brightness: 160.0,
            ..default()
        },
    ));
    app.world_mut().spawn((
        DirectionalLight {
            illuminance: 3000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let handle: Handle<UsdScene> = app
        .world()
        .resource::<AssetServer>()
        .load("morph_subsets.usda");
    let root = app
        .world_mut()
        .spawn((
            UsdSceneRoot(handle.clone()),
            UsdInstanceTime { current: 10.0 },
            Visibility::Hidden,
        ))
        .id();
    let frames = Frames {
        target,
        output: output.clone(),
        reference,
        deadline,
        deferred,
    };
    capture(&mut app, root, &frames, 0);
    *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Inherited;
    capture(&mut app, root, &frames, 1);
    *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Hidden;
    capture(&mut app, root, &frames, 2);
    *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Inherited;
    capture(&mut app, root, &frames, 3);
    app.world_mut().despawn(root);
    let replacement = app
        .world_mut()
        .spawn((UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 }))
        .id();
    capture(&mut app, replacement, &frames, 4);
    let rows = probe.0.lock().unwrap();
    std::fs::write(
        output.join("publication.log"),
        rows.iter()
            .map(|row| format!("{}\n", row.line))
            .collect::<String>(),
    )
    .unwrap();
    if gpu {
        for phase in VISIBLE_PHASES {
            let entities: std::collections::BTreeSet<_> = rows
                .iter()
                .filter(|row| row.phase == phase)
                .map(|row| row.entity)
                .collect();
            assert_eq!(
                entities.len(),
                2,
                "missing parent/subset publication observations"
            );
        }
        assert!(
            rows.iter().all(|row| row.valid),
            "stale morph descriptor in downstream render inputs"
        );
    } else {
        assert!(
            rows.is_empty(),
            "CPU reference unexpectedly used GPU morphs"
        );
    }
    println!(
        "CONSUMER_GPU_OK mode={} observations={}",
        args[0],
        rows.len()
    );
}
