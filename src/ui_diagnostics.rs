use std::time::{Duration, Instant};
use eframe::egui::{self, FullOutput, Shape};

pub fn install(ctx: &egui::Context) -> Result<(), String> {
    match std::env::var("USD_UI_DIAGNOSTICS") {
        Err(std::env::VarError::NotPresent) => Ok(()),
        Ok(value) if value == "0" => Ok(()),
        Ok(value) if value == "1" => {
            ctx.add_plugin(Diagnostics { started: Instant::now(), last: None, frames: 0, previous_causes: Vec::new() });
            Ok(())
        }
        _ => Err("USD_UI_DIAGNOSTICS must be 0 or 1".into()),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct OutputStats {
    clipped_shapes: usize, leaf_shapes: usize, text: usize, meshes: usize,
    callbacks: usize, noops: usize, nan_clips: usize,
    texture_sets: usize, texture_frees: usize,
}

fn stats(output: &FullOutput) -> OutputStats {
    let mut stats = OutputStats { clipped_shapes: output.shapes.len(), texture_sets: output.textures_delta.set.len(),
        texture_frees: output.textures_delta.free.len(), ..Default::default() };
    let mut pending = Vec::new();
    for clipped in &output.shapes {
        if [clipped.clip_rect.min.x, clipped.clip_rect.min.y, clipped.clip_rect.max.x, clipped.clip_rect.max.y]
            .iter().any(|value| value.is_nan()) { stats.nan_clips += 1; }
        pending.push(&clipped.shape);
    }
    while let Some(shape) = pending.pop() {
        match shape {
            Shape::Vec(shapes) => { pending.extend(shapes); continue; }
            Shape::Text(_) => stats.text += 1,
            Shape::Mesh(_) => stats.meshes += 1,
            Shape::Callback(_) => stats.callbacks += 1,
            Shape::Noop => stats.noops += 1,
            _ => {}
        }
        stats.leaf_shapes += 1;
    }
    stats
}

struct Diagnostics { started: Instant, last: Option<Instant>, frames: u64, previous_causes: Vec<egui::RepaintCause> }

impl egui::Plugin for Diagnostics {
    fn debug_name(&self) -> &'static str { "usd-ui-diagnostics" }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        if self.last.is_none_or(|last| last.elapsed() >= Duration::from_secs(1)) {
            self.previous_causes = ui.ctx().repaint_causes();
            self.previous_causes.truncate(32);
        }
    }

    fn output_hook(&mut self, output: &mut FullOutput) {
        self.frames = self.frames.saturating_add(1);
        let now = Instant::now();
        if self.last.is_some_and(|last| now.duration_since(last) < Duration::from_secs(1)) { return; }
        self.last = Some(now);
        let repaint_delay_ms = output.viewport_output.get(&egui::ViewportId::ROOT).map(|viewport| viewport.repaint_delay.as_secs_f64()*1000.);
        tracing::info!(target: "usdview::ui_diagnostics", millis = self.started.elapsed().as_millis(),
            frames = self.frames, pixels_per_point = output.pixels_per_point, repaint_delay_ms,
            previous_repaint_causes = ?self.previous_causes, stats = ?stats(output), "egui output before backend");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{epaint::ClippedShape, Plugin};

    #[test]
    fn diagnostic_counts_nested_shapes_without_changing_output() {
        let mut output = FullOutput::default();
        output.shapes.push(ClippedShape { clip_rect: egui::Rect::EVERYTHING,
            shape: Shape::Vec(vec![Shape::Noop, Shape::rect_filled(egui::Rect::from_min_size(egui::Pos2::ZERO,
                egui::vec2(10., 20.)), 0, egui::Color32::RED)]) });
        output.shapes.push(ClippedShape { clip_rect: egui::Rect::from_min_max(egui::pos2(f32::NAN, 0.), egui::pos2(1., 1.)), shape: Shape::Noop });
        output.textures_delta.free.push(egui::TextureId::Managed(9));
        assert_eq!(stats(&output), OutputStats { clipped_shapes: 2, leaf_shapes: 3, noops: 2, nan_clips: 1,
            texture_frees: 1, ..Default::default() });
        let before = format!("{:?}", output.shapes);
        let free = output.textures_delta.free.clone();
        let mut diagnostics = Diagnostics { started: Instant::now(), last: None, frames: 0, previous_causes: Vec::new() };
        diagnostics.output_hook(&mut output);
        let last = diagnostics.last;
        diagnostics.output_hook(&mut output);
        assert_eq!(diagnostics.frames, 2);
        assert_eq!(diagnostics.last, last);
        assert_eq!(format!("{:?}", output.shapes), before);
        assert_eq!(output.textures_delta.free, free);
        assert!(output.textures_delta.set.is_empty());
    }

    #[test]
    fn diagnostic_reads_previous_pass_repaint_causes() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| ui.ctx().request_repaint_after(Duration::from_millis(17)));
        let mut diagnostics = Diagnostics { started: Instant::now(), last: None, frames: 0, previous_causes: Vec::new() };
        let _ = ctx.run_ui(Default::default(), |ui| diagnostics.on_end_pass(ui));
        assert!(diagnostics.previous_causes.iter().any(|cause| cause.file.ends_with("ui_diagnostics.rs")));
        assert!(diagnostics.previous_causes.len() <= 32);
    }
}
