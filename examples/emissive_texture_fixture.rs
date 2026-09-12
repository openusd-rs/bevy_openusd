use std::path::Path;
use bevy::{prelude::Image, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    let bytes = [64, 128, 192, 255];
    let end = [192, 64, 128, 255];
    for (name, pixels) in [("emission", bytes), ("end", end)] {
        let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some(pixels.to_vec());
        image.try_into_dynamic()?.save(directory.join(format!("{name}.png")))?;
    }
    for variant in ["textured", "reference", "control", "animated", "animated_reference"] {
        let emission = match variant {
            "textured" | "animated" => "color3f inputs:emissiveColor.connect = </Material/Tex.outputs:rgb>".to_owned(),
            "reference" => format!("color3f inputs:emissiveColor = ({}, {}, {})",
                bytes[0] as f32 / 255.0, bytes[1] as f32 / 255.0, bytes[2] as f32 / 255.0),
            "animated_reference" => format!("color3f inputs:emissiveColor.timeSamples = {{0: ({}, {}, {}), 10: ({}, {}, {})}}",
                bytes[0] as f32 / 255.0, bytes[1] as f32 / 255.0, bytes[2] as f32 / 255.0,
                end[0] as f32 / 255.0, end[1] as f32 / 255.0, end[2] as f32 / 255.0),
            _ => "color3f inputs:emissiveColor = (0, 0, 0)".to_owned(),
        };
        let file = if variant == "animated" {
            "asset inputs:file.timeSamples = {0: @emission.png@, 10: @end.png@}"
        } else { "asset inputs:file = @emission.png@" };
        std::fs::write(directory.join(format!("{variant}.usda")), format!(r#"#usda 1.0
( upAxis = "Y" )
def Sphere "Model" {{
    double radius = 1
    rel material:binding = </Material>
}}
def Material "Material" {{
    token outputs:surface.connect = </Material/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0, 0, 0)
        float inputs:roughness = 1
        {emission}
        token outputs:surface
    }}
    def Shader "Tex" {{
        uniform token info:id = "UsdUVTexture"
        {file}
        token inputs:sourceColorSpace = "raw"
    }}
}}
"#))?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: emissive_texture_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
}

#[test]
fn emission_fixture_distinguishes_texture_from_constant() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("fixture");
    write_fixture(&directory).unwrap();
    assert!(write_fixture(&directory).is_err());
    for variant in ["textured", "reference", "control"] {
        let path = directory.join(format!("{variant}.usda"));
        let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
        let read = usd_bevy::read::shade::read_preview_material(&stage, &openusd::sdf::path("/Material").unwrap()).unwrap().unwrap();
        if variant == "textured" {
            assert!(read.emissive_color.is_none());
            assert!(read.emissive_texture.unwrap().ends_with("emission.png"));
        } else {
            assert!(read.emissive_texture.is_none());
            assert_eq!(read.emissive_color, Some(if variant == "control" { [0.0; 3] }
                else { [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0] }));
        }
    }
    for variant in ["animated", "animated_reference"] {
        let path = directory.join(format!("{variant}.usda"));
        let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
        for time in [0.0, 10.0, 0.0] {
            let read = usd_bevy::read::shade::read_preview_material_at(&stage,
                &openusd::sdf::path("/Material").unwrap(), Some(time)).unwrap().unwrap();
            if variant == "animated" {
                assert!(read.emissive_texture.unwrap().ends_with(if time == 0.0 { "emission.png" } else { "end.png" }));
                assert!(read.emissive_color.is_none());
            } else {
                assert_eq!(read.emissive_color, Some(if time == 0.0 { [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0] }
                    else { [192.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0] }));
            }
        }
    }
}
