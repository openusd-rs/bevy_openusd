use std::{collections::VecDeque, time::{Duration, Instant}};
use eframe::egui::{self, Event, Modifiers, PointerButton, RawInput};

pub fn install(ctx: &egui::Context) -> Result<(), String> {
    let Some(path) = std::env::var_os("USD_UI_REPLAY") else { return Ok(()) };
    let script = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    ctx.add_plugin(Replay { started: Instant::now(), events: parse(&script)? });
    Ok(())
}

struct Replay {
    started: Instant,
    events: VecDeque<(Duration, Event)>,
}

impl Replay {
    fn next_delay(&self, elapsed: Duration) -> Option<Duration> {
        self.events.front().map(|(time, _)| time.saturating_sub(elapsed))
    }
}

impl egui::Plugin for Replay {
    fn debug_name(&self) -> &'static str { "usd-ui-replay" }

    fn input_hook(&mut self, input: &mut RawInput) {
        while self.events.front().is_some_and(|(time, _)| *time <= self.started.elapsed()) {
            let (time, event) = self.events.pop_front().unwrap();
            tracing::info!(target: "usdview::ui_replay", millis = time.as_millis(), "replaying input event");
            input.events.push(event);
        }
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        if let Some(delay) = self.next_delay(self.started.elapsed()) { ui.ctx().request_repaint_after(delay); }
    }
}

fn parse(script: &str) -> Result<VecDeque<(Duration, Event)>, String> {
    let mut events = VecDeque::new();
    let mut previous = Duration::ZERO;
    for (line, input) in script.lines().enumerate() {
        let input = input.trim_start();
        if input.is_empty() || input.starts_with('#') { continue; }
        let error = || format!("invalid UI replay line {}", line + 1);
        let mut words = input.split_whitespace();
        let timestamp = words.next().ok_or_else(error)?;
        let time = Duration::from_millis(timestamp.parse::<u64>().map_err(|_| error())?);
        if time < previous { return Err(error()); }
        previous = time;
        let action = words.next().ok_or_else(error)?;
        let arguments: Vec<_> = words.collect();
        let event = match action {
            "move" | "down" | "up" | "scroll" => {
                if arguments.len() != 2 { return Err(error()); }
                let coordinates = arguments.iter().map(|word| word.parse::<f32>())
                    .collect::<Result<Vec<_>, _>>().map_err(|_| error())?;
                if !coordinates.iter().all(|value| value.is_finite()) { return Err(error()); }
                let [x, y] = [coordinates[0], coordinates[1]];
                match action {
                    "move" => Event::PointerMoved(egui::pos2(x, y)),
                    "scroll" => Event::MouseWheel { unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(x, y), modifiers: Modifiers::NONE },
                    _ => Event::PointerButton { pos: egui::pos2(x, y), button: PointerButton::Primary,
                        pressed: action == "down", modifiers: Modifiers::NONE },
                }
            }
            "text" => Event::Text(input[timestamp.len()..].trim_start().strip_prefix("text").unwrap()
                .strip_prefix(' ').unwrap_or("").to_owned()),
            _ => return Err(error()),
        };
        events.push_back((time, event));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_control_replays_are_valid() {
        for (script, count, last) in [
            (include_str!("../scripts/replays/draft_conflict.replay"), 10, 15000),
            (include_str!("../scripts/replays/draft_keep.replay"), 13, 18000),
            (include_str!("../scripts/replays/draft_reload.replay"), 13, 18000),
            (include_str!("../scripts/replays/draft_apply.replay"), 19, 21000),
            (include_str!("../scripts/replays/payload_provenance.replay"), 3, 12000),
            (include_str!("../scripts/replays/payload_prepend.replay"), 18, 18000),
            (include_str!("../scripts/replays/payload_load_local.replay"), 7, 16000),
            (include_str!("../scripts/replays/payload_load_state.replay"), 8, 21000),
            (include_str!("../scripts/replays/variant_history.replay"), 12, 27000),
            (include_str!("../scripts/replays/reference_provenance.replay"), 3, 12000),
            (include_str!("../scripts/replays/reference_clear.replay"), 6, 16000),
            (include_str!("../scripts/replays/reference_clear_undo.replay"), 9, 22000),
            (include_str!("../scripts/replays/reference_edit.replay"), 18, 23000),
            (include_str!("../scripts/replays/reference_identity_conflict.replay"), 28, 30000),
            (include_str!("../scripts/replays/reference_identity_keep.replay"), 34, 36000),
            (include_str!("../scripts/replays/reference_identity_reload.replay"), 34, 36000),
            (include_str!("../scripts/replays/reference_order.replay"), 11, 22000),
            (include_str!("../scripts/replays/reference_remove.replay"), 11, 22000),
            (include_str!("../scripts/replays/reference_create.replay"), 17, 25000),
            (include_str!("../scripts/replays/reference_bucket_layout.replay"), 8, 18000),
            (include_str!("../scripts/replays/reference_bucket_move.replay"), 11, 24000),
            (include_str!("../scripts/replays/payload_source_conflict.replay"), 10, 22000),
            (include_str!("../scripts/replays/payload_source_keep.replay"), 13, 26000),
            (include_str!("../scripts/replays/payload_source_reload.replay"), 13, 26000),
            (include_str!("../scripts/replays/payload_order_layout.replay"), 11, 22000),
            (include_str!("../scripts/replays/payload_order_apply.replay"), 17, 28000),
            (include_str!("../scripts/replays/payload_order_draft.replay"), 14, 26000),
            (include_str!("../scripts/replays/numeric_array_layout.replay"), 3, 14000),
            (include_str!("../scripts/replays/attribute_filter.replay"), 11, 37000),
            (include_str!("../scripts/replays/layer_muting_layout.replay"), 3, 14000),
            (include_str!("../scripts/replays/layer_muting_toggle.replay"), 12, 37000),
            (include_str!("../scripts/replays/close_cancel.replay"), 8, 28000),
            (include_str!("../scripts/replays/close_discard.replay"), 7, 26200),
            (include_str!("../scripts/replays/numeric_array_apply.replay"), 13, 23000),
            (include_str!("../scripts/replays/numeric_array_sample.replay"), 20, 26000),
            (include_str!("../scripts/replays/numeric_array_sample_clear.replay"), 24, 38000),
            (include_str!("../scripts/replays/payload_replace.replay"), 15, 16000),
            (include_str!("../scripts/replays/payload_clear.replay"), 18, 18000),
            (include_str!("../scripts/replays/payload_invalid.replay"), 7, 14000),
            (include_str!("../scripts/replays/payload_scroll.replay"), 9, 16000),
            (include_str!("../scripts/replays/open_confirmation.replay"), 4, 14000),
            (include_str!("../scripts/replays/open_confirmation_cancel.replay"), 7, 14000),
            (include_str!("../scripts/replays/timeline_play.replay"), 4, 14000),
            (include_str!("../scripts/replays/visibility_toggle.replay"), 4, 14000),
            (include_str!("../scripts/replays/visibility_frame.replay"), 7, 14000),
            (include_str!("../scripts/replays/timeline_pause.replay"), 7, 14000),
            (include_str!("../scripts/replays/timeline_next.replay"), 4, 14000),
            (include_str!("../scripts/replays/timeline_previous.replay"), 11, 15000),
            (include_str!("../scripts/replays/timeline_start.replay"), 11, 15000),
            (include_str!("../scripts/replays/curve_quality_low.replay"), 4, 14000),
            (include_str!("../scripts/replays/curve_quality_high.replay"), 7, 15000),
            (include_str!("../scripts/replays/curve_quality_medium.replay"), 4, 14000),
            (include_str!("../scripts/replays/curve_quality_default.replay"), 7, 15000),
        ] {
            let events = parse(script).unwrap();
            assert_eq!(events.len(), count);
            assert_eq!(events.back().unwrap().0, Duration::from_millis(last));
        }
    }

    #[test]
    fn checked_in_timeline_seek_replay_is_valid() {
        let events = parse(include_str!("../scripts/replays/timeline_seek.replay")).unwrap();
        assert_eq!(events.len(), 8);
        assert_eq!(events.back().unwrap().0, Duration::from_millis(14000));
        assert!(events.iter().any(|(_, event)| matches!(event, Event::Text(text) if text == "10")));
        let invalid = parse(include_str!("../scripts/replays/timeline_invalid.replay")).unwrap();
        assert_eq!(invalid.len(), 8);
        assert!(invalid.iter().any(|(_, event)| matches!(event, Event::Text(text) if text == "NaN")));
    }

    #[test]
    fn checked_in_texture_refresh_replay_is_valid() {
        let events = parse(include_str!("../scripts/replays/refresh_textures.replay")).unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events.back().unwrap().0, Duration::from_millis(12000));
    }

    #[test]
    fn checked_in_sample_history_replay_is_valid() {
        let events = parse(include_str!("../scripts/replays/sample_history.replay")).unwrap();
        assert_eq!(events.len(), 24);
        assert_eq!(events.back().unwrap().0, Duration::from_millis(25200));
    }

    #[test]
    fn checked_in_matrix_replays_are_valid() {
        let load = parse(include_str!("../scripts/replays/matrix_sample_load.replay")).unwrap();
        assert_eq!(load.len(), 3);
        assert_eq!(load.back().unwrap().0, Duration::from_millis(5200));
        let edit = parse(include_str!("../scripts/replays/matrix_sample_edit.replay")).unwrap();
        assert_eq!(edit.len(), 20);
        assert_eq!(edit.back().unwrap().0, Duration::from_millis(16200));
        assert!(edit.iter().any(|(_, event)| matches!(event, Event::Text(text) if text == "2 0 0 1")));
        let undo = parse(include_str!("../scripts/replays/matrix_sample_undo.replay")).unwrap();
        assert_eq!(undo.len(), 26);
        assert_eq!(undo.back().unwrap().0, Duration::from_millis(20200));
    }

    #[test]
    fn replay_dispatches_due_events_once_without_replacing_host_input() {
        use egui::Plugin;
        let mut replay = Replay { started: Instant::now(), events: parse("0 text   hello  \n60000 move 1 2").unwrap() };
        let mut input = RawInput::default();
        input.events.push(Event::Text("host".into()));
        replay.input_hook(&mut input);
        replay.input_hook(&mut input);
        assert_eq!(input.events.len(), 2);
        assert!(matches!(&input.events[1], Event::Text(text) if text == "  hello  "));
        assert_eq!(replay.events.len(), 1);
    }

    #[test]
    fn replay_waits_for_next_event_without_idle_busy_repainting() {
        let mut replay = Replay { started: Instant::now(), events: parse("10000 move 1 2\n12000 text next").unwrap() };
        assert_eq!(replay.next_delay(Duration::ZERO), Some(Duration::from_secs(10)));
        assert_eq!(replay.next_delay(Duration::from_secs(9)), Some(Duration::from_secs(1)));
        assert_eq!(replay.next_delay(Duration::from_secs(11)), Some(Duration::ZERO));
        replay.events.pop_front();
        assert_eq!(replay.next_delay(Duration::from_secs(11)), Some(Duration::from_secs(1)));
        replay.events.clear();
        assert_eq!(replay.next_delay(Duration::from_secs(20)), None);
    }

    #[test]
    fn replay_requires_ordered_times_and_finite_coordinates() {
        for script in ["1 move NaN 0", "1 scroll 0 inf", "2 move 0 0\n1 up 0 0", "1 click 0 0", "1 down 0", "-1 up 0 0"] {
            assert!(parse(script).is_err(), "{script}");
        }
        let events = parse("# local input\n100 move 400 200\n200 scroll 0 -400\n300 down 400 200\n400 up 400 200\n500 text hello world").unwrap();
        assert_eq!(events.len(), 5);
        assert!(matches!(&events[4].1, Event::Text(text) if text == "hello world"));
    }
}
