use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::Path;

pub struct CaptureConfig {
    output: String,
    time: Option<f64>,
    delay_ms: u64,
    seeks: Vec<f64>,
    residency: Option<crate::capture_residency::Config>,
    morph_probe: bool,
    renderdoc: bool,
    target_probe: bool,
}

impl CaptureConfig {
    fn parse(output: Option<String>, time: Option<String>) -> Result<Option<Self>, String> {
        let Some(output) = output else {
            return Ok(None);
        };
        if output.ends_with('/')
            || (cfg!(windows) && output.ends_with('\\'))
            || Path::new(&output)
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("png")
        {
            return Err("USD_SCREENSHOT must end in .png".into());
        }
        let time = time
            .map(|value| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|time| time.is_finite())
                    .ok_or_else(|| "USD_CAPTURE_TIME must be a finite number".to_owned())
            })
            .transpose()?;
        Ok(Some(Self {
            output,
            time,
            delay_ms: 0,
            seeks: Vec::new(),
            residency: None,
            morph_probe: false,
            renderdoc: false,
            target_probe: false,
        }))
    }

    fn delay(value: Option<String>) -> Result<u64, String> {
        match value {
            None => Ok(0),
            Some(value) => value
                .parse::<u64>()
                .ok()
                .filter(|value| *value <= 45_000)
                .ok_or_else(|| "USD_CAPTURE_DELAY_MS must be an integer from 0 to 45000".into()),
        }
    }

    fn seeks(value: Option<String>) -> Result<Vec<f64>, String> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        let mut seeks = Vec::new();
        for field in value.split(',') {
            let time = field.trim().parse::<f64>().ok().filter(|t| t.is_finite());
            if seeks.len() == 32 || time.is_none() {
                return Err(
                    "USD_CAPTURE_SEEKS must contain 1..32 finite comma-separated time codes".into(),
                );
            }
            seeks.push(time.unwrap());
        }
        Ok(seeks)
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        let read = |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(format!("{name}: {error}")),
        };
        let mut config = Self::parse(read("USD_SCREENSHOT")?, read("USD_CAPTURE_TIME")?)?;
        if let Some(config) = &mut config {
            config.delay_ms = Self::delay(read("USD_CAPTURE_DELAY_MS")?)?;
            config.seeks = Self::seeks(read("USD_CAPTURE_SEEKS")?)?;
            config.residency =
                crate::capture_residency::Config::parse(read("USD_CAPTURE_RESIDENCY")?)?;
            config.morph_probe =
                Self::morph_probe(read("USD_CAPTURE_MORPH_PROBE")?, config.residency.is_some())?;
            config.renderdoc = Self::renderdoc(read("USD_CAPTURE_RENDERDOC")?, config.morph_probe)?;
            config.target_probe = Self::target_probe(
                read("USD_CAPTURE_TARGET_PROBE")?,
                config.morph_probe,
                config.renderdoc,
            )?;
        }
        Ok(config)
    }

    fn morph_probe(value: Option<String>, has_residency: bool) -> Result<bool, String> {
        match value.as_deref() {
            None | Some("0") => Ok(false),
            Some("1") if has_residency => Ok(true),
            _ => Err(
                "USD_CAPTURE_MORPH_PROBE requires 0 or 1, and 1 requires USD_CAPTURE_RESIDENCY"
                    .into(),
            ),
        }
    }

    fn renderdoc(value: Option<String>, morph_probe: bool) -> Result<bool, String> {
        match value.as_deref() {
            None | Some("0") => Ok(false),
            Some("1") if morph_probe && cfg!(all(feature = "debugger_capture", target_os = "linux")) => Ok(true),
            _ => Err(
                "USD_CAPTURE_RENDERDOC requires 0 or 1; 1 requires USD_CAPTURE_MORPH_PROBE=1 and the Linux debugger_capture build"
                    .into(),
            ),
        }
    }

    fn target_probe(
        value: Option<String>,
        morph_probe: bool,
        renderdoc: bool,
    ) -> Result<bool, String> {
        match value.as_deref() {
            None => Ok(renderdoc),
            Some("0") => Ok(false),
            Some("1") if morph_probe => Ok(true),
            _ => Err(
                "USD_CAPTURE_TARGET_PROBE requires 0 or 1; 1 requires USD_CAPTURE_MORPH_PROBE=1"
                    .into(),
            ),
        }
    }
}

#[derive(Default)]
struct CaptureSeeks {
    document: Option<u64>,
    initial_time: Option<f64>,
    pending: Option<f64>,
    observed: Vec<f64>,
}

#[derive(Debug, PartialEq)]
enum SeekAction {
    Wait,
    Send(f64),
    Ready,
}

impl CaptureSeeks {
    fn advance(&mut self, document: Option<u64>, time: f64, seeks: &[f64]) -> SeekAction {
        if document != self.document {
            self.document = document;
            self.initial_time = document.map(|_| time);
            self.pending = None;
            self.observed.clear();
        }
        if document.is_none() {
            return SeekAction::Wait;
        }
        if let Some(expected) = self.pending {
            if time != expected {
                return SeekAction::Wait;
            }
            self.observed.push(time);
            self.pending = None;
        }
        if let Some(&next) = seeks.get(self.observed.len()) {
            self.pending = Some(next);
            return SeekAction::Send(next);
        }
        if seeks.last().is_some_and(|&last| time != last) {
            return SeekAction::Wait;
        }
        SeekAction::Ready
    }

    fn report(&self, seeks: &[f64]) -> String {
        format!(
            "seek_scope=live-editor-command\nseek_initial_time={:?}\nseek_requested={seeks:?}\nseek_observed={:?}\n",
            self.initial_time, self.observed
        )
    }
}

#[derive(Default)]
struct CaptureGate {
    document: Option<u64>,
    frames: u32,
    finished: bool,
    ready_since: Option<std::time::Duration>,
    delay: std::time::Duration,
}

#[derive(Debug, PartialEq)]
enum CaptureAction {
    Wait,
    Request,
    Timeout,
}

impl CaptureGate {
    fn advance(&mut self, document: Option<u64>, elapsed: std::time::Duration) -> CaptureAction {
        if self.finished {
            return CaptureAction::Wait;
        }
        if elapsed >= std::time::Duration::from_secs(60) {
            self.finished = true;
            return CaptureAction::Timeout;
        }
        if document != self.document {
            self.frames = 0;
            self.document = document;
            self.ready_since = document.map(|_| elapsed);
        }
        if document.is_none() {
            return CaptureAction::Wait;
        }
        self.frames = self.frames.saturating_add(1);
        if self.frames < 120 || elapsed.saturating_sub(self.ready_since.unwrap()) < self.delay {
            return CaptureAction::Wait;
        }
        self.finished = true;
        CaptureAction::Request
    }
}

fn save_readback(image: &Image, output: &Path, timing: &str) -> Result<(), String> {
    if output.extension().and_then(|value| value.to_str()) != Some("png") {
        return Err("USD_SCREENSHOT must end in .png".into());
    }
    let rgba = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    std::fs::write(output.with_extension("rgba"), rgba.as_raw())
        .map_err(|error| error.to_string())?;
    let report = format!(
        "source=embedded-viewer\nwidth={width}\nheight={height}\nlayout=rgba8\nsource_format={:?}\nrow_bytes={}\nbytes={}\n{timing}",
        image.texture_descriptor.format,
        u64::from(width) * 4,
        rgba.len()
    );
    std::fs::write(output.with_extension("capture.txt"), report)
        .map_err(|error| error.to_string())?;
    rgba.save(output).map_err(|error| error.to_string())
}

struct PendingReadback {
    image: Image,
    output: String,
    metadata: String,
    uniforms: Option<crate::capture_uniforms::UniformProbe>,
    screenshot: Entity,
    received: std::time::Instant,
}

#[derive(Default, Resource)]
struct ReadbackWriter(Option<PendingReadback>);

fn wait_for_report(ready: bool, elapsed: std::time::Duration) -> bool {
    !ready && elapsed < std::time::Duration::from_secs(10)
}

fn write_pending_readback(mut writer: ResMut<ReadbackWriter>) {
    let Some(pending) = &writer.0 else { return };
    let ready = pending
        .uniforms
        .as_ref()
        .is_none_or(|probe| probe.ready(pending.screenshot));
    if wait_for_report(ready, pending.received.elapsed()) {
        return;
    }
    let mut pending = writer.0.take().unwrap();
    let report = pending
        .uniforms
        .as_ref()
        .map(|probe| probe.report(pending.screenshot));
    match &report {
        Some(Ok(report)) => pending.metadata.push_str(report),
        Some(Err(error)) => pending
            .metadata
            .push_str(&format!("gpu_uniform_error={error}\n")),
        None => (),
    }
    let output = pending.output;
    match save_readback(&pending.image, Path::new(&output), &pending.metadata) {
        Ok(()) => match report {
            Some(Err(error)) => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: {error}"),
            _ => eprintln!("VIEWPORT_CAPTURE_OK {output}"),
        },
        Err(error) => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: {error}"),
    }
}

type CaptureCamera = (
    Entity,
    &'static Camera,
    &'static RenderTarget,
    &'static GlobalTransform,
    Option<&'static bevy::light::EnvironmentMapLight>,
    Option<&'static usd_bevy::route::dome_environment::UsdDomeEnvironmentState>,
);

pub fn configure(app: &mut App) {
    let Some(CaptureConfig {
        output,
        time,
        delay_ms,
        seeks,
        residency,
        morph_probe,
        renderdoc,
        target_probe,
    }) = CaptureConfig::from_env().expect("invalid capture configuration")
    else {
        return;
    };
    app.init_resource::<ReadbackWriter>()
        .add_systems(Update, write_pending_readback);
    if let Some(current) = time {
        app.insert_resource(usd_bevy::route::StageTime { current });
    }
    if std::env::var_os("USD_CAPTURE_PHASE_PROBE").is_some() {
        crate::transparent_probe::configure(app);
    }
    if morph_probe {
        let config = residency
            .as_ref()
            .expect("morph probe requires USD_CAPTURE_RESIDENCY");
        crate::morph_probe::configure(app, config.mesh_path());
        crate::capture_uniforms::configure(app, renderdoc, target_probe, Path::new(&output));
    }
    let uniforms = morph_probe.then(|| {
        app.world()
            .resource::<crate::capture_uniforms::UniformProbe>()
            .clone()
    });
    let started = std::time::Instant::now();
    app.add_systems(Last, move |mut commands: Commands, cameras: Query<CaptureCamera, With<Camera3d>>,
        generators: Query<(), With<bevy::light::GeneratedEnvironmentMapLight>>,
        diagnostics: Option<Res<usd_bevy::route::dome_environment::UsdDomeEnvironmentDiagnostics>>,
        session: Option<NonSend<usd_bevy::editor::EditorSession>>, time: Res<usd_bevy::route::StageTime>, mut gate: Local<CaptureGate>,
        bridge: Res<usd_bevy::editor::EditorBridge>, mut seek_state: Local<CaptureSeeks>,
        residency_inputs: crate::capture_residency::Inputs, mut residency_state: Local<crate::capture_residency::State>,
        (transparent, morph): (Option<Res<crate::transparent_probe::TransparentProbe>>, Option<Res<crate::morph_probe::MorphProbe>>),
        device: Option<Res<bevy::render::renderer::RenderDevice>>,
        adapter: Option<Res<bevy::render::renderer::RenderAdapter>>,
        meshes: Res<Assets<Mesh>>, mesh_references: Query<(&Mesh3d, &InheritedVisibility)>| {
        let active = cameras.iter().find(|(_, camera, ..)| camera.is_active);
        let document = active.and_then(|_| session.as_ref().map(|session| session.document_id()));
        gate.delay = std::time::Duration::from_millis(delay_ms);
        let elapsed = started.elapsed();
        if gate.finished {
            return;
        }
        let seek_action = seek_state.advance(document, time.current, &seeks);
        let mut ready_document = (seek_action == SeekAction::Ready).then_some(document).flatten();
        if ready_document.is_some() && let Some(config) = &residency {
            let (_, camera, _, transform, ..) = active.expect("residency capture requires a camera");
            let camera = crate::capture_metadata::camera_report(transform, camera.clip_from_view());
            match residency_state.advance(config, &residency_inputs, &bridge, camera) {
                Ok(true) => (),
                Ok(false) => ready_document = None,
                Err(error) => {
                    gate.finished = true;
                    eprintln!("VIEWPORT_CAPTURE_ERROR {output}: {error}");
                    return;
                }
            }
        }
        match gate.advance(ready_document, elapsed) {
            CaptureAction::Timeout => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: timed out waiting for an open document, active camera, seek sequence and capture delay"),
            CaptureAction::Request => {
                let (entity, camera, target, transform, environment, environment_state) = active.expect("capture gate requires an active camera");
                let output = output.clone();
                let mut timing = format!("minimum_ready_delay_ms={delay_ms}\nready_elapsed_ms={}\nready_updates_at_request={}\ndocument_id_at_request={}\nscene_time_at_request={}\n",
                    elapsed.saturating_sub(gate.ready_since.unwrap()).as_millis(), gate.frames, document.unwrap(), time.current);
                timing.push_str(&seek_state.report(&seeks));
                if let Some(device) = &device {
                    timing.push_str(&format!("render_device_storage_buffers_per_shader_stage={}\n",
                        device.limits().max_storage_buffers_per_shader_stage));
                }
                if let Some(adapter) = &adapter {
                    timing.push_str(&format!("render_adapter_storage_buffers_per_shader_stage={}\n",
                        adapter.limits().max_storage_buffers_per_shader_stage));
                }
                if let Some(config) = &residency {
                    timing.push_str(&residency_state.report(config));
                }
                if let Some(probe) = &transparent {
                    timing.push_str(&probe.report());
                }
                if let Some(probe) = &morph {
                    timing.push_str(&probe.report());
                }
                timing.push_str(&crate::capture_metadata::camera_report(transform, camera.clip_from_view()));
                timing.push_str(&crate::capture_metadata::mesh_residency_report(&meshes,
                    mesh_references.iter().map(|(mesh, visibility)| (mesh.0.id(), visibility.get()))));
                timing.push_str(&crate::capture_metadata::environment_report(environment, environment_state,
                    diagnostics.as_ref().map_or(0, |value| value.recorded_generations), generators.iter().count(), "Last-at-request"));
                let mut screenshot = commands.spawn(Screenshot(target.clone()));
                if uniforms.is_some() {
                    screenshot.insert(crate::capture_uniforms::UniformCapture(entity));
                }
                let uniforms = uniforms.clone();
                screenshot.observe(move |event: On<ScreenshotCaptured>,
                    mut writer: ResMut<ReadbackWriter>,
                    grids: Query<(&GlobalTransform, &bevy::dev_tools::infinite_grid::InfiniteGridSettings)>| {
                    let mut metadata = timing.clone();
                    metadata.push_str("readback_grid_scope=ScreenshotCaptured-main-world\n");
                    for (transform, settings) in &grids {
                        metadata.push_str("readback_");
                        metadata.push_str(&crate::transparent_probe::grid_report(transform, settings));
                    }
                    if writer.0.is_some() {
                        eprintln!("VIEWPORT_CAPTURE_ERROR {output}: another readback is pending");
                        return;
                    }
                    writer.0 = Some(PendingReadback {
                        image: event.image.clone(), output: output.clone(), metadata,
                        uniforms: uniforms.clone(), screenshot: event.entity,
                        received: std::time::Instant::now(),
                    });
                });
            }
            CaptureAction::Wait => {
                if let SeekAction::Send(current) = seek_action
                    && let Err(error) = bridge.send(usd_bevy::editor::EditorCommand::Seek(current)) {
                    gate.finished = true;
                    eprintln!("VIEWPORT_CAPTURE_ERROR {output}: seek command failed: {error}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_writer_waits_for_completion_with_a_deadline() {
        use std::time::Duration;
        assert!(wait_for_report(false, Duration::ZERO));
        assert!(wait_for_report(false, Duration::from_millis(9999)));
        assert!(!wait_for_report(false, Duration::from_secs(10)));
        assert!(!wait_for_report(true, Duration::ZERO));
    }

    #[test]
    fn target_probe_is_independent_of_debugger_injection() {
        assert_eq!(CaptureConfig::target_probe(None, true, true), Ok(true));
        assert_eq!(CaptureConfig::target_probe(None, true, false), Ok(false));
        for debugger in [false, true] {
            assert_eq!(
                CaptureConfig::target_probe(Some("0".into()), true, debugger),
                Ok(false)
            );
            assert_eq!(
                CaptureConfig::target_probe(Some("1".into()), true, debugger),
                Ok(true)
            );
        }
        assert!(CaptureConfig::target_probe(Some("1".into()), false, false).is_err());
        for invalid in ["", "true", "2", " 1"] {
            assert!(CaptureConfig::target_probe(Some(invalid.into()), true, false).is_err());
        }
    }
    use bevy::render::render_resource::TextureFormat;

    #[test]
    fn debugger_capture_is_explicit_and_requires_the_frame_probe() {
        for probe in [false, true] {
            assert_eq!(CaptureConfig::renderdoc(None, probe), Ok(false));
            assert_eq!(CaptureConfig::renderdoc(Some("0".into()), probe), Ok(false));
            assert_eq!(
                CaptureConfig::renderdoc(Some("1".into()), probe).is_ok(),
                probe && cfg!(all(feature = "debugger_capture", target_os = "linux"))
            );
            for invalid in ["", "true", "2"] {
                assert!(CaptureConfig::renderdoc(Some(invalid.into()), probe).is_err());
            }
        }
    }

    #[test]
    fn morph_probe_is_explicit_and_requires_residency() {
        for residency in [false, true] {
            assert_eq!(CaptureConfig::morph_probe(None, residency), Ok(false));
            assert_eq!(
                CaptureConfig::morph_probe(Some("0".into()), residency),
                Ok(false)
            );
            for value in ["", "true", "2"] {
                assert!(CaptureConfig::morph_probe(Some(value.into()), residency).is_err());
            }
        }
        assert!(CaptureConfig::morph_probe(Some("1".into()), false).is_err());
        assert_eq!(CaptureConfig::morph_probe(Some("1".into()), true), Ok(true));
    }

    #[test]
    fn capture_seek_options_are_finite_bounded_and_optional() {
        assert!(CaptureConfig::seeks(None).unwrap().is_empty());
        assert_eq!(
            CaptureConfig::seeks(Some(" -1, 0, 5, 10, 5, 5, 1e100 ".into())).unwrap(),
            [-1.0, 0.0, 5.0, 10.0, 5.0, 5.0, 1e100]
        );
        assert_eq!(
            CaptureConfig::seeks(Some(vec!["1"; 32].join(",")))
                .unwrap()
                .len(),
            32
        );
        for value in [
            "", "1,", ",1", "1,,2", "NaN", "inf", "-inf", "1e999", "five",
        ] {
            assert!(CaptureConfig::seeks(Some(value.into())).is_err(), "{value}");
        }
        assert!(CaptureConfig::seeks(Some(vec!["1"; 33].join(","))).is_err());
    }

    #[test]
    fn capture_seeks_acknowledge_each_update_and_reset_with_document() {
        let mut state = CaptureSeeks::default();
        let seeks = [0.0, 10.0, 5.0, 5.0];
        assert_eq!(state.advance(None, 0.0, &seeks), SeekAction::Wait);
        assert_eq!(state.advance(Some(1), 0.0, &seeks), SeekAction::Send(0.0));
        assert!(state.observed.is_empty());
        assert_eq!(state.advance(Some(1), 0.0, &seeks), SeekAction::Send(10.0));
        for _ in 0..200 {
            assert_eq!(state.advance(Some(1), 0.0, &seeks), SeekAction::Wait);
        }
        assert_eq!(state.observed, [0.0]);
        assert_eq!(state.advance(Some(1), 10.0, &seeks), SeekAction::Send(5.0));
        assert_eq!(state.advance(Some(1), 5.0, &seeks), SeekAction::Send(5.0));
        assert_eq!(state.observed, [0.0, 10.0, 5.0]);
        assert_eq!(state.advance(Some(1), 5.0, &seeks), SeekAction::Ready);
        assert_eq!(state.observed, seeks);
        assert_eq!(state.advance(Some(1), 7.0, &seeks), SeekAction::Wait);
        assert_eq!(state.advance(Some(1), 5.0, &seeks), SeekAction::Ready);
        assert_eq!(state.advance(Some(2), 3.0, &seeks), SeekAction::Send(0.0));
        assert_eq!(state.initial_time, Some(3.0));
        assert!(state.observed.is_empty());
        assert_eq!(state.advance(None, 0.0, &seeks), SeekAction::Wait);
        assert_eq!(state.initial_time, None);
        assert_eq!(state.pending, None);
        assert_eq!(state.advance(Some(3), 2.0, &[]), SeekAction::Ready);
        assert_eq!(
            state.report(&[]),
            "seek_scope=live-editor-command\nseek_initial_time=Some(2.0)\nseek_requested=[]\nseek_observed=[]\n"
        );
    }

    #[test]
    fn capture_waits_for_final_seek_before_settling_and_times_out_when_unacknowledged() {
        let mut state = CaptureSeeks::default();
        let mut gate = CaptureGate::default();
        let seconds = std::time::Duration::from_secs;
        let mut advance = |time, elapsed| {
            let ready = state.advance(Some(1), time, &[5.0]) == SeekAction::Ready;
            gate.advance(ready.then_some(1), seconds(elapsed))
        };
        for _ in 0..200 {
            assert_eq!(advance(0.0, 1), CaptureAction::Wait);
        }
        for _ in 0..119 {
            assert_eq!(advance(5.0, 2), CaptureAction::Wait);
        }
        assert_eq!(advance(5.0, 2), CaptureAction::Request);
        let mut state = CaptureSeeks::default();
        let mut gate = CaptureGate::default();
        for elapsed in 0..60 {
            assert_ne!(state.advance(Some(1), 0.0, &[5.0]), SeekAction::Ready);
            assert_eq!(gate.advance(None, seconds(elapsed)), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(None, seconds(60)), CaptureAction::Timeout);
    }

    #[test]
    fn capture_seek_sequence_changes_actual_live_editor_transforms() {
        use usd_bevy::{
            editor::{EditorBridge, EditorCommand, EditorPlugin, EditorSession},
            live::{LiveStagePlugin, PrimEntities},
            route::StageTime,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("animated.usda");
        std::fs::write(
            &path,
            r#"#usda 1.0
def Xform "Animated" {
    double3 xformOp:translate.timeSamples = { 0: (0, 0, 0), 5: (5, 0, 0), 10: (10, 0, 0) }
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#,
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LiveStagePlugin, EditorPlugin));
        let bridge = app.world().resource::<EditorBridge>().clone();
        bridge
            .send(EditorCommand::Open(path.to_string_lossy().into_owned()))
            .unwrap();
        app.update();
        let entity = app
            .world()
            .resource::<PrimEntities>()
            .entity("/Animated")
            .unwrap();
        let document = Some(app.world().non_send::<EditorSession>().document_id());
        let seeks = [5.0, 10.0, 5.0, 5.0, 0.0];
        let mut state = CaptureSeeks::default();
        for &expected in &seeks {
            let current = app.world().resource::<StageTime>().current;
            assert_eq!(
                state.advance(document, current, &seeks),
                SeekAction::Send(expected)
            );
            assert_eq!(
                app.world().get::<Transform>(entity).unwrap().translation.x,
                current as f32
            );
            bridge.send(EditorCommand::Seek(expected)).unwrap();
            app.update();
            assert_eq!(app.world().resource::<StageTime>().current, expected);
            assert_eq!(
                app.world().get::<Transform>(entity).unwrap().translation.x,
                expected as f32
            );
        }
        assert_eq!(state.advance(document, 0.0, &seeks), SeekAction::Ready);
        assert_eq!(state.initial_time, Some(0.0));
        assert_eq!(state.observed, seeks);
    }

    #[test]
    fn capture_delay_is_bounded_and_resets_with_document() {
        assert_eq!(CaptureConfig::delay(None).unwrap(), 0);
        for value in [0, 15000, 45000] {
            assert_eq!(
                CaptureConfig::delay(Some(value.to_string())).unwrap(),
                value
            );
        }
        for value in [
            "",
            "-1",
            "45001",
            "NaN",
            "1.5",
            "99999999999999999999999999",
        ] {
            assert!(CaptureConfig::delay(Some(value.into())).is_err());
        }
        let seconds = std::time::Duration::from_secs;
        let mut gate = CaptureGate {
            delay: seconds(15),
            ..Default::default()
        };
        for _ in 0..200 {
            assert_eq!(gate.advance(Some(1), seconds(1)), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(Some(1), seconds(15)), CaptureAction::Wait);
        for _ in 0..200 {
            assert_eq!(gate.advance(Some(2), seconds(15)), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(Some(2), seconds(29)), CaptureAction::Wait);
        assert_eq!(gate.advance(Some(2), seconds(30)), CaptureAction::Request);
        assert_eq!(gate.advance(Some(2), seconds(31)), CaptureAction::Wait);
        let mut gate = CaptureGate {
            delay: seconds(45),
            ..Default::default()
        };
        for _ in 0..200 {
            assert_eq!(gate.advance(Some(1), seconds(30)), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(Some(1), seconds(60)), CaptureAction::Timeout);
    }

    #[test]
    fn capture_options_reject_invalid_times_and_outputs() {
        for time in ["", "NaN", "inf", "-inf", "1e999", "ten"] {
            assert!(CaptureConfig::parse(Some("frame.png".into()), Some(time.into())).is_err());
        }
        for output in ["", "frame.jpg", "frame.png/", "frame"] {
            assert!(CaptureConfig::parse(Some(output.into()), None).is_err());
        }
    }

    #[test]
    fn capture_options_preserve_finite_times_and_optional_capture() {
        assert!(
            CaptureConfig::parse(None, Some("unused".into()))
                .unwrap()
                .is_none()
        );
        assert!(
            CaptureConfig::parse(Some("frame.png".into()), None)
                .unwrap()
                .unwrap()
                .time
                .is_none()
        );
        for time in [-10.5, 0.0, 1.25, 1.0e100] {
            let config = CaptureConfig::parse(Some("a frame.png".into()), Some(time.to_string()))
                .unwrap()
                .unwrap();
            assert_eq!(config.output, "a frame.png");
            assert_eq!(config.time, Some(time));
        }
    }

    #[test]
    fn capture_gate_requires_one_document_for_120_updates() {
        let mut gate = CaptureGate::default();
        let elapsed = std::time::Duration::from_secs(1);
        for _ in 0..200 {
            assert_eq!(gate.advance(None, elapsed), CaptureAction::Wait);
        }
        for _ in 0..119 {
            assert_eq!(gate.advance(Some(1), elapsed), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(None, elapsed), CaptureAction::Wait);
        for _ in 0..119 {
            assert_eq!(gate.advance(Some(1), elapsed), CaptureAction::Wait);
        }
        for _ in 0..119 {
            assert_eq!(gate.advance(Some(2), elapsed), CaptureAction::Wait);
        }
        assert_eq!(gate.advance(Some(2), elapsed), CaptureAction::Request);
        assert_eq!(gate.advance(Some(3), elapsed), CaptureAction::Wait);
    }

    #[test]
    fn capture_gate_times_out_once_even_with_a_document() {
        for document in [None, Some(1)] {
            let mut gate = CaptureGate::default();
            assert_eq!(
                gate.advance(document, std::time::Duration::from_secs(59)),
                CaptureAction::Wait
            );
            assert_eq!(
                gate.advance(document, std::time::Duration::from_secs(60)),
                CaptureAction::Timeout
            );
            assert_eq!(
                gate.advance(document, std::time::Duration::from_secs(61)),
                CaptureAction::Wait
            );
        }
    }

    #[test]
    fn embedded_readback_is_tightly_packed_and_reports_dimensions() {
        let directory =
            std::env::temp_dir().join(format!("usd-embedded-capture-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("viewport.png");
        let mut image = Image::new_target_texture(3, 2, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some((0..24).collect());
        let transform = GlobalTransform::from_translation(Vec3::new(2.0, 3.0, 4.0));
        let projection = bevy::math::proj::perspective_infinite_reverse(0.8, 1.5, 0.1);
        let timing = format!(
            "minimum_ready_delay_ms=15000\n{}",
            crate::capture_metadata::camera_report(&transform, projection)
        );
        save_readback(&image, &output, &timing).unwrap();
        assert_eq!(
            std::fs::read(output.with_extension("rgba")).unwrap(),
            image.data.unwrap()
        );
        let report = std::fs::read_to_string(output.with_extension("capture.txt")).unwrap();
        assert!(report.contains("width=3\nheight=2\n"));
        assert!(report.contains("row_bytes=12\nbytes=24\n"));
        assert!(report.contains("minimum_ready_delay_ms=15000\n"));
        assert!(report.contains("camera_eye=Vec3(2.0, 3.0, 4.0)\n"));
        assert!(report.contains(&format!(
            "camera_clip_from_view_cols={:?}\n",
            projection.to_cols_array()
        )));
        assert!(output.is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn embedded_readback_reports_invalid_output_and_io_errors() {
        let image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        assert!(save_readback(&image, Path::new("invalid.jpg"), "").is_err());
        let directory =
            std::env::temp_dir().join(format!("usd-capture-missing-{}", std::process::id()));
        assert!(save_readback(&image, &directory.join("missing/viewport.png"), "").is_err());
    }
}
