use usd_bevy::{UsdSource, editor::{EditorSession, SaveMode}};

fn verify(output: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let label = "\"\n}\ndef Xform \"Injected\" {}\n# \\ ' \t";
    let radius = 0.75_f64;
    let snippet = usd_bevy::usd!(r#"#usda 1.0
def Sphere "Model" {
    double radius = ${radius}
    string label = "${label}"
}
"#);
    let directory = tempfile::tempdir()?;
    let model = UsdSource::snapshot(directory.path().join("model.usda"), snippet.text().as_bytes())?;
    let assembly = UsdSource::build(directory.path().join("assembly.usda"), |_| Ok(()))?
        .with_references([("/First", &model, "/Model"), ("/Second", &model, "/Model")])?;
    let stage = assembly.open_stage()?;
    for path in ["/First", "/Second"] {
        assert_eq!(stage.prim(path)?.attribute("label").get::<String>()?.as_deref(), Some(label));
        assert_eq!(stage.prim(path)?.attribute("radius").get::<f64>()?, Some(radius));
    }
    assert!(!stage.prim("/Injected")?.is_valid()?);
    assert_eq!(stage.prim("/")?.children()?.len(), 2);
    EditorSession::new(stage).save(output.to_str().ok_or("non-UTF8 output")?, SaveMode::RootLayer)?;
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
    drop(directory);
    let reopened = UsdSource::new(output, std::fs::read(output)?)?.open_stage()?;
    for path in ["/First", "/Second"] {
        assert_eq!(reopened.prim(path)?.attribute("label").get::<String>()?.as_deref(), Some(label));
        assert_eq!(reopened.prim(path)?.attribute("radius").get::<f64>()?, Some(radius));
    }
    assert_eq!(reopened.prim("/")?.children()?.len(), 2);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() > 1 { return Err("usage: snippet_assembly [NEW_OUTPUT.usdz]".into()); }
    let directory = tempfile::tempdir()?;
    let output = args.first().map(std::path::PathBuf::from).unwrap_or_else(|| directory.path().join("assembly.usdz"));
    if output.try_exists()? || std::fs::symlink_metadata(&output).is_ok() { return Err("output must be new".into()); }
    if output.extension().is_none_or(|extension| extension != "usdz") { return Err("output must end in .usdz".into()); }
    verify(&output)?;
    println!("Verified escaped snippets, reusable composition and source-free package reopen.");
    Ok(())
}

#[test]
fn escaped_snippets_compose_and_reopen_without_sources() {
    let directory = tempfile::tempdir().unwrap();
    verify(&directory.path().join("assembly.usdz")).unwrap();
}
