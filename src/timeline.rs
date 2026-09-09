use std::sync::{Arc, Mutex};
use mara::ui::mara_core::{pane::PaneBody, pod::Pod};
use usd_bevy::editor::{EditorBridge, EditorCommand, EditorTimeline};

#[derive(Clone, Default)]
pub struct Draft(Arc<Mutex<(String, String)>>);

pub fn show(body: &mut PaneBody, timeline: &EditorTimeline, bridge: &EditorBridge, draft: &Draft) {
    let mut pods = vec![Pod::new("timeline.position").with_readout("Time code", &format!("{:.3}", timeline.current)),
        Pod::new("timeline.range").with_readout("Loop range", &format!("{}–{}", timeline.start, timeline.end))];
    for (id, label, command) in [
        ("timeline.play", if timeline.playing { "Pause" } else { "Play" }, EditorCommand::Play(!timeline.playing)),
        ("timeline.start", "Go to start", EditorCommand::Seek(timeline.start)),
        ("timeline.back", "Previous time code", EditorCommand::Seek(timeline.current - 1.0)),
        ("timeline.next", "Next time code", EditorCommand::Seek(timeline.current + 1.0)),
    ] {
        let bridge = bridge.clone();
        pods.push(Pod::new(id).with_custom_units(1, move |ui| {
            if ui.button(label).clicked { super::send(&bridge, command); }
        }));
    }
    let bridge = bridge.clone();
    let draft = draft.clone();
    pods.push(Pod::new("timeline.seek").with_custom_units(3, move |ui| {
        let Ok(mut value) = draft.0.lock() else { return };
        ui.text_input(&mut value.0, "USD time code");
        if ui.button("Seek and pause").clicked {
            match value.0.parse::<f64>() {
                Ok(current) if current.is_finite() => {
                    value.1.clear();
                    super::send(&bridge, EditorCommand::Seek(current));
                }
                _ => value.1 = "Enter a finite USD time code".into(),
            }
        }
        if !value.1.is_empty() { ui.label(&value.1); }
    }));
    body.add_normal("timeline.controls", "Playback", "options", pods);
}
