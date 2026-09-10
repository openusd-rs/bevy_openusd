use std::path::Path;
use bevy::{prelude::Image, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    for (name, bytes) in [("source", [64, 128, 192, 255]), ("reference", [191, 64, 129, 89]), ("end", [255, 0, 0, 26])] {
        let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some(bytes.to_vec());
        image.try_into_dynamic()?.save(directory.join(format!("{name}.png")))?;
    }
    for variant in ["mapped", "reference", "control", "animated", "animated_reference", "interface_animated", "file_interface_animated"] {
        let texture = if variant == "reference" { "reference" } else { "source" };
        let transform = if variant == "mapped" {
            "float4 inputs:scale = (-1, 0.5, 2, 0.25)\n        float4 inputs:bias = (1, 0, -1, 0.1)"
        } else if variant == "animated" {
            "float4 inputs:scale.timeSamples = {0: (-1, 0.5, 2, 0.25), 10: (0, 0, 0, 0)}\n        float4 inputs:bias = (1, 0, -1, 0.1)"
        } else if variant == "interface_animated" {
            "float4 inputs:scale.connect = </Material/Graph.outputs:scale>\n        float4 inputs:bias.connect = </Material.inputs:bias>"
        } else { "" };
        let interface = if variant == "interface_animated" { r#"
    float4 inputs:scale.timeSamples = {0: (-1, 0.5, 2, 0.25), 10: (0, 0, 0, 0)}
    float4 inputs:bias = (1, 0, -1, 0.1)
    def NodeGraph "Graph" {
        float4 outputs:scale.connect = </Material.inputs:scale>
    }
"# } else if variant == "file_interface_animated" { r#"
    asset inputs:image.timeSamples = {0: @reference.png@, 10: @end.png@}
    def NodeGraph "Graph" {
        asset outputs:image.connect = </Material.inputs:image>
    }
"# } else { "" };
        let file = if variant == "animated_reference" {
            "asset inputs:file.timeSamples = {0: @reference.png@, 10: @end.png@}".to_owned()
        } else if variant == "file_interface_animated" {
            "asset inputs:file.connect = </Material/Graph.outputs:image>".to_owned()
        } else { format!("asset inputs:file = @{texture}.png@") };
        let scene = format!(r#"#usda 1.0
( upAxis = "Y" )
def Sphere "Model" {{
    double radius = 1
    rel material:binding = </Material>
}}
def Material "Material" {{
    {interface}
    token outputs:surface.connect = </Material/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0.18, 0.5, 0.75)
        float inputs:roughness.connect = </Material/Tex.outputs:r>
        float inputs:metallic.connect = </Material/Tex.outputs:g>
        float inputs:occlusion.connect = </Material/Tex.outputs:b>
        float inputs:opacity.connect = </Material/Tex.outputs:a>
        token outputs:surface
    }}
    def Shader "Tex" {{
        uniform token info:id = "UsdUVTexture"
        {file}
        token inputs:sourceColorSpace = "raw"
        {transform}
    }}
}}
"#);
        std::fs::write(directory.join(format!("{variant}.usda")), scene)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: scalar_texture_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
}

#[test]
fn scalar_fixture_preserves_files_and_declares_transform() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    assert!(write_fixture(&directory).is_err());
    for (variant, transform) in [("mapped", [-1.0, 1.0]), ("reference", [1.0, 0.0]), ("control", [1.0, 0.0])] {
        let path = directory.join(format!("{variant}.usda"));
        let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
        let read = usd_bevy::read::shade::read_preview_material(&stage, &openusd::sdf::path("/Material").unwrap()).unwrap().unwrap();
        assert_eq!(read.scalar_texture_transform("roughness"), transform);
    }
    for variant in ["animated", "animated_reference", "interface_animated", "file_interface_animated"] {
        let path = directory.join(format!("{variant}.usda"));
        let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
        for time in [0.0, 10.0] {
            let read = usd_bevy::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Material").unwrap(), Some(time)).unwrap().unwrap();
            if matches!(variant, "animated" | "interface_animated") {
                assert_eq!(read.scalar_texture_transform("roughness"), [if time == 0.0 { -1.0 } else { 0.0 }, 1.0]);
            } else {
                assert_eq!(read.scalar_texture_transform("roughness"), [1.0, 0.0]);
                assert!(read.roughness_texture.unwrap().ends_with(if time == 0.0 { "reference.png" } else { "end.png" }));
            }
        }
    }
}
