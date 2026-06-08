//! `UsdUI` — cosmetic outliner/editor metadata, read from the stage via openusd.

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::read_token_or_string;

/// `ui:displayName` — friendly label for the prim tree.
pub fn read_display_name(stage: &Stage, prim: &Path) -> anyhow::Result<Option<String>> {
    read_token_or_string(stage, prim, "ui:displayName")
}

/// `ui:displayGroup` — grouping token.
pub fn read_display_group(stage: &Stage, prim: &Path) -> anyhow::Result<Option<String>> {
    read_token_or_string(stage, prim, "ui:displayGroup")
}
