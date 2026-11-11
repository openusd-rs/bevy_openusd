use super::*;
use std::time::{Duration, Instant};

const WORKERS: usize = 4;
const INPUT_BUDGET: usize = 16 * 1024 * 1024;
const WARMUP: usize = 4;
type Job = (usize, Mesh);
type Output = (usize, Mesh, Result<(), bevy::mesh::GenerateTangentsError>);

#[derive(Clone)]
struct Input {
    index: usize,
    mesh_prim: bool,
    mesh: Option<Mesh>,
}

struct Run {
    outputs: Vec<Output>,
    batches: usize,
    peak_jobs: usize,
    peak_bytes: usize,
    inline_jobs: usize,
}

fn generate((index, mut mesh): Job) -> Output {
    let result = mesh.generate_tangents();
    (index, mesh, result)
}

fn dispatch(mut jobs: Vec<Job>, pool: &TaskPool, queued: bool) -> Vec<Output> {
    if jobs.len() < 2 {
        return jobs.into_iter().map(generate).collect();
    }
    jobs.sort_unstable_by_key(|(_, mesh)| std::cmp::Reverse(mesh.count_vertices()));
    if !queued {
        return pool.scope_with_executor(false, None, |scope| {
            for (position, job) in jobs.into_iter().enumerate() {
                let task = async move { generate(job) };
                if position == 0 {
                    scope.spawn_on_scope(task);
                } else {
                    scope.spawn(task);
                }
            }
        });
    }
    let mut lanes: Vec<_> = (0..jobs.len().min(WORKERS))
        .map(|_| (0usize, Vec::new()))
        .collect();
    for job in jobs {
        let (load, lane) = lanes.iter_mut().min_by_key(|(load, _)| *load).unwrap();
        *load = load.saturating_add(job.1.count_vertices());
        lane.push(job);
    }
    lanes.sort_unstable_by_key(|(load, _)| std::cmp::Reverse(*load));
    pool.scope_with_executor(false, None, |scope| {
        for (position, (_, jobs)) in lanes.into_iter().enumerate() {
            let task = async move { jobs.into_iter().map(generate).collect::<Vec<_>>() };
            if position == 0 {
                scope.spawn_on_scope(task);
            } else {
                scope.spawn(task);
            }
        }
    })
    .into_iter()
    .flatten()
    .collect()
}

fn run(mut inputs: Vec<Input>, pool: &TaskPool, queued: bool, budget: usize) -> Run {
    assert!(budget > 0);
    let mut result = Run {
        outputs: Vec::new(),
        batches: 0,
        peak_jobs: 0,
        peak_bytes: 0,
        inline_jobs: 0,
    };
    for chunk in inputs.chunks_mut(MAX_BATCH_PRIMS) {
        let mut offset = 0;
        while offset < chunk.len() {
            let mut meshes = 0;
            let mut bytes = 0;
            let mut jobs = Vec::new();
            let mut oversized = None;
            let limit = if queued { 2 * WORKERS } else { WORKERS };
            while offset < chunk.len() && meshes < limit && bytes < budget {
                let input = &mut chunk[offset];
                offset += 1;
                meshes += usize::from(input.mesh_prim);
                let Some(mesh) = input.mesh.take() else {
                    continue;
                };
                let size = cache::mesh_payload_bytes(&mesh);
                if size > budget.saturating_sub(bytes) {
                    oversized = Some((input.index, mesh));
                    break;
                }
                bytes += size;
                jobs.push((input.index, mesh));
            }
            result.peak_jobs = result.peak_jobs.max(jobs.len());
            result.peak_bytes = result.peak_bytes.max(bytes);
            result.outputs.extend(dispatch(jobs, pool, queued));
            if let Some(job) = oversized {
                result.outputs.push(generate(job));
                result.inline_jobs += 1;
            }
            result.batches += 1;
        }
    }
    result.outputs.sort_unstable_by_key(|(index, _, _)| *index);
    result
}

fn compare(a: &Run, b: &Run) {
    assert_eq!(a.outputs.len(), b.outputs.len());
    for ((ai, a, ae), (bi, b, be)) in a.outputs.iter().zip(&b.outputs) {
        assert_eq!(ai, bi);
        assert_eq!(format!("{ae:?}"), format!("{be:?}"));
        assert_eq!(a.primitive_topology(), b.primitive_topology());
        assert_eq!(a.get_index_buffer_bytes(), b.get_index_buffer_bytes());
        assert_eq!(a.attributes().count(), b.attributes().count());
        for (attribute, values) in a.attributes() {
            assert_eq!(
                values.get_bytes(),
                b.attribute(attribute.id).unwrap().get_bytes()
            );
        }
        assert_eq!(
            a.get_morph_targets().map(bytemuck::cast_slice::<_, u8>),
            b.get_morph_targets().map(bytemuck::cast_slice::<_, u8>)
        );
    }
}

fn order(iteration: usize, reverse: bool) -> [usize; 2] {
    let first = usize::from(matches!(iteration % 4, 1 | 2) ^ reverse);
    [first, 1 - first]
}

fn clone_inputs(input: &[Input], first: usize) -> [Option<Vec<Input>>; 2] {
    let mut copies = [None, None];
    for index in [first, 1 - first] {
        copies[index] = Some(input.to_vec());
    }
    copies
}

fn inputs(
    stage: &openusd::usd::Stage,
    paths: &[openusd::sdf::Path],
    clock: f64,
    world: &World,
) -> anyhow::Result<Vec<Input>> {
    let _skin = skel::SkelSampleScope::begin(stage);
    let _primvars = crate::read::geom::PrimvarScope::begin(stage);
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let context = RouteCtx::at(stage, path, Some(clock));
            let mesh_prim = context.type_name.as_deref() == Some("Mesh");
            let mesh = if needs_deformation(&context, world, &mut None) {
                let read = super::super::skel::deformed_mesh(&context)?
                    .ok_or_else(|| anyhow::anyhow!("missing deformation for {path}"))?;
                Some(PreparedDeformation::new(read).mesh)
            } else {
                None
            };
            Ok(Input {
                index,
                mesh_prim,
                mesh,
            })
        })
        .collect()
}

#[test]
fn paired_probe_matches_serial_tangents_and_bounds_sparse_and_oversized_batches() {
    let stage = crate::UsdSource::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/skel_morph_blended_animated.usda"
    ))
    .unwrap()
    .open_stage()
    .unwrap();
    let path = openusd::sdf::path("/Test/Face").unwrap();
    let mut world = World::new();
    world.init_resource::<super::super::gpu_skin::GpuSkinningEnabled>();
    let pool = bevy::tasks::TaskPoolBuilder::new()
        .num_threads(WORKERS)
        .build();
    let mut paths = Vec::new();
    for _ in 0..11 {
        paths.extend((0..16).map(|_| openusd::sdf::path("/Test").unwrap()));
        paths.push(path.clone());
    }
    for clock in [0.0, 5.0, 10.0, 5.0] {
        let inputs = inputs(&stage, &paths, clock, &world).unwrap();
        let expected = Run {
            outputs: inputs
                .iter()
                .filter_map(|input| input.mesh.clone().map(|mesh| generate((input.index, mesh))))
                .collect(),
            batches: 0,
            peak_jobs: 0,
            peak_bytes: 0,
            inline_jobs: 0,
        };
        assert_eq!(expected.outputs.len(), 11);
        for budget in [1, 512, INPUT_BUDGET] {
            let mut copies = clone_inputs(&inputs, usize::from(budget == 512));
            let old = run(copies[0].take().unwrap(), &pool, false, budget);
            let new = run(copies[1].take().unwrap(), &pool, true, budget);
            compare(&old, &expected);
            compare(&new, &expected);
            assert!(old.peak_jobs <= WORKERS && new.peak_jobs <= 2 * WORKERS);
            assert!(old.peak_bytes <= budget && new.peak_bytes <= budget);
            if budget == 1 {
                assert_eq!(old.inline_jobs, 11);
                assert_eq!(new.inline_jobs, 11);
            }
        }
    }
    for reverse in [false, true] {
        let sequence: Vec<_> = (0..4).map(|i| order(i, reverse)[0]).collect();
        assert_eq!(
            sequence,
            if reverse {
                vec![1, 0, 0, 1]
            } else {
                vec![0, 1, 1, 0]
            }
        );
        let mut combinations: Vec<_> = (0..4)
            .map(|i| (order(i, reverse)[0], usize::from((i % 2 == 1) ^ reverse)))
            .collect();
        combinations.sort_unstable();
        assert_eq!(combinations, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    }
}

#[test]
#[ignore = "release diagnostic requiring USD_TANGENT_ASSET"]
fn real_asset_paired_tangent_scheduling() -> anyhow::Result<()> {
    anyhow::ensure!(!cfg!(debug_assertions), "probe requires release mode");
    let file = std::fs::canonicalize(std::env::var("USD_TANGENT_ASSET")?)?;
    let samples = std::env::var("USD_TANGENT_SAMPLES")
        .unwrap_or_else(|_| "200".into())
        .parse::<usize>()?;
    anyhow::ensure!(
        (4..=10000).contains(&samples) && samples.is_multiple_of(4),
        "samples must be a multiple of four in 4..10000"
    );
    let reverse = match std::env::var("USD_TANGENT_REVERSE").as_deref() {
        Ok("1") => true,
        Ok("0") | Err(_) => false,
        _ => anyhow::bail!("USD_TANGENT_REVERSE must be 0 or 1"),
    };
    let stage = crate::UsdSource::from_file(&file)?.open_stage()?;
    let (start, end) = (stage.start_time_code(), stage.end_time_code());
    anyhow::ensure!(
        start.is_finite() && end.is_finite() && end > start && (end - start).is_finite(),
        "invalid authored timeline"
    );
    let mut paths = Vec::new();
    stage.traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |path| {
        paths.push(path.clone())
    })?;
    paths.retain(|path| crate::live::prim_is_animated(&stage, path));
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let mut world = World::new();
    world.init_resource::<super::super::gpu_skin::GpuSkinningEnabled>();
    let pool = bevy::tasks::TaskPoolBuilder::new()
        .num_threads(WORKERS)
        .build();
    anyhow::ensure!(
        pool.thread_num() == WORKERS,
        "probe requires four worker threads"
    );
    println!(
        "tangent_probe asset={} samples={samples} warmup={WARMUP} reverse={reverse} start={start} end={end} paths={} pool_threads={} workers={WORKERS} input_budget={INPUT_BUDGET} cache=disabled scope=owned-posed-mesh-scheduling-not-app-or-FPS excludes=USD,eligibility,deformation,assembly,input-cloning,comparison,gpu",
        file.display(),
        paths.len(),
        pool.thread_num()
    );
    let mut timings: [Vec<Duration>; 2] = Default::default();
    let mut previous = start;
    for iteration in 0..samples + WARMUP {
        let fraction = if iteration < WARMUP {
            [0.0, 0.5, 1.0, 0.5][iteration]
        } else {
            (iteration - WARMUP + 1) as f64 / samples as f64
        };
        let clock = start + (end - start) * fraction;
        if iteration >= WARMUP {
            anyhow::ensure!(
                clock.is_finite() && clock > previous,
                "timeline cannot represent distinct clocks"
            );
            previous = clock;
        }
        let setup = Instant::now();
        let input = inputs(&stage, &paths, clock, &world)?;
        let jobs = input.iter().filter(|input| input.mesh.is_some()).count();
        anyhow::ensure!(jobs > 0, "no tangent jobs at clock {clock}");
        if iteration == 0 {
            for input in &input {
                if let Some(mesh) = &input.mesh {
                    println!(
                        "tangent_probe_job index={} path={} vertices={} bytes={}",
                        input.index,
                        paths[input.index],
                        mesh.count_vertices(),
                        cache::mesh_payload_bytes(mesh)
                    );
                }
            }
        }
        let first_clone = usize::from((iteration % 2 == 1) ^ reverse);
        let mut copies = clone_inputs(&input, first_clone);
        drop(input);
        let setup = setup.elapsed();
        let sequence = order(iteration, reverse);
        let mut elapsed = [Duration::ZERO; 2];
        let mut results: [Option<Run>; 2] = [None, None];
        for index in sequence {
            let input = copies[index].take().unwrap();
            let started = Instant::now();
            results[index] = Some(run(input, &pool, index == 1, INPUT_BUDGET));
            elapsed[index] = started.elapsed();
        }
        let [a, b] = results.map(Option::unwrap);
        compare(&a, &b);
        anyhow::ensure!(
            a.outputs.len() == jobs && a.outputs.iter().all(|(_, _, result)| result.is_ok()),
            "missing or failed tangent job at {clock}"
        );
        let phase = if iteration < WARMUP {
            "warmup"
        } else {
            "sample"
        };
        let index = if iteration < WARMUP {
            iteration
        } else {
            iteration - WARMUP
        };
        println!(
            "tangent_probe_{phase} index={index} clock={clock:.17} first={} first_clone={first_clone} jobs={jobs} setup_ns={} legacy_ns={} queued_ns={} legacy_batches={} queued_batches={} legacy_peak_jobs={} queued_peak_jobs={} legacy_peak_bytes={} queued_peak_bytes={} legacy_inline={} queued_inline={} exact=true",
            sequence[0],
            setup.as_nanos(),
            elapsed[0].as_nanos(),
            elapsed[1].as_nanos(),
            a.batches,
            b.batches,
            a.peak_jobs,
            b.peak_jobs,
            a.peak_bytes,
            b.peak_bytes,
            a.inline_jobs,
            b.inline_jobs
        );
        if iteration >= WARMUP {
            for (values, elapsed) in timings.iter_mut().zip(elapsed) {
                values.push(elapsed);
            }
        }
    }
    for (lane, mut values) in timings.into_iter().enumerate() {
        values.sort_unstable();
        println!(
            "tangent_probe_summary lane={lane} samples={samples} median_ns={} p95_ns={} max_ns={}",
            values[samples / 2 - 1].as_nanos(),
            values[(samples * 95).div_ceil(100) - 1].as_nanos(),
            values.last().unwrap().as_nanos()
        );
    }
    Ok(())
}
