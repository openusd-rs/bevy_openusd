//! Bounded, scoped tangent generation for built-in animation updates.

use bevy::{prelude::*, tasks::TaskPool};

use super::{RouteCtx, cache};
use crate::read::{geom::ReadMesh, skel};

pub(super) const MAX_BATCH_PRIMS: usize = 128;
const MAX_BATCH_MESHES: usize = 8;

/// Limits parallel tangent work per animation batch; zero disables preparation.
#[derive(Resource, Clone, Copy)]
pub struct ParallelMeshPreparation {
    /// Concurrent mesh jobs, capped at eight per batch.
    pub max_jobs: usize,
    /// Worker input mesh payload, excluding USD reads, tangent output and scratch.
    pub max_input_bytes: usize,
}

impl Default for ParallelMeshPreparation {
    fn default() -> Self {
        Self {
            max_jobs: 4,
            max_input_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Opt-in batch wall time, including stage reads, cache lookup, dispatch and join.
#[derive(Resource, Default)]
pub struct MeshPreparationTiming {
    pub batches: u64,
    pub parallel_batches: u64,
    pub inline_jobs: u64,
    pub jobs: u64,
    pub cache_hits: u64,
    pub peak_jobs: usize,
    pub peak_input_bytes: usize,
    pub elapsed: std::time::Duration,
}

/// Opt-in non-overlapping batch phases and overlapping tangent-job wall times.
#[derive(Resource, Default)]
pub struct MeshPreparationPhaseTiming {
    pub batches: u64,
    pub phases: std::collections::BTreeMap<&'static str, std::time::Duration>,
    pub tangent_jobs: u64,
    pub tangent_failures: u64,
    pub tangent_vertices: u64,
    pub tangent_elapsed: std::time::Duration,
    pub tangent_max: std::time::Duration,
}

struct PhaseSample {
    since: bevy::platform::time::Instant,
    timing: MeshPreparationPhaseTiming,
}

fn phase_lap(sample: &mut Option<PhaseSample>, phase: &'static str) {
    if let Some(sample) = sample {
        let now = bevy::platform::time::Instant::now();
        *sample.timing.phases.entry(phase).or_default() += now.duration_since(sample.since);
        sample.since = now;
    }
}

pub(super) struct PreparedDeformation {
    pub read: ReadMesh,
    pub mesh: Mesh,
    pub tangents_ready: bool,
    skin: Option<SkinnedTangentInput>,
}

/// Pre-skin mesh whose tangents are carried into the deformed mesh by the
/// blended skin matrix of each vertex.
struct SkinnedTangentInput {
    mesh: Mesh,
    matrices: Vec<Mat3>,
    morphed: bool,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationKinds {
    pub skinned: bool,
    pub blended: bool,
    pub varying: bool,
}

impl DeformationKinds {
    pub fn read(ctx: &RouteCtx) -> Self {
        let skinned = skel::is_skinned(ctx.stage, ctx.path);
        let blended = skel::has_blend_shapes(ctx.stage, ctx.path);
        let varying =
            (skinned || blended) && skel::deformation_is_time_varying(ctx.stage, ctx.path);
        Self {
            skinned,
            blended,
            varying,
        }
    }
}

impl PreparedDeformation {
    pub fn with_tangent_source(
        read: ReadMesh,
        source: Option<super::skel::SkinnedTangentSource>,
    ) -> Self {
        let mesh = crate::mesh::assemble_mesh(&read, None, false);
        let skin = source.and_then(|source| {
            let input = crate::mesh::assemble_mesh(&source.read, None, false);
            let mapping = crate::mesh::vertex_point_indices(&read);
            (input.count_vertices() == mesh.count_vertices()
                && mapping.len() == mesh.count_vertices()
                && mapping.iter().all(|&point| point < source.matrices.len()))
            .then(|| SkinnedTangentInput {
                mesh: input,
                matrices: mapping
                    .iter()
                    .map(|&point| source.matrices[point])
                    .collect(),
                morphed: source.morphed,
            })
        });
        Self {
            read,
            mesh,
            tangents_ready: false,
            skin,
        }
    }

    /// The mesh MikkTSpace runs on: pre-skin geometry when skinned.
    pub fn tangent_input(&self) -> &Mesh {
        self.skin.as_ref().map_or(&self.mesh, |skin| &skin.mesh)
    }

    fn tangent_input_mut(&mut self) -> &mut Mesh {
        self.skin
            .as_mut()
            .map_or(&mut self.mesh, |skin| &mut skin.mesh)
    }

    /// Whether the tangent input changes with the pose (cache admission).
    pub fn tangent_input_varies(&self) -> bool {
        self.skin.as_ref().is_none_or(|skin| skin.morphed)
    }

    pub fn take_tangent_input(&mut self) -> Mesh {
        let empty = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        std::mem::replace(self.tangent_input_mut(), empty)
    }

    pub fn restore_tangent_input(&mut self, mesh: Mesh) {
        *self.tangent_input_mut() = mesh;
        self.apply_skinned_tangents();
        self.tangents_ready = true;
    }

    /// Writes the skinned pre-skin tangents onto the deformed mesh.
    pub fn apply_skinned_tangents(&mut self) {
        let Some(skin) = &self.skin else {
            return;
        };
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(tangents)) =
            skin.mesh.attribute(Mesh::ATTRIBUTE_TANGENT)
        else {
            self.mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
            return;
        };
        let skinned: Vec<_> = tangents
            .iter()
            .zip(&skin.matrices)
            .map(|(tangent, matrix)| {
                let direction = Vec3::from_slice(&tangent[..3]);
                let value = (*matrix * direction).try_normalize().unwrap_or(direction);
                [value.x, value.y, value.z, tangent[3]]
            })
            .collect();
        self.mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, skinned);
    }

    pub fn generate_tangents(&mut self, world: &mut World) {
        if !self.tangents_ready && self.read.uvs.is_some() {
            if self.tangent_input_varies() {
                cache::generate_cached_deformed_tangents(world, self.tangent_input_mut());
            } else {
                cache::generate_cached_tangents(world, self.tangent_input_mut());
            }
            self.apply_skinned_tangents();
        }
        self.tangents_ready = true;
    }
}

fn needs_deformation(ctx: &RouteCtx, world: &World, phases: &mut Option<PhaseSample>) -> bool {
    if ctx.type_name.as_deref() != Some("Mesh") || super::subdivision::enabled(ctx, world) {
        return false;
    }
    let kinds = DeformationKinds::read(ctx);
    phase_lap(phases, "eligibility-kinds");
    ctx.prepared_kinds.set(Some(kinds));
    if !kinds.varying {
        return false;
    }
    let mesh = ctx.read_mesh();
    phase_lap(phases, "eligibility-mesh-read");
    let Ok(Some(read)) = mesh else {
        return false;
    };
    if read.uvs.is_none() || read.points.is_empty() {
        return false;
    }
    if !world.contains_resource::<super::gpu_skin::GpuSkinningEnabled>() {
        return true;
    }
    if !kinds.skinned {
        return false;
    }
    let sample = skel::gpu_skin_sample_with_mesh(ctx.stage, ctx.path, ctx.time, read);
    phase_lap(phases, "eligibility-skin-sample");
    let needed = sample.is_err();
    *ctx.prepared_skin.borrow_mut() = Some(sample);
    needed
}

pub(super) fn prepare_batch(
    contexts: &[(RouteCtx, Entity)],
    world: &mut World,
    pool: &TaskPool,
    settings: ParallelMeshPreparation,
) -> usize {
    let started = world
        .contains_resource::<MeshPreparationTiming>()
        .then(bevy::platform::time::Instant::now);
    let mut phases = world
        .contains_resource::<MeshPreparationPhaseTiming>()
        .then(|| PhaseSample {
            since: bevy::platform::time::Instant::now(),
            timing: default(),
        });
    let mut pending = Vec::new();
    let mut jobs = Vec::new();
    let mut input_bytes = 0usize;
    let mut cache_hits = 0;
    let mut mesh_prims = 0;
    let mut consumed = 0;
    let defer_initial = world.contains_resource::<super::residency::DeferHiddenMeshes>();
    for (index, (ctx, entity)) in contexts.iter().enumerate() {
        if mesh_prims >= settings.max_jobs.min(MAX_BATCH_MESHES)
            || input_bytes >= settings.max_input_bytes
        {
            break;
        }
        consumed = index + 1;
        mesh_prims += usize::from(ctx.type_name.as_deref() == Some("Mesh"));
        if defer_initial && world.get::<Mesh3d>(*entity).is_none() {
            continue;
        }
        let needed = needs_deformation(ctx, world, &mut phases);
        phase_lap(&mut phases, "eligibility");
        if !needed {
            continue;
        }
        let result = super::skel::deformed_mesh_with_tangent_source(ctx);
        phase_lap(&mut phases, "deformation");
        let result = result.map(|deformed| {
            deformed.map(|(read, source)| PreparedDeformation::with_tangent_source(read, source))
        });
        phase_lap(&mut phases, "assembly");
        let mut prepared = match result {
            Ok(Some(prepared)) => prepared,
            other => {
                *ctx.prepared_deformation.borrow_mut() = Some(other);
                continue;
            }
        };
        let bytes = cache::mesh_payload_bytes(prepared.tangent_input());
        if bytes > settings.max_input_bytes.saturating_sub(input_bytes) {
            *ctx.prepared_deformation.borrow_mut() = Some(Ok(Some(prepared)));
            phase_lap(&mut phases, "cache-lookup");
            break;
        }
        let varies = prepared.tangent_input_varies();
        match cache::prepare_tangents(world, prepared.tangent_input_mut(), varies) {
            Some(ticket) => {
                input_bytes += bytes;
                let job = prepared.take_tangent_input();
                pending.push((index, prepared, ticket));
                jobs.push((index, job));
            }
            None => {
                prepared.apply_skinned_tangents();
                prepared.tangents_ready = true;
                *ctx.prepared_deformation.borrow_mut() = Some(Ok(Some(prepared)));
                cache_hits += 1;
            }
        }
        phase_lap(&mut phases, "cache-lookup");
    }
    phase_lap(&mut phases, "traversal");
    let count = jobs.len();
    let profiled = phases.is_some();
    if let Some(phases) = phases.as_mut() {
        phases.timing.tangent_vertices = jobs
            .iter()
            .map(|(_, mesh)| mesh.count_vertices() as u64)
            .sum();
    }
    let generate = |(index, mut mesh): (usize, Mesh)| {
        let started = profiled.then(bevy::platform::time::Instant::now);
        let result = mesh.generate_tangents();
        let elapsed = started.map(|started| started.elapsed());
        (index, mesh, result, elapsed)
    };
    let mut results = if count < 2 {
        jobs.into_iter().map(generate).collect()
    } else {
        jobs.sort_unstable_by_key(|(_, mesh)| std::cmp::Reverse(mesh.count_vertices()));
        pool.scope_with_executor(false, None, |scope| {
            for (position, job) in jobs.into_iter().enumerate() {
                let task = async move { generate(job) };
                if position == 0 {
                    scope.spawn_on_scope(task);
                } else {
                    scope.spawn(task);
                }
            }
        })
    };
    phase_lap(&mut phases, "tangent-dispatch-join");
    results.sort_unstable_by_key(|(index, _, _, _)| *index);
    for ((index, mesh, result, elapsed), (expected, mut prepared, ticket)) in
        results.into_iter().zip(pending)
    {
        assert_eq!(index, expected);
        if let (Some(phases), Some(elapsed)) = (phases.as_mut(), elapsed) {
            phases.timing.tangent_jobs += 1;
            phases.timing.tangent_failures += u64::from(result.is_err());
            phases.timing.tangent_elapsed += elapsed;
            phases.timing.tangent_max = phases.timing.tangent_max.max(elapsed);
        }
        cache::finish_tangents(world, &mesh, ticket, result);
        prepared.restore_tangent_input(mesh);
        *contexts[index].0.prepared_deformation.borrow_mut() = Some(Ok(Some(prepared)));
    }
    phase_lap(&mut phases, "publication");
    if let Some(started) = started {
        let mut timing = world.resource_mut::<MeshPreparationTiming>();
        timing.batches += 1;
        timing.parallel_batches += u64::from(count > 1);
        timing.inline_jobs += u64::from(count == 1);
        timing.jobs += count as u64;
        timing.cache_hits += cache_hits;
        timing.peak_jobs = timing.peak_jobs.max(count);
        timing.peak_input_bytes = timing.peak_input_bytes.max(input_bytes);
        timing.elapsed += started.elapsed();
    }
    if let Some(phases) = phases {
        let mut timing = world.resource_mut::<MeshPreparationPhaseTiming>();
        timing.batches += 1;
        for (phase, elapsed) in phases.timing.phases {
            *timing.phases.entry(phase).or_default() += elapsed;
        }
        timing.tangent_jobs += phases.timing.tangent_jobs;
        timing.tangent_failures += phases.timing.tangent_failures;
        timing.tangent_vertices += phases.timing.tangent_vertices;
        timing.tangent_elapsed += phases.timing.tangent_elapsed;
        timing.tangent_max = timing.tangent_max.max(phases.timing.tangent_max);
    }
    consumed
}

#[cfg(test)]
mod paired_probe;

#[cfg(test)]
mod tests {
    use super::super::{PrimRoute, SchemaRegistry, StageTime, gpu_skin, residency};
    use super::*;
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
    use openusd::{sdf::Path, usd::Stage, vt::Value};

    fn stage() -> Stage {
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_morph_blended_animated.usda"
        );
        crate::UsdSource::new(file, std::fs::read(file).unwrap())
            .unwrap()
            .open_stage()
            .unwrap()
    }

    fn world(gpu: bool) -> (World, Vec<(Path, Entity)>) {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        world.init_resource::<MeshPreparationTiming>();
        world.init_resource::<cache::MeshCacheMetrics>();
        world.insert_resource(ParallelMeshPreparation {
            max_jobs: 2,
            max_input_bytes: 4096,
        });
        if gpu {
            world.init_resource::<gpu_skin::GpuSkinningEnabled>();
        }
        let prims = (0..11)
            .map(|_| {
                (
                    openusd::sdf::path("/Test/Face").unwrap(),
                    world.spawn_empty().id(),
                )
            })
            .collect();
        (world, prims)
    }

    fn same_mesh(a: &Mesh, b: &Mesh) {
        assert_eq!(a.primitive_topology(), b.primitive_topology());
        assert_eq!(a.get_index_buffer_bytes(), b.get_index_buffer_bytes());
        assert_eq!(a.attributes().count(), b.attributes().count());
        for (attribute, values) in a.attributes() {
            assert_eq!(
                values.get_bytes(),
                b.attribute(attribute.id).unwrap().get_bytes(),
                "{}",
                attribute.name
            );
        }
        assert_eq!(
            a.get_morph_targets().map(bytemuck::cast_slice::<_, u8>),
            b.get_morph_targets().map(bytemuck::cast_slice::<_, u8>)
        );
        assert_eq!(a.morph_target_names(), b.morph_target_names());
    }

    fn same_entities(a: &World, ae: Entity, b: &World, be: Entity) {
        match (a.get::<Mesh3d>(ae), b.get::<Mesh3d>(be)) {
            (Some(am), Some(bm)) => same_mesh(
                a.resource::<Assets<Mesh>>().get(&am.0).unwrap(),
                b.resource::<Assets<Mesh>>().get(&bm.0).unwrap(),
            ),
            (None, None) => (),
            _ => panic!("mesh presence differs"),
        }
        assert_eq!(
            a.get::<super::super::skel::UsdDeformationError>(ae),
            b.get::<super::super::skel::UsdDeformationError>(be)
        );
        assert_eq!(
            a.get::<gpu_skin::UsdCpuSkinFallback>(ae)
                .map(|error| &error.0),
            b.get::<gpu_skin::UsdCpuSkinFallback>(be)
                .map(|error| &error.0)
        );
        assert_eq!(
            a.get::<gpu_skin::UsdGpuSkin>(ae).map(|skin| &skin.matrices),
            b.get::<gpu_skin::UsdGpuSkin>(be).map(|skin| &skin.matrices)
        );
    }

    #[test]
    fn phase_timing_is_optional_and_preserves_parallel_geometry() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let stage = stage();
        let registry = SchemaRegistry::builtin();
        let (mut plain, a) = world(false);
        let (mut profiled, b) = world(false);
        profiled.init_resource::<MeshPreparationPhaseTiming>();
        for time in [0.0, 2.5, 10.0, 5.0, 0.0] {
            for world in [&mut plain, &mut profiled] {
                world.insert_resource(StageTime { current: time });
                world.insert_resource(cache::MeshTangentCache::with_byte_budget(0));
            }
            let _scope = skel::SkelSampleScope::begin(&stage);
            registry.patch_animation_with_pool(&stage, a.clone(), &mut plain, Some(&pool));
            registry.patch_animation_with_pool(&stage, b.clone(), &mut profiled, Some(&pool));
            for ((_, ae), (_, be)) in a.iter().zip(&b) {
                same_entities(&plain, *ae, &profiled, *be);
            }
        }
        assert!(!plain.contains_resource::<MeshPreparationPhaseTiming>());
        let phases = profiled.resource::<MeshPreparationPhaseTiming>();
        let complete = profiled.resource::<MeshPreparationTiming>();
        assert_eq!(phases.batches, complete.batches);
        assert_eq!(phases.tangent_jobs, complete.jobs);
        assert_eq!(phases.tangent_failures, 0);
        if pool.thread_num() > 1 {
            assert_eq!(phases.tangent_jobs, 55);
            assert!(phases.tangent_vertices > 0);
            assert!(phases.tangent_elapsed >= phases.tangent_max);
            assert!(phases.phases.values().sum::<std::time::Duration>() <= complete.elapsed);
            for phase in [
                "eligibility",
                "eligibility-kinds",
                "eligibility-mesh-read",
                "deformation",
                "assembly",
                "cache-lookup",
                "traversal",
                "tangent-dispatch-join",
                "publication",
            ] {
                assert!(phases.phases.contains_key(phase), "{phase}");
            }
        }
        profiled.insert_resource(MeshPreparationPhaseTiming::default());
        assert_eq!(
            profiled
                .resource::<MeshPreparationPhaseTiming>()
                .tangent_jobs,
            0
        );
    }

    #[test]
    fn phase_timing_counts_empty_and_failed_batches_without_jobs() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let stage = stage();
        let (mut world, _) = world(false);
        world.remove_resource::<MeshPreparationTiming>();
        world.init_resource::<MeshPreparationPhaseTiming>();
        assert_eq!(prepare_batch(&[], &mut world, &pool, default()), 0);
        stage
            .attribute("/Test/Face.primvars:skel:jointIndices")
            .unwrap()
            .set(Value::IntVec(vec![-1; 8]))
            .unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let contexts = [(
            RouteCtx::at(&stage, &path, Some(5.0)),
            world.spawn_empty().id(),
        )];
        assert_eq!(prepare_batch(&contexts, &mut world, &pool, default()), 1);
        assert!(matches!(
            contexts[0].0.prepared_deformation.borrow().as_ref(),
            Some(Ok(None))
        ));
        assert!(!world.contains_resource::<MeshPreparationTiming>());
        let timing = world.resource::<MeshPreparationPhaseTiming>();
        assert_eq!(timing.batches, 2);
        assert_eq!(timing.tangent_jobs, 0);
        assert_eq!(timing.tangent_vertices, 0);
        assert_eq!(timing.tangent_elapsed, std::time::Duration::ZERO);
        for phase in ["eligibility", "deformation", "assembly", "publication"] {
            assert!(timing.phases.contains_key(phase), "{phase}");
        }
    }

    #[test]
    fn phase_timing_separates_gpu_skin_sampling_without_changing_geometry() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let stage = stage();
        let registry = SchemaRegistry::builtin();
        let (mut plain, a) = world(true);
        let (mut profiled, b) = world(true);
        profiled.init_resource::<MeshPreparationPhaseTiming>();
        for world in [&mut plain, &mut profiled] {
            world.insert_resource(StageTime { current: 5.0 });
        }
        let _scope = skel::SkelSampleScope::begin(&stage);
        registry.patch_animation_with_pool(&stage, a.clone(), &mut plain, Some(&pool));
        registry.patch_animation_with_pool(&stage, b.clone(), &mut profiled, Some(&pool));
        for ((_, ae), (_, be)) in a.iter().zip(&b) {
            same_entities(&plain, *ae, &profiled, *be);
        }
        if pool.thread_num() > 1 {
            let phases = profiled.resource::<MeshPreparationPhaseTiming>();
            for phase in [
                "eligibility-kinds",
                "eligibility-mesh-read",
                "eligibility-skin-sample",
            ] {
                assert!(phases.phases.contains_key(phase), "{phase}");
            }
            assert!(
                phases.phases.values().sum::<std::time::Duration>()
                    <= profiled.resource::<MeshPreparationTiming>().elapsed
            );
        }
    }

    #[test]
    fn animation_primvars_match_uncached_meshes_after_inherited_edits() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let registry = SchemaRegistry::builtin();
        for gpu in [false, true] {
            for pool in [None, Some(&pool)] {
                let stage = stage();
                stage
                    .create_attribute("/Test.primvars:displayOpacity", "float[]")
                    .unwrap();
                let (mut fresh, a) = world(gpu);
                let (mut scoped, b) = world(gpu);
                for (iteration, time) in [0.0, 2.5, 10.0, 5.0, 5.0, 0.0].into_iter().enumerate() {
                    let attribute = stage.attribute("/Test.primvars:displayOpacity").unwrap();
                    attribute
                        .clone()
                        .set(Value::FloatVec(vec![iteration as f32 / 10.0]))
                        .unwrap();
                    attribute
                        .set_metadata(
                            "interpolation",
                            Value::Token(
                                if iteration % 2 == 0 {
                                    "constant"
                                } else {
                                    "vertex"
                                }
                                .into(),
                            ),
                        )
                        .unwrap();
                    fresh.insert_resource(StageTime { current: time });
                    scoped.insert_resource(StageTime { current: time });
                    for (path, entity) in &a {
                        registry.patch_prim(&stage, path, &mut fresh, *entity, &[]);
                    }
                    registry.patch_animation_with_pool(&stage, b.clone(), &mut scoped, pool);
                    for ((_, ae), (_, be)) in a.iter().zip(&b) {
                        assert!(fresh.get::<Mesh3d>(*ae).is_some());
                        same_entities(&fresh, *ae, &scoped, *be);
                    }
                }
            }
        }
    }

    #[test]
    fn parallel_animation_matches_serial_after_clocks_and_edits() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let registry = SchemaRegistry::builtin();
        for (gpu, budget) in [(false, 0), (false, 4096), (true, 0), (true, 4096)] {
            let stage = stage();
            let (mut serial, a) = world(gpu);
            let (mut parallel, b) = world(gpu);
            serial.insert_resource(cache::MeshTangentCache::with_byte_budget(budget));
            parallel.insert_resource(cache::MeshTangentCache::with_byte_budget(budget));
            for (iteration, time) in (0..=20)
                .map(|step| step as f64 * 0.5)
                .chain([5.0, 0.0, 10.0, 5.0])
                .enumerate()
            {
                let edit = match iteration {
                    4 => Some((
                        "primvars:st",
                        Value::Vec2fVec(
                            [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]
                                .map(Into::into)
                                .to_vec(),
                        ),
                    )),
                    7 => Some(("normals", Value::Vec3fVec(vec![[0.0, 1.0, 1.0].into(); 4]))),
                    10 => Some(("orientation", Value::Token("leftHanded".into()))),
                    13 => Some((
                        "primvars:skel:skinningMethod",
                        Value::Token("dualQuaternion".into()),
                    )),
                    16 => Some((
                        "primvars:skel:skinningMethod",
                        Value::Token("unsupported".into()),
                    )),
                    18 => Some((
                        "primvars:skel:skinningMethod",
                        Value::Token("classicLinear".into()),
                    )),
                    21 => Some(("primvars:skel:jointWeights", Value::FloatVec(vec![0.8; 8]))),
                    23 => Some(("primvars:skel:jointIndices", Value::IntVec(vec![-1; 8]))),
                    _ => None,
                };
                if let Some((name, value)) = edit {
                    stage
                        .prim(a[0].0.clone())
                        .unwrap()
                        .attribute(name)
                        .set(value)
                        .unwrap();
                }
                serial.insert_resource(StageTime { current: time });
                parallel.insert_resource(StageTime { current: time });
                let _scope = skel::SkelSampleScope::begin(&stage);
                registry.patch_animation_with_pool(&stage, a.clone(), &mut serial, None);
                registry.patch_animation_with_pool(&stage, b.clone(), &mut parallel, Some(&pool));
                for ((_, ae), (_, be)) in a.iter().zip(&b) {
                    same_entities(&serial, *ae, &parallel, *be);
                }
            }
            let timing = parallel.resource::<MeshPreparationTiming>();
            if pool.thread_num() > 1 {
                assert!(timing.jobs > 0);
                assert_eq!(timing.cache_hits > 0, budget > 0);
                assert_eq!(timing.peak_jobs, 2);
                assert!(timing.peak_input_bytes <= 4096);
            }
            assert_eq!(serial.resource::<MeshPreparationTiming>().jobs, 0);
            assert!(
                parallel
                    .resource::<cache::MeshTangentCache>()
                    .retained_payload_bytes()
                    <= budget
            );
        }
    }

    #[test]
    fn preparation_limits_and_serial_fallbacks_preserve_geometry() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let single = bevy::tasks::TaskPoolBuilder::new().num_threads(1).build();
        let stage = stage();
        let registry = SchemaRegistry::builtin();
        let (mut serial, a) = world(true);
        serial.insert_resource(StageTime { current: 5.0 });
        registry.patch_animation_with_pool(&stage, a.clone(), &mut serial, None);
        for mode in 0..6 {
            let (mut candidate, b) = world(true);
            candidate.insert_resource(StageTime { current: 5.0 });
            match mode {
                0 => candidate.resource_mut::<ParallelMeshPreparation>().max_jobs = 0,
                1 => {
                    candidate
                        .resource_mut::<ParallelMeshPreparation>()
                        .max_input_bytes = 0
                }
                2 => {
                    candidate
                        .resource_mut::<ParallelMeshPreparation>()
                        .max_input_bytes = 1
                }
                3 => {
                    candidate.init_resource::<residency::DeferHiddenMeshes>();
                }
                4 => {
                    candidate
                        .insert_resource(residency::UsdResidencyBudget(std::time::Duration::ZERO));
                }
                _ => (),
            }
            registry.patch_animation_with_pool(
                &stage,
                b.clone(),
                &mut candidate,
                Some(if mode == 5 { &single } else { &pool }),
            );
            for ((_, ae), (_, be)) in a.iter().zip(&b) {
                same_entities(&serial, *ae, &candidate, *be);
            }
            assert_eq!(candidate.resource::<MeshPreparationTiming>().jobs, 0);
        }
    }

    #[test]
    fn sparse_meshes_share_a_bounded_batch() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let stage = stage();
        let registry = SchemaRegistry::builtin();
        for (limit, count, gap, batches) in [(4, 4, 20, 1), (usize::MAX, 11, 8, 2)] {
            let (mut world, meshes) = world(false);
            world.insert_resource(StageTime { current: 5.0 });
            world.insert_resource(cache::MeshTangentCache::with_byte_budget(0));
            world.resource_mut::<ParallelMeshPreparation>().max_jobs = limit;
            let mut prims = Vec::new();
            for mesh in meshes.into_iter().take(count) {
                for _ in 0..gap {
                    prims.push((
                        openusd::sdf::path("/Test").unwrap(),
                        world.spawn_empty().id(),
                    ));
                }
                prims.push(mesh);
            }
            registry.patch_animation_with_pool(&stage, prims.clone(), &mut world, Some(&pool));
            if pool.thread_num() > 1 {
                let timing = world.resource::<MeshPreparationTiming>();
                assert_eq!(timing.batches, batches);
                assert_eq!(timing.parallel_batches, batches);
                assert_eq!(timing.inline_jobs, 0);
                assert_eq!(timing.peak_jobs, limit.min(MAX_BATCH_MESHES));
                assert!(timing.peak_input_bytes <= 4096);
            }
            for (path, entity) in prims {
                assert_eq!(
                    world.get::<Mesh3d>(entity).is_some(),
                    path.as_str() == "/Test/Face"
                );
            }
        }
    }

    struct EditLaterPrim;

    impl PrimRoute for EditLaterPrim {
        fn matches(&self, _: &RouteCtx) -> bool {
            true
        }
        fn project(&self, ctx: &RouteCtx, world: &mut World, _: Entity) {
            assert_eq!(world.resource::<MeshPreparationTiming>().jobs, 0);
            ctx.stage
                .attribute("/Test/Face.orientation")
                .unwrap()
                .set(Value::Token("leftHanded".into()))
                .unwrap();
        }
    }

    #[test]
    fn custom_routes_keep_synchronous_reads_and_publication() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let mut registry = SchemaRegistry::builtin();
        registry.register(EditLaterPrim);
        let sa = stage();
        let sb = stage();
        let (mut serial, a) = world(false);
        let (mut parallel, b) = world(false);
        serial.insert_resource(StageTime { current: 5.0 });
        parallel.insert_resource(StageTime { current: 5.0 });
        registry.patch_animation_with_pool(&sa, a.clone(), &mut serial, None);
        registry.patch_animation_with_pool(&sb, b.clone(), &mut parallel, Some(&pool));
        for ((_, ae), (_, be)) in a.iter().zip(&b) {
            same_entities(&serial, *ae, &parallel, *be);
        }
        assert_eq!(parallel.resource::<MeshPreparationTiming>().batches, 0);
    }

    #[test]
    fn deferred_hidden_meshes_are_not_decoded_by_preparation() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let stage = stage();
        stage
            .attribute("/Test/Face.visibility")
            .unwrap()
            .set(Value::Token("invisible".into()))
            .unwrap();
        let (mut world, prims) = world(true);
        world.init_resource::<residency::DeferHiddenMeshes>();
        world.init_resource::<super::super::MeshReadTiming>();
        world.insert_resource(StageTime { current: 5.0 });
        SchemaRegistry::builtin().patch_animation_with_pool(
            &stage,
            prims.clone(),
            &mut world,
            Some(&pool),
        );
        assert_eq!(world.resource::<super::super::MeshReadTiming>().decodes, 0);
        assert_eq!(world.resource::<MeshPreparationTiming>().jobs, 0);
        for (_, entity) in prims {
            assert!(world.get::<Mesh3d>(entity).is_none());
            assert!(world.get::<residency::UsdDeferredMesh>(entity).is_some());
        }
    }

    #[test]
    fn deferred_resident_meshes_prepare_without_decoding_hidden_siblings() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let registry = SchemaRegistry::builtin();
        for gpu in [false, true] {
            let stage = stage();
            let setup = || {
                let (mut world, prims) = world(gpu);
                world.init_resource::<residency::DeferHiddenMeshes>();
                world.init_resource::<super::super::MeshReadTiming>();
                world.insert_resource(cache::MeshTangentCache::with_byte_budget(0));
                world.insert_resource(StageTime { current: 0.0 });
                let parent = world.spawn(Visibility::Hidden).id();
                for (_, entity) in prims.iter().take(6) {
                    world.entity_mut(*entity).insert(ChildOf(parent));
                }
                registry.patch_animation_with_pool(&stage, prims.clone(), &mut world, None);
                for (index, (_, entity)) in prims.iter().enumerate() {
                    assert_eq!(world.get::<Mesh3d>(*entity).is_some(), index >= 6);
                }
                (world, prims, parent)
            };
            let (mut serial, a, ap) = setup();
            let (mut parallel, b, bp) = setup();
            let mut promoted = false;
            for (clock, reveal) in [
                (5.0, false),
                (10.0, false),
                (3.0, true),
                (5.0, false),
                (3.0, true),
            ] {
                if clock == 10.0 {
                    stage
                        .attribute("/Test/Face.orientation")
                        .unwrap()
                        .set(Value::Token("leftHanded".into()))
                        .unwrap();
                }
                let resident_before = b
                    .iter()
                    .filter(|(_, entity)| parallel.get::<Mesh3d>(*entity).is_some())
                    .count();
                for (world, parent) in [(&mut serial, ap), (&mut parallel, bp)] {
                    world.entity_mut(parent).insert(if reveal {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    });
                    world.insert_resource(StageTime { current: clock });
                    world.insert_resource(MeshPreparationTiming::default());
                    world.insert_resource(super::super::MeshReadTiming::default());
                }
                registry.patch_animation_with_pool(&stage, a.clone(), &mut serial, None);
                registry.patch_animation_with_pool(&stage, b.clone(), &mut parallel, Some(&pool));
                promoted |= reveal;
                for (index, ((_, ae), (_, be))) in a.iter().zip(&b).enumerate() {
                    same_entities(&serial, *ae, &parallel, *be);
                    assert_eq!(
                        parallel.get::<Mesh3d>(*be).is_some(),
                        promoted || index >= 6
                    );
                    assert_eq!(
                        parallel.get::<residency::UsdDeferredMesh>(*be).is_some(),
                        !promoted && index < 6
                    );
                }
                assert_eq!(serial.resource::<MeshPreparationTiming>().jobs, 0);
                let timing = parallel.resource::<MeshPreparationTiming>();
                if pool.thread_num() > 1 {
                    let expected = if gpu { 0 } else { resident_before };
                    assert_eq!(timing.jobs as usize, expected);
                    assert!(timing.peak_jobs <= 2 && timing.peak_input_bytes <= 4096);
                }
                assert_eq!(
                    parallel.resource::<super::super::MeshReadTiming>().decodes,
                    if promoted { 11 } else { 5 }
                );
                assert_eq!(
                    serial.resource::<super::super::MeshReadTiming>().decodes,
                    parallel.resource::<super::super::MeshReadTiming>().decodes
                );
            }
            parallel.insert_resource(residency::UsdResidencyBudget(std::time::Duration::ZERO));
            parallel.insert_resource(MeshPreparationTiming::default());
            parallel.insert_resource(StageTime { current: 7.0 });
            serial.insert_resource(StageTime { current: 7.0 });
            registry.patch_animation_with_pool(&stage, a.clone(), &mut serial, None);
            registry.patch_animation_with_pool(&stage, b.clone(), &mut parallel, Some(&pool));
            assert_eq!(parallel.resource::<MeshPreparationTiming>().jobs, 0);
            for ((_, ae), (_, be)) in a.iter().zip(&b) {
                same_entities(&serial, *ae, &parallel, *be);
            }
        }
    }

    fn instance_source(orientation: &str) -> crate::UsdSource {
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_morph_blended_animated.usda"
        );
        crate::UsdSource::new(
            "parallel-instance.usda",
            format!(
                r#"#usda 1.0
(subLayers = [@{file}@])
over "Test" {{
    over "Face" {{
        uniform token orientation = "{orientation}"
        def GeomSubset "Part" {{
            uniform token familyName = "materialBind"
            uniform token elementType = "face"
            int[] indices = [0]
        }}
    }}
}}
def "Copy" (references = @{file}@</Test>) {{}}
"#
            )
            .into_bytes(),
        )
        .unwrap()
    }

    fn instance_app(parallel: bool, gpu: bool) -> (App, Handle<crate::UsdScene>, [Entity; 2]) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::UsdPlugin,
            crate::UsdAssetPlugin,
        ));
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<MeshPreparationTiming>();
        app.insert_resource(crate::asset::UsdProjectionBudget(std::time::Duration::MAX));
        if gpu {
            app.add_plugins(gpu_skin::UsdGpuSkinningPlugin);
        }
        if !parallel {
            app.insert_resource(ParallelMeshPreparation {
                max_jobs: 0,
                ..default()
            });
        }
        let handle = app
            .world_mut()
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source: instance_source("rightHanded"),
                textures: default(),
            });
        let roots = [0.0, 10.0].map(|current| {
            app.world_mut()
                .spawn((
                    crate::UsdSceneRoot(handle.clone()),
                    crate::instance::UsdInstanceTime { current },
                ))
                .id()
        });
        app.update();
        for root in roots {
            assert_eq!(
                app.world().get::<crate::asset::UsdSceneState>(root),
                Some(&crate::asset::UsdSceneState::Ready)
            );
        }
        (app, handle, roots)
    }

    fn check_instances(serial: &App, a: [Entity; 2], parallel: &App, b: [Entity; 2]) {
        use super::super::subset::UsdSubset;
        use crate::instance::UsdInstances;
        for (ar, br) in a.into_iter().zip(b) {
            for path in ["/Test/Face", "/Copy/Face"] {
                let ae = serial
                    .world()
                    .non_send::<UsdInstances>()
                    .entity(ar, path)
                    .unwrap();
                let be = parallel
                    .world()
                    .non_send::<UsdInstances>()
                    .entity(br, path)
                    .unwrap();
                same_entities(serial.world(), ae, parallel.world(), be);
                let subsets = |world: &World, entity| {
                    let mut children: Vec<_> = world
                        .get::<Children>(entity)
                        .into_iter()
                        .flatten()
                        .filter_map(|child| {
                            world
                                .get::<UsdSubset>(*child)
                                .map(|part| (part.0.clone(), *child))
                        })
                        .collect();
                    children.sort_by(|a, b| a.0.cmp(&b.0));
                    children
                };
                let ac = subsets(serial.world(), ae);
                let bc = subsets(parallel.world(), be);
                assert_eq!(ac.len(), bc.len());
                if path == "/Test/Face" {
                    assert_eq!(ac.len(), 1);
                }
                for ((an, ae), (bn, be)) in ac.into_iter().zip(bc) {
                    assert_eq!(an, bn);
                    same_entities(serial.world(), ae, parallel.world(), be);
                }
            }
        }
    }

    #[test]
    fn asset_plugin_preparation_preserves_root_clocks_reload_and_close() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        for (gpu, defer) in [(false, false), (false, true), (true, false), (true, true)] {
            let (mut serial, ah, a) = instance_app(false, gpu);
            let (mut parallel, bh, b) = instance_app(true, gpu);
            if defer {
                serial.init_resource::<residency::DeferHiddenMeshes>();
                parallel.init_resource::<residency::DeferHiddenMeshes>();
            }
            let mut previous = None;
            for clocks in [[2.0, 8.0], [8.0, 2.0], [5.0, 5.0], [0.0, 10.0]] {
                for index in 0..2 {
                    serial
                        .world_mut()
                        .get_mut::<UsdInstanceTime>(a[index])
                        .unwrap()
                        .current = clocks[index];
                    parallel
                        .world_mut()
                        .get_mut::<UsdInstanceTime>(b[index])
                        .unwrap()
                        .current = clocks[index];
                }
                serial.update();
                parallel.update();
                check_instances(&serial, a, &parallel, b);
                let meshes: Vec<_> = b
                    .iter()
                    .map(|root| {
                        let entity = parallel
                            .world()
                            .non_send::<UsdInstances>()
                            .entity(*root, "/Test/Face")
                            .unwrap();
                        let mesh = &parallel.world().get::<Mesh3d>(entity).unwrap().0;
                        let skin = parallel
                            .world()
                            .get::<gpu_skin::UsdGpuSkin>(entity)
                            .map(|skin| skin.matrices.clone());
                        (mesh.clone(), skin)
                    })
                    .collect();
                if let Some(previous) = previous {
                    assert_ne!(meshes, previous);
                }
                previous = Some(meshes);
            }
            for (app, handle) in [(&mut serial, &ah), (&mut parallel, &bh)] {
                app.world_mut()
                    .resource_mut::<Assets<crate::UsdScene>>()
                    .get_mut(handle)
                    .unwrap()
                    .source = instance_source("leftHanded");
                app.update();
                app.update();
            }
            check_instances(&serial, a, &parallel, b);
            let face = parallel
                .world()
                .non_send::<UsdInstances>()
                .entity(b[0], "/Test/Face")
                .unwrap();
            assert_eq!(
                parallel
                    .world()
                    .non_send::<UsdInstances>()
                    .stage(b[0])
                    .unwrap()
                    .attribute("/Test/Face.orientation")
                    .unwrap()
                    .get::<Value>()
                    .unwrap(),
                Some(Value::Token("leftHanded".into()))
            );
            if bevy::tasks::ComputeTaskPool::get().thread_num() > 1 {
                let jobs = parallel.world().resource::<MeshPreparationTiming>().jobs;
                assert_eq!(jobs > 0, !gpu);
            }
            assert_eq!(serial.world().resource::<MeshPreparationTiming>().jobs, 0);
            parallel
                .world_mut()
                .entity_mut(b[0])
                .remove::<crate::UsdSceneRoot>();
            parallel.update();
            assert!(parallel.world().get_entity(face).is_err());
            assert_eq!(parallel.world().non_send::<UsdInstances>().len(), 1);
            assert!(
                parallel
                    .world()
                    .non_send::<UsdInstances>()
                    .entity(b[1], "/Test/Face")
                    .is_some()
            );
            parallel.world_mut().despawn(b[1]);
            parallel.update();
            assert!(parallel.world().non_send::<UsdInstances>().is_empty());
            let jobs = parallel.world().resource::<MeshPreparationTiming>().jobs;
            for _ in 0..3 {
                parallel.update();
            }
            assert_eq!(
                parallel.world().resource::<MeshPreparationTiming>().jobs,
                jobs
            );
            assert_eq!(
                parallel
                    .world_mut()
                    .query::<&gpu_skin::UsdGpuSkin>()
                    .iter(parallel.world())
                    .count(),
                0
            );
        }
    }
}
