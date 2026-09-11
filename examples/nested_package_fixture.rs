use std::{io::Cursor, path::Path};
use usd_bevy::UsdSource;

fn archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut writer = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries { writer.add_layer(name, bytes)?; }
    Ok(writer.finish()?.into_inner())
}

fn inner_package() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    archive(&[
        ("scenes/model.usda", b"#usda 1.0\n(defaultPrim = \"Model\")\ndef Xform \"Model\" (references = @part.usda@</Part>) {}\n"),
        ("scenes/part.usda", b"#usda 1.0\ndef Xform \"Part\" {\n    def Cube \"Box\" {\n        double size = 2\n    }\n}\n"),
    ])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 { return Err("usage: nested_package_fixture NEW_OUTPUT_DIRECTORY".into()); }
    let output = Path::new(&args[0]);
    std::fs::create_dir(output)?;
    let inner = inner_package()?;
    let mut failures = Vec::new();
    for (name, reference) in [("implicit", "inner.usdz"), ("explicit", "inner.usdz[scenes/model.usda]")] {
        let root = format!("#usda 1.0\n(defaultPrim = \"Root\")\ndef Xform \"Root\" (references = @{reference}@</Model>) {{}}\n");
        let bytes = archive(&[("root.usda", root.as_bytes()), ("inner.usdz", &inner)])?;
        std::fs::write(output.join(format!("{name}.usdz")), &bytes)?;
        let source = UsdSource::snapshot(output.join(format!("virtual-{name}.usdz")), bytes)?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let stage = source.open_stage()?;
            stage.traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |_| {})?;
            let errors = stage.composition_errors();
            if !errors.is_empty() { return Err(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ").into()); }
            if stage.prim("/Root/Box")?.attribute("size").get::<f64>()? != Some(2.0) {
                return Err("nested cube missing or incorrect".into());
            }
            Ok(())
        })();
        match result {
            Ok(()) => println!("{name}: memory-backed nested composition ok"),
            Err(error) => { println!("{name}: {error:#}"); failures.push(name); }
        }
    }
    if !failures.is_empty() { return Err(format!("nested package probes failed: {}", failures.join(", ")).into()); }
    Ok(())
}

#[test]
fn inner_control_resolves_relative_reference_without_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("virtual-inner.usdz");
    let source = UsdSource::snapshot(&path, inner_package().unwrap()).unwrap();
    let stage = source.open_stage().unwrap();
    assert_eq!(stage.prim("/Model/Box").unwrap().attribute("size").get::<f64>().unwrap(), Some(2.0));
    assert!(stage.composition_errors().is_empty());
    assert!(!path.exists());
}
