//! `UsdSkelAnimation` decode types.
//!
//! Holds the time-sampled joint/blend-shape data for a SkelAnimation prim.
//! The values are read straight from openusd's composed time samples (see
//! [`super::skel::read_skel_animation`]); the old USDA text-scrape workaround
//! is gone now that openusd's parser round-trips tuple-valued time samples.

use std::collections::BTreeMap;

/// Decoded `UsdSkelAnimation` — per-joint TRS tracks + blend-shape weights,
/// keyed by timecode.
#[derive(Debug, Clone, Default)]
pub struct ReadSkelAnimText {
    pub prim_name: String,
    pub joints: Vec<String>,
    pub blend_shapes: Vec<String>,
    pub translations: BTreeMap<OrdF64, Vec<[f32; 3]>>,
    pub rotations: BTreeMap<OrdF64, Vec<[f32; 4]>>,
    pub scales: BTreeMap<OrdF64, Vec<[f32; 3]>>,
    pub blend_shape_weights: BTreeMap<OrdF64, Vec<f32>>,
}

/// Wraps `f64` so it can key a `BTreeMap`. USD timecodes are finite, so a
/// total order is safe.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct OrdF64(pub f64);

impl Eq for OrdF64 {}

impl Ord for OrdF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .expect("OrdF64: NaN timecode in SkelAnimation samples")
    }
}
