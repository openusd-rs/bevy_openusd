use openusd::sdf::Value;
use usd_bevy::{UsdSource, editor::EditorEdit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = UsdSource::snapshot("source_edits/model.usda", &b"#usda 1.0\ndef Cube \"Model\" {}\n"[..])?;
    let root = UsdSource::snapshot("source_edits/root.usda", &b"#usda 1.0\n"[..])?;
    let assembly = root.with_references([("/Small", &model, "/Model"), ("/Large", &model, "/Model")])?;
    let customized = assembly.with_edits([
        EditorEdit::Attribute { prim: "/Small".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(1.) },
        EditorEdit::Attribute { prim: "/Large".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(4.) },
    ])?;
    let stage = customized.open_stage()?;
    assert_eq!(stage.prim("/Small")?.attribute("size").get::<f64>()?, Some(1.));
    assert_eq!(stage.prim("/Large")?.attribute("size").get::<f64>()?, Some(4.));
    assert_eq!(assembly.open_stage()?.prim("/Small")?.attribute("size").get::<f64>()?, Some(2.));
    assert_eq!(customized.dependencies().count(), assembly.dependencies().count());
    Ok(())
}

#[test]
fn customized_assembly_keeps_original_snapshot() { main().unwrap(); }
