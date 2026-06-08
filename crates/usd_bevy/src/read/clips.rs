//! Value-clip introspection — deferred until openusd exposes a public
//! `ClipsAPI` (mxpv/openusd PR #118). The data structures stay so the rest
//! of the loader compiles; `read_clips` returns empty until the pin bumps.

use openusd::sdf::Path;
use openusd::usd::Stage;

#[derive(Debug, Clone, Default)]
pub struct ReadClipSet {
    pub name: String,
    pub clip_prim_path: Option<String>,
    pub asset_paths: Vec<String>,
    pub active: Vec<(f64, i64)>,
    pub times: Vec<(f64, f64)>,
    pub manifest_asset_path: Option<String>,
}

/// Deferred: returns no clip sets until openusd's `ClipsAPI` is available.
pub fn read_clips(_stage: &Stage, _prim: &Path) -> anyhow::Result<Vec<ReadClipSet>> {
    Ok(Vec::new())
}
