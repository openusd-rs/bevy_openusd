use crate::capture_tools::ScreenshotSlot;
use eframe::egui;
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct Capture {
    path: PathBuf,
    delay: Duration,
    started: Instant,
    armed: bool,
    requested: Option<Instant>,
    slot: ScreenshotSlot,
    complete: bool,
}

pub fn from_env() -> Result<Option<Capture>, Box<dyn std::error::Error>> {
    let path = std::env::var_os("USD_HOST_SCREENSHOT").map(PathBuf::from);
    let delay = match std::env::var("USD_HOST_SCREENSHOT_DELAY_MS") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    configure(path, delay.as_deref())
}

fn configure(
    path: Option<PathBuf>,
    delay: Option<&str>,
) -> Result<Option<Capture>, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        if delay.is_some() {
            return Err("USD_HOST_SCREENSHOT_DELAY_MS requires USD_HOST_SCREENSHOT".into());
        }
        return Ok(None);
    };
    let delay = delay.unwrap_or("20000").parse::<u64>()?;
    if delay > 300_000 {
        return Err("USD_HOST_SCREENSHOT_DELAY_MS must be 0..300000".into());
    }
    if path.extension().is_none_or(|extension| extension != "png")
        || std::fs::symlink_metadata(&path).is_ok()
    {
        return Err("USD_HOST_SCREENSHOT must name a new .png file".into());
    }
    Ok(Some(Capture {
        path,
        delay: Duration::from_millis(delay),
        started: Instant::now(),
        armed: false,
        requested: None,
        slot: ScreenshotSlot::default(),
        complete: false,
    }))
}

impl Capture {
    pub fn update(&mut self, ctx: &egui::Context) {
        self.update_at(ctx, Instant::now());
    }

    fn update_at(&mut self, ctx: &egui::Context, now: Instant) {
        if self.complete {
            return;
        }
        if !self.armed {
            self.started = now;
            self.armed = true;
        }
        let screenshot = self.slot.lock().unwrap().take();
        if self.requested.is_some()
            && let Some(image) = screenshot
        {
            self.complete = true;
            match write_png(&self.path, &image) {
                Ok(()) => eprintln!(
                    "HOST_CAPTURE_OK {} {}x{}",
                    self.path.display(),
                    image.size[0],
                    image.size[1]
                ),
                Err(error) => eprintln!("HOST_CAPTURE_ERROR {}: {error}", self.path.display()),
            }
        } else if let Some(requested) = self.requested {
            if now.saturating_duration_since(requested) >= Duration::from_secs(30) {
                self.complete = true;
                eprintln!(
                    "HOST_CAPTURE_ERROR {}: screenshot callback timed out",
                    self.path.display()
                );
            }
        } else if now.saturating_duration_since(self.started) >= self.delay {
            self.requested = Some(now);
            let sink = self.slot.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |image| {
                    *sink.lock().unwrap() = Some(image);
                }),
            ));
            eprintln!("HOST_CAPTURE_REQUESTED {}", self.path.display());
        }
    }
}

fn write_png(
    path: &std::path::Path,
    image: &egui::ColorImage,
) -> Result<(), Box<dyn std::error::Error>> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || width.checked_mul(height) != Some(image.pixels.len()) {
        return Err("invalid screenshot dimensions".into());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder =
            png::Encoder::new(&mut bytes, u32::try_from(width)?, u32::try_from(height)?);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<_> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
/// egui 0.37 panics when a pass's texture uploads are dropped unapplied. These tests
/// never upload textures, so drop the delta explicitly.
fn run_pass(
    ctx: &egui::Context,
    input: egui::RawInput,
    ui: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, ui);
    output.textures_delta.clear();
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_time_does_not_consume_the_capture_delay() {
        let dir = tempfile::tempdir().unwrap();
        let mut capture = configure(Some(dir.path().join("host.png")), Some("1000"))
            .unwrap()
            .unwrap();
        let ctx = egui::Context::default();
        let first_ui = capture.started + Duration::from_secs(180);
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), first_ui)
        });
        assert_eq!(capture.started, first_ui);
        assert!(capture.requested.is_none());
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), first_ui + Duration::from_millis(999))
        });
        assert!(capture.requested.is_none());
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), first_ui + Duration::from_secs(1))
        });
        assert!(capture.requested.is_some());
    }

    #[test]
    fn configuration_rejects_invalid_or_existing_outputs() {
        assert!(configure(None, None).unwrap().is_none());
        assert!(configure(None, Some("0")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.png");
        for invalid in ["", "-1", "NaN", "300001"] {
            assert!(configure(Some(path.clone()), Some(invalid)).is_err());
        }
        assert!(configure(Some(path.clone()), Some("300000")).is_ok());
        std::fs::write(&path, "preserve").unwrap();
        assert!(configure(Some(path), None).is_err());
        assert!(configure(Some(dir.path().join("host.jpg")), None).is_err());
    }

    #[test]
    fn capture_requests_once_and_writes_the_delivered_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.png");
        let mut capture = configure(Some(path.clone()), Some("1000"))
            .unwrap()
            .unwrap();
        let ctx = egui::Context::default();
        let start = capture.started;
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), start)
        });
        assert!(capture.requested.is_none());
        let now = start + Duration::from_secs(1);
        let output = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), now)
        });
        let callbacks: Vec<_> = output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::ViewportCommand::Screenshot(callback) => Some(callback.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(callbacks.len(), 1);
        let image = std::sync::Arc::new(egui::ColorImage::filled([2, 1], egui::Color32::RED));
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), now)
        });
        assert!(!path.exists());
        callbacks[0].clone().complete(image.clone());
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), now)
        });
        assert!(path.exists());
        assert!(capture.complete);
        let bytes = std::fs::read(&path).unwrap();
        assert!(write_png(&path, &image).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn png_output_preserves_pixels_and_rejects_invalid_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pixels.png");
        let image = egui::ColorImage::new([2, 1], vec![egui::Color32::RED, egui::Color32::BLUE]);
        write_png(&path, &image).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(
            (info.width, info.height, info.color_type),
            (2, 1, png::ColorType::Rgba)
        );
        assert_eq!(pixels, [255, 0, 0, 255, 0, 0, 255, 255]);
        let mut invalid = image;
        invalid.size = [3, 1];
        let bad_path = dir.path().join("invalid.png");
        assert!(write_png(&bad_path, &invalid).is_err());
        assert!(!bad_path.exists());
        invalid.size = [0, 0];
        assert!(write_png(&bad_path, &invalid).is_err());
    }

    #[test]
    fn missing_callback_times_out_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.png");
        let mut capture = configure(Some(path.clone()), Some("0")).unwrap().unwrap();
        let ctx = egui::Context::default();
        let start = capture.started;
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), start)
        });
        let _ = run_pass(&ctx, Default::default(), |ui| {
            capture.update_at(ui.ctx(), start + Duration::from_secs(30))
        });
        assert!(capture.complete);
        assert!(!path.exists());
    }
}
