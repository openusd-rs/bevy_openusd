use std::time::Instant;

#[derive(Debug)]
pub struct OpeningTiming {
    started: Instant,
    requested: Option<Instant>,
    scene_ready: Option<Instant>,
    pipelines_observed: Option<Instant>,
    screenshot_requested: Option<Instant>,
    readback: Option<Instant>,
}

impl OpeningTiming {
    pub fn new(started: Instant) -> Self {
        Self {
            started,
            requested: None,
            scene_ready: None,
            pipelines_observed: None,
            screenshot_requested: None,
            readback: None,
        }
    }

    pub fn request(&mut self, now: Instant) {
        self.requested.get_or_insert(now);
    }

    pub fn scene_ready(&mut self, now: Instant) {
        if self.requested.is_some() {
            self.scene_ready.get_or_insert(now);
        }
    }

    pub fn pipelines_observed(&mut self, now: Instant) {
        if self.scene_ready.is_some() {
            self.pipelines_observed.get_or_insert(now);
        }
    }

    pub fn screenshot_requested(&mut self, now: Instant) {
        if self.pipelines_observed.is_some() {
            self.screenshot_requested.get_or_insert(now);
        }
    }

    pub fn readback(&mut self, now: Instant) {
        if self.screenshot_requested.is_some() {
            self.readback.get_or_insert(now);
        }
    }

    pub fn report(&self) -> String {
        let milliseconds = |start: Option<Instant>, end: Option<Instant>| match start.zip(end) {
            Some((start, end)) => format!("{:.6}", (end - start).as_secs_f64() * 1000.0),
            None => "pending".into(),
        };
        let mut report = format!(
            "opening_scope=native-offscreen-asset-request-to-main-observed-milestones\nopening_pipeline_scope=compiled-cache-after-scene-camera-environment-extraction-not-device-completion-or-first-visible\nopening_readback_scope=includes-settle-and-optional-playback-or-gpu-measurement-excludes-file-writes\nopening_settle_frames=60\nopening_projection_budget=unlimited\nopening_bootstrap_ms={}\n",
            milliseconds(Some(self.started), self.requested),
        );
        for (name, end) in [
            ("scene_ready", self.scene_ready),
            ("pipelines_observed", self.pipelines_observed),
            ("screenshot_requested", self.screenshot_requested),
            ("readback", self.readback),
        ] {
            report.push_str(&format!(
                "opening_request_to_{name}_ms={}\n",
                milliseconds(self.requested, end),
            ));
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn milestones_separate_bootstrap_loading_settle_and_readback() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut timing = OpeningTiming::new(start);
        timing.request(at(100));
        timing.scene_ready(at(230));
        timing.pipelines_observed(at(270));
        timing.screenshot_requested(at(1270));
        timing.readback(at(1320));
        let report = timing.report();
        for line in [
            "opening_bootstrap_ms=100.000000",
            "opening_request_to_scene_ready_ms=130.000000",
            "opening_request_to_pipelines_observed_ms=170.000000",
            "opening_request_to_screenshot_requested_ms=1170.000000",
            "opening_request_to_readback_ms=1220.000000",
        ] {
            assert!(report.lines().any(|actual| actual == line));
        }
        assert!(report.contains("not-device-completion-or-first-visible"));
        assert!(report.contains("includes-settle-and-optional-playback"));
        assert!(report.contains("excludes-file-writes"));
        assert!(report.contains("opening_projection_budget=unlimited"));
        timing.request(at(2000));
        timing.scene_ready(at(2001));
        timing.pipelines_observed(at(2002));
        timing.screenshot_requested(at(2003));
        timing.readback(at(2004));
        assert_eq!(timing.report(), report);
    }

    #[test]
    fn missing_or_out_of_order_milestones_remain_pending() {
        let start = Instant::now();
        let mut timing = OpeningTiming::new(start);
        timing.readback(start);
        timing.screenshot_requested(start);
        timing.pipelines_observed(start);
        timing.scene_ready(start);
        assert_eq!(timing.report().matches("=pending").count(), 5);
        timing.request(start);
        timing.pipelines_observed(start);
        timing.screenshot_requested(start);
        assert_eq!(timing.report().matches("=pending").count(), 4);
        timing.scene_ready(start);
        timing.readback(start);
        assert_eq!(timing.report().matches("=pending").count(), 3);
        timing.pipelines_observed(start);
        timing.readback(start);
        assert_eq!(timing.report().matches("=pending").count(), 2);
        timing.screenshot_requested(start);
        assert_eq!(timing.report().matches("=pending").count(), 1);
        timing.readback(start);
        assert!(!timing.report().contains("pending"));
    }
}
