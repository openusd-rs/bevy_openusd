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

fn textured_inner(binary: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let extension = if binary { "usdc" } else { "usda" };
    let model = format!("#usda 1.0\n(defaultPrim = \"Model\")\ndef Xform \"Model\" (references = @part.{extension}@</Part>) {{}}\n");
    let part = r#"#usda 1.0
def Xform "Part" {
    def Cube "Box" {
        double size = 2
        rel material:binding = </Part/Mat>
    }
    def Material "Mat" {
        token outputs:surface.connect = </Part/Mat/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor.connect = </Part/Mat/Tex.outputs:rgb>
            float inputs:roughness = 1
            token outputs:surface
        }
        def Shader "Tex" {
            uniform token info:id = "UsdUVTexture"
            asset inputs:file = @paint.png@
            token inputs:sourceColorSpace = "sRGB"
            float3 outputs:rgb
        }
    }
}
"#;
    let encode = |text: &str| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if !binary { return Ok(text.as_bytes().to_vec()); }
        let stage = UsdSource::snapshot("virtual-encode.usda", text.as_bytes().to_vec())?.open_stage()?;
        let mut output = Cursor::new(Vec::new());
        openusd::sdf::LayerRegistry::find_by_extension("usdc").unwrap().write(stage.root_layer().data(), &mut output)?;
        let bytes = output.into_inner();
        if !bytes.starts_with(b"PXR-USDC") { return Err("binary layer signature missing".into()); }
        Ok(bytes)
    };
    let mut pixel = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut pixel, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&[0, 255, 255, 255])?;
    }
    archive(&[( &format!("scenes/model.{extension}"), &encode(&model)?),
        (&format!("scenes/part.{extension}"), &encode(part)?), ("scenes/paint.png", &pixel)])
}

fn outer_package(inner: &[u8], reference: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let root = format!("#usda 1.0\n(defaultPrim = \"Root\")\ndef Xform \"Root\" (references = @{reference}@</Model>) {{}}\n");
    archive(&[("root.usda", root.as_bytes()), ("inner.usdz", inner)])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 { return Err("usage: nested_package_fixture NEW_OUTPUT_DIRECTORY".into()); }
    let output = Path::new(&args[0]);
    std::fs::create_dir(output)?;
    std::fs::write(output.join("textured-reference.usda"), r#"#usda 1.0
def Xform "Root" {
    def Cube "Box" {
        double size = 2
        rel material:binding = </Root/Mat>
    }
    def Material "Mat" {
        token outputs:surface.connect = </Root/Mat/Surface.outputs:surface>
        def Shader "Surface" {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0, 1, 1)
            float inputs:roughness = 1
            token outputs:surface
        }
    }
}
"#)?;
    let inner = inner_package()?;
    let mut failures = Vec::new();
    let mut cases = vec![("implicit".to_owned(), outer_package(&inner, "inner.usdz")?),
        ("explicit".to_owned(), outer_package(&inner, "inner.usdz[scenes/model.usda]")?)];
    for (extension, binary) in [("usda", false), ("usdc", true)] {
        let inner = textured_inner(binary)?;
        for (name, reference) in [("implicit", "inner.usdz".to_owned()), ("explicit", format!("inner.usdz[scenes/model.{extension}]"))] {
            cases.push((format!("textured-{extension}-{name}"), outer_package(&inner, &reference)?));
        }
    }
    for (name, bytes) in cases {
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

#[test]
fn editor_decodes_nested_png_materials_from_text_and_binary_layers() {
    use bevy::prelude::*;
    use usd_bevy::{UsdPlugin, live::LiveStagePlugin, editor::{EditorPlugin, EditorBridge, EditorCommand}};
    let directory = tempfile::tempdir().unwrap();
    for (extension, binary) in [("usda", false), ("usdc", true)] {
        let inner = textured_inner(binary).unwrap();
        for (name, reference) in [("implicit", "inner.usdz".to_owned()), ("explicit", format!("inner.usdz[scenes/model.{extension}]"))] {
            let path = directory.path().join(format!("{extension}-{name}.usdz"));
            let bytes = outer_package(&inner, &reference).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default(), UsdPlugin, LiveStagePlugin, EditorPlugin));
            app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
            let bridge = app.world().resource::<EditorBridge>().clone();
            bridge.send(EditorCommand::Open(path.to_str().unwrap().into())).unwrap();
            app.update();
            assert_eq!(bridge.view().unwrap().status, "Ready");
            let materials: Vec<_> = app.world_mut().query::<&MeshMaterial3d<StandardMaterial>>().iter(app.world()).map(|m| m.0.clone()).collect();
            assert_eq!(materials.len(), 1);
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&materials[0]).unwrap();
            let image = app.world().resource::<Assets<Image>>().get(material.base_color_texture.as_ref().unwrap()).unwrap();
            assert_eq!(image.data.as_deref(), Some([0, 255, 255, 255].as_slice()));
            assert_eq!(image.texture_descriptor.format, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
