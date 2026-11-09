use eframe::egui;
use std::time::Duration;

/// Returns the repaint interval for 60 Hz updates.
fn periodic_delay() -> Duration {
    Duration::from_secs_f64(1.0 / 60.0)
}

pub fn request(ctx: &egui::Context) {
    ctx.request_repaint_after(periodic_delay());
}

#[cfg(test)]
/// Runs a headless UI pass and clears its texture deltas.
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
    fn periodic_repaint_retains_delay_after_egui_prediction() {
        for predicted in [0., 1. / 60., 1. / 144.] {
            let ctx = egui::Context::default();
            let mut output = egui::FullOutput::default();
            for _ in 0..5 {
                output = run_pass(
                    &ctx,
                    egui::RawInput {
                        predicted_dt: predicted,
                        ..Default::default()
                    },
                    |ui| request(ui.ctx()),
                );
            }
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            assert!(
                delay >= Duration::from_millis(16) && delay <= Duration::from_millis(17),
                "{predicted}: {delay:?}"
            );
        }
    }
}
