use bevy::prelude::*;
use openusd::sdf::{Reference, Value};
use openusd_schemas::geom::{Sphere, SphereSchema};
use usd_bevy::{UsdPlugin, UsdSource, editor::{EditorEdit, EditorSession, SaveMode}, live::{LiveStage, LiveStagePlugin, PrimEntities}};

fn assembly(path: &str) -> Result<EditorEdit, Box<dyn std::error::Error>> {
    let mut edits = vec![EditorEdit::Define { path: path.into(), type_name: "Xform".into() }];
    for (name, x, color) in [("Warm", -1.5, [0.85, 0.18, 0.05]), ("Cool", 1.5, [0.04, 0.4, 0.8])] {
        let prim = format!("{path}/{name}");
        edits.push(EditorEdit::Batch(vec![
            EditorEdit::Define { path: prim.clone(), type_name: String::new() },
            EditorEdit::References { prim: prim.clone(), references: vec![Reference {
                asset_path: "model.usda".into(), prim_path: openusd::sdf::path("/Model")?, ..default()
            }] },
            EditorEdit::TransformMatrix { prim: prim.clone(), matrix: Mat4::from_translation(Vec3::new(x, 1.0, 0.0)).to_cols_array().map(f64::from), reset: false },
            EditorEdit::Attribute { prim, name: "primvars:displayColor".into(), type_name: "color3f[]".into(),
                value: Value::Vec3fVec(vec![openusd::gf::Vec3f::from(color)]) },
        ]));
    }
    Ok(EditorEdit::Batch(edits))
}

fn verify(output: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let model = UsdSource::build("editor_assembly/model.usda", |stage| {
        Sphere::define(stage, "/Model")?.create_radius_attr()?.set(0.85_f64)?;
        Ok(())
    })?;
    let source = UsdSource::build("editor_assembly/root.usda", |_| Ok(()))?.with_dependency(&model)?;
    let stage = source.open_stage()?;
    let before = stage.root_layer().export_to_string()?;
    let mut editor = EditorSession::new(stage.clone());
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin, bevy::asset::AssetPlugin::default(), UsdPlugin, LiveStagePlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    app.insert_non_send(LiveStage::new(stage.clone()));
    app.update();
    editor.edit(assembly("/Assembly")?)?;
    app.update();
    let after = stage.root_layer().export_to_string()?;
    let lookup = |app: &App, path: &str| app.world().resource::<PrimEntities>().entity(path).expect("projected prim");
    for name in ["Warm", "Cool"] {
        let path = format!("/Assembly/{name}");
        assert!(app.world().get::<Mesh3d>(lookup(&app, &path)).is_some());
        assert_eq!(stage.prim(path.as_str())?.attribute("radius").get::<f64>()?, Some(0.85));
    }
    assert!(editor.undo()?);
    app.update();
    assert_eq!(stage.root_layer().export_to_string()?, before);
    assert!(app.world().resource::<PrimEntities>().entity("/Assembly").is_none());
    assert!(!editor.undo()?);
    assert!(editor.redo()?);
    app.update();
    assert_eq!(stage.root_layer().export_to_string()?, after);
    let cool = lookup(&app, "/Assembly/Cool");
    app.world_mut().entity_mut(cool).insert(Name::new("runtime annotation"));
    let matrix = Mat4::from_cols(Vec4::new(1.0, 0.0, 0.0, 0.0), Vec4::new(0.5, 1.5, 0.0, 0.0), Vec4::Z, Vec4::new(1.5, 1.5, 0.0, 1.0));
    editor.edit(EditorEdit::TransformMatrix { prim: "/Assembly/Cool".into(), matrix: matrix.to_cols_array().map(f64::from), reset: false })?;
    app.update();
    assert_eq!(lookup(&app, "/Assembly/Cool"), cool);
    assert_eq!(app.world().get::<Name>(cool).unwrap().as_str(), "runtime annotation");
    assert!(app.world().get::<GlobalTransform>(cool).unwrap().to_matrix().abs_diff_eq(matrix, 1e-5));
    if let Some(output) = output {
        editor.save(output, SaveMode::Flattened)?;
        let reopened = UsdSource::new(output, std::fs::read(output)?)?.open_stage()?;
        for name in ["Warm", "Cool"] {
            let path = openusd::sdf::path(&format!("/Assembly/{name}"))?;
            assert_eq!(reopened.prim(path.clone())?.attribute("radius").get::<f64>()?, Some(0.85));
            assert_eq!(usd_bevy::read::xform::read_transform_stack_f64_at(&stage, &path, None)?,
                usd_bevy::read::xform::read_transform_stack_f64_at(&reopened, &path, None)?);
            assert_eq!(stage.prim(path.clone())?.attribute("primvars:displayColor").get::<Value>()?,
                reopened.prim(path)?.attribute("primvars:displayColor").get::<Value>()?);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() > 1 { return Err("usage: editor_assembly [OUTPUT.usda|OUTPUT.usdc|OUTPUT.usdz]".into()); }
    verify(args.first().map(String::as_str))?;
    println!("Verified typed source assembly, grouped undo/redo, live affine editing{}.", if args.is_empty() { "" } else { " and flattened save/reopen" });
    Ok(())
}

#[test]
fn reusable_assembly_projects_and_saves() {
    let directory = tempfile::tempdir().unwrap();
    for extension in ["usda", "usdc", "usdz"] {
        verify(Some(directory.path().join(format!("assembly.{extension}")).to_str().unwrap())).unwrap();
    }
}
