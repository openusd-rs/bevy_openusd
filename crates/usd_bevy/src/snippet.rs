//! [`UsdSnippet`] (PLAN P3) — the runtime value the [`usd!`](macro@crate::usd)
//! macro expands to: a validated `usda` fragment that can be opened as a stage
//! and projected through the routing registry (dogfooding P1).
//!
//! [`UsdSnippet::open_stage`] opens a standalone byte-backed stage.

use std::sync::atomic::{AtomicU64, Ordering};

use openusd::sdf;
use openusd::usd::Stage;

mod sealed {
    pub trait Scalar {}
    macro_rules! scalars {
        ($($ty:ty),*) => { $(impl Scalar for $ty {})* };
    }
    scalars!(bool, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);
    impl<T: Scalar + ?Sized> Scalar for &T {}
}

/// Built-in scalar types accepted in unquoted macro interpolation.
pub trait UsdScalar: sealed::Scalar + std::fmt::Display {}
impl<T: sealed::Scalar + std::fmt::Display + ?Sized> UsdScalar for T {}

#[doc(hidden)]
pub fn scalar_interpolation(value: &(impl UsdScalar + ?Sized)) -> String { value.to_string() }

/// Escapes text interpolated inside a USD quoted string or identifier.
pub fn escape_interpolation(value: &(impl std::fmt::Display + ?Sized)) -> String {
    let mut output = String::new();
    for c in value.to_string().chars() {
        match c {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\'' => output.push_str("\\'"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(c),
        }
    }
    output
}

/// A `usda` text fragment produced by the [`usd!`](macro@crate::usd) macro (or
/// built directly). The macro validates the template structure; [`parse`](UsdSnippet::parse)
/// validates the final runtime text, including dynamic identifiers and values.
#[derive(Debug, Clone)]
pub struct UsdSnippet {
    text: String,
}

impl UsdSnippet {
    /// Wrap raw `usda` text. The macro calls this with validated text; callers
    /// building text by hand should [`parse`](UsdSnippet::parse) to check it.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// The `usda` source.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Parse the snippet into an in-memory [`sdf::Data`] (validation only; does
    /// not compose a stage).
    pub fn parse(&self) -> anyhow::Result<sdf::Data> {
        Ok(openusd::usda::parse(&self.text)?)
    }

    /// Open the snippet as a standalone [`Stage`]. Wrap the result in
    /// `LiveStage` to project it through the routing registry.
    ///
    /// Anchors relative dependencies at the current directory without writing files.
    pub fn open_stage(&self) -> anyhow::Result<Stage> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        self.open_stage_at(format!("usd-snippet-{id}.usda"))
    }

    /// Open with an explicit source filename for relative dependency resolution.
    pub fn open_stage_at(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<Stage> {
        Ok(crate::UsdSource::new(path, self.text.as_bytes())?.open_stage()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_quoted_values_cannot_inject_scene_structure() {
        let text = "\"\n}\ndef Xform \"Injected\" {}\n# \\ ' \t";
        let snippet = crate::usd!("#usda 1.0\ndef Scope \"Safe\" { string text = \"${text}\" }\n");
        let stage = snippet.open_stage().unwrap();
        assert_eq!(stage.prim("/Safe").unwrap().attribute("text").get::<String>().unwrap(), Some(text.into()));
        assert!(!stage.prim("/Injected").unwrap().is_valid().unwrap());
        let single = crate::usd!("#usda 1.0\ndef Scope \"Safe\" { string text = '${text}' }\n");
        assert_eq!(single.open_stage().unwrap().prim("/Safe").unwrap().attribute("text").get::<String>().unwrap(), Some(text.into()));
    }

    #[test]
    fn macro_scalar_interpolation_retains_numbers_and_booleans() {
        let value = -3.25_f64;
        let enabled = true;
        let snippet = crate::usd!("#usda 1.0\ndef Scope \"Safe\" { double value = ${value}\n bool enabled = ${enabled}\n }\n");
        let stage = snippet.open_stage().unwrap();
        assert_eq!(stage.prim("/Safe").unwrap().attribute("value").get::<f64>().unwrap(), Some(value));
        assert_eq!(stage.prim("/Safe").unwrap().attribute("enabled").get::<bool>().unwrap(), Some(enabled));
    }

    #[test]
    fn upstream_composes_bytes_and_flattens_without_files() {
        let layer = sdf::Layer::from_bytes(
            "inline.usda",
            b"#usda 1.0\ndef Xform \"Inline\" {}\n".to_vec(),
        )
        .unwrap();
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .in_memory("composed.usda")
            .unwrap();
        let root = stage.root_layer().identifier().to_string();
        stage
            .insert_layer(&root, 0, layer, sdf::LayerOffset::default())
            .unwrap();
        assert!(stage.prim("/Inline").unwrap().is_valid().unwrap());
        let flattened = stage.flatten().unwrap().export_to_string().unwrap();
        assert!(flattened.contains("Inline"));
        assert!(!flattened.contains("subLayers"));
    }

    #[test]
    fn valid_snippet_parses_and_opens() {
        let s = UsdSnippet::new("#usda 1.0\ndef Xform \"Foo\"\n{\n    custom double x = 2\n}\n");
        assert!(s.parse().is_ok(), "well-formed usda parses");
        let stage = s.open_stage().expect("opens as a stage");
        assert!(
            stage
                .prim(openusd::sdf::path("/Foo").unwrap())
                .expect("validated USD path")
                .is_valid()
                .unwrap_or(false),
            "the prim exists on the opened stage"
        );
    }

    #[test]
    fn malformed_snippet_fails_to_parse() {
        // This is the same validator the `usd!` macro runs at compile time; a
        // broken body here is a compile error there.
        let s = UsdSnippet::new("#usda 1.0\ndef Xform \"Foo\" { this is not usda ]]");
        assert!(s.parse().is_err(), "malformed usda is rejected");
    }

    #[test]
    fn macro_escapes_braces_and_interpolates_multiple() {
        let a = 1_i32;
        let b = 2_i32;
        let s = crate::usd!(
            "#usda 1.0\n\
             def Scope \"S\"\n\
             {\n\
                 int x = ${a}\n\
                 int y = ${b}\n\
             }\n"
        );
        let t = s.text();
        assert!(t.contains("int x = 1"), "first interpolation: {t}");
        assert!(t.contains("int y = 2"), "second interpolation: {t}");
        assert!(
            t.contains('{') && t.contains('}'),
            "literal usda braces survive format escaping: {t}"
        );
        assert!(s.parse().is_ok(), "interpolated result is valid usda");
    }

    #[test]
    fn macro_static_only() {
        let s = crate::usd!("#usda 1.0\ndef Scope \"S\"\n{\n}\n");
        assert!(s.parse().is_ok());
        assert!(s.text().contains("def Scope"));
    }

    #[test]
    fn macro_string_interpolation_inside_quotes() {
        let name = "Widget";
        let s = crate::usd!("#usda 1.0\ndef Xform \"${name}\"\n{\n}\n");
        assert!(
            s.text().contains("\"Widget\""),
            "interpolation inside quotes: {}",
            s.text()
        );
        assert!(s.parse().is_ok());
    }
}
