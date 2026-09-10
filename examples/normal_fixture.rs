use std::path::Path;
use bevy::{prelude::*, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
    image.data = Some(vec![128, 218, 218, 255]);
    image.try_into_dynamic()?.save(directory.join("normal.png"))?;
    let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
    image.data = Some(vec![255; 4]);
    image.try_into_dynamic()?.save(directory.join("white.png"))?;
    let normal = Vec3::new(1.0, 181.0, 181.0).normalize();
    for variant in ["mapped", "reference", "opposite", "scaled_mapped", "scaled_reference", "scaled_opposite"] {
        let scaled = variant.starts_with("scaled_");
        let reference = !variant.ends_with("mapped");
        let normal = if scaled { Vec3::new(0.0, 0.5, 0.5).normalize() } else { normal };
        let normal = if variant.ends_with("opposite") { normal * Vec3::new(1.0, -1.0, 1.0) } else { normal };
        let (file, scale, bias) = if scaled { ("white.png", "0,0.5,0.5,1", "0,0,0,0") }
            else { ("normal.png", "2,2,2,1", "-1,-1,-1,0") };
        let normals = if reference { format!("normal3f[] normals = [({},{},{})] (interpolation = \"constant\")", normal.x, normal.y, normal.z) }
            else { "normal3f[] normals = [(0,0,1)] (interpolation = \"constant\")".into() };
        let connection = if reference { "" } else { "normal3f inputs:normal.connect = </Material/Texture.outputs:rgb>" };
        let text = format!(r#"#usda 1.0
( upAxis = "Y" )
def Mesh "Quad" {{
    uniform token subdivisionScheme = "none"
    point3f[] points = [(-1,0,0), (1,0,0), (1,2,0), (-1,2,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    texCoord2f[] primvars:st = [(0,0), (1,0), (1,1), (0,1)] (interpolation = "vertex")
    {normals}
    rel material:binding = </Material>
}}
def Material "Material" {{
    token outputs:surface.connect = </Material/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0.5,0.5,0.5)
        float inputs:roughness = 1
        {connection}
        token outputs:surface
    }}
    def Shader "Texture" {{
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @{file}@
        token inputs:sourceColorSpace = "raw"
        float4 inputs:scale = ({scale})
        float4 inputs:bias = ({bias})
        float3 outputs:rgb
    }}
}}
"#);
        std::fs::write(directory.join(format!("{variant}.usda")), text)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: normal_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
}

#[test]
fn fixture_uses_linear_normal_data_and_refuses_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    assert!(write_fixture(&directory).is_err());
    let path = directory.join("mapped.usda");
    let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
    let material = usd_bevy::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Material").unwrap(), None).unwrap().unwrap();
    assert!(!material.texture_srgb("normal"));
    assert_eq!(material.normal_texture_transform, Some([[2.0; 3], [-1.0; 3]]));
    assert!(material.normal_texture.unwrap().ends_with("normal.png"));
    let read = usd_bevy::read::geom::read_mesh_at(&stage, &openusd::sdf::path("/Quad").unwrap(), None).unwrap().unwrap();
    let mesh = usd_bevy::mesh::mesh_from_usd(&read);
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
    let Some(bevy::mesh::VertexAttributeValues::Float32x4(tangents)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT) else { panic!("tangents") };
    let sample = Vec3::new(1.0, 181.0, 181.0).normalize();
    for (normal, tangent) in normals.iter().zip(tangents) {
        let n = Vec3::from(*normal);
        let t = Vec4::from(*tangent);
        let world = (t.truncate() * sample.x + t.w * n.cross(t.truncate()) * sample.y + n * sample.z).normalize();
        assert!(world.abs_diff_eq(sample, 1e-6));
        let flipped = (t.truncate() * sample.x - t.w * n.cross(t.truncate()) * sample.y + n * sample.z).normalize();
        assert!((world - flipped).length() > 1.0);
    }
}

#[test]
fn signed_transform_is_limited_to_preview_surface_texture_normals() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    let text = std::fs::read_to_string(directory.join("scaled_mapped.usda")).unwrap();
    let wrapper = text.replace("inputs:normal.connect = </Material/Texture.outputs:rgb>", "inputs:normal.connect = </Material/Normal.outputs:out>")
        .replace("def Shader \"Texture\"", "def Shader \"Normal\" { uniform token info:id = \"ND_normalmap\"\n float3 inputs:in.connect = </Material/Texture.outputs:rgb>\n float3 outputs:out\n }\n def Shader \"Texture\"");
    for (text, expected) in [
        (text.clone(), Some([[0.0, 0.5, 0.5], [0.0; 3]])),
        (text.replace("UsdPreviewSurface", "ND_standard_surface_surfaceshader"), None),
        (text.replace("UsdUVTexture", "ND_image_color3"), None),
        (wrapper, None),
    ] {
        let stage = usd_bevy::UsdSource::snapshot("normal.usda", text.as_bytes()).unwrap().open_stage().unwrap();
        let read = usd_bevy::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Material").unwrap(), None).unwrap().unwrap();
        assert_eq!(read.normal_texture_transform, expected);
    }
}
