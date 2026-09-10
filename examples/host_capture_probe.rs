use eframe::egui::{self, Align2, Color32, FontId, LayerId};
use mara::{host::MaraHostCtx, window::{CreationContext, WindowApp}};

struct Probe {
    context: egui::Context,
    announced: bool,
}

impl WindowApp for Probe {
    fn new(ctx: CreationContext<'_>) -> Self {
        eprintln!("HOST_CAPTURE_PROBE no Bevy app, no USD stage");
        Self { context: ctx.__internal_egui_ctx().clone(), announced: false }
    }

    fn update(&mut self, _host: &mut MaraHostCtx<'_>) {
        paint(&self.context);
        self.context.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        if !self.announced {
            eprintln!("USD_VIEWER_UI_UPDATED");
            self.announced = true;
        }
    }
}

fn paint(ctx: &egui::Context) {
    let painter = ctx.layer_painter(LayerId::background());
    let rect = ctx.content_rect();
    for (index, color) in [Color32::from_rgb(180, 40, 40), Color32::from_rgb(40, 150, 60),
        Color32::from_rgb(40, 70, 180)].into_iter().enumerate() {
        let left = rect.left() + rect.width() * index as f32 / 3.0;
        let right = rect.left() + rect.width() * (index + 1) as f32 / 3.0;
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom())), 0, color);
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, "Mara host only - no Bevy / USD",
        FontId::proportional(32.0), Color32::WHITE);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    mara::window::run::<Probe>()
}

#[test]
fn host_probe_paints_without_bevy_or_usd_initialization() {
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO, egui::vec2(1440.0, 920.0))), ..Default::default() }, |ui| paint(ui.ctx()));
    let rectangles: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
        egui::Shape::Rect(rect) => Some((rect.rect, rect.fill)),
        _ => None,
    }).collect();
    assert_eq!(rectangles.len(), 3);
    for (index, color) in [Color32::from_rgb(180, 40, 40), Color32::from_rgb(40, 150, 60),
        Color32::from_rgb(40, 70, 180)].into_iter().enumerate() {
        assert_eq!(rectangles[index], (egui::Rect::from_min_size(
            egui::pos2(index as f32 * 480.0, 0.0), egui::vec2(480.0, 920.0)), color));
    }
    assert!(output.shapes.iter().any(|shape| matches!(shape.shape, egui::Shape::Text(_))));
}
