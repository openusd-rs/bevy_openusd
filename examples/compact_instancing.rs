//! Experimental forward-PBR vertex pulling; no USD route or indirect draw integration.

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{RenderTarget, visibility::NoFrustumCulling},
    core_pipeline::core_3d::Opaque3d,
    ecs::{query::ROQueryItem, system::{lifetimeless::SRes, SystemParamItem}},
    mesh::{MeshVertexBufferLayoutRef, VertexAttributeValues},
    pbr::{DrawMaterial, DrawMesh, ExtendedMaterial, MaterialExtension, MaterialExtensionKey,
        MaterialExtensionPipeline, PbrPlugin, SetMaterialBindGroup, SetMeshBindGroup,
        SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup},
    prelude::*,
    render::{Extract, ExtractSchedule, RenderApp, batching::NoAutomaticBatching,
        render_phase::{DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, RenderCommandState, SetItemPipeline, TrackedRenderPass},
        render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureFormat, TextureUsages},
        renderer::RenderDevice,
        storage::ShaderBuffer, sync_world::MainEntity,
        view::screenshot::{Screenshot, ScreenshotCaptured}},
    shader::ShaderRef,
};
use std::collections::HashMap;

const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("3e5e507a-7301-4c35-82ac-ae906a8b9178");

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
struct Pulling {
    #[storage(100, read_only)]
    vertices: Handle<ShaderBuffer>,
    #[storage(101, read_only)]
    points: Handle<ShaderBuffer>,
    #[uniform(102)]
    vertices_per_point: u32,
}

impl MaterialExtension for Pulling {
    fn vertex_shader() -> ShaderRef { SHADER.into() }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    fn specialize(_: &MaterialExtensionPipeline, descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef, _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers[0].attributes.clear();
        descriptor.vertex.buffers[0].array_stride = 0;
        Ok(())
    }
}

#[derive(Clone, ShaderType)]
struct Point {
    translation: Vec4,
    rotation: Vec4,
    scale: Vec4,
}

impl From<Transform> for Point {
    fn from(transform: Transform) -> Self {
        Self { translation: transform.translation.extend(0.0), rotation: Vec4::from_array(transform.rotation.to_array()),
            scale: transform.scale.extend(0.0) }
    }
}

fn batch_capacity(storage_limit: u64, buffer_limit: u64, vertices_per_point: u32, requested: usize) -> Option<usize> {
    if vertices_per_point == 0 { return None; }
    let by_bytes = storage_limit.min(buffer_limit) / Point::min_size().get();
    let by_draw = u32::MAX / vertices_per_point;
    let count = by_bytes.min(u64::from(by_draw)).min(requested as u64);
    usize::try_from(count).ok().filter(|count| *count > 0)
}

fn point_transform(index: usize, count: usize) -> Transform {
    let width = (count as f32).sqrt().ceil() as usize;
    let spacing = 3.2 / width as f32;
    Transform {
        translation: Vec3::new((index % width) as f32 * spacing - 1.6, 0.0, (index / width) as f32 * spacing - 1.6),
        rotation: Quat::from_rotation_y((index % 7) as f32 * 0.17),
        scale: Vec3::new(1.0, 0.6 + (index % 3) as f32 * 0.25, 0.8) * (spacing / 0.4),
    }
}

#[derive(Component)]
struct VirtualVertices(u32);

#[derive(Resource, Default)]
struct VirtualCounts(HashMap<MainEntity, u32>);

fn extract_counts(mut counts: ResMut<VirtualCounts>, query: Extract<Query<(Entity, &VirtualVertices)>>) {
    counts.0.clear();
    counts.0.extend(query.iter().map(|(entity, count)| (entity.into(), count.0)));
}

struct DrawVirtualMesh;

impl<P: PhaseItem> RenderCommand<P> for DrawVirtualMesh {
    type Param = (<DrawMesh as RenderCommand<P>>::Param, SRes<VirtualCounts>);
    type ViewQuery = <DrawMesh as RenderCommand<P>>::ViewQuery;
    type ItemQuery = ();

    fn render<'w>(item: &P, view: ROQueryItem<'w, '_, Self::ViewQuery>, entity: Option<()>,
        (original, counts): SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(&count) = counts.0.get(&item.main_entity()) else {
            return DrawMesh::render(item, view, entity, original, pass);
        };
        if !matches!(item.extra_index(), PhaseItemExtraIndex::None | PhaseItemExtraIndex::DynamicOffset(_)) {
            return RenderCommandResult::Failure("compact experiment requires direct draws");
        }
        if item.batch_range().len() != 1 {
            return RenderCommandResult::Failure("compact experiment requires one root per draw");
        }
        let Some(mesh) = original.1.into_inner().mesh_asset_id(item.main_entity()) else { return RenderCommandResult::Skip; };
        let Some(slice) = original.4.into_inner().mesh_vertex_slice(&mesh) else { return RenderCommandResult::Skip; };
        pass.set_vertex_buffer(0, slice.buffer.slice(..));
        pass.draw(0..count, item.batch_range().clone());
        RenderCommandResult::Success
    }
}

type DrawPulling = (SetItemPipeline, SetMeshViewBindGroup<0>, SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>, SetMaterialBindGroup<3>, DrawVirtualMesh);

#[derive(Resource)]
struct Options { expanded: bool, count: usize, output: String, batch_limit: usize }

#[derive(Resource)]
struct BatchReport { render_entities: usize, batches: usize, point_bytes: usize }

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!((3..=4).contains(&args.len()) && matches!(args[0].as_str(), "compact" | "expanded"),
        "usage: compact_instancing compact|expanded POINTS_PER_ROOT OUTPUT.png [POINTS_PER_BATCH]");
    let count: usize = args[1].parse().expect("point count");
    assert!((1..=1_000_000).contains(&count), "point count must be 1..1000000");
    let mut app = App::new();
    let batch_limit = args.get(3).map_or(usize::MAX, |value| value.parse().expect("batch limit"));
    assert!(batch_limit > 0, "batch limit must be nonzero");
    app.insert_resource(Options { expanded: args[0] == "expanded", count, output: args[2].clone(), batch_limit });
    app.add_plugins(DefaultPlugins.set(PbrPlugin { use_gpu_instance_buffer_builder: false, ..default() })
        .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
        .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
        .disable::<bevy::winit::WinitPlugin>());
    app.add_plugins(ScheduleRunnerPlugin::run_loop(std::time::Duration::from_secs_f64(1.0 / 60.0)));
    app.add_plugins(MaterialPlugin::<ExtendedMaterial<StandardMaterial, Pulling>>::default());
    app.world_mut().resource_mut::<Assets<Shader>>().insert(SHADER.id(),
        Shader::from_wgsl(include_str!("compact_instancing.wgsl"), "compact_instancing.wgsl")).unwrap();
    let render = app.sub_app_mut(RenderApp);
    use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingSupport, GpuPreprocessingMode};
    render.add_systems(bevy::render::RenderStartup,
        (|mut support: ResMut<GpuPreprocessingSupport>| support.max_supported_mode = GpuPreprocessingMode::None)
            .after(bevy::render::init_gpu_resource::<GpuPreprocessingSupport>));
    render.init_resource::<VirtualCounts>().add_systems(ExtractSchedule, extract_counts);
    let state = RenderCommandState::<Opaque3d, DrawPulling>::new(render.world_mut());
    render.world().resource::<DrawFunctions<Opaque3d>>().write().add_with::<DrawMaterial, _>(state);
    app.add_systems(Startup, setup).add_systems(Update, capture).run();
}

fn setup(mut commands: Commands, options: Res<Options>, mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>, mut pulling: ResMut<Assets<ExtendedMaterial<StandardMaterial, Pulling>>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>, mut images: ResMut<Assets<Image>>, device: Res<RenderDevice>,
) {
    let mut prototype = Mesh::from(Cuboid::new(0.28, 0.4, 0.22));
    prototype.duplicate_vertices();
    let VertexAttributeValues::Float32x3(positions) = prototype.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
    let VertexAttributeValues::Float32x3(normals) = prototype.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!("normals") };
    let VertexAttributeValues::Float32x2(uvs) = prototype.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else { panic!("uvs") };
    let vertices_per_point = positions.len() as u32;
    let packed: Vec<Vec4> = positions.iter().zip(normals).zip(uvs).flat_map(|((p, n), uv)|
        [Vec3::from(*p).extend(uv[0]), Vec3::from(*n).extend(uv[1])]).collect();
    let vertices = buffers.add(ShaderBuffer::from(packed));
    let prototype = meshes.add(prototype);
    let limits = device.limits();
    let capacity = batch_capacity(u64::from(limits.max_storage_buffer_binding_size), limits.max_buffer_size,
        vertices_per_point, options.batch_limit).expect("device cannot fit a point batch");
    let batches: Vec<_> = if options.expanded { Vec::new() } else {
        (0..options.count).step_by(capacity).map(|start| {
            let end = start.saturating_add(capacity).min(options.count);
            let points: Vec<Point> = (start..end).map(|index| point_transform(index, options.count).into()).collect();
            (buffers.add(ShaderBuffer::from(points)), end - start)
        }).collect()
    };
    for (root_index, color) in [Color::srgb(0.9, 0.12, 0.08), Color::srgb(0.08, 0.2, 0.9)].into_iter().enumerate() {
        let root = Transform::from_xyz(if root_index == 0 { -2.0 } else { 2.0 }, root_index as f32 * 0.3, 0.0)
            .with_rotation(Quat::from_rotation_y(root_index as f32 * 0.3));
        let base = StandardMaterial { base_color: color, perceptual_roughness: 0.7, ..default() };
        if options.expanded {
            let parent = commands.spawn((root, Visibility::default())).id();
            let material = materials.add(base);
            for index in 0..options.count {
                commands.spawn((Mesh3d(prototype.clone()), MeshMaterial3d(material.clone()), point_transform(index, options.count), ChildOf(parent)));
            }
        } else {
            for (points, count) in &batches {
                let material = pulling.add(ExtendedMaterial { base: base.clone(), extension: Pulling {
                    vertices: vertices.clone(), points: points.clone(), vertices_per_point,
                } });
                commands.spawn((Mesh3d(prototype.clone()), MeshMaterial3d(material), root,
                    VirtualVertices(vertices_per_point.checked_mul(*count as u32).unwrap()),
                    NoAutomaticBatching, NoFrustumCulling));
            }
        }
    }
    commands.spawn((DirectionalLight { illuminance: 15_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(3.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y)));
    let mut image = Image::new_target_texture(800, 600, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    commands.spawn((Camera3d::default(), RenderTarget::from(images.add(image)),
        Transform::from_xyz(7.0, 8.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y), Msaa::Off));
    let render_entities = if options.expanded { options.count * 2 } else { batches.len() * 2 };
    commands.insert_resource(BatchReport { render_entities, batches: batches.len(),
        point_bytes: if options.expanded { 0 } else { options.count * Point::min_size().get() as usize } });
    println!("COMPACT_EXPERIMENT mode={} roots=2 points={} render_entities={} batch_capacity={capacity} scope=forward-pbr-direct excludes=usd,shadows,prepass,indirect,picking,reload",
        if options.expanded { "expanded" } else { "compact" }, options.count * 2,
        render_entities);
}

fn capture(mut commands: Commands, mut frames: Local<u32>, camera: Query<&RenderTarget, With<Camera3d>>) {
    *frames += 1;
    if *frames != 120 { return; }
    commands.spawn(Screenshot(camera.single().unwrap().clone())).observe(
        |event: On<ScreenshotCaptured>, options: Res<Options>, report: Res<BatchReport>, meshes: Query<&Mesh3d>, mut exit: MessageWriter<AppExit>| {
            let count = meshes.iter().count();
            assert_eq!(count, report.render_entities);
            let memory = std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines()
                .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
                .collect::<Vec<_>>().join(" ");
            println!("COMPACT_CAPTURE render_entities={count} shared_point_buffers={} point_buffer_bytes={} {memory}", report.batches, report.point_bytes);
            let result = event.image.clone().try_into_dynamic().map_err(|error| error.to_string())
                .and_then(|image| {
                    std::fs::write(format!("{}.rgba", options.output), image.to_rgba8().as_raw()).map_err(|error| error.to_string())?;
                    image.save(&options.output).map_err(|error| error.to_string())
                });
            match result {
                Ok(()) => { println!("CAPTURE_OK {}", options.output); exit.write(AppExit::Success); }
                Err(error) => { eprintln!("CAPTURE_FAILED {error}"); exit.write(AppExit::error()); }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_batches_obey_storage_allocation_and_draw_limits() {
        assert_eq!(Point::min_size().get(), 48);
        assert_eq!(batch_capacity(47, 1024, 36, usize::MAX), None);
        assert_eq!(batch_capacity(1024, 47, 36, usize::MAX), None);
        assert_eq!(batch_capacity(48, 48, 36, usize::MAX), Some(1));
        assert_eq!(batch_capacity(480, 1024, 36, usize::MAX), Some(10));
        assert_eq!(batch_capacity(1024, 480, 36, usize::MAX), Some(10));
        assert_eq!(batch_capacity(480, 480, 36, 7), Some(7));
        assert_eq!(batch_capacity(480, 480, 36, 0), None);
        assert_eq!(batch_capacity(480, 480, 0, 7), None);
        assert_eq!(batch_capacity(u64::MAX, u64::MAX, u32::MAX, usize::MAX), Some(1));
        assert_eq!(batch_capacity(u64::MAX, u64::MAX, 36, usize::MAX), Some((u32::MAX / 36) as usize));
    }
}
