//! Bounded, scoped tangent generation for built-in animation updates.

use bevy::{prelude::*, tasks::TaskPool};

use super::{RouteCtx, cache};
use crate::read::{geom::ReadMesh, skel};

pub(super) const MAX_BATCH_PRIMS: usize = 8;

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
    pub jobs: u64,
    pub cache_hits: u64,
    pub peak_jobs: usize,
    pub peak_input_bytes: usize,
    pub elapsed: std::time::Duration,
}

pub(super) struct PreparedDeformation {
    pub read: ReadMesh,
    pub mesh: Mesh,
    pub tangents_ready: bool,
}

impl PreparedDeformation {
    pub fn new(read: ReadMesh) -> Self {
        let mesh = crate::mesh::assemble_mesh(&read, None, false);
        Self {
            read,
            mesh,
            tangents_ready: false,
        }
    }

    pub fn generate_tangents(&mut self, world: &mut World) {
        if !self.tangents_ready && self.read.uvs.is_some() {
            cache::generate_cached_deformed_tangents(world, &mut self.mesh);
        }
        self.tangents_ready = true;
    }
}

fn needs_deformation(ctx: &RouteCtx, world: &World) -> bool {
    if ctx.type_name.as_deref() != Some("Mesh")
        || super::subdivision::enabled(ctx, world)
        || !skel::deformation_is_time_varying(ctx.stage, ctx.path)
    {
        return false;
    }
    let Ok(Some(read)) = ctx.read_mesh() else {
        return false;
    };
    if read.uvs.is_none() || read.points.is_empty() {
        return false;
    }
    if !world.contains_resource::<super::gpu_skin::GpuSkinningEnabled>() {
        return true;
    }
    if !skel::is_skinned(ctx.stage, ctx.path) {
        return false;
    }
    let sample = skel::gpu_skin_sample_with_mesh(ctx.stage, ctx.path, ctx.time, read);
    let needed = match &sample {
        Ok(sample) => sample
            .normal_corrections
            .iter()
            .any(|matrix| *matrix != Mat3::IDENTITY),
        Err(_) => true,
    };
    *ctx.prepared_skin.borrow_mut() = Some(sample);
    needed
}

pub(super) fn prepare_batch(
    contexts: &[RouteCtx],
    world: &mut World,
    pool: &TaskPool,
    settings: ParallelMeshPreparation,
) {
    let started = world
        .contains_resource::<MeshPreparationTiming>()
        .then(bevy::platform::time::Instant::now);
    let mut pending = Vec::new();
    let mut jobs = Vec::new();
    let mut input_bytes = 0usize;
    let mut cache_hits = 0;
    for (index, ctx) in contexts.iter().enumerate() {
        if jobs.len() >= settings.max_jobs || input_bytes >= settings.max_input_bytes {
            break;
        }
        if !needs_deformation(ctx, world) {
            continue;
        }
        let result = super::skel::deformed_mesh(ctx).map(|read| read.map(PreparedDeformation::new));
        let mut prepared = match result {
            Ok(Some(prepared)) => prepared,
            other => {
                *ctx.prepared_deformation.borrow_mut() = Some(other);
                continue;
            }
        };
        let bytes = cache::mesh_payload_bytes(&prepared.mesh);
        if bytes > settings.max_input_bytes.saturating_sub(input_bytes) {
            *ctx.prepared_deformation.borrow_mut() = Some(Ok(Some(prepared)));
            break;
        }
        match cache::prepare_tangents(world, &mut prepared.mesh, true) {
            Some(ticket) => {
                input_bytes += bytes;
                pending.push((index, prepared.read, ticket));
                jobs.push((index, prepared.mesh));
            }
            None => {
                prepared.tangents_ready = true;
                *ctx.prepared_deformation.borrow_mut() = Some(Ok(Some(prepared)));
                cache_hits += 1;
            }
        }
    }
    let count = jobs.len();
    let mut results = pool.scope_with_executor(false, None, |scope| {
        for (index, mut mesh) in jobs {
            scope.spawn(async move {
                let result = mesh.generate_tangents();
                (index, mesh, result)
            });
        }
    });
    results.sort_unstable_by_key(|(index, _, _)| *index);
    for ((index, mesh, result), (expected, read, ticket)) in results.into_iter().zip(pending) {
        assert_eq!(index, expected);
        cache::finish_tangents(world, &mesh, ticket, result);
        *contexts[index].prepared_deformation.borrow_mut() = Some(Ok(Some(PreparedDeformation {
            read,
            mesh,
            tangents_ready: true,
        })));
    }
    if let Some(started) = started {
        let mut timing = world.resource_mut::<MeshPreparationTiming>();
        timing.batches += 1;
        timing.jobs += count as u64;
        timing.cache_hits += cache_hits;
        timing.peak_jobs = timing.peak_jobs.max(count);
        timing.peak_input_bytes = timing.peak_input_bytes.max(input_bytes);
        timing.elapsed += started.elapsed();
    }
}

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
    fn parallel_animation_matches_serial_after_clocks_and_edits() {
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
        let registry = SchemaRegistry::builtin();
        for gpu in [false, true] {
            let stage = stage();
            let (mut serial, a) = world(gpu);
            let (mut parallel, b) = world(gpu);
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
                assert!(timing.cache_hits > 0);
                assert_eq!(timing.peak_jobs, 2);
                assert!(timing.peak_input_bytes <= 4096);
            }
            assert_eq!(serial.resource::<MeshPreparationTiming>().jobs, 0);
            assert!(
                parallel
                    .resource::<cache::MeshTangentCache>()
                    .retained_payload_bytes()
                    <= 16 * 1024 * 1024
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
}
