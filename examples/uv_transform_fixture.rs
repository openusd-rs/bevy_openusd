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
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { return Err("usage: uv_transform_fixture NEW_DIRECTORY".into()); }
    write_fixture(Path::new(&args[0]))
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
    let reference = open("reference.usda");
    for time in [0.0, 10.0] {
        let read = usd_bevy::read::shade::read_preview_material_at(&mapped, &openusd::sdf::path("/Material").unwrap(), Some(time)).unwrap().unwrap();
        let transform = read.uv_transform.unwrap();
        let mesh = usd_bevy::read::geom::read_mesh_at(&reference, &openusd::sdf::path("/Quad").unwrap(), Some(time)).unwrap().unwrap();
        assert!(transform.transform_point2(Vec2::splat(0.25)).abs_diff_eq(Vec2::from(mesh.uvs.unwrap().values[0]), 1e-6));
    }
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
    let handle = app.world().resource::<AssetServer>().load("mapped.usda");
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
