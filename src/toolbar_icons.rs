pub const OUTLINER: &str = "cube-tree";
pub const PROPERTIES: &str = "options";
pub const TIMELINE: &str = "clock";
pub const LIGHTING: &str = "weather-sunny";
pub const RENDERING: &str = "settings";
pub const OPEN: &str = "folder-open";
pub const SAVE_ROOT: &str = "save";
pub const SAVE_LAYER: &str = "save-edit";
pub const EXPORT: &str = "arrow-export";
pub const UNDO: &str = "arrow-undo";
pub const REDO: &str = "arrow-redo";
pub const REFRESH_TEXTURES: &str = "image-arrow-counterclockwise";
pub const FRAME: &str = "scan";
pub const CAMERA_PLAN: &str = "camera";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_icons_resolve_to_distinct_glyphs() {
        let mut glyphs = std::collections::HashSet::new();
        for name in [
            OUTLINER, PROPERTIES, TIMELINE, LIGHTING, RENDERING, OPEN,
            SAVE_ROOT, SAVE_LAYER, EXPORT, UNDO, REDO, REFRESH_TEXTURES, FRAME, CAMERA_PLAN,
        ] {
            let glyph = mara::ui::mara_core::icons::icon_glyph(name)
                .unwrap_or_else(|| panic!("missing icon: {name}"));
            assert!(glyphs.insert(glyph), "duplicate toolbar glyph: {name}");
        }
    }
}
