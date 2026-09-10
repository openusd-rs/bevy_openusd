use std::{error::Error, path::Path};
use usd_bevy::{UsdSource, editor::{EditorSession, SaveMode}};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 { return Err("usage: flatten_scene INPUT NEW_OUTPUT".into()); }
    flatten(Path::new(&args[0]), Path::new(&args[1]))?;
    println!("FLATTEN_OK {}", Path::new(&args[1]).display());
    Ok(())
}

fn flatten(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    if std::fs::symlink_metadata(output).is_ok() { return Err("output already exists".into()); }
    let source = UsdSource::new(input.canonicalize()?, std::fs::read(input)?)?;
    let editor = EditorSession::new(source.open_stage()?);
    editor.save(output.to_str().ok_or("output path must be UTF-8")?, SaveMode::Flattened)?;
    Ok(())
}

#[test]
fn flatten_example_preserves_existing_output_and_reports_invalid_input() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.usda");
    let output = directory.path().join("flat.usdz");
    std::fs::write(&input, "#usda 1.0\ndef Cube \"Model\" {\n    double size = 3\n}\n").unwrap();
    flatten(&input, &output).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let stage = UsdSource::new(&output, bytes.clone()).unwrap().open_stage().unwrap();
    assert_eq!(stage.attribute("/Model.size").unwrap().get::<f64>().unwrap(), Some(3.0));
    assert!(flatten(&input, &output).unwrap_err().to_string().contains("already exists"));
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    std::fs::write(&input, "not USD").unwrap();
    let invalid = directory.path().join("invalid.usda");
    assert!(flatten(&input, &invalid).is_err());
    assert!(!invalid.exists());
}
