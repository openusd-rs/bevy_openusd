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

fn output_digest(outputs: &[Output]) -> blake3::Hash {
    let mut hash = blake3::Hasher::new();
    let mut write = |bytes: &[u8]| {
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    for (index, mesh, result) in outputs {
        assert!(result.is_ok());
        write(&(*index as u64).to_le_bytes());
        write(format!("{:?}", mesh.primitive_topology()).as_bytes());
        write(format!("{:?}", mesh.indices()).as_bytes());
        write(&(mesh.attributes().count() as u64).to_le_bytes());
        for (attribute, values) in mesh.attributes() {
            write(attribute.name.as_bytes());
            write(
                format!(
                    "{:?}",
                    bevy::render::render_resource::VertexFormat::from(values)
                )
                .as_bytes(),
            );
            write(values.get_bytes());
        }
        if let Some(targets) = mesh.get_morph_targets() {
            write(&[1]);
            write(bytemuck::cast_slice(targets));
        } else {
            write(&[0]);
        }
    }
    hash.finalize()
}

#[test]
fn tangent_probe_digest_tracks_geometry_and_order() {
    let triangle = || {
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0_f32; 3]; 3]);
        mesh
    };
    let original = vec![(0, triangle(), Ok(()))];
    assert_eq!(output_digest(&original), output_digest(&original));
    let mut changed = vec![(1, triangle(), Ok(()))];
    assert_ne!(output_digest(&original), output_digest(&changed));
    changed[0].0 = 0;
    changed[0]
        .1
        .insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 3]);
    assert_ne!(output_digest(&original), output_digest(&changed));
    changed[0].1.remove_attribute(Mesh::ATTRIBUTE_NORMAL);
    changed[0]
        .1
        .insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-0.0_f32; 3]; 3]);
    assert_ne!(output_digest(&original), output_digest(&changed));
    changed[0]
        .1
        .insert_indices(bevy::mesh::Indices::U16(vec![0, 1, 2]));
    let a = output_digest(&changed);
    changed[0]
        .1
        .insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    assert_ne!(a, output_digest(&changed));
}

#[test]
#[ignore = "release diagnostic requiring USD_TANGENT_ASSET"]
fn real_asset_tangent_kernel() -> anyhow::Result<()> {
    anyhow::ensure!(!cfg!(debug_assertions), "probe requires release mode");
    let file = std::fs::canonicalize(std::env::var("USD_TANGENT_ASSET")?)?;
    let samples = std::env::var("USD_TANGENT_SAMPLES")
        .unwrap_or_else(|_| "100".into())
        .parse::<usize>()?;
    anyhow::ensure!((4..=1000).contains(&samples), "samples must be in 4..1000");
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
    let world = World::new();
    let pool = bevy::tasks::TaskPoolBuilder::new()
        .num_threads(WORKERS)
        .build();
    anyhow::ensure!(pool.thread_num() == WORKERS, "unexpected pool size");
    println!(
        "tangent_kernel asset={} start={start} end={end} samples={samples} warmup={WARMUP} workers={WORKERS} input_budget={INPUT_BUDGET} scope=owned-posed-mesh-kernel-not-app-or-FPS",
        file.display()
    );
    let mut previous = start;
    let mut timings = Vec::with_capacity(samples);
    for iteration in 0..samples + WARMUP {
        let fraction = if iteration < WARMUP {
            [0.0, 0.5, 1.0, 0.5][iteration]
        } else {
            (iteration - WARMUP + 1) as f64 / samples as f64
        };
        let clock = start + (end - start) * fraction;
        if iteration >= WARMUP {
            anyhow::ensure!(clock.is_finite() && clock > previous, "non-distinct clock");
            previous = clock;
        }
        let input = inputs(&stage, &paths, clock, &world)?;
        let expected = input.iter().filter(|input| input.mesh.is_some()).count();
        anyhow::ensure!(expected > 0, "no tangent jobs");
        let started = Instant::now();
        let result = run(input, &pool, false, INPUT_BUDGET);
        let elapsed = started.elapsed();
        anyhow::ensure!(result.outputs.len() == expected, "lost tangent jobs");
        anyhow::ensure!(
            result.outputs.iter().all(|(_, _, value)| value.is_ok()),
            "failed tangent job"
        );
        anyhow::ensure!(
            result.peak_jobs <= WORKERS && result.peak_bytes <= INPUT_BUDGET,
            "unbounded work"
        );
        let digest = output_digest(&result.outputs);
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
            "tangent_kernel_{phase} index={index} clock={clock:.17} elapsed_ns={} jobs={expected} batches={} peak_jobs={} peak_bytes={} inline={} digest={digest}",
            elapsed.as_nanos(),
            result.batches,
            result.peak_jobs,
            result.peak_bytes,
            result.inline_jobs
        );
        if iteration >= WARMUP {
            timings.push(elapsed);
        }
    }
    timings.sort_unstable();
    println!(
        "tangent_kernel_summary samples={samples} median_ns={} p95_ns={} max_ns={}",
        timings[samples.div_ceil(2) - 1].as_nanos(),
        timings[(samples * 95).div_ceil(100) - 1].as_nanos(),
        timings.last().unwrap().as_nanos()
    );
    Ok(())
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
                let (read, source) =
                    super::super::skel::deformed_mesh_with_tangent_source(&context)?
                        .ok_or_else(|| anyhow::anyhow!("missing deformation for {path}"))?;
                let prepared = PreparedDeformation::with_tangent_source(read, source);
                Some(prepared.tangent_input().clone())
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
    let world = World::new();
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
    let world = World::new();
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
