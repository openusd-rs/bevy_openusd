use std::{sync::{Arc, Mutex, mpsc}, time::{Duration, Instant}, path::PathBuf};
use bevy::{prelude::*, camera::RenderTarget, render::view::screenshot::{Screenshot, ScreenshotCaptured}};
use eframe::egui;

pub struct Frame { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

#[derive(Clone)]
struct Request { id: u64, ui: bool, submitted: bool, started: Instant, token: egui::UserData }

#[derive(Default)]
struct Readback {
    next: u64,
    pending: Option<Request>,
    result: Option<Result<Frame, String>>,
}

#[derive(Resource, Clone, Default)]
pub struct CaptureBridge(Arc<Mutex<Readback>>);

impl CaptureBridge {
    pub fn request(&self, include_ui: bool) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if state.pending.is_some() || state.result.is_some() { return Err("A capture is already pending".into()); }
        state.next = state.next.checked_add(1).ok_or("Capture ID exhausted")?;
        state.pending = Some(Request { id: state.next, ui: include_ui, submitted: false, started: Instant::now(),
            token: egui::UserData::new(("usdview-interactive-capture", state.next)) });
        Ok(())
    }

    fn finish(&self, id: u64, result: Result<Frame, String>) {
        let mut state = self.0.lock().unwrap();
        if state.pending.as_ref().is_some_and(|request| request.id == id) {
            state.pending = None;
            state.result = Some(result);
        }
    }

    pub fn take(&self) -> Option<Result<Frame, String>> {
        let mut state = self.0.lock().unwrap();
        if state.pending.as_ref().is_some_and(|r| r.started.elapsed() >= Duration::from_secs(30)) {
            state.pending = None;
            state.result = Some(Err("Capture timed out".into()));
        }
        state.result.take()
    }

    pub fn update_host(&self, ctx: &egui::Context) {
        let request = {
            let mut state = self.0.lock().unwrap();
            let Some(request) = state.pending.as_mut().filter(|r| r.ui) else { return };
            let snapshot = request.clone();
            request.submitted = true;
            snapshot
        };
        let token = request.token;
        if !request.submitted { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(token.clone())); }
        let image = ctx.input(|input| input.events.iter().find_map(|event| match event {
            egui::Event::Screenshot { viewport_id, user_data, image }
                if *viewport_id == egui::ViewportId::ROOT && *user_data == token => Some(image.clone()),
            _ => None,
        }));
        if let Some(image) = image {
            self.finish(request.id, Ok(Frame { width: image.width() as u32, height: image.height() as u32,
                rgba: image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect() }));
        }
    }
}

pub fn configure(app: &mut App, bridge: CaptureBridge) {
    app.insert_resource(bridge).add_systems(Last, request_scene);
}

fn request_scene(bridge: Res<CaptureBridge>, mut commands: Commands,
    cameras: Query<(&Camera, &RenderTarget), With<Camera3d>>) {
    let request = {
        let mut state = bridge.0.lock().unwrap();
        let Some(request) = state.pending.as_mut().filter(|r| !r.ui && !r.submitted) else { return };
        request.submitted = true;
        request.clone()
    };
    let Some((_, target)) = cameras.iter().find(|(camera, _)| camera.is_active) else {
        bridge.finish(request.id, Err("No active scene camera".into())); return;
    };
    let bridge = bridge.clone();
    commands.spawn(Screenshot(target.clone())).observe(move |event: On<ScreenshotCaptured>| {
        let result = event.image.clone().try_into_dynamic().map_err(|e| e.to_string()).map(|image| {
            let rgba = image.to_rgba8();
            Frame { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
        });
        bridge.finish(request.id, result);
    });
}

fn write_png(path: &std::path::Path, frame: &Frame) -> Result<(), String> {
    use std::io::Write;
    if path.extension().and_then(|s| s.to_str()) != Some("png") { return Err("Screenshot path must end in .png".into()); }
    let expected = (frame.width as usize).checked_mul(frame.height as usize).and_then(|n| n.checked_mul(4));
    if frame.width == 0 || frame.height == 0 || expected != Some(frame.rgba.len()) { return Err("Invalid screenshot dimensions".into()); }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().map_err(|e| e.to_string())?.write_image_data(&frame.rgba).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())
}

#[derive(Default)]
struct CaptureState {
    waiting: Option<PathBuf>,
    saving: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    reply: Option<mpsc::Sender<Result<PathBuf, String>>>,
}

#[derive(Clone, Default)]
pub struct CaptureTools { pub bridge: CaptureBridge, state: Arc<Mutex<CaptureState>> }

impl CaptureTools {
    pub fn screenshot(&self, path: PathBuf, include_ui: bool) -> Result<mpsc::Receiver<Result<PathBuf, String>>, String> {
        if !path.is_absolute() { return Err("API output path must be absolute".into()); }
        if path.extension().and_then(|s| s.to_str()) != Some("png") { return Err("Screenshot path must end in .png".into()); }
        if std::fs::symlink_metadata(&path).is_ok() { return Err("Output already exists".into()); }
        let mut state = self.state.lock().unwrap();
        if state.waiting.is_some() || state.saving.is_some() { return Err("A capture is already pending".into()); }
        self.bridge.request(include_ui)?;
        let (tx, rx) = mpsc::channel();
        state.reply = Some(tx); state.waiting = Some(path);
        Ok(rx)
    }

    pub fn busy(&self) -> bool {
        let state = self.state.lock().unwrap(); state.waiting.is_some() || state.saving.is_some()
    }

    pub fn update(&self, ctx: &egui::Context) {
        self.bridge.update_host(ctx);
        let mut state = self.state.lock().unwrap();
        if state.waiting.is_some() && let Some(result) = self.bridge.take() {
            let path = state.waiting.take().unwrap();
            match result {
                Ok(frame) => {
                    let (tx, rx) = mpsc::channel();
                    state.saving = Some(rx);
                    std::thread::spawn(move || { let result = write_png(&path, &frame).map(|_| path); let _ = tx.send(result); });
                }
                Err(error) => { if let Some(reply) = state.reply.take() { let _ = reply.send(Err(error)); } }
            }
        }
        if let Some(rx) = &state.saving {
            match rx.try_recv() {
                Ok(result) => { state.saving = None; if let Some(reply) = state.reply.take() { let _ = reply.send(result); } }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.saving = None;
                    if let Some(reply) = state.reply.take() { let _ = reply.send(Err("Screenshot writer stopped".into())); }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readbacks_reject_overlap_and_ignore_stale_callbacks() {
        let bridge = CaptureBridge::default(); bridge.request(true).unwrap();
        assert!(bridge.request(false).is_err());
        bridge.finish(0, Err("stale".into())); assert!(bridge.take().is_none());
        bridge.finish(1, Err("first".into())); assert_eq!(bridge.take().unwrap().err().unwrap(), "first");
        bridge.request(false).unwrap(); bridge.finish(1, Err("old".into())); assert!(bridge.take().is_none());
    }
    #[test]
    fn screenshots_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("frame.png");
        let frame = Frame { width: 1, height: 1, rgba: vec![255, 0, 0, 255] };
        write_png(&path, &frame).unwrap(); let before = std::fs::read(&path).unwrap();
        assert!(write_png(&path, &frame).is_err()); assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn host_callback_matches_the_original_request_token() {
        let bridge = CaptureBridge::default(); bridge.request(true).unwrap();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| bridge.update_host(ui.ctx()));
        let command = output.viewport_output[&egui::ViewportId::ROOT].commands.iter()
            .find_map(|command| if let egui::ViewportCommand::Screenshot(token) = command { Some(token.clone()) } else { None }).unwrap();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Screenshot { viewport_id: egui::ViewportId::ROOT, user_data: command,
            image: Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::RED)) });
        let _ = ctx.run_ui(input, |ui| bridge.update_host(ui.ctx()));
        let frame = bridge.take().unwrap().unwrap();
        assert_eq!(frame.rgba, [255, 0, 0, 255]);
    }
}
