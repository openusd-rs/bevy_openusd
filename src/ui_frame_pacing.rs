use std::time::Duration;
use eframe::egui;

fn periodic_delay(predicted_dt: f32) -> Duration {
    Duration::from_secs_f64(1.0 / 60.0) + Duration::try_from_secs_f32(predicted_dt).unwrap_or_default()
}

pub fn request(ctx: &egui::Context) {
    ctx.request_repaint_after(periodic_delay(ctx.input(|input| input.predicted_dt)));
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
                output = ctx.run_ui(egui::RawInput { predicted_dt: predicted, ..Default::default() }, |ui| request(ui.ctx()));
            }
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            assert!(delay >= Duration::from_millis(16) && delay <= Duration::from_millis(17), "{predicted}: {delay:?}");
        }
        for invalid in [f32::NAN, f32::INFINITY, -1.] {
            assert_eq!(periodic_delay(invalid), Duration::from_secs_f64(1. / 60.));
        }
    }
}
