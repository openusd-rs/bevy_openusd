//! GPU instancing for large point instancers.
//!
//! A `PointInstancer` with more instances than [`UsdGpuInstancing::threshold`]
//! spawns no entity per instance. Its instances go to the GPU once, split into
//! chunks per prototype, and each prototype mesh draws all of its chunk's
//! instances with one indirect draw. Every frame a compute pass keeps only the
//! instances inside the view and larger than [`UsdGpuInstancing::min_pixels`]
//! on screen, so tens of millions of instances cost only what is visible.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::camera::visibility::{self, VisibilityClass};
use bevy::core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, Transparent3d, TransparentSortingInfo3d};
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::ecs::{
    query::ROQueryItem,
    system::{
        SystemParamItem,
        lifetimeless::{Read, SRes},
    },
};
use bevy::math::primitives::ViewFrustum;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, MeshPipelineSystems, PrepassPipeline, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup,
    Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache, ViewLightEntities,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::{
    Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
    render_asset::RenderAssets,
    render_phase::{
        AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
        PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
        TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
    },
    render_resource::{
        BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
        BufferDescriptor, BufferInitDescriptor, BufferUsages, CachedComputePipelineId,
        CompareFunction, ComputePassDescriptor, ComputePipelineDescriptor, DepthStencilState,
        PipelineCache, PrimitiveState, RenderPipelineDescriptor, ShaderStages,
        SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines,
        VertexState, binding_types,
    },
    renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
    sync_world::{MainEntity, RenderEntity, SyncToRenderWorld},
    view::{ExtractedView, RenderVisibleEntities, RetainedViewEntity},
};
use bevy::shader::ShaderDefVal;
use bytemuck::{Pod, Zeroable};

/// Instances per chunk: one buffer, one culling dispatch, one bounding box.
pub(crate) const CHUNK: usize = 1 << 20;
const WORKGROUP: u32 = 256;
/// Byte stride between per-view culling parameters in their shared buffer.
const VIEW_STRIDE: u64 = 256;

/// Turns on GPU instancing for point instancers. Present in the main world
/// once [`UsdGpuInstancingPlugin`] is added.
#[derive(Resource, ExtractResource, Debug, Clone, Copy)]
pub struct UsdGpuInstancing {
    /// Instancers with more instances than this are drawn on the GPU; smaller
    /// ones keep one entity per instance.
    pub threshold: usize,
    /// Instances whose bounding sphere projects to a smaller radius than this
    /// many pixels are not drawn.
    pub min_pixels: f32,
    /// Whether the instances a camera draws also cast its directional light
    /// shadows.
    pub shadows: bool,
}

impl Default for UsdGpuInstancing {
    fn default() -> Self {
        Self {
            threshold: 4096,
            min_pixels: 0.75,
            shadows: false,
        }
    }
}

/// Draws large point instancers with GPU culling and indirect draws.
pub struct UsdGpuInstancingPlugin;

impl Plugin for UsdGpuInstancingPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "gpu_instancing.wgsl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_cull.wgsl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_shadow.wgsl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_types.wgsl");
        bevy::asset::embedded_asset!(app, "sheen_functions.wgsl");
        if !app.world().contains_resource::<UsdGpuInstancing>() {
            app.init_resource::<UsdGpuInstancing>();
        }
        app.add_plugins(ExtractResourcePlugin::<UsdGpuInstancing>::default());
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<SpecializedMeshPipelines<InstancingPipeline>>()
            .init_resource::<SpecializedMeshPipelines<InstancedShadowPipeline>>()
            .init_resource::<GpuChunks>()
            .init_resource::<CullViews>()
            .init_resource::<QueuedShadows>()
            .add_render_command::<Transparent3d, DrawInstanced>()
            .add_render_command::<Shadow, DrawInstancedShadow>()
            .add_systems(
                RenderStartup,
                (
                    init_pipeline.after(MeshPipelineSystems),
                    init_shadow_pipeline
                        .after(init_pipeline)
                        .after(bevy::pbr::init_prepass_pipeline),
                ),
            )
            .add_systems(ExtractSchedule, extract_draws)
            .add_systems(
                Render,
                (
                    (prepare_chunks, prepare_views).in_set(RenderSystems::PrepareResources),
                    prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                    (queue_draws, queue_shadows).in_set(RenderSystems::QueueMeshes),
                ),
            )
            // Shadows draw the instances the camera kept, so culling runs first.
            .add_systems(
                Core3d,
                cull_instances
                    .before(bevy::pbr::per_view_shadow_pass::<{ bevy::pbr::EARLY_SHADOW_PASS }>)
                    .before(Core3dSystems::MainPass),
            );
    }
}

/// One instance in its instancer's space: translation, rotation as four
/// snorm16 values, and scale. Matches `Instance` in the WGSL.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct GpuInstance {
    translation: [f32; 3],
    rotation: [u32; 2],
    scale: [f32; 3],
}

impl GpuInstance {
    pub(crate) fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        let pack = |a: f32, b: f32| {
            let snorm = |v: f32| u32::from((v.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16);
            snorm(a) | (snorm(b) << 16)
        };
        let q = rotation.normalize();
        Self {
            translation: translation.to_array(),
            rotation: [pack(q.x, q.y), pack(q.z, q.w)],
            scale: scale.to_array(),
        }
    }

    /// Where a prototype-space sphere lands in instancer space.
    fn sphere(&self, sphere: Vec4) -> (Vec3, f32) {
        let unpack = |v: u32| {
            let snorm = |b: u32| (f32::from(b as u16 as i16) / 32767.0).max(-1.0);
            (snorm(v & 0xffff), snorm(v >> 16))
        };
        let (x, y) = unpack(self.rotation[0]);
        let (z, w) = unpack(self.rotation[1]);
        let rotation = Quat::from_xyzw(x, y, z, w).normalize();
        let scale = Vec3::from_array(self.scale);
        let center = rotation * (sphere.truncate() * scale) + Vec3::from_array(self.translation);
        (center, sphere.w * scale.abs().max_element())
    }
}

/// Instances of one prototype, uploaded to the GPU once and shared by every
/// mesh of that prototype.
pub(crate) struct InstanceChunk {
    id: u64,
    instances: Vec<GpuInstance>,
    /// The prototype's bounding sphere in prototype space: center and radius.
    sphere: Vec4,
}

impl InstanceChunk {
    /// Wraps `instances` as a chunk, with its bounding box in instancer space.
    pub(crate) fn new(instances: Vec<GpuInstance>, sphere: Vec4) -> (Arc<Self>, Aabb) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for instance in &instances {
            let (center, radius) = instance.sphere(sphere);
            min = min.min(center - radius);
            max = max.max(center + radius);
        }
        let chunk = Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            instances,
            sphere,
        };
        (Arc::new(chunk), Aabb::from_min_max(min, max))
    }
}

/// The material inputs an instanced draw shades with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InstancedMaterial {
    base_color: LinearRgba,
    emissive: LinearRgba,
    roughness: f32,
    metallic: f32,
    /// Disney sheen's weight and tint.
    sheen: [f32; 2],
}

impl From<&StandardMaterial> for InstancedMaterial {
    fn from(material: &StandardMaterial) -> Self {
        Self {
            base_color: material.base_color.to_linear(),
            emissive: material.emissive,
            roughness: material.perceptual_roughness,
            metallic: material.metallic,
            sheen: [0.0; 2],
        }
    }
}

/// One prototype mesh drawn for every instance of a chunk.
#[derive(Component, Clone)]
#[require(VisibilityClass, Transform, Visibility, SyncToRenderWorld)]
#[component(on_add = visibility::add_visibility_class::<UsdInstancedDraw>)]
pub struct UsdInstancedDraw {
    pub(crate) mesh: Handle<Mesh>,
    /// The mesh's placement inside the prototype.
    pub(crate) part: Mat4,
    pub(crate) material: InstancedMaterial,
    pub(crate) chunk: Arc<InstanceChunk>,
}

impl UsdInstancedDraw {
    /// How many instances this draw covers before culling.
    pub fn instance_count(&self) -> usize {
        self.chunk.instances.len()
    }
}

#[derive(Component)]
struct ExtractedDraw {
    mesh: AssetId<Mesh>,
    world: Mat4,
    part: Mat4,
    material: InstancedMaterial,
    chunk: Arc<InstanceChunk>,
}

fn extract_draws(
    mut commands: Commands,
    draws: Extract<Query<(RenderEntity, &UsdInstancedDraw, &GlobalTransform)>>,
) {
    for (entity, draw, transform) in &draws {
        commands.entity(entity).insert(ExtractedDraw {
            mesh: draw.mesh.id(),
            world: transform.to_matrix(),
            part: draw.part,
            material: draw.material,
            chunk: draw.chunk.clone(),
        });
    }
}

/// Per-chunk culling inputs. Matches `Chunk` in the culling WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct ChunkUniform {
    world_from_instancer: [[f32; 4]; 4],
    sphere: [f32; 4],
    len: u32,
    world_scale: f32,
    pad: [f32; 2],
}

/// Per-view culling inputs. Matches `CullView` in the culling WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct CullView {
    planes: [[f32; 4]; 6],
    /// Camera position, and pixels per unit of radius at unit distance.
    camera: [f32; 4],
    min_pixels: f32,
    pad: [f32; 3],
}

/// Per-draw shading inputs. Matches `Draw` in the drawing WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct DrawUniform {
    world_from_instancer: [[f32; 4]; 4],
    part: [[f32; 4]; 4],
    base_color: [f32; 4],
    emissive: [f32; 4],
    roughness: f32,
    metallic: f32,
    sheen: f32,
    sheen_tint: f32,
}

struct GpuChunk {
    instances: Buffer,
    visible: Buffer,
    count: Buffer,
    uniform: Buffer,
    len: u32,
    cull: Option<BindGroup>,
    seen: u64,
}

#[derive(Resource, Default)]
struct GpuChunks {
    chunks: HashMap<u64, GpuChunk>,
    frame: u64,
    /// The view buffer generation the culling bind groups were built for.
    views: u64,
}

#[derive(Resource, Default)]
struct CullViews {
    buffer: Option<Buffer>,
    capacity: u64,
    generation: u64,
}

/// Where a view's culling parameters sit in [`CullViews`].
#[derive(Component, Clone, Copy)]
struct CullViewOffset(u32);

#[derive(Component)]
struct GpuDraw {
    args: Buffer,
    uniform: Buffer,
    bind_group: BindGroup,
    chunk: u64,
}

#[derive(Resource)]
struct InstancingPipeline {
    mesh_pipeline: MeshPipeline,
    shader: Handle<Shader>,
    draw_layout: BindGroupLayoutDescriptor,
    cull_layout: BindGroupLayoutDescriptor,
    cull: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mesh_pipeline: Res<MeshPipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let draw_layout = BindGroupLayoutDescriptor::new(
        "usd_instanced_draw",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::uniform_buffer_sized(false, None),
            ),
        ),
    );
    let cull_layout = BindGroupLayoutDescriptor::new(
        "usd_instanced_cull",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_sized(false, None),
                binding_types::storage_buffer_sized(false, None),
                binding_types::uniform_buffer_sized(false, None),
                binding_types::uniform_buffer_sized(true, None),
            ),
        ),
    );
    let cull = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("usd_instanced_cull".into()),
        layout: vec![cull_layout.clone()],
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing_cull.wgsl"),
        ..default()
    });
    commands.insert_resource(InstancingPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing.wgsl"),
        draw_layout,
        cull_layout,
        cull,
    });
}

impl SpecializedMeshPipeline for InstancingPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;
        descriptor.label = Some("usd_instanced".into());
        descriptor.layout.truncate(2);
        descriptor.layout.push(self.draw_layout.clone());
        descriptor.primitive.cull_mode = None;
        descriptor.vertex.shader = self.shader.clone();
        let material_group = ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3);
        descriptor.vertex.shader_defs.push(material_group.clone());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = self.shader.clone();
            fragment.shader_defs.push(material_group);
        }
        Ok(descriptor)
    }
}

/// Depth-only drawing of instanced prototypes into directional shadow maps.
#[derive(Resource)]
struct InstancedShadowPipeline {
    view_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    draw_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    unclipped_depth: bool,
}

fn init_shadow_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    instancing: Option<Res<InstancingPipeline>>,
    prepass: Option<Res<PrepassPipeline>>,
) {
    let (Some(instancing), Some(prepass)) = (instancing, prepass) else {
        return;
    };
    commands.insert_resource(InstancedShadowPipeline {
        view_layout: prepass.view_layout_no_motion_vectors.clone(),
        empty_layout: prepass.empty_layout.clone(),
        draw_layout: instancing.draw_layout.clone(),
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing_shadow.wgsl"),
        unclipped_depth: prepass.depth_clip_control_supported,
    });
}

impl SpecializedMeshPipeline for InstancedShadowPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let shader_defs = if self.unclipped_depth {
            Vec::new()
        } else {
            vec!["CLAMP_DEPTH".into()]
        };
        Ok(RenderPipelineDescriptor {
            label: Some("usd_instanced_shadow".into()),
            layout: vec![
                self.view_layout.clone(),
                self.empty_layout.clone(),
                self.draw_layout.clone(),
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs,
                buffers: vec![
                    layout
                        .0
                        .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?,
                ],
                ..default()
            },
            primitive: PrimitiveState {
                topology: key.primitive_topology(),
                strip_index_format: key.strip_index_format(),
                unclipped_depth: self.unclipped_depth,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            ..default()
        })
    }
}

fn prepare_chunks(
    draws: Query<&ExtractedDraw>,
    mut chunks: ResMut<GpuChunks>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    chunks.frame += 1;
    let frame = chunks.frame;
    for draw in &draws {
        let source = &draw.chunk;
        if chunks
            .chunks
            .get(&source.id)
            .is_some_and(|chunk| chunk.seen == frame)
        {
            continue;
        }
        let chunk = chunks.chunks.entry(source.id).or_insert_with(|| {
            let len = source.instances.len() as u32;
            let storage = |label, size: u64, usage| {
                device.create_buffer(&BufferDescriptor {
                    label: Some(label),
                    size: size.max(4),
                    usage,
                    mapped_at_creation: false,
                })
            };
            GpuChunk {
                instances: device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("usd_instances"),
                    contents: bytemuck::cast_slice(&source.instances),
                    usage: BufferUsages::STORAGE,
                }),
                visible: storage(
                    "usd_visible_instances",
                    u64::from(len) * 4,
                    BufferUsages::STORAGE,
                ),
                count: storage(
                    "usd_visible_count",
                    4,
                    BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                ),
                uniform: storage(
                    "usd_instance_chunk",
                    size_of::<ChunkUniform>() as u64,
                    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                ),
                len,
                cull: None,
                seen: frame,
            }
        });
        chunk.seen = frame;
        let (scale, _, _) = draw.world.to_scale_rotation_translation();
        let uniform = ChunkUniform {
            world_from_instancer: draw.world.to_cols_array_2d(),
            sphere: source.sphere.to_array(),
            len: chunk.len,
            world_scale: scale.abs().max_element(),
            pad: [0.0; 2],
        };
        queue.write_buffer(&chunk.uniform, 0, bytemuck::bytes_of(&uniform));
    }
    chunks.chunks.retain(|_, chunk| chunk.seen + 2 >= frame);
}

fn prepare_views(
    mut commands: Commands,
    views: Query<(Entity, &ExtractedView)>,
    mut cull_views: ResMut<CullViews>,
    settings: Option<Res<UsdGpuInstancing>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let min_pixels = settings.map_or(UsdGpuInstancing::default().min_pixels, |s| s.min_pixels);
    let mut bytes = Vec::new();
    for (index, (entity, view)) in views.iter().enumerate() {
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let frustum = ViewFrustum::from_clip_from_world(&clip_from_world);
        let mut planes = [[0.0, 0.0, 0.0, 1.0]; 6];
        for (plane, half_space) in planes.iter_mut().zip(&frustum.half_spaces[..5]) {
            *plane = half_space.normal_d().to_array();
        }
        let orthographic = view.clip_from_view.w_axis.w == 1.0;
        let pixels = view.clip_from_view.y_axis.y * view.viewport.w as f32 * 0.5;
        let camera = view.world_from_view.translation();
        let cull = CullView {
            planes,
            camera: camera.extend(pixels).to_array(),
            min_pixels: if orthographic { 0.0 } else { min_pixels },
            pad: [0.0; 3],
        };
        bytes.resize(index * VIEW_STRIDE as usize, 0);
        bytes.extend_from_slice(bytemuck::bytes_of(&cull));
        commands
            .entity(entity)
            .insert(CullViewOffset((index as u64 * VIEW_STRIDE) as u32));
    }
    if bytes.is_empty() {
        return;
    }
    let needed = bytes.len() as u64;
    if cull_views.capacity < needed {
        cull_views.capacity = needed.next_power_of_two().max(VIEW_STRIDE * 4);
        cull_views.buffer = Some(device.create_buffer(&BufferDescriptor {
            label: Some("usd_instanced_views"),
            size: cull_views.capacity,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        cull_views.generation += 1;
    }
    queue.write_buffer(cull_views.buffer.as_ref().unwrap(), 0, &bytes);
}

fn prepare_bind_groups(
    mut commands: Commands,
    draws: Query<(Entity, &ExtractedDraw, Option<&GpuDraw>)>,
    mut chunks: ResMut<GpuChunks>,
    cull_views: Res<CullViews>,
    pipeline: Option<Res<InstancingPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let (Some(pipeline), Some(views)) = (pipeline, cull_views.buffer.as_ref()) else {
        return;
    };
    let rebuild = chunks.views != cull_views.generation;
    chunks.views = cull_views.generation;
    let cull_layout = pipeline_cache.get_bind_group_layout(&pipeline.cull_layout);
    for chunk in chunks.chunks.values_mut() {
        if chunk.cull.is_none() || rebuild {
            chunk.cull = Some(device.create_bind_group(
                "usd_instanced_cull",
                &cull_layout,
                &BindGroupEntries::sequential((
                    chunk.instances.as_entire_binding(),
                    chunk.visible.as_entire_binding(),
                    chunk.count.as_entire_binding(),
                    chunk.uniform.as_entire_binding(),
                    bevy::render::render_resource::BufferBinding {
                        buffer: views,
                        offset: 0,
                        size: std::num::NonZero::new(size_of::<CullView>() as u64),
                    },
                )),
            ));
        }
    }
    let draw_layout = pipeline_cache.get_bind_group_layout(&pipeline.draw_layout);
    for (entity, draw, existing) in &draws {
        let Some(chunk) = chunks.chunks.get(&draw.chunk.id) else {
            continue;
        };
        let (Some(mesh), Some(vertices), Some(indices)) = (
            meshes.get(draw.mesh),
            allocator.mesh_vertex_slice(&draw.mesh),
            allocator.mesh_index_slice(&draw.mesh),
        ) else {
            continue;
        };
        let RenderMeshBufferInfo::Indexed { count, .. } = mesh.buffer_info else {
            continue;
        };
        let gpu = match existing {
            Some(gpu) if gpu.chunk == draw.chunk.id => None,
            _ => {
                let args = device.create_buffer(&BufferDescriptor {
                    label: Some("usd_instanced_args"),
                    size: 20,
                    usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let uniform = device.create_buffer(&BufferDescriptor {
                    label: Some("usd_instanced_draw"),
                    size: size_of::<DrawUniform>() as u64,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = device.create_bind_group(
                    "usd_instanced_draw",
                    &draw_layout,
                    &BindGroupEntries::sequential((
                        chunk.instances.as_entire_binding(),
                        chunk.visible.as_entire_binding(),
                        uniform.as_entire_binding(),
                    )),
                );
                Some(GpuDraw {
                    args,
                    uniform,
                    bind_group,
                    chunk: draw.chunk.id,
                })
            }
        };
        let target = gpu.as_ref().or(existing).unwrap();
        let args: [u32; 5] = [count, 0, indices.range.start, vertices.range.start, 0];
        queue.write_buffer(&target.args, 0, bytemuck::cast_slice(&args));
        let uniform = DrawUniform {
            world_from_instancer: draw.world.to_cols_array_2d(),
            part: draw.part.to_cols_array_2d(),
            base_color: draw.material.base_color.to_f32_array(),
            emissive: draw.material.emissive.to_f32_array(),
            roughness: draw.material.roughness,
            metallic: draw.material.metallic,
            sheen: draw.material.sheen[0],
            sheen_tint: draw.material.sheen[1],
        };
        queue.write_buffer(&target.uniform, 0, bytemuck::bytes_of(&uniform));
        if let Some(gpu) = gpu {
            commands.entity(entity).insert(gpu);
        }
    }
}

fn queue_draws(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    pipeline: Option<Res<InstancingPipeline>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<InstancingPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    draws: Query<&ExtractedDraw, With<GpuDraw>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    view_keys: Res<ViewKeyCache>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draw_functions.read().id::<DrawInstanced>();
    for (view, visible) in &views {
        let (Some(phase), Some(&view_key), Some(visible)) = (
            phases.get_mut(&view.retained_view_entity),
            view_keys.get(&view.retained_view_entity),
            visible.get::<UsdInstancedDraw>(),
        ) else {
            continue;
        };
        for (&entity, &main_entity) in visible.iter_visible() {
            let Ok(draw) = draws.get(entity) else {
                continue;
            };
            let Some(mesh) = meshes.get(draw.mesh) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let Ok(pipeline_id) =
                pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout)
            else {
                continue;
            };
            phase.add_transient(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: draw.world.transform_point3(draw.chunk.sphere.truncate()),
                    depth_bias: 0.0,
                },
                entity: (entity, main_entity),
                pipeline: pipeline_id,
                draw_function,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

/// The instanced draws queued last frame into each shadow phase, which keeps
/// its items until they are removed.
#[derive(Resource, Default)]
struct QueuedShadows(HashMap<RetainedViewEntity, Vec<MainEntity>>);

/// Queues the draws each camera sees into its directional shadow cascades.
fn queue_shadows(
    draw_functions: Res<DrawFunctions<Shadow>>,
    pipeline: Option<Res<InstancedShadowPipeline>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<InstancedShadowPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    draws: Query<&ExtractedDraw, With<GpuDraw>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    cameras: Query<(&RenderVisibleEntities, &ViewLightEntities)>,
    light_views: Query<&ExtractedView>,
    mut queued: ResMut<QueuedShadows>,
    settings: Option<Res<UsdGpuInstancing>>,
) {
    for (view, entities) in queued.0.drain() {
        if let Some(phase) = phases.get_mut(&view) {
            for entity in entities {
                phase.remove(entity);
            }
        }
    }
    let (Some(pipeline), true) = (pipeline, settings.is_some_and(|settings| settings.shadows))
    else {
        return;
    };
    let draw_function = draw_functions.read().id::<DrawInstancedShadow>();
    for (visible, lights) in &cameras {
        let Some(visible) = visible.get::<UsdInstancedDraw>() else {
            continue;
        };
        for light in &lights.lights {
            let Ok(light_view) = light_views.get(*light) else {
                continue;
            };
            let Some(phase) = phases.get_mut(&light_view.retained_view_entity) else {
                continue;
            };
            let added = queued.0.entry(light_view.retained_view_entity).or_default();
            for (&entity, &main_entity) in visible.iter_visible() {
                let Ok(draw) = draws.get(entity) else {
                    continue;
                };
                let (Some(mesh), Some(slabs)) =
                    (meshes.get(draw.mesh), allocator.mesh_slabs(&draw.mesh))
                else {
                    continue;
                };
                let key = MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
                let Ok(pipeline_id) =
                    pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout)
                else {
                    continue;
                };
                phase.add(
                    ShadowBatchSetKey {
                        pipeline: pipeline_id,
                        draw_function,
                        material_bind_group_index: None,
                        slabs,
                    },
                    ShadowBinKey {
                        asset_id: draw.mesh.untyped(),
                    },
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
                added.push(main_entity);
            }
        }
    }
}

fn cull_instances(
    view: ViewQuery<(&CullViewOffset, &RenderVisibleEntities)>,
    draws: Query<(&ExtractedDraw, &GpuDraw)>,
    chunks: Res<GpuChunks>,
    pipeline: Option<Res<InstancingPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (offset, visible) = view.into_inner();
    let Some(visible) = visible.get::<UsdInstancedDraw>() else {
        return;
    };
    let Some(cull) = pipeline
        .as_ref()
        .and_then(|pipeline| pipeline_cache.get_compute_pipeline(pipeline.cull))
    else {
        return;
    };
    let mut culled = HashSet::new();
    let mut copies = Vec::new();
    for (&entity, _) in visible.iter_visible() {
        let Ok((draw, gpu)) = draws.get(entity) else {
            continue;
        };
        let Some(chunk) = chunks.chunks.get(&draw.chunk.id) else {
            continue;
        };
        culled.insert(draw.chunk.id);
        copies.push((chunk, gpu));
    }
    if copies.is_empty() {
        return;
    }
    let encoder = ctx.command_encoder();
    for id in &culled {
        encoder.clear_buffer(&chunks.chunks[id].count, 0, None);
    }
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("usd_instanced_cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(cull);
        for id in &culled {
            let chunk = &chunks.chunks[id];
            let Some(bind_group) = &chunk.cull else {
                continue;
            };
            pass.set_bind_group(0, bind_group, &[offset.0]);
            pass.dispatch_workgroups(chunk.len.div_ceil(WORKGROUP), 1, 1);
        }
    }
    for (chunk, gpu) in copies {
        encoder.copy_buffer_to_buffer(&chunk.count, 0, &gpu.args, 4, 4);
    }
}

type DrawInstanced = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawInstancedIndirect,
);

type DrawInstancedShadow = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    DrawInstancedIndirect,
);

struct DrawInstancedIndirect;

impl<P: PhaseItem> RenderCommand<P> for DrawInstancedIndirect {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>);
    type ViewQuery = ();
    type ItemQuery = (Read<ExtractedDraw>, Read<GpuDraw>);

    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        item: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (meshes, allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some((draw, gpu)) = item else {
            return RenderCommandResult::Skip;
        };
        let allocator = allocator.into_inner();
        let Some(mesh) = meshes.into_inner().get(draw.mesh) else {
            return RenderCommandResult::Skip;
        };
        let (Some(vertices), Some(indices)) = (
            allocator.mesh_vertex_slice(&draw.mesh),
            allocator.mesh_index_slice(&draw.mesh),
        ) else {
            return RenderCommandResult::Skip;
        };
        let RenderMeshBufferInfo::Indexed { index_format, .. } = mesh.buffer_info else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(2, &gpu.bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.buffer.slice(..));
        pass.set_index_buffer(indices.buffer.slice(..), index_format);
        pass.draw_indexed_indirect(&gpu.args, 0);
        RenderCommandResult::Success
    }
}

/// Whether a point instancer with `count` instances draws on the GPU.
pub(crate) fn enabled(world: &World, count: usize) -> bool {
    world
        .get_resource::<UsdGpuInstancing>()
        .is_some_and(|settings| count > settings.threshold)
}

/// Despawns the instanced draws under `instancer`.
pub(crate) fn clear(world: &mut World, instancer: Entity) {
    let draws: Vec<Entity> = world
        .get::<Children>(instancer)
        .into_iter()
        .flat_map(|children| children.iter())
        .filter(|child| world.get::<UsdInstancedDraw>(*child).is_some())
        .collect();
    for draw in draws {
        world.despawn(draw);
    }
}

/// Spawns one instanced draw per chunk for each prototype mesh.
pub(crate) fn spawn(
    world: &mut World,
    instancer: Entity,
    meshes: &[(Handle<Mesh>, Handle<StandardMaterial>, Mat4)],
    chunks: Vec<Vec<GpuInstance>>,
) {
    let sphere = prototype_sphere(world, meshes);
    for instances in chunks {
        let (chunk, bounds) = InstanceChunk::new(instances, sphere);
        for (mesh, material, part) in meshes {
            let sheen = super::cache::sheen_of(world, material.id())
                .map_or([0.0; 2], |sheen| [sheen.weight, sheen.tint]);
            let material = world
                .resource::<Assets<StandardMaterial>>()
                .get(material)
                .map(InstancedMaterial::from)
                .map(|material| InstancedMaterial { sheen, ..material })
                .unwrap_or(InstancedMaterial::from(&StandardMaterial::default()));
            world.spawn((
                UsdInstancedDraw {
                    mesh: mesh.clone(),
                    part: *part,
                    material,
                    chunk: chunk.clone(),
                },
                bounds,
                ChildOf(instancer),
            ));
        }
    }
}

/// The bounding sphere of a prototype's meshes, in prototype space.
fn prototype_sphere(
    world: &World,
    meshes: &[(Handle<Mesh>, Handle<StandardMaterial>, Mat4)],
) -> Vec4 {
    let assets = world.resource::<Assets<Mesh>>();
    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for (mesh, _, part) in meshes {
        let Some(aabb) = assets.get(mesh).and_then(|mesh| mesh.compute_aabb()) else {
            continue;
        };
        let (center, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for corner in 0..8 {
            let sign = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let point = part.transform_point3(center + half * sign);
            min = min.min(point);
            max = max.max(point);
        }
    }
    if !min.is_finite() {
        return Vec4::ZERO;
    }
    let center = (min + max) * 0.5;
    center.extend((max - center).length())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::instancer::UsdInstance;

    #[test]
    fn large_instancers_become_shared_draws_without_instance_entities() {
        let count = 5000;
        let positions: Vec<_> = (0..count).map(|i| format!("({i},0,0)")).collect();
        let indices: Vec<_> = (0..count).map(|i| (i % 2).to_string()).collect();
        let text = format!(
            r#"#usda 1.0
def PointInstancer "Field" {{
    point3f[] positions = [{}]
    int[] protoIndices = [{}]
    int64[] invisibleIds = [0, 1, 2]
    rel prototypes = [</Field/Protos/Box>, </Field/Protos/Tree>]
    def Scope "Protos" {{
        def Cube "Box" {{ double size = 1 }}
        def Xform "Tree" {{
            def Cube "Trunk" {{ double size = 1 }}
            def Sphere "Crown" {{
                double3 xformOp:translate = (0, 2, 0)
                uniform token[] xformOpOrder = ["xformOp:translate"]
            }}
        }}
    }}
}}
"#,
            positions.join(","),
            indices.join(",")
        );
        let stage = crate::snippet::UsdSnippet::new(text).open_stage().unwrap();
        let live = crate::live::LiveStage::new(stage);
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<UsdGpuInstancing>();
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let field = map.entity("/Field").unwrap();
        let children: Vec<Entity> = world.get::<Children>(field).unwrap().iter().collect();
        assert!(
            children
                .iter()
                .all(|child| world.get::<UsdInstance>(*child).is_none())
        );
        let draws: Vec<_> = children
            .iter()
            .filter_map(|child| world.get::<UsdInstancedDraw>(*child))
            .collect();
        assert_eq!(draws.len(), 3, "one box and the tree's trunk and crown");
        let mut chunks: Vec<_> = draws
            .iter()
            .map(|draw| (draw.chunk.id, draw.instance_count()))
            .collect();
        chunks.sort_unstable();
        chunks.dedup();
        assert_eq!(
            chunks.iter().map(|(_, count)| count).sum::<usize>(),
            count - 3
        );
        let crown = draws
            .iter()
            .find(|draw| draw.part.w_axis.y == 2.0)
            .expect("the crown keeps its placement inside the prototype");
        assert_eq!(crown.instance_count(), count / 2 - 1);
        for prototype in ["/Field/Protos/Box", "/Field/Protos/Tree"] {
            let entity = map.entity(prototype).unwrap();
            assert_eq!(world.get::<Visibility>(entity), Some(&Visibility::Hidden));
        }
        let scope = map.entity("/Field/Protos").unwrap();
        assert_ne!(world.get::<Visibility>(scope), Some(&Visibility::Hidden));
    }
}
