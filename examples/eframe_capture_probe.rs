use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
use eframe::egui;

struct Probe {
    output: Option<PathBuf>,
    started: Instant,
    requested: bool,
    announced: bool,
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
        let rect = ui.max_rect();
        for (index, color) in [egui::Color32::from_rgb(180,40,40),
            egui::Color32::from_rgb(40,150,60), egui::Color32::from_rgb(40,70,180)].into_iter().enumerate() {
            let left = rect.left() + rect.width() * index as f32 / 3.0;
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(left, rect.top()),
                egui::vec2(rect.width() / 3.0, rect.height())), 0, color);
        }
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER,
            "eframe only - no Mara / Bevy / USD", egui::FontId::proportional(32.0), egui::Color32::WHITE);
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
        if self.output.is_some() && !self.requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
            eprintln!("HOST_READBACK_REQUESTED");
        }
        if self.output.is_some() && self.started.elapsed() >= Duration::from_secs(30) {
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
    let output = std::env::var_os("USD_HOST_PROBE_SCREENSHOT").map(PathBuf::from);
    if let Some(path) = &output {
        if path.extension().is_none_or(|extension| extension != "png") || std::fs::symlink_metadata(path).is_ok() {
            return Err("USD_HOST_PROBE_SCREENSHOT must be a new .png path".into());
        }
    }
    eprintln!("EFRAME_CAPTURE_PROBE no Mara runner, no Bevy app, no USD stage");
    eframe::run_native("eframe capture probe", eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu, vsync: false,
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 920.0]).with_decorations(false),
        ..Default::default()
    }, Box::new(move |_| Ok(Box::new(Probe { output, started: Instant::now(), requested: false, announced: false }))))?;
    Ok(())
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
