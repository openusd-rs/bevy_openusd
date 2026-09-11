use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
use eframe::egui;

struct Probe {
    output: Option<PathBuf>,
    started: Instant,
    requested: bool,
    announced: bool,
    delay: Duration,
}

struct MaraProbe {
    context: egui::Context,
    probe: Probe,
}

impl mara::window::WindowApp for MaraProbe {
    fn new(ctx: mara::window::CreationContext<'_>) -> Self {
        Self {
            context: ctx.__internal_egui_ctx().clone(),
            probe: Probe {
                output: std::env::var_os("USD_HOST_PROBE_SCREENSHOT").map(PathBuf::from),
                started: Instant::now(), requested: false, announced: false,
                delay: capture_delay().expect("validated capture delay"),
            },
        }
    }

    fn update(&mut self, _: &mut mara::host::MaraHostCtx<'_>) {
        let mut ui = egui::Ui::new(
            self.context.clone(),
            egui::Id::new("host-readback-probe"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(self.context.content_rect()),
        );
        self.probe.paint(&mut ui);
    }
}

fn capture_delay() -> Result<Duration, Box<dyn std::error::Error>> {
    let value = match std::env::var("USD_HOST_PROBE_DELAY_MS") {
        Ok(value) => value.parse::<u64>()?,
        Err(std::env::VarError::NotPresent) => 0,
        Err(error) => return Err(error.into()),
    };
    if value > 60_000 {
        return Err("USD_HOST_PROBE_DELAY_MS must be between 0 and 60000".into());
    }
    Ok(Duration::from_millis(value))
}

fn present_mode(value: Option<&str>) -> Result<wgpu::PresentMode, &'static str> {
    match value {
        None | Some("auto-no-vsync") => Ok(wgpu::PresentMode::AutoNoVsync),
        Some("auto-vsync") => Ok(wgpu::PresentMode::AutoVsync),
        _ => Err("USD_HOST_PROBE_PRESENT_MODE must be auto-no-vsync or auto-vsync"),
    }
}

fn write_capture(path: &Path, image: &egui::ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || width.checked_mul(height) != Some(image.pixels.len()) {
        return Err("invalid screenshot dimensions".into());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, u32::try_from(width)?, u32::try_from(height)?);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<_> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.paint(ui);
    }
}

impl Probe {
    fn paint(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        for (index, color) in [egui::Color32::from_rgb(180,40,40),
            egui::Color32::from_rgb(40,150,60), egui::Color32::from_rgb(40,70,180)].into_iter().enumerate() {
            let left = rect.left() + rect.width() * index as f32 / 3.0;
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(left, rect.top()),
                egui::vec2(rect.width() / 3.0, rect.height())), 0, color);
        }
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER,
            "Host GPU readback - no Bevy / USD", egui::FontId::proportional(32.0), egui::Color32::WHITE);
        for event in ui.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.output.take() {
                    match write_capture(&path, &image) {
                        Ok(()) => eprintln!("HOST_READBACK_OK {} {}x{}", path.display(), image.size[0], image.size[1]),
                        Err(error) => eprintln!("HOST_READBACK_ERROR {error}"),
                    }
                }
            }
        }
        if self.output.is_some() && !self.requested && self.started.elapsed() >= self.delay {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
            eprintln!("HOST_READBACK_REQUESTED");
        }
        if self.output.is_some() && self.started.elapsed() >= self.delay + Duration::from_secs(30) {
            self.output = None;
            eprintln!("HOST_READBACK_ERROR screenshot callback timed out");
        }
        ui.ctx().request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
        if !self.announced {
            eprintln!("USD_VIEWER_UI_UPDATED");
            self.announced = true;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let delay = capture_delay()?;
    let mode = std::env::var("USD_HOST_PROBE_PRESENT_MODE");
    if matches!(mode, Err(std::env::VarError::NotUnicode(_))) {
        return Err("USD_HOST_PROBE_PRESENT_MODE must be Unicode".into());
    }
    let present_mode = present_mode(mode.ok().as_deref())?;
    let output = std::env::var_os("USD_HOST_PROBE_SCREENSHOT").map(PathBuf::from);
    if let Some(path) = &output {
        if path.extension().is_none_or(|extension| extension != "png") || std::fs::symlink_metadata(path).is_ok() {
            return Err("USD_HOST_PROBE_SCREENSHOT must be a new .png path".into());
        }
    }
    match std::env::var("USD_HOST_PROBE_RUNNER").as_deref() {
        Ok("mara") => {
            eprintln!("MARA_CAPTURE_PROBE no Bevy app, no USD stage");
            mara::window::run::<MaraProbe>()?;
            return Ok(());
        }
        Ok("eframe") | Err(std::env::VarError::NotPresent) => {}
        _ => return Err("USD_HOST_PROBE_RUNNER must be mara or eframe".into()),
    }
    eprintln!("EFRAME_CAPTURE_PROBE no Mara runner, no Bevy app, no USD stage, present_mode={present_mode:?}");
    eframe::run_native("eframe capture probe", eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration { present_mode, ..Default::default() },
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 920.0]).with_decorations(false),
        ..Default::default()
    }, Box::new(move |_| Ok(Box::new(Probe { output, started: Instant::now(), requested: false, announced: false, delay }))))?;
    Ok(())
}

#[test]
fn readback_request_waits_for_delay_and_is_single_use() {
    let ctx = egui::Context::default();
    let mut probe = Probe {
        output: Some(PathBuf::from("unused.png")),
        started: Instant::now(),
        requested: false,
        announced: true,
        delay: Duration::from_secs(60),
    };
    let capture_count = |output: egui::FullOutput| {
        output.viewport_output[&egui::ViewportId::ROOT].commands.iter()
            .filter(|command| matches!(command, egui::ViewportCommand::Screenshot(_))).count()
    };
    assert_eq!(capture_count(ctx.run_ui(Default::default(), |ui| probe.paint(ui))), 0);
    probe.delay = Duration::ZERO;
    assert_eq!(capture_count(ctx.run_ui(Default::default(), |ui| probe.paint(ui))), 1);
    assert_eq!(capture_count(ctx.run_ui(Default::default(), |ui| probe.paint(ui))), 0);
}

#[test]
fn wgpu_present_mode_is_explicit() {
    assert_eq!(present_mode(None), Ok(wgpu::PresentMode::AutoNoVsync));
    assert_eq!(present_mode(Some("auto-no-vsync")), Ok(wgpu::PresentMode::AutoNoVsync));
    assert_eq!(present_mode(Some("auto-vsync")), Ok(wgpu::PresentMode::AutoVsync));
    for invalid in ["", "false", "fifo", "AutoVsync"] { assert!(present_mode(Some(invalid)).is_err()); }
}

#[test]
fn host_readback_writes_rgba_and_preserves_existing_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capture.png");
    let image = egui::ColorImage::new([2, 1], vec![egui::Color32::RED, egui::Color32::BLUE]);
    write_capture(&path, &image).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgba));
    assert_eq!(pixels, [255,0,0,255,0,0,255,255]);
    assert!(write_capture(&path, &image).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
