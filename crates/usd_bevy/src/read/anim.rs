//! Time-sampled xformOp helpers — read authored time samples off the stage
//! via openusd and evaluate them with linear / held interpolation.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::util::read_time_samples;

/// Sample list as authored: ordered `(timeCode, value)` pairs.
pub type Samples = Vec<(f64, Value)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpMode {
    #[default]
    Linear,
    Held,
}

impl InterpMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Self::Linear),
            "held" => Some(Self::Held),
            _ => None,
        }
    }
}

/// Read the `interpolation` metadata on a time-sampled attribute. Defaults
/// to `Linear` when unauthored.
pub fn read_interp_mode(stage: &Stage, prim: &Path, prop: &str) -> anyhow::Result<InterpMode> {
    let raw = stage.prim_at(prim.clone()).attribute(prop).get_metadata::<Value>("interpolation")?;
    if let Some(Value::Token(s)) | Some(Value::String(s)) = raw {
        if let Some(m) = InterpMode::parse(&s) {
            return Ok(m);
        }
    }
    Ok(InterpMode::Linear)
}

pub type Vec3Samples = Vec<(f64, [f32; 3])>;
pub type ScalarSamples = Vec<(f64, f32)>;

#[derive(Debug, Clone)]
pub struct Vec3Track {
    pub samples: Vec3Samples,
    pub mode: InterpMode,
}

#[derive(Debug, Clone)]
pub struct ScalarTrack {
    pub samples: ScalarSamples,
    pub mode: InterpMode,
}

#[derive(Debug, Clone, Default)]
pub struct AnimatedPrim {
    pub translate: Option<Vec3Track>,
    pub rotate_xyz: Option<Vec3Track>,
    pub scale: Option<Vec3Track>,
    pub rotate_x: Option<ScalarTrack>,
    pub rotate_y: Option<ScalarTrack>,
    pub rotate_z: Option<ScalarTrack>,
}

impl AnimatedPrim {
    pub fn is_empty(&self) -> bool {
        self.translate.is_none()
            && self.rotate_xyz.is_none()
            && self.scale.is_none()
            && self.rotate_x.is_none()
            && self.rotate_y.is_none()
            && self.rotate_z.is_none()
    }
}

/// Read any time-sampled xformOp on `prim` and pre-convert the samples.
pub fn read_animated_prim(stage: &Stage, prim: &Path) -> anyhow::Result<Option<AnimatedPrim>> {
    fn vec3_track(stage: &Stage, prim: &Path, prop: &str) -> anyhow::Result<Option<Vec3Track>> {
        let Some(samples) = read_samples(stage, prim, prop)? else {
            return Ok(None);
        };
        Ok(Some(Vec3Track {
            samples: samples_to_vec3(samples),
            mode: read_interp_mode(stage, prim, prop)?,
        }))
    }
    fn scalar_track(stage: &Stage, prim: &Path, prop: &str) -> anyhow::Result<Option<ScalarTrack>> {
        let Some(samples) = read_samples(stage, prim, prop)? else {
            return Ok(None);
        };
        Ok(Some(ScalarTrack {
            samples: samples_to_scalar(samples),
            mode: read_interp_mode(stage, prim, prop)?,
        }))
    }

    let out = AnimatedPrim {
        translate: vec3_track(stage, prim, "xformOp:translate")?,
        rotate_xyz: vec3_track(stage, prim, "xformOp:rotateXYZ")?,
        scale: vec3_track(stage, prim, "xformOp:scale")?,
        rotate_x: scalar_track(stage, prim, "xformOp:rotateX")?,
        rotate_y: scalar_track(stage, prim, "xformOp:rotateY")?,
        rotate_z: scalar_track(stage, prim, "xformOp:rotateZ")?,
    };
    Ok((!out.is_empty()).then_some(out))
}

/// Raw timeSamples on `prim.<prop>`; `None` when none are authored.
pub fn read_samples(stage: &Stage, prim: &Path, prop: &str) -> anyhow::Result<Option<Samples>> {
    let samples = read_time_samples(stage, prim, prop)?;
    Ok((!samples.is_empty()).then_some(samples))
}

fn samples_to_vec3(samples: Samples) -> Vec3Samples {
    samples.into_iter().filter_map(|(t, v)| value_to_vec3f(&v).map(|a| (t, a))).collect()
}

fn samples_to_scalar(samples: Samples) -> ScalarSamples {
    samples.into_iter().filter_map(|(t, v)| value_to_scalar_f32(&v).map(|a| (t, a))).collect()
}

fn value_to_scalar_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Double(d) => Some(*d as f32),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        _ => None,
    }
}

fn value_to_vec3f(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Vec3f(a) => Some([a.x, a.y, a.z]),
        Value::Vec3d(a) => Some([a.x as f32, a.y as f32, a.z as f32]),
        _ => None,
    }
}

pub fn sample_scalar_concrete(samples: &[(f64, f32)], t: f64) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let t_first = samples.first().unwrap().0;
    let t_last = samples.last().unwrap().0;
    if t <= t_first {
        return Some(samples.first().unwrap().1);
    }
    if t >= t_last {
        return Some(samples.last().unwrap().1);
    }
    let idx = samples.binary_search_by(|(tt, _)| tt.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, hi) = match idx {
        Ok(i) => return Some(samples[i].1),
        Err(i) => (i - 1, i),
    };
    let (tl, vl) = &samples[lo];
    let (th, vh) = &samples[hi];
    let u = ((t - tl) / (th - tl)) as f32;
    Some(vl + (vh - vl) * u)
}

pub fn sample_vec3_held(samples: &[(f64, [f32; 3])], t: f64) -> Option<[f32; 3]> {
    if samples.is_empty() {
        return None;
    }
    if t < samples.first().unwrap().0 {
        return Some(samples.first().unwrap().1);
    }
    let mut chosen = samples.first().unwrap().1;
    for (tt, v) in samples {
        if *tt <= t {
            chosen = *v;
        } else {
            break;
        }
    }
    Some(chosen)
}

pub fn sample_scalar_held(samples: &[(f64, f32)], t: f64) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    if t < samples.first().unwrap().0 {
        return Some(samples.first().unwrap().1);
    }
    let mut chosen = samples.first().unwrap().1;
    for (tt, v) in samples {
        if *tt <= t {
            chosen = *v;
        } else {
            break;
        }
    }
    Some(chosen)
}

pub fn sample_vec3_concrete(samples: &[(f64, [f32; 3])], t: f64) -> Option<[f32; 3]> {
    if samples.is_empty() {
        return None;
    }
    let t_first = samples.first().unwrap().0;
    let t_last = samples.last().unwrap().0;
    if t <= t_first {
        return Some(samples.first().unwrap().1);
    }
    if t >= t_last {
        return Some(samples.last().unwrap().1);
    }
    let idx = samples.binary_search_by(|(tt, _)| tt.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, hi) = match idx {
        Ok(i) => return Some(samples[i].1),
        Err(i) => (i - 1, i),
    };
    let (tl, vl) = &samples[lo];
    let (th, vh) = &samples[hi];
    let u = ((t - tl) / (th - tl)) as f32;
    Some([vl[0] + (vh[0] - vl[0]) * u, vl[1] + (vh[1] - vl[1]) * u, vl[2] + (vh[2] - vl[2]) * u])
}

pub fn eval_vec3_track(track: &Vec3Track, t: f64) -> Option<[f32; 3]> {
    match track.mode {
        InterpMode::Linear => sample_vec3_concrete(&track.samples, t),
        InterpMode::Held => sample_vec3_held(&track.samples, t),
    }
}

pub fn eval_scalar_track(track: &ScalarTrack, t: f64) -> Option<f32> {
    match track.mode {
        InterpMode::Linear => sample_scalar_concrete(&track.samples, t),
        InterpMode::Held => sample_scalar_held(&track.samples, t),
    }
}
