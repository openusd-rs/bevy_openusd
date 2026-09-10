use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::Path;

pub struct CaptureConfig {
    output: String,
    time: Option<f64>,
    delay_ms: u64,
}

impl CaptureConfig {
    fn parse(output: Option<String>, time: Option<String>) -> Result<Option<Self>, String> {
        let Some(output) = output else { return Ok(None); };
        if output.ends_with('/') || (cfg!(windows) && output.ends_with('\\'))
            || Path::new(&output).extension().and_then(|extension| extension.to_str()) != Some("png") {
            return Err("USD_SCREENSHOT must end in .png".into());
        }
        let time = time.map(|value| value.parse::<f64>()
            .ok().filter(|time| time.is_finite())
            .ok_or_else(|| "USD_CAPTURE_TIME must be a finite number".to_owned())).transpose()?;
        Ok(Some(Self { output, time, delay_ms: 0 }))
    }

    fn delay(value: Option<String>) -> Result<u64, String> {
        match value {
            None => Ok(0),
            Some(value) => value.parse::<u64>().ok().filter(|value| *value <= 45_000)
                .ok_or_else(|| "USD_CAPTURE_DELAY_MS must be an integer from 0 to 45000".into()),
        }
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        let read = |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(format!("{name}: {error}")),
        };
        let mut config = Self::parse(read("USD_SCREENSHOT")?, read("USD_CAPTURE_TIME")?)?;
        if let Some(config) = &mut config { config.delay_ms = Self::delay(read("USD_CAPTURE_DELAY_MS")?)?; }
        Ok(config)
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
enum CaptureAction { Wait, Request, Timeout }

impl CaptureGate {
    fn advance(&mut self, document: Option<u64>, elapsed: std::time::Duration) -> CaptureAction {
        if self.finished { return CaptureAction::Wait; }
        if elapsed >= std::time::Duration::from_secs(60) {
            self.finished = true;
            return CaptureAction::Timeout;
        }
        if document != self.document {
            self.frames = 0;
            self.document = document;
            self.ready_since = document.map(|_| elapsed);
        }
        if document.is_none() { return CaptureAction::Wait; }
        self.frames = self.frames.saturating_add(1);
        if self.frames < 120 || elapsed.saturating_sub(self.ready_since.unwrap()) < self.delay { return CaptureAction::Wait; }
        self.finished = true;
        CaptureAction::Request
    }
}

fn save_readback(image: &Image, output: &Path, timing: &str) -> Result<(), String> {
    if output.extension().and_then(|value| value.to_str()) != Some("png") {
        return Err("USD_SCREENSHOT must end in .png".into());
    }
    let rgba = image.clone().try_into_dynamic().map_err(|error| error.to_string())?.to_rgba8();
    let (width, height) = rgba.dimensions();
    std::fs::write(output.with_extension("rgba"), rgba.as_raw()).map_err(|error| error.to_string())?;
    let report = format!("source=embedded-viewer\nwidth={width}\nheight={height}\nlayout=rgba8\nsource_format={:?}\nrow_bytes={}\nbytes={}\n{timing}", image.texture_descriptor.format, u64::from(width) * 4, rgba.len());
    std::fs::write(output.with_extension("capture.txt"), report).map_err(|error| error.to_string())?;
    rgba.save(output).map_err(|error| error.to_string())
}

pub fn configure(app: &mut App) {
    let Some(CaptureConfig { output, time, delay_ms }) = CaptureConfig::from_env().expect("invalid capture configuration") else { return };
    if let Some(current) = time { app.insert_resource(usd_bevy::route::StageTime { current }); }
    let started = std::time::Instant::now();
    app.add_systems(Last, move |mut commands: Commands, cameras: Query<(&Camera, &RenderTarget, &GlobalTransform), With<Camera3d>>,
        session: Option<NonSend<usd_bevy::editor::EditorSession>>, time: Res<usd_bevy::route::StageTime>, mut gate: Local<CaptureGate>| {
        let active = cameras.iter().find(|(camera, _, _)| camera.is_active);
        let document = active.and_then(|_| session.as_ref().map(|session| session.document_id()));
        gate.delay = std::time::Duration::from_millis(delay_ms);
        let elapsed = started.elapsed();
        match gate.advance(document, elapsed) {
            CaptureAction::Timeout => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: timed out waiting for an open document, active camera and capture delay"),
            CaptureAction::Request => {
                let (camera, target, transform) = active.expect("capture gate requires an active camera");
                let output = output.clone();
                let mut timing = format!("minimum_ready_delay_ms={delay_ms}\nready_elapsed_ms={}\nready_updates_at_request={}\ndocument_id_at_request={}\nscene_time_at_request={}\n",
                    elapsed.saturating_sub(gate.ready_since.unwrap()).as_millis(), gate.frames, document.unwrap(), time.current);
                timing.push_str(&crate::capture_metadata::camera_report(transform, camera.clip_from_view()));
                commands.spawn(Screenshot(target.clone())).observe(move |event: On<ScreenshotCaptured>| {
                    match save_readback(&event.image, Path::new(&output), &timing) {
                        Ok(()) => eprintln!("VIEWPORT_CAPTURE_OK {output}"),
                        Err(error) => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: {error}"),
                    }
                });
            }
            CaptureAction::Wait => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::TextureFormat;

    #[test]
    fn capture_delay_is_bounded_and_resets_with_document() {
        assert_eq!(CaptureConfig::delay(None).unwrap(), 0);
        for value in [0, 15000, 45000] { assert_eq!(CaptureConfig::delay(Some(value.to_string())).unwrap(), value); }
        for value in ["", "-1", "45001", "NaN", "1.5", "99999999999999999999999999"] {
            assert!(CaptureConfig::delay(Some(value.into())).is_err());
        }
        let seconds = std::time::Duration::from_secs;
        let mut gate = CaptureGate { delay: seconds(15), ..Default::default() };
        for _ in 0..200 { assert_eq!(gate.advance(Some(1), seconds(1)), CaptureAction::Wait); }
        assert_eq!(gate.advance(Some(1), seconds(15)), CaptureAction::Wait);
        for _ in 0..200 { assert_eq!(gate.advance(Some(2), seconds(15)), CaptureAction::Wait); }
        assert_eq!(gate.advance(Some(2), seconds(29)), CaptureAction::Wait);
        assert_eq!(gate.advance(Some(2), seconds(30)), CaptureAction::Request);
        assert_eq!(gate.advance(Some(2), seconds(31)), CaptureAction::Wait);
        let mut gate = CaptureGate { delay: seconds(45), ..Default::default() };
        for _ in 0..200 { assert_eq!(gate.advance(Some(1), seconds(30)), CaptureAction::Wait); }
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
        assert!(CaptureConfig::parse(None, Some("unused".into())).unwrap().is_none());
        assert!(CaptureConfig::parse(Some("frame.png".into()), None).unwrap().unwrap().time.is_none());
        for time in [-10.5, 0.0, 1.25, 1.0e100] {
            let config = CaptureConfig::parse(Some("a frame.png".into()), Some(time.to_string())).unwrap().unwrap();
            assert_eq!(config.output, "a frame.png");
            assert_eq!(config.time, Some(time));
        }
    }

    #[test]
    fn capture_gate_requires_one_document_for_120_updates() {
        let mut gate = CaptureGate::default();
        let elapsed = std::time::Duration::from_secs(1);
        for _ in 0..200 { assert_eq!(gate.advance(None, elapsed), CaptureAction::Wait); }
        for _ in 0..119 { assert_eq!(gate.advance(Some(1), elapsed), CaptureAction::Wait); }
        assert_eq!(gate.advance(None, elapsed), CaptureAction::Wait);
        for _ in 0..119 { assert_eq!(gate.advance(Some(1), elapsed), CaptureAction::Wait); }
        for _ in 0..119 { assert_eq!(gate.advance(Some(2), elapsed), CaptureAction::Wait); }
        assert_eq!(gate.advance(Some(2), elapsed), CaptureAction::Request);
        assert_eq!(gate.advance(Some(3), elapsed), CaptureAction::Wait);
    }

    #[test]
    fn capture_gate_times_out_once_even_with_a_document() {
        for document in [None, Some(1)] {
            let mut gate = CaptureGate::default();
            assert_eq!(gate.advance(document, std::time::Duration::from_secs(59)), CaptureAction::Wait);
            assert_eq!(gate.advance(document, std::time::Duration::from_secs(60)), CaptureAction::Timeout);
            assert_eq!(gate.advance(document, std::time::Duration::from_secs(61)), CaptureAction::Wait);
        }
    }

    #[test]
    fn embedded_readback_is_tightly_packed_and_reports_dimensions() {
        let directory = std::env::temp_dir().join(format!("usd-embedded-capture-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("viewport.png");
        let mut image = Image::new_target_texture(3, 2, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some((0..24).collect());
        let transform = GlobalTransform::from_translation(Vec3::new(2.0, 3.0, 4.0));
        let projection = Mat4::perspective_infinite_reverse_rh(0.8, 1.5, 0.1);
        let timing = format!("minimum_ready_delay_ms=15000\n{}", crate::capture_metadata::camera_report(&transform, projection));
        save_readback(&image, &output, &timing).unwrap();
        assert_eq!(std::fs::read(output.with_extension("rgba")).unwrap(), image.data.unwrap());
        let report = std::fs::read_to_string(output.with_extension("capture.txt")).unwrap();
        assert!(report.contains("width=3\nheight=2\n"));
        assert!(report.contains("row_bytes=12\nbytes=24\n"));
        assert!(report.contains("minimum_ready_delay_ms=15000\n"));
        assert!(report.contains("camera_eye=Vec3(2.0, 3.0, 4.0)\n"));
        assert!(report.contains(&format!("camera_clip_from_view_cols={:?}\n", projection.to_cols_array())));
        assert!(output.is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn embedded_readback_reports_invalid_output_and_io_errors() {
        let image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        assert!(save_readback(&image, Path::new("invalid.jpg"), "").is_err());
        let directory = std::env::temp_dir().join(format!("usd-capture-missing-{}", std::process::id()));
        assert!(save_readback(&image, &directory.join("missing/viewport.png"), "").is_err());
    }
}
