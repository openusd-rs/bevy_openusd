//! Bounded offscreen playback, main-schedule intervals and published GPU samples.

use bevy::{
    app::MainScheduleOrder, diagnostic::DiagnosticsStore, ecs::schedule::ScheduleLabel, prelude::*,
};
use std::{collections::BTreeMap, time::Instant};
use usd_bevy::instance::{UsdInstanceTime, UsdInstances, UsdPlayback};

use super::{Capture, CaptureInstance, PipelineProbe};

const WARMUP: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Warmup,
    Measuring,
    Complete,
    Failed,
}

struct Frame {
    begin: Instant,
    main_ms: f64,
    interval_ms: Option<f64>,
    delta_seconds: f64,
    clocks: Vec<f64>,
}

#[derive(Default)]
struct Pass {
    last: Option<Instant>,
    values: Vec<f64>,
}

#[derive(Resource)]
pub struct PlaybackTiming {
    count: usize,
    speed: f64,
    phase: Phase,
    warmup_ready: usize,
    warmup_updates: usize,
    pending: Option<Instant>,
    started: Option<Instant>,
    previous_clocks: Vec<f64>,
    motion: Vec<usize>,
    frames: Vec<Frame>,
    passes: BTreeMap<String, Pass>,
    stage_info: String,
    error: Option<String>,
}

impl PlaybackTiming {
    pub fn parse(count: Option<&str>, speed: Option<&str>) -> Result<Option<Self>, String> {
        let Some(count) = count else {
            return if speed.is_some() {
                Err("USD_CAPTURE_PLAYBACK_SPEED requires USD_CAPTURE_PLAYBACK_SAMPLES".into())
            } else {
                Ok(None)
            };
        };
        let count = count
            .parse::<usize>()
            .ok()
            .filter(|count| (1..=600).contains(count))
            .ok_or("USD_CAPTURE_PLAYBACK_SAMPLES must be 1..600")?;
        let speed = speed
            .unwrap_or("1")
            .parse::<f64>()
            .ok()
            .filter(|speed| speed.is_finite() && speed.abs() <= 100.0)
            .ok_or("USD_CAPTURE_PLAYBACK_SPEED must be finite and within -100..100")?;
        Ok(Some(Self {
            count,
            speed,
            phase: Phase::Waiting,
            warmup_ready: 0,
            warmup_updates: 0,
            pending: None,
            started: None,
            previous_clocks: Vec::new(),
            motion: Vec::new(),
            frames: Vec::with_capacity(count),
            passes: BTreeMap::new(),
            stage_info: String::new(),
            error: None,
        }))
    }

    pub fn arm(&mut self) {
        if self.phase == Phase::Waiting {
            self.phase = Phase::Warmup;
        }
    }

    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn fail(&mut self, reason: impl Into<String>) {
        self.error = Some(reason.into());
        self.phase = Phase::Failed;
        self.pending = None;
    }

    fn begin(&mut self, now: Instant) {
        if !matches!(self.phase, Phase::Warmup | Phase::Measuring) {
            return;
        }
        if let Some(frame) = self.frames.last_mut()
            && frame.interval_ms.is_none()
        {
            frame.interval_ms = Some(now.duration_since(frame.begin).as_secs_f64() * 1000.0);
        }
        if self.frames.len() == self.count {
            if self.speed != 0.0 && self.motion.contains(&0) {
                self.fail("playback completed without clock movement for every root");
            } else if !self
                .passes
                .keys()
                .any(|path| path.ends_with("/elapsed_gpu"))
            {
                self.fail("playback has no fresh GPU timestamp samples");
            } else {
                self.phase = Phase::Complete;
            }
            return;
        }
        if self.phase == Phase::Measuring {
            self.started.get_or_insert(now);
        }
        self.pending = Some(now);
    }

    fn end(
        &mut self,
        now: Instant,
        ready: bool,
        clocks: Vec<f64>,
        delta: f64,
        diagnostics: &DiagnosticsStore,
    ) {
        let Some(begin) = self.pending.take() else {
            return;
        };
        if clocks.is_empty()
            || clocks.iter().any(|time| !time.is_finite())
            || !delta.is_finite()
            || delta < 0.0
        {
            self.fail("playback roots or frame delta are invalid");
            return;
        }
        if self.phase == Phase::Warmup {
            self.warmup_updates += 1;
            self.warmup_ready = if ready { self.warmup_ready + 1 } else { 0 };
            self.motion = vec![0; clocks.len()];
            self.previous_clocks = clocks;
            if self.warmup_ready == WARMUP {
                self.phase = Phase::Measuring;
            }
            return;
        }
        if !ready || clocks.len() != self.previous_clocks.len() {
            self.fail("playback lost scene/camera/pipeline readiness during measurement");
            return;
        }
        for ((motion, previous), clock) in self
            .motion
            .iter_mut()
            .zip(&self.previous_clocks)
            .zip(&clocks)
        {
            *motion += usize::from(previous != clock);
        }
        self.previous_clocks.clone_from(&clocks);
        self.frames.push(Frame {
            begin,
            main_ms: now.duration_since(begin).as_secs_f64() * 1000.0,
            interval_ms: None,
            delta_seconds: delta,
            clocks,
        });
        let started = self.started.unwrap();
        for diagnostic in diagnostics.iter() {
            let path = diagnostic.path().to_string();
            if !path.ends_with("/elapsed_gpu") && !path.ends_with("/elapsed_cpu") {
                continue;
            }
            let Some(measurement) = diagnostic.measurement() else {
                continue;
            };
            if measurement.time < started || measurement.time > now {
                continue;
            }
            if !measurement.value.is_finite() || measurement.value < 0.0 {
                self.fail("playback received an invalid render diagnostic duration");
                return;
            }
            if self.passes.len() >= 128 && !self.passes.contains_key(&path) {
                self.fail("playback exceeded 128 render diagnostic paths");
                return;
            }
            let pass = self.passes.entry(path).or_default();
            if pass.last.is_none_or(|last| measurement.time > last)
                && pass.values.len() < self.count
            {
                pass.last = Some(measurement.time);
                pass.values.push(measurement.value);
            }
        }
    }

    pub fn report(&self) -> String {
        let mut output = format!(
            "playback_complete={}\nplayback_samples={}\nplayback_speed={}\nplayback_warmup_updates={}\nplayback_warmup_consecutive_ready={}\nplayback_scope=native-gpu-offscreen-no-window-present-or-editor-ui\nplayback_runner_target_hz=60\nplayback_main_scope=before-First-through-after-Last-excludes-render-subapp\nplayback_interval_scope=begin-to-begin-includes-render-extraction-waits-and-runner-sleep\nplayback_gpu_scope=latest-published-per-pass-per-main-update-not-frame-paired\nplayback_gpu_counts=observed-not-guaranteed-every-rendered-frame\nplayback_clock_changes={:?}\n{}",
            self.complete(),
            self.frames.len(),
            self.speed,
            self.warmup_updates,
            WARMUP,
            self.motion,
            self.stage_info,
        );
        let main: Vec<_> = self.frames.iter().map(|frame| frame.main_ms).collect();
        let intervals: Vec<_> = self
            .frames
            .iter()
            .filter_map(|frame| frame.interval_ms)
            .collect();
        output.push_str(&summary("playback_main_ms", &main));
        output.push_str(&summary("playback_interval_ms", &intervals));
        if !intervals.is_empty() {
            output.push_str(&format!("playback_effective_rate_hz={:.6}\nplayback_intervals_over_20ms={}\nplayback_intervals_over_33_333ms={}\n",
                1000.0 * intervals.len() as f64 / intervals.iter().sum::<f64>(),
                intervals.iter().filter(|&&ms| ms > 20.0).count(),
                intervals.iter().filter(|&&ms| ms > 1000.0 / 30.0).count()));
        }
        for (index, frame) in self.frames.iter().enumerate() {
            output.push_str(&format!("playback_frame[{index}]=main_ms:{:.6},interval_ms:{:?},delta_seconds:{:.9},clocks:{:?}\n",
                frame.main_ms, frame.interval_ms, frame.delta_seconds, frame.clocks));
        }
        for (path, pass) in &self.passes {
            let kind = if path.ends_with("/elapsed_gpu") {
                "gpu"
            } else {
                "render_cpu"
            };
            output.push_str(&summary(
                &format!("playback_{kind}_ms[{path}]"),
                &pass.values,
            ));
            output.push_str(&format!("playback_{kind}_raw[{path}]={:?}\n", pass.values));
        }
        output
    }
}

fn summary(name: &str, values: &[f64]) -> String {
    if values.is_empty() {
        return format!("{name}=samples:0\n");
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    format!(
        "{name}=samples:{},median:{:.6},p95:{:.6},p99:{:.6},max:{:.6}\n",
        sorted.len(),
        sorted[sorted.len().div_ceil(2) - 1],
        sorted[(sorted.len() * 95).div_ceil(100) - 1],
        sorted[(sorted.len() * 99).div_ceil(100) - 1],
        sorted.last().unwrap()
    )
}

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct PlaybackBegin;
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct PlaybackEnd;

pub fn configure(app: &mut App, timing: PlaybackTiming) {
    app.insert_resource(timing)
        .add_systems(PlaybackBegin, begin_frame)
        .add_systems(PlaybackEnd, end_frame);
    let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
    order.insert_before(First, PlaybackBegin);
    order.insert_after(Last, PlaybackEnd);
}

fn begin_frame(
    mut timing: ResMut<PlaybackTiming>,
    mut capture: ResMut<Capture>,
    instances: Option<NonSend<UsdInstances>>,
    mut roots: Query<(Entity, &CaptureInstance, &UsdInstanceTime, &mut UsdPlayback)>,
) {
    let now = Instant::now();
    if !matches!(timing.phase, Phase::Warmup | Phase::Measuring) {
        return;
    }
    if roots.iter().count() != capture.instance_times.len() {
        timing.fail("playback root count changed");
        return;
    }
    if timing.warmup_updates == 0 {
        let Some(instances) = instances else {
            timing.fail("playback has no projected USD instances");
            return;
        };
        for (entity, instance, time, mut playback) in &mut roots {
            let Some(stage) = instances.stage(entity) else {
                timing.fail("playback root has no stage");
                return;
            };
            let start = stage.start_time_code();
            let end = stage.end_time_code();
            let rate = stage.time_codes_per_second();
            if !start.is_finite()
                || !end.is_finite()
                || !rate.is_finite()
                || rate <= 0.0
                || (timing.speed != 0.0 && (end <= start || !(end - start).is_finite()))
            {
                timing.fail("playback requires a finite authored range and positive stage rate");
                return;
            }
            *playback = UsdPlayback {
                playing: timing.speed != 0.0,
                speed: timing.speed,
                ..default()
            };
            timing.stage_info.push_str(&format!(
                "playback_stage[{}]=start:{start},end:{end},rate:{rate},initial:{}\n",
                instance.0, time.current
            ));
        }
    }
    timing.begin(now);
    if timing.complete() || timing.error().is_some() {
        for (_, instance, time, mut playback) in &mut roots {
            playback.playing = false;
            capture.instance_times[instance.0] = time.current;
        }
        capture.time = capture.instance_times[0];
        capture.ready_frames = 0;
    }
}

fn end_frame(
    mut timing: ResMut<PlaybackTiming>,
    capture: Res<Capture>,
    progress: Res<PipelineProbe>,
    diagnostics: Res<DiagnosticsStore>,
    time: Res<Time>,
    roots: Query<(
        &CaptureInstance,
        &UsdInstanceTime,
        &usd_bevy::asset::UsdSceneState,
    )>,
) {
    let now = Instant::now();
    if timing.pending.is_none() {
        return;
    }
    let mut clocks = vec![f64::NAN; capture.instance_times.len()];
    let mut ready = capture.camera_ready && progress.ready_count().is_some();
    for (instance, time, state) in &roots {
        if let Some(clock) = clocks.get_mut(instance.0) {
            *clock = time.current;
        }
        ready &= matches!(state, usd_bevy::asset::UsdSceneState::Ready);
    }
    timing.end(now, ready, clocks, time.delta_secs_f64(), &diagnostics);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::diagnostic::{Diagnostic, DiagnosticMeasurement, DiagnosticPath};
    use std::time::Duration;

    fn ready(timing: &mut PlaybackTiming, start: Instant, clocks: Vec<f64>) {
        timing.arm();
        for _ in 0..WARMUP {
            timing.begin(start);
            timing.end(
                start,
                true,
                clocks.clone(),
                0.01,
                &DiagnosticsStore::default(),
            );
        }
        assert_eq!(timing.phase, Phase::Measuring);
    }

    fn gpu(store: &mut DiagnosticsStore, now: Instant, value: f64) {
        let path = DiagnosticPath::new("render/pass/elapsed_gpu");
        if store.get(&path).is_none() {
            store.add(Diagnostic::new(path.clone()));
        }
        store
            .get_mut(&path)
            .unwrap()
            .add_measurement(DiagnosticMeasurement { time: now, value });
    }

    #[test]
    fn configuration_is_explicit_finite_and_bounded() {
        assert!(PlaybackTiming::parse(None, None).unwrap().is_none());
        assert!(PlaybackTiming::parse(None, Some("0")).is_err());
        for count in ["0", "601", "-1", "nan", ""] {
            assert!(PlaybackTiming::parse(Some(count), None).is_err());
        }
        for speed in ["NaN", "inf", "-inf", "101", "-101", ""] {
            assert!(PlaybackTiming::parse(Some("2"), Some(speed)).is_err());
        }
        for speed in ["-100", "-1", "0", "1", "100"] {
            assert!(PlaybackTiming::parse(Some("600"), Some(speed)).is_ok());
        }
    }

    #[test]
    fn warmup_and_gpu_duplicates_are_excluded_and_final_interval_is_complete() {
        let start = Instant::now();
        let mut timing = PlaybackTiming::parse(Some("2"), None).unwrap().unwrap();
        let mut store = DiagnosticsStore::default();
        ready(&mut timing, start, vec![0.0, 10.0]);
        assert!(timing.frames.is_empty());
        gpu(&mut store, start, 500.0);
        let first = start + Duration::from_millis(100);
        timing.begin(first);
        timing.end(
            first + Duration::from_millis(3),
            true,
            vec![1.0, 11.0],
            0.02,
            &store,
        );
        assert!(timing.passes.is_empty());
        timing.begin(first + Duration::from_millis(20));
        gpu(&mut store, first + Duration::from_millis(21), 2.0);
        timing.end(
            first + Duration::from_millis(25),
            true,
            vec![2.0, 12.0],
            0.02,
            &store,
        );
        assert!(!timing.complete());
        assert_eq!(timing.frames[0].interval_ms, Some(20.0));
        timing.begin(first + Duration::from_millis(50));
        assert!(timing.complete());
        assert_eq!(timing.frames[1].interval_ms, Some(30.0));
        assert_eq!(timing.motion, [2, 2]);
        assert!(
            timing
                .report()
                .contains("playback_main_ms=samples:2,median:3.000000,p95:5.000000")
        );
        assert!(
            timing
                .report()
                .contains("playback_gpu_raw[render/pass/elapsed_gpu]=[2.0]")
        );
        timing.begin(first + Duration::from_secs(2));
        assert!(timing.pending.is_none());
    }

    #[test]
    fn readiness_loss_resets_warmup_but_fails_a_measured_run() {
        let now = Instant::now();
        let mut timing = PlaybackTiming::parse(Some("2"), None).unwrap().unwrap();
        timing.arm();
        for ready in [true, true, false] {
            timing.begin(now);
            timing.end(now, ready, vec![0.0], 0.01, &DiagnosticsStore::default());
        }
        assert_eq!(timing.warmup_ready, 0);
        ready(&mut timing, now, vec![0.0]);
        timing.begin(now);
        timing.end(now, false, vec![1.0], 0.01, &DiagnosticsStore::default());
        assert!(timing.error().unwrap().contains("readiness"));
        assert!(timing.frames.is_empty());
    }

    #[test]
    fn stationary_playback_fails_but_explicit_idle_is_valid() {
        let now = Instant::now();
        for speed in ["1", "0"] {
            let mut timing = PlaybackTiming::parse(Some("1"), Some(speed))
                .unwrap()
                .unwrap();
            let mut store = DiagnosticsStore::default();
            ready(&mut timing, now, vec![0.0]);
            timing.begin(now);
            gpu(&mut store, now, 1.0);
            timing.end(
                now + Duration::from_millis(1),
                true,
                vec![0.0],
                0.01,
                &store,
            );
            timing.begin(now + Duration::from_millis(20));
            assert_eq!(timing.complete(), speed == "0");
        }
    }

    #[test]
    fn stale_or_duplicate_gpu_samples_cannot_fill_the_window() {
        let now = Instant::now();
        let mut timing = PlaybackTiming::parse(Some("3"), None).unwrap().unwrap();
        let mut store = DiagnosticsStore::default();
        ready(&mut timing, now, vec![0.0]);
        for index in 0..3 {
            let start = now + Duration::from_millis(index * 20);
            timing.begin(start);
            if index == 0 {
                gpu(&mut store, start, 2.0);
            }
            timing.end(
                start + Duration::from_millis(2),
                true,
                vec![index as f64 + 1.0],
                0.02,
                &store,
            );
        }
        assert_eq!(timing.passes.values().next().unwrap().values, [2.0]);
        timing.begin(now + Duration::from_millis(60));
        assert!(timing.complete());
    }

    #[test]
    fn missing_gpu_data_invalid_durations_and_root_changes_fail() {
        let now = Instant::now();
        for case in ["missing", "invalid", "roots"] {
            let mut timing = PlaybackTiming::parse(Some("1"), None).unwrap().unwrap();
            let mut store = DiagnosticsStore::default();
            ready(&mut timing, now, vec![0.0]);
            timing.begin(now);
            if case == "invalid" {
                gpu(&mut store, now, -1.0);
            }
            let clocks = if case == "roots" {
                vec![1.0, 2.0]
            } else {
                vec![1.0]
            };
            timing.end(now, true, clocks, 0.02, &store);
            timing.begin(now + Duration::from_millis(20));
            assert!(!timing.complete(), "{case}");
            assert!(timing.error().is_some(), "{case}");
        }
    }

    #[test]
    fn render_cpu_diagnostics_remain_separate_and_cannot_replace_gpu_samples() {
        let now = Instant::now();
        let mut timing = PlaybackTiming::parse(Some("1"), None).unwrap().unwrap();
        let mut store = DiagnosticsStore::default();
        let path = DiagnosticPath::new("render/pass/elapsed_cpu");
        store.add(Diagnostic::new(path.clone()));
        store
            .get_mut(&path)
            .unwrap()
            .add_measurement(DiagnosticMeasurement {
                time: now,
                value: 3.0,
            });
        ready(&mut timing, now, vec![0.0]);
        timing.begin(now);
        timing.end(
            now + Duration::from_millis(2),
            true,
            vec![1.0],
            0.02,
            &store,
        );
        timing.begin(now + Duration::from_millis(20));
        assert!(timing.error().unwrap().contains("GPU timestamp"));
        assert!(
            timing
                .report()
                .contains("playback_render_cpu_raw[render/pass/elapsed_cpu]=[3.0]")
        );
        assert!(
            !timing
                .report()
                .contains("playback_gpu_raw[render/pass/elapsed_cpu]")
        );
    }

    #[test]
    fn recorder_schedules_enclose_first_and_last_without_starting_playback() {
        #[derive(Resource, Default)]
        struct Order(Vec<&'static str>);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Order>()
            .init_resource::<DiagnosticsStore>()
            .init_resource::<PipelineProbe>()
            .insert_resource(
                Capture::parse(&["fixture.usda".into(), "unused.png".into(), "0".into()]).unwrap(),
            );
        configure(
            &mut app,
            PlaybackTiming::parse(Some("1"), None).unwrap().unwrap(),
        );
        app.add_systems(PlaybackBegin, |mut order: ResMut<Order>| {
            order.0.push("begin")
        })
        .add_systems(First, |mut order: ResMut<Order>| order.0.push("first"))
        .add_systems(Last, |mut order: ResMut<Order>| order.0.push("last"))
        .add_systems(PlaybackEnd, |mut order: ResMut<Order>| order.0.push("end"));
        app.update();
        assert_eq!(
            app.world().resource::<Order>().0,
            ["begin", "first", "last", "end"]
        );
        assert_eq!(
            app.world().resource::<PlaybackTiming>().phase,
            Phase::Waiting
        );
        assert!(app.world().resource::<PlaybackTiming>().frames.is_empty());
    }
}
