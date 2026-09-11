use bevy::{diagnostic::DiagnosticsStore, prelude::*};
use std::{collections::BTreeMap, time::Instant};

#[derive(Default)]
struct Samples {
    last: Option<Instant>,
    values: Vec<f64>,
}

#[derive(Resource, Default)]
pub struct GpuTiming {
    count: usize,
    passes: BTreeMap<String, Samples>,
}

impl GpuTiming {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let count = match value {
            None => 0,
            Some(value) => value.parse::<usize>().ok().filter(|value| (1..=600).contains(value))
                .ok_or("USD_CAPTURE_GPU_SAMPLES must be 1..600")?,
        };
        Ok(Self { count, ..Default::default() })
    }

    pub fn enabled(&self) -> bool { self.count != 0 }

    pub fn collect(&mut self, diagnostics: &DiagnosticsStore) -> bool {
        if !self.enabled() { return true; }
        for diagnostic in diagnostics.iter() {
            let path = diagnostic.path().to_string();
            if !path.ends_with("/elapsed_gpu") { continue; }
            let Some(measurement) = diagnostic.measurement() else { continue };
            let samples = self.passes.entry(path).or_default();
            if samples.last == Some(measurement.time) || samples.values.len() >= self.count { continue; }
            samples.last = Some(measurement.time);
            if measurement.value.is_finite() && measurement.value >= 0.0 { samples.values.push(measurement.value); }
        }
        !self.passes.is_empty() && self.passes.values().all(|samples| samples.values.len() == self.count)
    }

    pub fn report(&self) -> String {
        if !self.enabled() { return String::new(); }
        let mut output = format!("gpu_timing_samples_per_pass={}\ngpu_timing_units=ms\ngpu_timing_scope=render-pass-timestamps-not-frame-time\n", self.count);
        for (path, samples) in &self.passes {
            let mut sorted = samples.values.clone();
            sorted.sort_by(f64::total_cmp);
            if sorted.is_empty() { continue; }
            output.push_str(&format!("gpu_timing[{path}]=median:{:.6},p95:{:.6},max:{:.6}\ngpu_timing_raw[{path}]={:?}\n",
                sorted[sorted.len().div_ceil(2)-1], sorted[(sorted.len()*95).div_ceil(100)-1],
                sorted.last().unwrap(), samples.values));
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::diagnostic::{Diagnostic, DiagnosticMeasurement, DiagnosticPath};

    #[test]
    fn gpu_samples_require_fresh_measurements_and_preserve_raw_values() {
        for value in ["0", "601", "-1", "NaN", ""] { assert!(GpuTiming::parse(Some(value)).is_err()); }
        let mut timing = GpuTiming::parse(Some("2")).unwrap();
        let mut store = DiagnosticsStore::default();
        assert!(!timing.collect(&store));
        let path = DiagnosticPath::new("render/pass/elapsed_gpu");
        store.add(Diagnostic::new(path.clone()));
        let now = Instant::now();
        for (offset, value) in [(0, 3.0), (1, 1.0)] {
            store.get_mut(&path).unwrap().add_measurement(DiagnosticMeasurement {
                time: now + std::time::Duration::from_millis(offset), value,
            });
            assert_eq!(timing.collect(&store), offset == 1);
            assert_eq!(timing.collect(&store), offset == 1);
        }
        let report = timing.report();
        assert!(report.contains("median:1.000000,p95:3.000000,max:3.000000"));
        assert!(report.contains("=[3.0, 1.0]"));
    }
}
