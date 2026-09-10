use std::path::Path;
use bevy::{prelude::Image, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
    image.data = Some(vec![255; 4]);
    image.try_into_dynamic()?.save(directory.join("white.png"))?;
    for semantic in ["emissive", "diffuse", "alpha"] {
        for variant in ["mapped", "reference", "control"] {
            let channel = if semantic == "emissive" { "emissiveColor" } else { "diffuseColor" };
            let color = if variant == "mapped" {
                format!("color3f inputs:{channel}.connect = </Material/Tex.outputs:rgb>")
            } else { format!("color3f inputs:{channel} = {}",
                if variant == "reference" { "(0.375, 0.75, 1.5)" } else { "(1, 1, 1)" }) };
            let diffuse = if semantic == "emissive" { "color3f inputs:diffuseColor = (0,0,0)" } else { "" };
            let opacity = if semantic != "alpha" { "" } else if variant == "mapped" {
                "float inputs:opacity.connect = </Material/Tex.outputs:a>"
            } else { "float inputs:opacity = 0.501953125" };
            std::fs::write(directory.join(format!("{semantic}_{variant}.usda")), format!(r#"#usda 1.0
( upAxis = "Y" )
def Sphere "Model" {{
    double radius = 1
    rel material:binding = </Material>
}}
def Material "Material" {{
    token outputs:surface.connect = </Material/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        {diffuse}
        {color}
        {opacity}
        float inputs:roughness = 1
        token outputs:surface
    }}
    def Shader "Tex" {{
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @white.png@
        token inputs:sourceColorSpace = "raw"
        float4 inputs:scale = (0.25, 0.5, 2, 0.5)
        float4 inputs:bias = (0.125, 0.25, -0.5, 0)
    }}
}}
"#))?;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: color_texture_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
}

#[test]
fn rgb_fixture_declares_hdr_transforms_and_constant_references() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("fixture");
    write_fixture(&directory).unwrap();
    assert!(write_fixture(&directory).is_err());
    for semantic in ["emissive", "diffuse", "alpha"] {
        for variant in ["mapped", "reference", "control"] {
            let path = directory.join(format!("{semantic}_{variant}.usda"));
            let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
            let read = usd_bevy::read::shade::read_preview_material(&stage, &openusd::sdf::path("/Material").unwrap()).unwrap().unwrap();
            let channel = if semantic == "emissive" { "emissive" } else { "diffuse" };
            if variant == "mapped" {
                assert_eq!(read.color_texture_transform(channel), [[0.25,0.5,2.0], [0.125,0.25,-0.5]]);
            } else {
                let color = if semantic == "emissive" { read.emissive_color } else { read.diffuse_color };
                assert_eq!(color, Some(if variant == "reference" { [0.375,0.75,1.5] } else { [1.0; 3] }));
            }
        }
    }
}
