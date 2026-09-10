use std::path::Path;
use bevy::{prelude::*, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    let mut image = Image::new_target_texture(2, 2, TextureFormat::Rgba8UnormSrgb, None);
    image.data = Some(vec![255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,0,255]);
    image.try_into_dynamic()?.save(directory.join("quadrants.png"))?;
    for reference in [false, true] {
        let uv = if reference {
            "texCoord2f[] primvars:st = [(0.25,0.75)] (interpolation = \"constant\")\n    texCoord2f[] primvars:st.timeSamples = {0: [(0.25,0.75)], 10: [(0.375,0.375)]}"
        } else { "texCoord2f[] primvars:st = [(0.25,0.25)] (interpolation = \"constant\")" };
        let transform = if reference { "" } else { r#"
    def Shader "Transform" {
        uniform token info:id = "UsdTransform2d"
        float2 inputs:in.connect = </Material/Linear.outputs:result>
        float2 inputs:translation.timeSamples = {0: (0,0.5), 10: (0.75,0.25)}
        float2 outputs:result
    }
    def Shader "Linear" {
        uniform token info:id = "UsdTransform2d"
        float2 inputs:in.connect = </Material/Reader.outputs:result>
        float2 inputs:scale.timeSamples = {0: (1,1), 10: (0.5,1.5)}
        float inputs:rotation.timeSamples = {0: 0, 10: 90}
        float2 outputs:result
    }
"# };
        let st = if reference { "/Material/Reader.outputs:result" } else { "/Material/Transform.outputs:result" };
        let text = format!(r#"#usda 1.0
( upAxis = "Y"
  startTimeCode = 0
  endTimeCode = 10 )
def Mesh "Quad" {{
    uniform token subdivisionScheme = "none"
    point3f[] points = [(-1,0,0), (1,0,0), (1,2,0), (-1,2,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    {uv}
    rel material:binding = </Material>
}}
def Material "Material" {{
    token outputs:surface.connect = </Material/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Material/Texture.outputs:rgb>
        float inputs:roughness = 1
        token outputs:surface
    }}
    def Shader "Texture" {{
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @quadrants.png@
        token inputs:sourceColorSpace = "sRGB"
        float2 inputs:st.connect = <{st}>
        float3 outputs:rgb
    }}
    def Shader "Reader" {{
        uniform token info:id = "UsdPrimvarReader_float2"
        token inputs:varname = "st"
        float2 outputs:result
    }}
    {transform}
}}
"#);
        std::fs::write(directory.join(if reference { "reference.usda" } else { "mapped.usda" }), text)?;
    }
    let mapped = std::fs::read_to_string(directory.join("mapped.usda"))?;
    let interfaces = mapped.replace("def Material \"Material\" {", r#"def Material "Material" {
    float2 inputs:move.timeSamples = {0: (0,0.5), 10: (0.75,0.25)}
    float2 inputs:size.timeSamples = {0: (1,1), 10: (0.5,1.5)}
    float inputs:angle.timeSamples = {0: 0, 10: 90}"#)
        .replace("float2 inputs:translation.timeSamples = {0: (0,0.5), 10: (0.75,0.25)}", "float2 inputs:translation.connect = </Material.inputs:move>")
        .replace("float2 inputs:scale.timeSamples = {0: (1,1), 10: (0.5,1.5)}", "float2 inputs:scale.connect = </Material.inputs:size>")
        .replace("float inputs:rotation.timeSamples = {0: 0, 10: 90}", "float inputs:rotation.connect = </Material.inputs:angle>");
    std::fs::write(directory.join("interface_mapped.usda"), interfaces)?;
    let samples = [0.0, 2.5, 5.0, 7.5, 10.0].map(|time| {
        let [u, v] = sampled_uv(time);
        format!("{time}: [({u:.9},{v:.9})]")
    }).join(", ");
    let reference = std::fs::read_to_string(directory.join("reference.usda"))?.replace(
        "{0: [(0.25,0.75)], 10: [(0.375,0.375)]}", &format!("{{{samples}}}"));
    std::fs::write(directory.join("sampled_reference.usda"), reference)?;
    let source = mapped
        .replace("float2 inputs:translation.timeSamples = {0: (0,0.5), 10: (0.75,0.25)}",
            "float2 inputs:translation = (0.4,0.1)\n        float2 inputs:scale = (2,3)")
        .replace("float2 inputs:scale.timeSamples = {0: (1,1), 10: (0.5,1.5)}", "float2 inputs:scale = (1,1)")
        .replace("float inputs:rotation.timeSamples = {0: 0, 10: 90}", "float inputs:rotation = 37");
    let coordinates = [[0.0,0.0], [0.2,0.0], [0.2,0.2], [0.0,0.2]];
    for reference in [false, true] {
        let values = coordinates.map(|uv| if reference { shear_uv(uv) } else { uv })
            .map(|[u,v]| format!("({u:.9},{v:.9})")).join(", ");
        let mut text = source.replace(
            "texCoord2f[] primvars:st = [(0.25,0.25)] (interpolation = \"constant\")",
            &format!("texCoord2f[] primvars:st = [{values}] (interpolation = \"vertex\")"));
        if reference {
            text = text.replace("float2 inputs:st.connect = </Material/Transform.outputs:result>",
                "float2 inputs:st.connect = </Material/Reader.outputs:result>");
        }
        std::fs::write(directory.join(if reference { "shear_reference.usda" } else { "shear_mapped.usda" }), text)?;
    }
    let values = coordinates.map(lossy_uv).map(|[u,v]| format!("({u:.9},{v:.9})")).join(", ");
    let text = source.replace(
        "texCoord2f[] primvars:st = [(0.25,0.25)] (interpolation = \"constant\")",
        &format!("texCoord2f[] primvars:st = [{values}] (interpolation = \"vertex\")"))
        .replace("float2 inputs:st.connect = </Material/Transform.outputs:result>",
            "float2 inputs:st.connect = </Material/Reader.outputs:result>");
    std::fs::write(directory.join("shear_lossy.usda"), text)?;
    for (name, pixel) in [("red.png", [255,0,0,255]), ("blue.png", [0,0,255,255])] {
        let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some(pixel.to_vec());
        image.try_into_dynamic()?.save(directory.join(name))?;
    }
    let sampled = std::fs::read_to_string(directory.join("mapped.usda"))?
        .replace("asset inputs:file = @quadrants.png@",
            "asset inputs:file.timeSamples = {0: @red.png@, 10: @blue.png@}")
        .replace("float2 inputs:st.connect = </Material/Transform.outputs:result>",
            "float2 inputs:st.connect = </Material/Reader.outputs:result>");
    std::fs::write(directory.join("file_samples.usda"), sampled)?;
    let reference = std::fs::read_to_string(directory.join("reference.usda"))?
        .replace("10: [(0.375,0.375)]", "10: [(0.25,0.25)]");
    std::fs::write(directory.join("file_reference.usda"), reference)?;
    let mut gray = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
    gray.data = Some(vec![128,128,128,255]);
    gray.try_into_dynamic()?.save(directory.join("gray.png"))?;
    let color = std::fs::read_to_string(directory.join("file_samples.usda"))?
        .replace("asset inputs:file.timeSamples = {0: @red.png@, 10: @blue.png@}", "asset inputs:file = @gray.png@")
        .replace("token inputs:sourceColorSpace = \"sRGB\"", "token inputs:sourceColorSpace.timeSamples = {0: \"raw\", 10: \"sRGB\" }");
    std::fs::write(directory.join("color_space_samples.usda"), color)?;
    Ok(())
}

fn sampled_uv(time: f64) -> [f64; 2] {
    let w = time / 10.0;
    let (sin, cos) = (w * std::f64::consts::FRAC_PI_2).sin_cos();
    let x = 0.25 * (1.0 - 0.5*w);
    let y = 0.25 * (1.0 + 0.5*w);
    [cos*x - sin*y + 0.75*w, sin*x + cos*y + 0.5 - 0.25*w]
}

fn shear_uv([u,v]: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = 37_f32.to_radians().sin_cos();
    [0.4 + 2.0 * (cos*u - sin*v), 0.1 + 3.0 * (sin*u + cos*v)]
}

fn lossy_uv([u,v]: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = 37_f32.to_radians().sin_cos();
    let x = Vec2::new(2.0*cos, 3.0*sin);
    let y = Vec2::new(-2.0*sin, 3.0*cos);
    let (sin, cos) = x.y.atan2(x.x).sin_cos();
    [0.4 + cos*x.length()*u - sin*y.length()*v,
     0.1 + sin*x.length()*u + cos*y.length()*v]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: uv_transform_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
}

#[test]
fn sample_only_texture_fixture_selects_explicit_images() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    let path = directory.join("file_samples.usda");
    let stage = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
    let material = openusd::sdf::path("/Material").unwrap();
    assert!(usd_bevy::read::shade::read_preview_material_at(&stage, &material, None).unwrap().unwrap().diffuse_texture.is_none());
    for (time, name) in [(0.0, "red.png"), (10.0, "blue.png")] {
        let preview = usd_bevy::read::shade::read_preview_material_at(&stage, &material, Some(time)).unwrap().unwrap();
        assert!(Path::new(preview.diffuse_texture.as_ref().unwrap()).ends_with(name));
        assert!(preview.uv_transform.is_none());
    }
}

#[test]
fn sheared_uv_chain_matches_baked_vertex_coordinates() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    let open = |name| {
        let path = directory.join(name);
        usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap()
    };
    let mapped = open("shear_mapped.usda");
    let reference = open("shear_reference.usda");
    let material = openusd::sdf::path("/Material").unwrap();
    let transform = usd_bevy::read::shade::read_preview_material_at(&mapped, &material, None).unwrap().unwrap().uv_transform.unwrap();
    assert!(transform.matrix2.x_axis.dot(transform.matrix2.y_axis).abs() > 1.0);
    assert!(!Vec2::from(shear_uv([0.2,0.2])).abs_diff_eq(Vec2::from(lossy_uv([0.2,0.2])), 0.01));
    assert!(usd_bevy::read::shade::read_preview_material_at(&reference, &material, None).unwrap().unwrap().uv_transform.is_none());
    let mesh_path = openusd::sdf::path("/Quad").unwrap();
    let a = usd_bevy::read::geom::read_mesh_at(&mapped, &mesh_path, None).unwrap().unwrap();
    let b = usd_bevy::read::geom::read_mesh_at(&reference, &mesh_path, None).unwrap().unwrap();
    let actual = a.uvs.unwrap().values;
    let expected = b.uvs.unwrap().values;
    assert_eq!(actual.len(), 4);
    assert_eq!(expected.len(), 4);
    for (uv, expected) in actual.iter().zip(expected) {
        assert!(transform.transform_point2(Vec2::from(*uv)).abs_diff_eq(Vec2::from(expected), 1e-6));
    }
}

#[test]
fn fixture_samples_match_explicit_uv_endpoints() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    assert!(write_fixture(&directory).is_err());
    let open = |name| {
        let path = directory.join(name);
        usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap()
    };
    let mapped = open("mapped.usda");
    let interfaces = open("interface_mapped.usda");
    let reference = open("sampled_reference.usda");
    for time in [0.0, 2.5, 5.0, 7.5, 10.0, 0.0] {
        let read = usd_bevy::read::shade::read_preview_material_at(&mapped, &openusd::sdf::path("/Material").unwrap(), Some(time)).unwrap().unwrap();
        let transform = read.uv_transform.unwrap();
        let interface = usd_bevy::read::shade::read_preview_material_at(&interfaces, &openusd::sdf::path("/Material").unwrap(), Some(time)).unwrap().unwrap();
        assert_eq!(interface.uv_transform, Some(transform));
        let mesh = usd_bevy::read::geom::read_mesh_at(&reference, &openusd::sdf::path("/Quad").unwrap(), Some(time)).unwrap().unwrap();
        assert!(transform.transform_point2(Vec2::splat(0.25)).abs_diff_eq(Vec2::from(mesh.uvs.unwrap().values[0]), 1e-6));
    }
    let linear = usd_bevy::read::geom::read_mesh_at(&open("reference.usda"), &openusd::sdf::path("/Quad").unwrap(), Some(5.0)).unwrap().unwrap();
    assert!(!Vec2::from(linear.uvs.unwrap().values[0]).abs_diff_eq(Vec2::from(sampled_uv(5.0).map(|v| v as f32)), 0.1));
}

#[test]
fn asset_server_instances_animate_uv_chains_independently() {
    use usd_bevy::{UsdAssetPlugin, UsdPlugin, UsdSceneRoot, UsdSceneState, instance::{UsdInstances, UsdInstanceTime}};
    use bevy::math::Affine2;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("fixture");
    write_fixture(&directory).unwrap();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin {
        file_path: directory.to_string_lossy().into_owned(), ..default()
    }, UsdPlugin, UsdAssetPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>();
    app.finish();
    app.cleanup();
    let handle = app.world().resource::<AssetServer>().load("interface_mapped.usda");
    let roots = [0.0, 10.0].map(|current| app.world_mut().spawn((UsdSceneRoot(handle.clone()), UsdInstanceTime { current })).id());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.update();
        if roots.iter().all(|root| app.world().get::<UsdSceneState>(*root) == Some(&UsdSceneState::Ready)) { break; }
        for root in roots {
            if let Some(UsdSceneState::Failed(error)) = app.world().get::<UsdSceneState>(root) { panic!("load failed: {error}"); }
        }
        assert!(std::time::Instant::now() < deadline, "USD asset load timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let lookup = |app: &App, root| app.world().non_send::<UsdInstances>().entity(root, "/Quad").unwrap();
    let entities = roots.map(|root| lookup(&app, root));
    assert_ne!(entities[0], entities[1]);
    let before = roots.map(|root| app.world().non_send::<UsdInstances>().stage(root).unwrap().root_layer().export_to_string().unwrap());
    let meshes = entities.map(|entity| app.world().get::<Mesh3d>(entity).unwrap().0.clone());
    assert_eq!(meshes[0], meshes[1]);
    let children = entities.map(|entity| {
        app.world_mut().entity_mut(entity).insert(Name::new("runtime annotation"));
        app.world_mut().spawn((Name::new("runtime child"), ChildOf(entity))).id()
    });
    let flip = Affine2::from_scale_angle_translation(Vec2::new(1.0,-1.0), 0.0, Vec2::Y);
    for times in [[0.0,10.0], [10.0,0.0], [2.5,7.5], [5.0,5.0], [10.0,10.0]] {
        for (root, current) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = current; }
        app.update();
        let materials = entities.map(|entity| app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0.clone());
        assert_eq!(materials[0] == materials[1], times[0] == times[1]);
        let textures = materials.each_ref().map(|handle| app.world().resource::<Assets<StandardMaterial>>().get(handle).unwrap().base_color_texture.as_ref().unwrap());
        assert_eq!(textures[0], textures[1]);
        for (index, time) in times.into_iter().enumerate() {
            let weight = time as f32 / 10.0;
            let expected = flip * Affine2::from_scale_angle_translation(Vec2::new(1.0-0.5*weight, 1.0+0.5*weight),
                (90.0*weight).to_radians(), Vec2::new(0.75*weight, 0.5-0.25*weight)) * flip;
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&materials[index]).unwrap();
            assert!(material.uv_transform.abs_diff_eq(expected, 1e-6));
            let image = app.world().resource::<Assets<Image>>().get(material.base_color_texture.as_ref().expect("captured texture")).unwrap();
            assert_eq!(image.data.as_ref().unwrap(), &[255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,0,255]);
            assert_eq!(lookup(&app, roots[index]), entities[index]);
            assert_eq!(app.world().get::<Mesh3d>(entities[index]).unwrap().0, meshes[index]);
            assert_eq!(app.world().get::<Name>(entities[index]).unwrap().as_str(), "runtime annotation");
            assert_eq!(app.world().get::<ChildOf>(children[index]).unwrap().parent(), entities[index]);
            assert_eq!(app.world().non_send::<UsdInstances>().stage(roots[index]).unwrap().root_layer().export_to_string().unwrap(), before[index]);
        }
    }
}
