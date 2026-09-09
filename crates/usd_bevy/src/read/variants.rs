//! Variant-set reads (PLAN Phase 2). A USD variant set is a named switch whose
//! selection changes which opinions compose onto a prim — a capability BSN has
//! no equivalent for. Authoring a selection ([`crate::authoring::set_variant`])
//! is a composition change, so it fires a `resynced` notice and the live loop
//! reconciles the affected subtree automatically.

use openusd::sdf::Path;
use openusd::usd::Stage;

/// The composed `(variant set, selection)` pairs on `prim` — the effective
/// selections (authored, fallback, or default), sorted by set name.
pub fn variant_selections(stage: &Stage, prim: &Path) -> anyhow::Result<Vec<(String, String)>> {
    Ok(stage.prim(prim.clone()).expect("validated USD path").variant_sets().get_all_variant_selections()?)
}

/// The names of the variant sets that currently contribute a selection to
/// `prim`.
pub fn variant_set_names(stage: &Stage, prim: &Path) -> Vec<String> {
    variant_selections(stage, prim)
        .map(|v| v.into_iter().map(|(set, _)| set).collect())
        .unwrap_or_default()
}

/// The current selection for `set` on `prim`, if any.
pub fn variant_selection(stage: &Stage, prim: &Path, set: &str) -> Option<String> {
    variant_selections(stage, prim)
        .ok()?
        .into_iter()
        .find(|(s, _)| s == set)
        .map(|(_, sel)| sel)
}
