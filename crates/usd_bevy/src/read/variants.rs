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

/// The variants `set` offers on `prim`, strongest site first.
pub fn variant_options(stage: &Stage, prim: &Path, set: &str) -> Vec<String> {
    stage
        .prim(prim.clone())
        .ok()
        .and_then(|prim| prim.variant_sets().variant_names(set).ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    const VARIANTS: &str = r#"#usda 1.0
def Xform "Tractor" (
    variants = { string wheels = "wide" }
    prepend variantSets = "wheels"
)
{
    variantSet "wheels" = {
        "narrow" { }
        "wide" { }
    }
}
"#;

    #[test]
    fn variant_options_list_every_authored_variant() {
        let stage = crate::UsdSource::new("variants.usda", VARIANTS.as_bytes())
            .unwrap()
            .open_stage()
            .unwrap();
        let prim = openusd::sdf::path("/Tractor").unwrap();
        assert_eq!(variant_set_names(&stage, &prim), vec!["wheels".to_string()]);
        assert_eq!(variant_selection(&stage, &prim, "wheels").as_deref(), Some("wide"));
        assert_eq!(
            variant_options(&stage, &prim, "wheels"),
            vec!["narrow".to_string(), "wide".to_string()]
        );
        assert!(variant_options(&stage, &prim, "colour").is_empty());
    }
}
