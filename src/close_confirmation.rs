use eframe::egui::{self, FullOutput, ViewportCommand, ViewportId};
use usd_bevy::editor::EditorBridge;
use std::sync::{Arc, Mutex};

pub fn install(ctx: &egui::Context, bridge: EditorBridge) -> CloseConfirmation {
    let confirmation = CloseConfirmation { bridge, state: Arc::default() };
    ctx.add_plugin(OutputFilter(confirmation.clone()));
    confirmation
}

#[derive(Clone)]
pub struct CloseConfirmation {
    bridge: EditorBridge,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    pending: bool,
    approved: Option<(u64, u64)>,
}

impl CloseConfirmation {
    fn context(&self) -> Option<(u64, u64)> {
        self.bridge.view().ok().map(|view| (view.document.document_id, view.document.revision))
    }

}

impl State {
    fn filter(&mut self, output: &mut FullOutput, current: Option<(u64, u64)>) {
        let approved = self.approved.take();
        let Some(root) = output.viewport_output.get_mut(&ViewportId::ROOT) else { return };
        if !root.commands.contains(&ViewportCommand::Close) { return; }
        if approved.is_some_and(|approved| Some(approved) == current) {
            self.pending = false;
            return;
        }
        root.commands.retain(|command| *command != ViewportCommand::Close);
        root.repaint_delay = std::time::Duration::ZERO;
        self.pending = true;
    }
}

impl CloseConfirmation {
    pub fn show(&self, ctx: &egui::Context) {
        if !self.state.lock().unwrap().pending { return; }
        let context = self.context();
        let mut cancel = false;
        let mut discard = false;
        let response = egui::Modal::new(egui::Id::new("usd.close.confirmation")).show(ctx, |ui| {
            ui.set_max_width(420.);
            ui.heading("Close USD viewer?");
            ui.label("Changes you have not saved will be lost. Cancel to export the layers you need first.");
            if context.is_none() { ui.label("Document state is unavailable; close confirmation is disabled."); }
            ui.horizontal(|ui| {
                cancel = ui.button("Cancel").clicked();
                discard = ui.add_enabled(context.is_some(), egui::Button::new("Discard and close")).clicked();
            });
        });
        let mut state = self.state.lock().unwrap();
        if cancel || response.should_close() {
            state.pending = false;
            state.approved = None;
        } else if discard {
            state.approved = context;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

struct OutputFilter(CloseConfirmation);

impl egui::Plugin for OutputFilter {
    fn debug_name(&self) -> &'static str { "usd-close-confirmation" }

    fn output_hook(&mut self, output: &mut FullOutput) {
        self.0.state.lock().unwrap().filter(output, self.0.context());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> FullOutput {
        let mut output = egui::Context::default().run_ui(egui::RawInput::default(), |_| {});
        output.viewport_output.get_mut(&ViewportId::ROOT).unwrap().commands.extend([
            ViewportCommand::Close, ViewportCommand::Maximized(true), ViewportCommand::Close,
        ]);
        output
    }

    #[test]
    fn modal_pointer_events_and_escape_do_not_reenter_plugin_lock() {
        let ctx = egui::Context::default();
        let confirmation = install(&ctx, EditorBridge::default());
        let frame = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 920.))),
            events, ..Default::default()
        };
        let first = ctx.run_ui(frame(vec![]), |ui| ui.ctx().send_viewport_cmd(ViewportCommand::Close));
        assert!(!first.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
        for position in [egui::pos2(20., 20.), egui::pos2(720., 460.)] {
            let output = ctx.run_ui(frame(vec![egui::Event::PointerMoved(position)]), |ui| confirmation.show(ui.ctx()));
            assert!(!output.shapes.is_empty());
            assert!(confirmation.state.lock().unwrap().pending);
        }
        let output = ctx.run_ui(frame(vec![egui::Event::Key {
            key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
        }]), |ui| confirmation.show(ui.ctx()));
        assert!(!confirmation.state.lock().unwrap().pending);
        assert!(!output.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    }

    #[test]
    fn close_requires_single_use_current_document_approval() {
        let mut guard = State::default();
        let mut blocked = output();
        guard.filter(&mut blocked, Some((1, 2)));
        assert!(guard.pending);
        assert_eq!(blocked.viewport_output[&ViewportId::ROOT].commands, [ViewportCommand::Maximized(true)]);
        for current in [Some((2, 2)), Some((1, 3)), None] {
            guard.approved = Some((1, 2));
            let mut stale = output();
            guard.filter(&mut stale, current);
            assert!(!stale.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
        }
        guard.approved = Some((1, 2));
        let mut accepted = output();
        guard.filter(&mut accepted, Some((1, 2)));
        assert!(accepted.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
        assert!(!guard.pending);
        let mut repeated = output();
        guard.filter(&mut repeated, Some((1, 2)));
        assert!(!repeated.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
        guard.approved = Some((1, 2));
        guard.filter(&mut FullOutput::default(), Some((1, 2)));
        let mut later = output();
        guard.filter(&mut later, Some((1, 2)));
        assert!(!later.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    }
}
