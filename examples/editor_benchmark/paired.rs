use super::*;
use usd_bevy::route::preparation::{MeshPreparationTiming, ParallelMeshPreparation};

const NAMES: [&str; 2] = ["serial", "batched"];

struct Lane {
    app: App,
    bridge: EditorBridge,
    measurement: Measurement,
    document_id: u64,
    bounds: [f64; 2],
    timings: Vec<Duration>,
    warmup: [Duration; SEEK_WARMUP],
}

impl Lane {
    fn open(path: &Path, gpu: bool, serial: bool) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_policy(path, gpu, serial, false)
    }

    fn open_policy(
        path: &Path,
        gpu: bool,
        serial: bool,
        defer: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let settings = ParallelMeshPreparation {
            max_jobs: if serial {
                0
            } else {
                ParallelMeshPreparation::default().max_jobs
            },
            ..default()
        };
        let (mut app, bridge, measurement) =
            open_editor_with_deferral(path, gpu, Some(settings), defer)?;
        app.init_resource::<MeshPreparationTiming>();
        let view = bridge.view()?;
        Ok(Self {
            app,
            bridge,
            measurement,
            document_id: view.document.document_id,
            bounds: [view.timeline.start, view.timeline.end],
            timings: Vec::new(),
            warmup: [Duration::ZERO; SEEK_WARMUP],
        })
    }

    fn seek(&mut self, time: f64) -> Result<Duration, Box<dyn std::error::Error>> {
        self.bridge.send(EditorCommand::Seek(time))?;
        let started = Instant::now();
        self.app.update();
        let elapsed = started.elapsed();
        let state = self.bridge.view()?;
        if state.status != "Ready"
            || state.timeline.current != time
            || state.document.document_id != self.document_id
        {
            return Err("paired seek did not preserve document, readiness and clock".into());
        }
        for (peak, current) in self
            .measurement
            .peak_asset_counts
            .iter_mut()
            .zip(asset_counts(self.app.world()))
        {
            *peak = (*peak).max(current);
        }
        Ok(elapsed)
    }
}

fn order(iteration: usize, reverse: bool) -> [usize; 2] {
    let first = usize::from(matches!(iteration % 4, 1 | 2) ^ reverse);
    [first, 1 - first]
}

fn clocks(bounds: [f64; 2], samples: usize) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    let [start, end] = bounds;
    let span = end - start;
    if samples == 0
        || !samples.is_multiple_of(4)
        || !start.is_finite()
        || !end.is_finite()
        || !span.is_finite()
        || span <= 0.0
    {
        return Err("paired seeks require a finite increasing timeline and a positive multiple of four samples".into());
    }
    let clocks: Vec<_> = (0..samples + SEEK_WARMUP)
        .map(|iteration| {
            let fraction = if iteration < SEEK_WARMUP {
                [0.0, 0.5, 1.0, 0.5][iteration]
            } else {
                (iteration - SEEK_WARMUP + 1) as f64 / samples as f64
            };
            start + span * fraction
        })
        .collect();
    if clocks[SEEK_WARMUP] <= start
        || clocks[SEEK_WARMUP..]
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err("timeline precision cannot represent distinct paired seek clocks".into());
    }
    Ok(clocks)
}

fn quantile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

pub(super) fn payload_counts(m: &Measurement) -> [usize; 14] {
    [
        m.mesh_entities,
        m.subset_entities,
        m.mesh_assets,
        m.vertices,
        m.unreferenced_vertices,
        m.vertex_bytes,
        m.index_bytes,
        m.morph_bytes,
        m.image_bytes,
        m.gpu_morph_entities,
        m.cached_meshes,
        m.cached_mesh_payload_bytes,
        m.cached_tangent_payload_bytes,
        m.cached_assembly_payload_bytes,
    ]
}

pub(super) fn run(
    path: &Path,
    gpu: bool,
    samples: usize,
    reverse: bool,
) -> Result<[Measurement; 2], Box<dyn std::error::Error>> {
    run_policy(path, gpu, samples, reverse, false)
}

pub(super) fn run_residency(
    path: &Path,
    gpu: bool,
    samples: usize,
    reverse: bool,
) -> Result<[Measurement; 2], Box<dyn std::error::Error>> {
    run_policy(path, gpu, samples, reverse, true)
}

fn run_policy(
    path: &Path,
    gpu: bool,
    samples: usize,
    reverse: bool,
    residency: bool,
) -> Result<[Measurement; 2], Box<dyn std::error::Error>> {
    if [
        "USD_DEFER_HIDDEN_MESHES",
        "USD_RESIDENCY_BUDGET_MS",
        "USD_BENCH_PREWARM_HIDDEN",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
    {
        return Err(
            "paired probes reject external deferral, residency budget and prewarm overrides".into(),
        );
    }
    let names = if residency {
        ["eager", "deferred"]
    } else {
        NAMES
    };
    let open = |candidate| {
        if residency {
            Lane::open_policy(path, gpu, false, candidate)
        } else {
            Lane::open(path, gpu, !candidate)
        }
    };
    let mut lanes = if reverse {
        let candidate = open(true)?;
        [open(false)?, candidate]
    } else {
        [open(false)?, open(true)?]
    };
    if lanes[0].bounds != lanes[1].bounds || lanes[0].document_id == lanes[1].document_id {
        return Err(
            "paired apps must have equal timelines and distinct document identities".into(),
        );
    }
    let clocks = clocks(lanes[0].bounds, samples)?;
    println!(
        "paired_probe scope=same-binary-policy-not-native-fps residency={residency} apps=independent updates=sequential order=ABBA reverse={reverse} initial_first={} warmup={SEEK_WARMUP} samples={samples} start={} end={} rss=combined-two-apps-process-lifetime document_ids={},{} pool_threads={}",
        names[usize::from(reverse)],
        lanes[0].bounds[0],
        lanes[0].bounds[1],
        lanes[0].document_id,
        lanes[1].document_id,
        bevy::tasks::ComputeTaskPool::get().thread_num()
    );
    let rss_before = resident_bytes();
    for lane in &mut lanes {
        lane.measurement.rss_before_seeks = rss_before;
    }
    for (iteration, time) in clocks.into_iter().enumerate() {
        if iteration == SEEK_WARMUP {
            for (lane, name) in lanes.iter_mut().zip(names) {
                println!(
                    "paired_warmup_summary lane={name} total_ns={} max_ns={}",
                    lane.warmup.iter().sum::<Duration>().as_nanos(),
                    lane.warmup.iter().max().unwrap().as_nanos()
                );
                report_routes(
                    lane.app.world(),
                    &format!("{name}-after-warmup"),
                    SEEK_WARMUP,
                );
                reset_profiles(lane.app.world_mut());
            }
        }
        let sequence = order(iteration, reverse);
        let mut elapsed = [Duration::ZERO; 2];
        for index in sequence {
            elapsed[index] = lanes[index].seek(time)?;
        }
        for (lane, elapsed) in lanes.iter_mut().zip(elapsed) {
            if iteration < SEEK_WARMUP {
                lane.warmup[iteration] = elapsed;
            } else {
                lane.timings.push(elapsed);
            }
        }
        let (phase, index) = seek_sample_phase(iteration);
        if residency {
            let (a, b) = lanes.split_at_mut(1);
            let visible = residency_probe::compare_visible(&mut a[0].app, &mut b[0].app)?;
            println!(
                "paired_visible phase={phase} index={index} meshes={visible} exact=true excludes=timer"
            );
        }
        println!(
            "paired_{phase}_sample index={index} clock={time:.17} first={} {}_ns={} {}_ns={}",
            names[sequence[0]],
            names[0],
            elapsed[0].as_nanos(),
            names[1],
            elapsed[1].as_nanos()
        );
    }
    if !residency
        && lanes[0]
            .app
            .world()
            .resource::<MeshPreparationTiming>()
            .jobs
            != 0
    {
        return Err("serial control unexpectedly used preparation jobs".into());
    }
    let batched_wins = lanes[0]
        .timings
        .iter()
        .zip(&lanes[1].timings)
        .filter(|(serial, batched)| batched < serial)
        .count();
    for (lane, name) in lanes.iter_mut().zip(names) {
        lane.timings.sort_unstable();
        lane.measurement.seek_median = quantile(&lane.timings, 50);
        lane.measurement.seek_p95 = quantile(&lane.timings, 95);
        lane.measurement.seek_max = *lane.timings.last().unwrap();
        measure_payload(&mut lane.app, &mut lane.measurement)?;
        println!(
            "paired_summary lane={name} samples={samples} mean_ns={} median_ns={} p95_ns={} max_ns={}",
            lane.timings.iter().sum::<Duration>().as_nanos() / samples as u128,
            lane.measurement.seek_median.as_nanos(),
            lane.measurement.seek_p95.as_nanos(),
            lane.measurement.seek_max.as_nanos()
        );
        println!("paired_payload lane={name} {:?}", lane.measurement);
        report_routes(lane.app.world(), &format!("{name}-measured-seeks"), samples);
    }
    println!(
        "paired_comparison {}_wins={batched_wins} samples={samples} payload_counts_equal={} peak_asset_counts_equal={} combined_rss_before={rss_before:?} combined_rss_after={:?} combined_rss_process_peak={:?}",
        names[1],
        payload_counts(&lanes[0].measurement) == payload_counts(&lanes[1].measurement),
        lanes[0].measurement.peak_asset_counts == lanes[1].measurement.peak_asset_counts,
        resident_bytes(),
        peak_resident_bytes()
    );
    Ok(lanes.map(|lane| lane.measurement))
}

#[test]
fn paired_schedule_balances_order_and_preserves_unique_clocks() {
    assert_eq!([0, 1, 2, 3].map(|i| order(i, false)[0]), [0, 1, 1, 0]);
    assert_eq!([0, 1, 2, 3].map(|i| order(i, true)[0]), [1, 0, 0, 1]);
    let times = clocks([24.0, 96.0], 1000).unwrap();
    assert_eq!(&times[..4], &[24.0, 60.0, 96.0, 60.0]);
    assert_eq!(times.last(), Some(&96.0));
    assert!(times[4..].windows(2).all(|pair| pair[0] < pair[1]));
    for reverse in [false, true] {
        for group in (0..1004).step_by(4) {
            let orders: Vec<_> = (group..group + 4).map(|i| order(i, reverse)).collect();
            assert_eq!(orders.iter().filter(|pair| pair[0] == 0).count(), 2);
            assert!(orders.iter().all(|pair| pair[0] != pair[1]));
        }
    }
    for bounds in [
        [0.0, 0.0],
        [1.0, 0.0],
        [f64::NAN, 1.0],
        [-f64::MAX, f64::MAX],
        [1.0, 1.0 + f64::EPSILON],
    ] {
        assert!(clocks(bounds, 1000).is_err());
    }
    assert!(clocks([0.0, 1.0], 0).is_err());
    assert!(clocks([0.0, 1.0], 7).is_err());
    let sorted: Vec<_> = (1..=100).map(Duration::from_nanos).collect();
    assert_eq!(quantile(&sorted, 50), Duration::from_nanos(50));
    assert_eq!(quantile(&sorted, 95), Duration::from_nanos(95));
}

#[test]
fn profile_reset_excludes_warmup_without_changing_preparation_policy() {
    use usd_bevy::route::{gpu_skin::GpuSkinUpdateTiming, residency::ResidencyPreparationTiming};
    let mut world = World::new();
    world.insert_resource(ParallelMeshPreparation {
        max_jobs: 0,
        ..default()
    });
    world.insert_resource(MeshPreparationTiming {
        jobs: 3,
        ..default()
    });
    world.insert_resource(GpuSkinUpdateTiming {
        elapsed: Duration::from_secs(1),
        ..default()
    });
    world.insert_resource(ResidencyPreparationTiming {
        attempts: 2,
        ..default()
    });
    reset_profiles(&mut world);
    assert_eq!(world.resource::<MeshPreparationTiming>().jobs, 0);
    assert_eq!(
        world.resource::<GpuSkinUpdateTiming>().elapsed,
        Duration::ZERO
    );
    assert_eq!(world.resource::<ResidencyPreparationTiming>().attempts, 0);
    assert_eq!(world.resource::<ParallelMeshPreparation>().max_jobs, 0);
    assert!(!world.contains_resource::<usd_bevy::route::ProjectionTimings>());
}

#[test]
fn paired_editors_preserve_payloads_and_disable_only_serial_preparation() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skel_morph_blended_animated.usda");
    for gpu in [false, true] {
        for reverse in [false, true] {
            let results = run(&path, gpu, 8, reverse).unwrap();
            assert_eq!(payload_counts(&results[0]), payload_counts(&results[1]));
            assert_eq!(results[0].peak_asset_counts, results[1].peak_asset_counts);
            assert!(
                results
                    .iter()
                    .all(|result| result.seek_median > Duration::ZERO)
            );
        }
        let mut serial = Lane::open(&path, gpu, true).unwrap();
        let mut batched = Lane::open(&path, gpu, false).unwrap();
        serial.seek(2.5).unwrap();
        batched.seek(2.5).unwrap();
        assert_eq!(
            serial.app.world().resource::<MeshPreparationTiming>().jobs,
            0
        );
        if bevy::tasks::ComputeTaskPool::get().thread_num() > 1 {
            assert!(batched.app.world().resource::<MeshPreparationTiming>().jobs > 0);
        }
    }
}
