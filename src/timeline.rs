use std::sync::{Arc, Mutex};
use mara::ui::mara_core::{layout::{ChildRegion, StackAlign}, pane::PaneBody, pod::Pod, vocab::{Rect, Vec2}};
use usd_bevy::editor::{EditorBridge, EditorCommand, EditorTimeline};

#[derive(Clone, Default)]
pub struct Draft(Arc<Mutex<(String, String)>>);

pub fn show(body: &mut PaneBody, timeline: &EditorTimeline, bridge: &EditorBridge, draft: &Draft) {
    let mut pods = vec![Pod::new("timeline.position").with_readout(
        if timeline.playing { "Playing" } else { "Paused" }, format!("{:.3} / {}", timeline.current, timeline.end),
    )];
    let (start, end, current, playing) = (timeline.start, timeline.end, timeline.current, timeline.playing);
    let transport = bridge.clone();
    pods.push(Pod::new("timeline.transport").with_custom_units(1, move |ui| {
        let row = ui.reserve_space(Vec2::new(ui.available_width(), 28.));
        let width = ((row.width() - 18.) / 4.).max(0.);
        for (index, (label, tooltip, command)) in [
            ("Start", "Go to the first time code", EditorCommand::Seek(start)),
            ("Back", "Previous time code", EditorCommand::Seek(current - 1.)),
            (if playing { "Pause" } else { "Play" }, "Toggle playback", EditorCommand::Play(!playing)),
            ("Next", "Next time code", EditorCommand::Seek(current + 1.)),
        ].into_iter().enumerate() {
            let rect = Rect::from_min_size(row.min + Vec2::new(index as f32 * (width + 6.), 0.), Vec2::new(width, 28.));
            ui.in_region(ChildRegion::top_down(rect, StackAlign::Min), &mut |ui| {
                let response = ui.button(label);
                ui.hover_text(&response, tooltip);
                if response.clicked { super::send(&transport, command.clone()); }
            });
        }
    }));
    let scrub = bridge.clone();
    pods.push(Pod::new("timeline.scrub").with_custom_units(1, move |ui| {
        if start.is_finite() && end.is_finite() && end > start {
            let mut time = current;
            if ui.slider("Time", &mut time, start..=end, 2, "").changed() {
                super::send(&scrub, EditorCommand::Seek(time));
            }
        } else { ui.label("No animated time range"); }
    }));
    let bridge = bridge.clone();
    let draft = draft.clone();
    let seek = Pod::new("timeline.seek").with_custom_units(3, move |ui| {
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
    });
    pods.push(seek);
    body.add_normal("timeline.controls", "Playback", super::toolbar_icons::TIMELINE, pods);
}
