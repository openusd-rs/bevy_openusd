//! Material route (PLAN P4): a gprim's bound `UsdShade` Material →
//! [`StandardMaterial`].
//!
//! Runs after the mesh route, replacing the placeholder material the mesh route
//! attaches. Reads the `material:binding` and decodes the bound
//! `UsdPreviewSurface` (and Omni/MaterialX equivalents) via [`read::shade`].
//! Texture channels use snapshot image handles or the AssetServer.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};
use crate::read::shade::{ReadPreviewMaterial, read_material_binding, read_preview_material_at};

/// Maps a bound Material → the entity's [`MeshMaterial3d`].
pub struct MaterialRoute;

#[derive(Component, Debug)]
pub struct UsdMaterialWarning(pub String);

pub(crate) fn warn_geometry_inputs(mesh: &Mesh, material: &StandardMaterial, warnings: &mut Vec<String>) {
    for (channel, attribute, label) in [
        (bevy::mesh::UvChannel::Uv0, Mesh::ATTRIBUTE_UV_0, "UV0"),
        (bevy::mesh::UvChannel::Uv1, Mesh::ATTRIBUTE_UV_1, "UV1"),
    ] {
        if mesh.attribute(attribute).is_some() { continue; }
        let textures = [
            ("base color", material.base_color_texture.is_some(), &material.base_color_channel),
            ("emissive", material.emissive_texture.is_some(), &material.emissive_channel),
            ("metallic/roughness", material.metallic_roughness_texture.is_some(), &material.metallic_roughness_channel),
            ("normal", material.normal_map_texture.is_some(), &material.normal_map_channel),
            ("occlusion", material.occlusion_texture.is_some(), &material.occlusion_channel),
            ("clearcoat", material.clearcoat_texture.is_some(), &material.clearcoat_channel),
            ("clearcoat roughness", material.clearcoat_roughness_texture.is_some(), &material.clearcoat_roughness_channel),
        ].into_iter().filter_map(|(name, present, requested)| (present && requested == &channel).then_some(name)).collect::<Vec<_>>();
        if !textures.is_empty() { warnings.push(format!("mesh has no {label} coordinates requested by {} textures", textures.join(", "))); }
    }
    if material.normal_map_texture.is_some() && mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_none() {
        warnings.push("normal mapping is ignored because the mesh has no tangent frame; provide nondegenerate UVs".into());
    }
}

fn double_sided(ctx: &RouteCtx) -> bool {
    ctx.stage.prim(ctx.path.clone()).ok()
        .and_then(|prim| prim.attribute("doubleSided").get::<bool>().ok().flatten()).unwrap_or(false)
}

pub(crate) fn apply_sidedness(ctx: &RouteCtx, material: &mut StandardMaterial) {
    let double_sided = double_sided(ctx);
    material.double_sided = double_sided;
    material.cull_mode = if double_sided { None } else { Some(bevy::render::render_resource::Face::Back) };
}

pub(crate) fn default_material(ctx: &RouteCtx) -> StandardMaterial {
    let opacity = crate::read::geom::read_primvar_float(ctx.stage, ctx.path, "primvars:displayOpacity", ctx.time).ok().flatten();
    default_material_with_opacity(ctx, opacity.as_ref())
}

pub(crate) fn default_material_with_opacity(ctx: &RouteCtx, opacity: Option<&crate::read::geom::MeshPrimvar<f32>>) -> StandardMaterial {
    let mut material = StandardMaterial::default();
    apply_sidedness(ctx, &mut material);
    if let Some(opacity) = opacity {
        let translucent = |value: &f32| value.is_finite() && *value < 1.0;
        let blend = if opacity.indices.is_empty() { opacity.values.iter().any(translucent) }
            else { opacity.indices.iter().filter_map(|index| usize::try_from(*index).ok().and_then(|index| opacity.values.get(index))).any(translucent) };
        if blend { material.alpha_mode = AlphaMode::Blend; }
    }
    material
}

/// The prim's decoded preview material, if it has a binding that resolves.
fn material_at(ctx: &RouteCtx, binding: &openusd::sdf::Path) -> anyhow::Result<ReadPreviewMaterial> {
    read_preview_material_at(ctx.stage, binding, ctx.time)?
        .ok_or_else(|| anyhow::anyhow!("unsupported material surface at {binding}"))
}

/// Materials resolved while one projection job runs. The gprims of a stage
/// mostly share a few materials, and resolving one packs its textures, so
/// the job keeps each (binding, time, sidedness) result until it finishes.
/// Non-send because it pins the stage it belongs to.
pub(crate) struct ProjectionMaterials {
    stage: openusd::usd::Stage,
    resolved: std::collections::HashMap<(String, Option<u64>, bool), (Handle<StandardMaterial>, Vec<String>)>,
}

impl ProjectionMaterials {
    pub(crate) fn new(stage: &openusd::usd::Stage) -> Self {
        Self { stage: stage.clone(), resolved: Default::default() }
    }
}

fn memo_key(ctx: &RouteCtx, binding: &openusd::sdf::Path) -> (String, Option<u64>, bool) {
    (binding.to_string(), ctx.time.map(f64::to_bits), double_sided(ctx))
}

fn warn_material(world: &mut World, entity: Entity, ctx: &RouteCtx, message: String) {
    if world.get::<UsdMaterialWarning>(entity).is_none_or(|old| old.0 != message) {
        bevy::log::warn!("{}: {message}", ctx.prim_str());
    }
    world.entity_mut(entity).insert(UsdMaterialWarning(message));
}

fn to_standard_material(
    read: &ReadPreviewMaterial,
    assets: Option<&AssetServer>,
    textures: Option<&crate::asset::SnapshotTextures>,
) -> StandardMaterial {
    let mut m = StandardMaterial::default();
    if let Some(c) = read.diffuse_color {
        let a = read.opacity.unwrap_or(1.0);
        m.base_color = Color::linear_rgba(c[0], c[1], c[2], a);
    } else if let Some(a) = read.opacity {
        m.base_color.set_alpha(a);
    }
    if let Some(threshold) = read.opacity_threshold.filter(|threshold| *threshold > 0.0) {
        m.alpha_mode = AlphaMode::Mask(threshold);
    } else if read.opacity.is_some_and(|a| a < 1.0) || read.opacity_texture.is_some() {
        m.alpha_mode = AlphaMode::Blend;
    }
    if let Some(r) = read.roughness {
        m.perceptual_roughness = r;
    }
    if let Some(coat) = read.clearcoat { m.clearcoat = coat; }
    if read.clearcoat.is_some() || read.clearcoat_roughness.is_some() || read.clearcoat_texture.is_some() || read.clearcoat_roughness_texture.is_some() {
        m.clearcoat_perceptual_roughness = read.clearcoat_roughness.unwrap_or(0.01);
    }
    if let Some(mtl) = read.metallic {
        m.metallic = mtl;
    }
    if let Some(e) = read.emissive_color {
        m.emissive = LinearRgba::rgb(e[0], e[1], e[2]);
    }
    if let Some(ior) = read.ior {
        m.ior = ior;
    }
    // Convert the USD UV transform through the mesh's V-flipped coordinate basis.
    if let Some(uv) = &read.uv_transform {
        let flip = bevy::math::Affine2::from_scale_angle_translation(Vec2::new(1.0, -1.0), 0.0, Vec2::Y);
        m.uv_transform = flip * *uv * flip;
    }
    let texture = |path: &Option<String>, srgb: bool| -> Option<Handle<Image>> {
        let path = path.as_ref()?;
        if let Some(textures) = textures {
            textures.0.get(&(path.clone(), srgb)).cloned()
        } else {
            assets.map(|server| server.load(path.clone()))
        }
    };
    m.base_color_texture = texture(&read.diffuse_texture, read.texture_srgb("diffuse"));
    m.normal_map_texture = texture(&read.normal_texture, read.texture_srgb("normal"));
    m.emissive_texture = texture(&read.emissive_texture, read.texture_srgb("emissive"));
    if m.emissive_texture.is_some() && read.emissive_color.is_none() {
        m.emissive = LinearRgba::WHITE;
    }
    m
}

pub(crate) fn resolve_material(
    ctx: &RouteCtx,
    world: &mut World,
) -> anyhow::Result<Option<(Handle<StandardMaterial>, Vec<String>)>> {
    let Some(binding) = read_material_binding(ctx.stage, ctx.path)? else { return Ok(None) };
    let key = memo_key(ctx, &binding);
    if let Some(memo) = world.get_non_send::<ProjectionMaterials>()
        && memo.stage.ptr_eq(ctx.stage)
        && let Some(resolved) = memo.resolved.get(&key)
    {
        return Ok(Some(resolved.clone()));
    }
    let read = material_at(ctx, &binding)?;
    let assets = world.get_resource::<AssetServer>().cloned();
    let textures = world.get_resource::<crate::asset::SnapshotTextures>();
    let mut material = to_standard_material(&read, assets.as_ref(), textures);
    apply_sidedness(ctx, &mut material);
    let mut warnings = read.warnings.clone();
    for semantic in ["diffuse", "emissive", "normal"] {
        match super::color_texture::transformed(world, &read, semantic) {
            Ok(Some(handle)) => {
                if semantic == "diffuse" { material.base_color_texture = Some(handle); }
                else if semantic == "normal" { material.normal_map_texture = Some(handle); }
                else { material.emissive_texture = Some(handle); }
            }
            Ok(None) => {}
            Err(error) => warnings.push(error.to_string()),
        }
    }
    match super::texture_pack::base_color_alpha(world, &read) {
        Ok(Some(packed)) => {
            material.base_color_texture = Some(packed);
        }
        Ok(None) => {}
        Err(error) => warnings.push(error.to_string()),
    }
    match super::texture_pack::metallic_roughness(world, &read) {
        Ok(packed) => {
            if packed.is_some() {
                if read.metallic_texture.is_some() { material.metallic = read.metallic.unwrap_or(1.0); }
                if read.roughness_texture.is_some() { material.perceptual_roughness = read.roughness.unwrap_or(1.0); }
            }
            material.metallic_roughness_texture = packed;
        }
        Err(error) => {
            warnings.push(error.to_string());
        }
    }
    match super::texture_pack::occlusion(world, &read) {
        Ok(packed) => material.occlusion_texture = packed,
        Err(error) => warnings.push(error.to_string()),
    }
    for roughness in [false, true] {
        match super::texture_pack::clearcoat(world, &read, roughness) {
            Ok(Some(texture)) if roughness => {
                material.clearcoat_roughness_texture = Some(texture);
                material.clearcoat_perceptual_roughness = read.clearcoat_roughness.unwrap_or(1.0);
            }
            Ok(Some(texture)) => {
                material.clearcoat_texture = Some(texture);
                material.clearcoat = read.clearcoat.unwrap_or(1.0);
            }
            Ok(None) => {}
            Err(error) => warnings.push(error.to_string()),
        }
    }
    let handle = super::cache::intern_material(world, material);
    if let Some(mut memo) = world.get_non_send_mut::<ProjectionMaterials>()
        && memo.stage.ptr_eq(ctx.stage)
    {
        memo.resolved.insert(key, (handle.clone(), warnings.clone()));
    }
    Ok(Some((handle, warnings)))
}

impl PrimRoute for MaterialRoute {
    fn matches(&self, _: &RouteCtx) -> bool { true }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get::<Mesh3d>(entity).is_none() || world.get_resource::<Assets<StandardMaterial>>().is_none() {
            return;
        }
        let (handle, mut warnings) = match resolve_material(ctx, world) {
            Ok(Some(material)) => material,
            Ok(None) => { world.entity_mut(entity).remove::<UsdMaterialWarning>(); return; }
            Err(error) => { warn_material(world, entity, ctx, error.to_string()); return; }
        };
        if let Some(mesh) = world.get::<Mesh3d>(entity)
            .and_then(|mesh| world.get_resource::<Assets<Mesh>>()?.get(&mesh.0))
            && let Some(material) = world.resource::<Assets<StandardMaterial>>().get(&handle) {
            warn_geometry_inputs(mesh, material, &mut warnings);
        }
        if warnings.is_empty() {
            world.entity_mut(entity).remove::<UsdMaterialWarning>();
        } else {
            let message = warnings.join("; ");
            warn_material(world, entity, ctx, message);
        }
        if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
            mat.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(MeshMaterial3d(handle));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn interleaved_projection_jobs_retain_separate_material_memos() {
        use crate::live::ProjectionJob;
        let stage = || crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Shader.outputs:surface>
    def Shader "Shader" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (1, 0, 0)
        token outputs:surface
    }
}
def Cube "A" { rel material:binding = </Mat> }
def Cube "B" { rel material:binding = </Mat> }
"#).open_stage().unwrap();
        let first = stage();
        let second = stage();
        let surrounding = stage();
        let mut world = World::new();
        world.insert_resource(crate::route::SchemaRegistry::builtin());
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.insert_non_send(ProjectionMaterials::new(&surrounding));
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let (mut one, mut map_one) = ProjectionJob::begin(&mut world, &first, a);
        let (mut two, mut map_two) = ProjectionJob::begin(&mut world, &second, b);
        assert!(world.non_send::<ProjectionMaterials>().stage.ptr_eq(&surrounding));
        for _ in 0..16 {
            let done_one = one.step(&mut world, &first, &mut map_one, std::time::Duration::ZERO);
            let done_two = two.step(&mut world, &second, &mut map_two, std::time::Duration::ZERO);
            assert!(world.non_send::<ProjectionMaterials>().stage.ptr_eq(&surrounding));
            if done_one && done_two { break; }
        }
        let material = |map: &crate::live::PrimEntities, path|
            world.get::<MeshMaterial3d<StandardMaterial>>(map.entity(path).unwrap()).unwrap().0.id();
        assert_eq!(material(&map_one, "/A"), material(&map_one, "/B"));
        assert_eq!(material(&map_two, "/A"), material(&map_two, "/B"));
        assert_ne!(material(&map_one, "/A"), material(&map_two, "/A"));
        assert!(!world.contains_resource::<super::super::cache::MaterialCache>());
    }

    use super::*;

    #[test]
    fn texture_coordinate_diagnostics_follow_requested_channels() {
        let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
        let mut material = StandardMaterial::default();
        let mut warnings = Vec::new();
        warn_geometry_inputs(&mesh, &material, &mut warnings);
        assert!(warnings.is_empty());
        material.base_color_texture = Some(Handle::default());
        material.emissive_texture = Some(Handle::default());
        material.emissive_channel = bevy::mesh::UvChannel::Uv1;
        material.metallic_roughness_texture = Some(Handle::default());
        material.occlusion_texture = Some(Handle::default());
        warn_geometry_inputs(&mesh, &material, &mut warnings);
        assert_eq!(warnings, ["mesh has no UV0 coordinates requested by base color, metallic/roughness, occlusion textures",
            "mesh has no UV1 coordinates requested by emissive textures"]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.,0.];3]);
        warnings.clear();
        warn_geometry_inputs(&mesh, &material, &mut warnings);
        assert_eq!(warnings, ["mesh has no UV1 coordinates requested by emissive textures"]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.,0.];3]);
        warnings.clear();
        warn_geometry_inputs(&mesh, &material, &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn normal_tangent_diagnostics_cover_mesh_subsets_and_prototypes() {
        for uv in [false, true] {
            let coords = if uv { "texCoord2f[] primvars:st = [(0,0),(1,0),(0,1)] (interpolation = \"vertex\")" } else { "" };
            let mesh = format!(r#"
    uniform token subdivisionScheme = "none"
    point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    {coords}
    rel material:binding = </Mat>
    def GeomSubset "Part" {{
        uniform token elementType = "face"
        uniform token familyName = "materialBind"
        int[] indices = [0]
        rel material:binding = </Mat>
    }}
"#);
            let text = format!(r#"#usda 1.0
def Mesh "Plain" {{ {mesh} }}
def Mesh "Proto" {{ {mesh} }}
def BasisCurves "Curve" {{
    uniform token type = "linear"
    int[] curveVertexCounts = [2]
    point3f[] points = [(0,0,0),(1,0,0)]
    float[] widths = [0.2,0.2]
    rel material:binding = </Mat>
}}
def PointInstancer "Copies" {{
    rel prototypes = [</Proto>]
    int[] protoIndices = [0]
    point3f[] positions = [(2,0,0)]
}}
def Material "Mat" {{
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {{
        uniform token info:id = "UsdPreviewSurface"
        normal3f inputs:normal = (0,0.5,0.5)
        token outputs:surface
    }}
}}
"#);
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
            app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>().init_resource::<Assets<Image>>();
            app.insert_resource(super::super::curves::UsdCurveSettings::default().with_surface_sides(Some(8)).unwrap());
            let source = crate::UsdSource::snapshot("normal.usda", text.as_bytes()).unwrap();
            let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
            app.world_mut().spawn(crate::UsdSceneRoot(handle));
            app.update();
            let mut count = 0;
            let mut query = app.world_mut().query::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>, Option<&UsdMaterialWarning>)>();
            for (mesh, material, warning) in query.iter(app.world()) {
                if app.world().resource::<Assets<StandardMaterial>>().get(&material.0).unwrap().normal_map_texture.is_none() { continue; }
                count += 1;
                let mesh = app.world().resource::<Assets<Mesh>>().get(&mesh.0).unwrap();
                assert_eq!(warning.is_some_and(|warning| warning.0.contains("no tangent frame")), mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_none());
                assert_eq!(warning.is_some_and(|warning| warning.0.contains("no UV0 coordinates")), mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_none());
            }
            assert!(count >= 5, "expected ordinary, subset, curve and instanced material entities, got {count}");
        }
    }

    #[test]
    fn fallback_opacity_tracks_independent_mesh_shape_and_prototype_clocks() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        for (inherited, bytes) in [(false, include_bytes!("../../../../assets/display_opacity.usda").as_slice()),
            (true, include_bytes!("../../../../assets/inherited_display.usda").as_slice())] {
        let source = crate::UsdSource::new("opacity.usda", bytes).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        for times in [[0.0,10.0], [10.0,0.0], [5.0,10.0]] {
            for (root,time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
            app.update();
            for (root,time) in roots.into_iter().zip(times) {
                let instances = app.world().get_non_send::<UsdInstances>().unwrap();
                let cube = instances.entity(root, "/Scene/Cube").unwrap();
                let mesh = instances.entity(root, "/Scene/Mesh").unwrap();
                let pi = instances.entity(root, "/Scene/PI").unwrap();
                let copy = app.world().get::<Children>(pi).unwrap().iter().find(|entity|
                    app.world().get::<super::super::instancer::UsdInstance>(*entity).is_some()).unwrap();
                let prototype = app.world().get::<Children>(copy).unwrap().iter().find(|entity|
                    app.world().get::<Mesh3d>(*entity).is_some()).unwrap();
                for entity in [cube,mesh,prototype] {
                    let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
                    assert_eq!(material.alpha_mode, if time == 10.0 { AlphaMode::Opaque } else { AlphaMode::Blend });
                    assert_eq!(material.base_color.alpha(), 1.0);
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
                    let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
                    let alpha = 0.2 + time as f32 * 0.08;
                    assert!(colors.iter().all(|color| (color[3]-alpha).abs() < 1e-5));
                    let fraction = if inherited { time as f32 / 10.0 } else { 0.0 };
                    let expected = Vec3::new(0.8*(1.0-fraction),0.2,0.1+0.7*fraction);
                    assert!(colors.iter().all(|color| Vec3::new(color[0],color[1],color[2]).abs_diff_eq(expected, 1e-5)));
                }
            }
        }
        }
    }

    #[test]
    fn shader_dependencies_sample_materials_for_independent_roots() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("animated-material.usda", &br#"#usda 1.0
def Mesh "Mesh" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    rel material:binding = </Mat>
}
def PointInstancer "PI" {
    point3f[] positions = [(2,0,0)]
    int[] protoIndices = [0]
    rel prototypes = [</Mesh>]
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.timeSamples = { 0: (1,0,0), 10: (0,0,1) }
        float inputs:roughness.connect = </External.outputs:out>
        float inputs:opacity.timeSamples = { 0: 0.25, 10: 1 }
        token outputs:surface
    }
    def Shader "UV" {
        uniform token info:id = "UsdTransform2d"
        float2 inputs:translation.timeSamples = { 0: (0,0), 10: (2,0) }
    }
}
def Shader "External" {
    uniform token info:id = "ND_multiply_float"
    float inputs:in1.timeSamples = { 0: 0.4, 10: 1.6 }
    float inputs:in2 = 0.5
    float outputs:out
}
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let entities = |world: &World, root| {
            let instances = world.get_non_send::<UsdInstances>().unwrap();
            let mesh = instances.entity(root, "/Mesh").unwrap();
            let pi = instances.entity(root, "/PI").unwrap();
            let child = world.get::<Children>(pi).unwrap().iter().find(|child| world.get::<crate::route::instancer::UsdInstance>(*child).is_some()).unwrap();
            [mesh, child]
        };
        let a = entities(app.world(), first);
        let b = entities(app.world(), second);
        let check = |world: &World, entities: [Entity; 2], t: f32| {
            for entity in entities {
                let handle = &world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0;
                let material = world.resource::<Assets<StandardMaterial>>().get(handle).unwrap();
                assert_eq!(material.base_color, Color::linear_rgba(1.0-t, 0.0, t, 0.25+0.75*t));
                assert!((material.perceptual_roughness - (0.2+0.6*t)).abs() < 1e-6);
                assert_eq!(material.uv_transform.translation, Vec2::ZERO);
                assert_eq!(material.alpha_mode, if t < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque });
            }
        };
        check(app.world(), a, 0.0);
        check(app.world(), b, 1.0);
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.update();
        assert_eq!(entities(app.world(), first), a);
        assert_eq!(entities(app.world(), second), b);
        check(app.world(), a, 0.5);
        check(app.world(), b, 1.0);
    }

    #[test]
    fn dispatch_skips_non_geometry_and_reports_material_failures() {
        let stage = crate::UsdSource::new("dispatch.usda", &b"#usda 1.0\ndef Xform \"Root\" { rel material:binding = </Missing> }\n"[..]).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        let ctx = RouteCtx::new(&stage, &path);
        let mut world = World::new();
        world.insert_resource(Assets::<StandardMaterial>::default());
        let entity = world.spawn_empty().id();
        MaterialRoute.project(&ctx, &mut world, entity);
        assert!(world.get::<UsdMaterialWarning>(entity).is_none());
        assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
        world.entity_mut(entity).insert(Mesh3d::default());
        MaterialRoute.project(&ctx, &mut world, entity);
        assert!(world.get::<UsdMaterialWarning>(entity).unwrap().0.contains("unsupported material surface"));
        crate::authoring::set_relationship_targets(&stage, "/Root", "material:binding", &[]).unwrap();
        MaterialRoute.project(&ctx, &mut world, entity);
        assert!(world.get::<UsdMaterialWarning>(entity).is_none());
    }

    #[test]
    fn inherited_materials_keep_per_prim_sidedness_when_shared() {
        let source = crate::UsdSource::new("sided.usda", &br#"#usda 1.0
def Xform "Group" {
    rel material:binding = </Mat>
    def Plane "A" {}
    def Plane "B" { bool doubleSided = false }
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0.2, 0.4, 0.6)
        token outputs:surface
    }
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.init_resource::<super::super::cache::MaterialCache>();
        let a = world.spawn(Mesh3d::default()).id();
        let b = world.spawn(Mesh3d::default()).id();
        MaterialRoute.project(&RouteCtx::new(&stage, &openusd::sdf::path("/Group/A").unwrap()), &mut world, a);
        MaterialRoute.project(&RouteCtx::new(&stage, &openusd::sdf::path("/Group/B").unwrap()), &mut world, b);
        let a = &world.get::<MeshMaterial3d<StandardMaterial>>(a).unwrap().0;
        let b = &world.get::<MeshMaterial3d<StandardMaterial>>(b).unwrap().0;
        assert_ne!(a, b);
        let assets = world.resource::<Assets<StandardMaterial>>();
        assert!(assets.get(a).unwrap().double_sided);
        assert!(assets.get(a).unwrap().cull_mode.is_none());
        assert!(!assets.get(b).unwrap().double_sided);
        assert_eq!(assets.get(b).unwrap().cull_mode, Some(bevy::render::render_resource::Face::Back));
    }

    #[test]
    fn uv_transforms_preserve_usd_coordinates_after_v_flip() {
        for (scale, rotation_deg, translation) in [([1.0,1.0], 0.0_f32, [0.0,0.0]),
            ([1.0,1.0], 0.0, [0.0,0.5]), ([0.5,1.5], 90.0, [0.75,0.25]), ([-1.5,0.75], 37.0, [-0.2,0.6])] {
            let uv = bevy::math::Affine2::from_scale_angle_translation(scale.into(), rotation_deg.to_radians(), translation.into());
            let read = ReadPreviewMaterial { uv_transform: Some(uv), ..default() };
            let material = to_standard_material(&read, None, None);
            let (sin, cos) = rotation_deg.to_radians().sin_cos();
            for st in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.25, 0.75), Vec2::new(-0.5, 1.25)] {
                let scaled = st * Vec2::from(scale);
                let transformed = Vec2::new(cos * scaled.x - sin * scaled.y, sin * scaled.x + cos * scaled.y) + Vec2::from(translation);
                let expected = Vec2::new(transformed.x, 1.0 - transformed.y);
                let actual = material.uv_transform.transform_point2(Vec2::new(st.x, 1.0 - st.y));
                assert!(actual.abs_diff_eq(expected, 1e-6), "{actual:?} != {expected:?}");
            }
        }
    }

    #[test]
    fn preview_clearcoat_samples_map_to_bevy_and_keep_usd_roughness_default() {
        let source = crate::UsdSource::snapshot("coat.usda", br#"#usda 1.0
def Material "Mat" {
    float inputs:coat.timeSamples = { 0: 0.2, 10: 0.8 }
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:clearcoat.connect = </Mat.inputs:coat>
        float inputs:clearcoatRoughness = 0.15
        token outputs:surface
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        for (time, expected) in [(0., 0.2), (5., 0.5), (10., 0.8)] {
            let read = crate::read::shade::read_preview_material_at(&stage,
                &openusd::sdf::path("/Mat").unwrap(), Some(time)).unwrap().unwrap();
            let material = to_standard_material(&read, None, None);
            assert!((material.clearcoat - expected).abs() < 1e-6);
            assert_eq!(material.clearcoat_perceptual_roughness, 0.15);
        }
        let material = to_standard_material(&ReadPreviewMaterial { clearcoat: Some(0.5), ..default() }, None, None);
        assert_eq!(material.clearcoat_perceptual_roughness, 0.01);
        assert_eq!(to_standard_material(&ReadPreviewMaterial::default(), None, None).clearcoat, 0.0);
    }

    #[test]
    fn opaque_material_maps_channels() {
        let read = ReadPreviewMaterial {
            diffuse_color: Some([0.2, 0.4, 0.6]),
            roughness: Some(0.3),
            metallic: Some(0.8),
            emissive_color: Some([1.0, 0.0, 0.0]),
            ior: Some(1.4),
            ..Default::default()
        };
        let m = to_standard_material(&read, None, None);
        let c = m.base_color.to_linear();
        assert!((c.red - 0.2).abs() < 1e-3 && (c.blue - 0.6).abs() < 1e-3);
        assert!((c.alpha - 1.0).abs() < 1e-6, "opaque by default");
        assert!((m.perceptual_roughness - 0.3).abs() < 1e-6);
        assert!((m.metallic - 0.8).abs() < 1e-6);
        assert!((m.ior - 1.4).abs() < 1e-6);
        assert_eq!(m.emissive, LinearRgba::rgb(1.0, 0.0, 0.0));
        assert!(matches!(m.alpha_mode, AlphaMode::Opaque));
    }

    #[test]
    fn emissive_texture_uses_identity_multiplier_without_authored_color() {
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::default());
        let mut textures = crate::asset::SnapshotTextures::default();
        textures.0.insert(("emission.png".into(), true), handle.clone());
        let mut read = ReadPreviewMaterial { emissive_texture: Some("emission.png".into()), ..default() };
        let material = to_standard_material(&read, None, Some(&textures));
        assert_eq!(material.emissive_texture, Some(handle));
        assert_eq!(material.emissive, LinearRgba::WHITE);
        assert_eq!(to_standard_material(&read, None, None).emissive, LinearRgba::BLACK);
        for color in [[0.0; 3], [0.25, 0.5, 2.0]] {
            read.emissive_color = Some(color);
            assert_eq!(to_standard_material(&read, None, Some(&textures)).emissive,
                LinearRgba::rgb(color[0], color[1], color[2]));
        }
        assert_eq!(to_standard_material(&ReadPreviewMaterial::default(), None, None).emissive, LinearRgba::BLACK);
    }

    #[test]
    fn translucent_material_sets_blend() {
        let read = ReadPreviewMaterial {
            diffuse_color: Some([1.0, 1.0, 1.0]),
            opacity: Some(0.5),
            ..Default::default()
        };
        let m = to_standard_material(&read, None, None);
        assert!(
            (m.base_color.to_srgba().alpha - 0.5).abs() < 1e-3,
            "alpha from opacity"
        );
        assert!(
            matches!(m.alpha_mode, AlphaMode::Blend),
            "opacity<1 → Blend"
        );
    }

    #[test]
    fn opacity_threshold_uses_alpha_mask() {
        let read = ReadPreviewMaterial { opacity: Some(0.25), opacity_texture: Some("alpha.png".into()), opacity_threshold: Some(0.5), ..Default::default() };
        assert_eq!(to_standard_material(&read, None, None).alpha_mode, AlphaMode::Mask(0.5));
        let read = ReadPreviewMaterial { opacity_threshold: None, ..read };
        assert_eq!(to_standard_material(&read, None, None).alpha_mode, AlphaMode::Blend);
    }

    #[test]
    fn empty_material_is_default_ish() {
        // An all-None read yields (essentially) the StandardMaterial default.
        let m = to_standard_material(&ReadPreviewMaterial::default(), None, None);
        let def = StandardMaterial::default();
        assert_eq!(m.base_color.to_srgba(), def.base_color.to_srgba());
        assert!(matches!(m.alpha_mode, AlphaMode::Opaque));
    }
}
