//! UsdShade read side: resolve a `Material`'s surface shader and harvest its
//! `UsdPreviewSurface` (and MDL / MaterialX equivalents) inputs into a
//! [`ReadPreviewMaterial`], following connections through `UsdUVTexture` and
//! the MaterialX node graph. Reads through openusd only.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::util::{
    connections_at, read_asset_path, read_token_or_string,
};

/// Decoded UsdPreviewSurface material. Each channel is `None` (unauthored),
/// a scalar, or a texture asset path (caller resolves via the AssetServer).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadPreviewMaterial {
    pub diffuse_color: Option<[f32; 3]>,
    pub opacity: Option<f32>,
    pub opacity_threshold: Option<f32>,
    pub roughness: Option<f32>,
    pub clearcoat: Option<f32>,
    pub clearcoat_roughness: Option<f32>,
    pub metallic: Option<f32>,
    pub emissive_color: Option<[f32; 3]>,
    pub ior: Option<f32>,

    pub diffuse_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub normal: Option<[f32; 3]>,
    pub roughness_texture: Option<String>,
    pub metallic_texture: Option<String>,
    pub roughness_channel: usize,
    pub metallic_channel: usize,
    pub opacity_texture: Option<String>,
    pub opacity_channel: usize,
    pub emissive_texture: Option<String>,
    pub occlusion_texture: Option<String>,
    pub occlusion_channel: usize,
    pub texture_color_spaces: std::collections::BTreeMap<String, bool>,
    /// Selected-channel [scale, bias] for scalar texture semantics.
    pub scalar_texture_transforms: std::collections::BTreeMap<String, [f32; 2]>,
    /// RGB [scale, bias] for diffuse and emissive textures.
    pub color_texture_transforms: std::collections::BTreeMap<String, [[f32; 3]; 2]>,
    /// Signed tangent-space RGB transform for Preview Surface UsdUVTexture normals.
    pub normal_texture_transform: Option<[[f32; 3]; 2]>,
    pub warnings: Vec<String>,

    /// Composed texture-coordinate transform in USD's unflipped coordinate basis.
    pub uv_transform: Option<bevy::math::Affine2>,
}

/// Resolves preview-purpose material binding, falling back to all-purpose.
pub fn read_material_binding(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Path>> {
    read_material_binding_for_purpose(stage, prim, "preview")
}

/// Resolves inherited, collection and direct bindings using USD binding strength.
pub fn read_material_binding_for_purpose(stage: &Stage, prim: &Path, purpose: &str) -> anyhow::Result<Option<Path>> {
    Ok(openusd_schemas::shade::MaterialBindingAPI::new(stage.prim(prim.clone())?).compute_bound_material(purpose)?)
}

/// Read a `Material` prim and return its decoded surface inputs.
pub fn read_preview_material(
    stage: &Stage,
    material: &Path,
) -> anyhow::Result<Option<ReadPreviewMaterial>> {
    read_preview_material_at(stage, material, None)
}

pub(crate) fn bound_material_is_time_varying(stage: &Stage, prim: &Path) -> bool {
    let Ok(Some(material)) = read_material_binding(stage, prim) else { return false };
    let mut pending = vec![material.clone()];
    match resolve_surface_shader(stage, &material) {
        Ok(Some((surface, _))) => pending.push(surface),
        Ok(None) => (),
        Err(_) => return true,
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.to_string()) { continue; }
        if seen.len() > 256 { return true; }
        let Ok(node) = stage.prim(path.clone()) else { continue };
        if let Ok(attributes) = node.attributes() {
            for attribute in attributes {
                if attribute.time_sample_times().is_ok_and(|times| !times.is_empty()) { return true; }
                if let Ok(connections) = attribute.connections() {
                    pending.extend(connections.into_iter().map(|connection| connection.prim_path()));
                }
            }
        }
    }
    false
}

/// Sorted texture-file and color-space sample times reachable from a material.
pub(crate) fn material_texture_sample_times(stage: &Stage, material: &Path) -> anyhow::Result<Vec<f64>> {
    let mut pending = vec![material.clone()];
    if let Some((shader, _)) = resolve_surface_shader(stage, material)? { pending.push(shader); }
    let mut seen = std::collections::HashSet::new();
    let mut times = Vec::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) { continue; }
        anyhow::ensure!(seen.len() <= 4096, "texture discovery exceeds the 4096-node traversal budget");
        let node = stage.prim(&path)?;
        times.extend(node.attribute("inputs:file").time_sample_times()?);
        times.extend(node.attribute("inputs:sourceColorSpace").time_sample_times()?);
        if matches!(shader_kind(stage, &path)?, ShaderKind::Texture) {
            for name in ["file", "sourceColorSpace"] {
                if let Some(attribute) = texture_input_attribute(stage, &path, name)? {
                    times.extend(attribute.time_sample_times()?);
                }
            }
        }
        for attribute in node.attributes()? {
            pending.extend(attribute.connections()?.into_iter().map(|connection| connection.prim_path()));
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    Ok(times)
}

/// Resolve preview inputs at a USD time code, or their default values.
pub fn read_preview_material_at(stage: &Stage, material: &Path, time: Option<f64>) -> anyhow::Result<Option<ReadPreviewMaterial>> {
    let Some((shader, dialect)) = resolve_surface_shader(stage, material)? else {
        return Ok(None);
    };

    let shader_id = read_token_or_string(stage, &shader, "info:id")?;
    let mdl_subid = read_token_or_string(stage, &shader, "info:mdl:sourceAsset:subIdentifier")?;
    let mdl_source = read_asset_path(stage, &shader, "info:mdl:sourceAsset")?;
    let mdl_basename = mdl_source.as_deref().and_then(|p| {
        std::path::Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    });
    let mdl_id = mdl_subid.as_deref().or(mdl_basename.as_deref());

    let channels: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = match dialect {
        SurfaceDialect::Mdl => match mdl_id {
            Some("OmniSurface") | Some("OmniSurfaceLite") | Some("OmniSurfaceBase") => {
                OMNISURFACE_CHANNELS
            }
            _ => OMNIPBR_CHANNELS,
        },
        SurfaceDialect::MaterialX => match shader_id.as_deref() {
            Some("ND_standard_surface_surfaceshader") => MATERIALX_STD_SURFACE_CHANNELS,
            _ => PREVIEW_CHANNELS,
        },
        SurfaceDialect::Preview => match shader_id.as_deref() {
            Some("UsdPreviewSurface") | Some("ND_UsdPreviewSurface_surfaceshader") | None => {
                PREVIEW_CHANNELS
            }
            Some("OmniPBR") | Some("OmniPBR_Opacity") | Some("OmniPBR_ClearCoat") => {
                OMNIPBR_CHANNELS
            }
            Some("OmniSurface") | Some("OmniSurfaceLite") | Some("OmniSurfaceBase") => {
                OMNISURFACE_CHANNELS
            }
            Some("ND_standard_surface_surfaceshader") => MATERIALX_STD_SURFACE_CHANNELS,
            _ => return Ok(None),
        },
    };

    let mut out = ReadPreviewMaterial::default();
    let mut textures = Vec::new();
    for (channel, bind_colour, bind_scalar, bind_texture) in channels {
        let (value, texture) = resolve_channel(stage, material, &shader, channel, &mut out.warnings, time)?;
        if let Some(tex) = texture {
            if !textures.contains(&tex.3) { textures.push(tex.3.clone()); }
            bind_texture(&mut out, tex);
        }
        match value {
            Some(ResolvedValue::Color3(c)) => bind_colour(&mut out, c),
            Some(ResolvedValue::Scalar(s)) => bind_scalar(&mut out, s),
            None => {}
        }
    }
    out.uv_transform = read_uv_transform(stage, &textures, time)?;
    anyhow::ensure!(out.clearcoat.into_iter().chain(out.clearcoat_roughness).all(f32::is_finite),
        "nonfinite clearcoat input");
    if !matches!(shader_id.as_deref(), None | Some("UsdPreviewSurface") | Some("ND_UsdPreviewSurface_surfaceshader")) {
        out.normal_texture_transform = None;
        out.normal = None;
    }
    out.warnings.sort();
    out.warnings.dedup();
    Ok(Some(out))
}

#[cfg(test)]
fn texture_value_transform(stage: &Stage, prim: &Path, channel: usize, time: Option<f64>) -> anyhow::Result<[f32; 2]> {
    let result = texture_rgba_transform(stage, prim, time)?;
    Ok([result[0][channel], result[1][channel]])
}

fn texture_rgba_transform(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<[[f32; 4]; 2]> {
    use openusd_schemas::shade::{Connectable, ProducerFilter, Shader};
    if read_token_or_string(stage, prim, "info:id")?.as_deref() != Some("UsdUVTexture") { return Ok([[1.0; 4], [0.0; 4]]); }
    let mut result = [[1.0; 4], [0.0; 4]];
    let shader = Shader::new(stage.prim(prim)?);
    for (index, name) in ["scale", "bias"].into_iter().enumerate() {
        let input = shader.input(name);
        let path = input.path();
        let connections = input.connected_sources()?;
        anyhow::ensure!(connections.invalid_source_paths().is_empty(), "connected texture scale/bias has invalid sources at {path}");
        let producers = input.value_producing_attributes(ProducerFilter::Any)?;
        anyhow::ensure!(producers.len() <= 1, "multiple texture scale/bias value producers at {path}");
        let value = producers.first().map(|source| source.attribute()
            .get_at::<Value>(time.map(openusd::usd::TimeCode::new))).transpose()?.flatten();
        let sampled_default = value.is_none() && time.is_none() && producers.first()
            .map(|source| source.attribute().time_sample_times()).transpose()?.is_some_and(|times| !times.is_empty());
        anyhow::ensure!(connections.sources().is_empty() || value.is_some() || sampled_default,
            "connected texture scale/bias has no readable value at {path}");
        if let Some(value) = value {
            let Value::Vec4f(value) = value else { anyhow::bail!("texture scale/bias must be float4 at {path}"); };
            let values = [value.x, value.y, value.z, value.w];
            anyhow::ensure!(values.iter().all(|value| value.is_finite()), "nonfinite texture scale/bias at {path}");
            result[index] = values;
        }
    }
    Ok(result)
}

fn sampled_value(stage: &Stage, path: &Path, time: Option<f64>) -> anyhow::Result<Option<Value>> {
    let Some((prim, name)) = path.split_property() else { return Ok(None) };
    Ok(stage.prim(prim)?.attribute(name)
        .get_at::<Value>(time.map(openusd::usd::TimeCode::new))?)
}

#[test]
fn texture_scale_bias_resolves_sampled_container_interfaces() {
    let source = crate::UsdSource::snapshot("interface.usda", br#"#usda 1.0
def Material "Mat" {
    float4 inputs:scale.timeSamples = {0: (2,3,4,5), 10: (4,5,6,7)}
    float4 inputs:bias = (0.1,0.2,0.3,0.4)
    def NodeGraph "Graph" {
        float4 outputs:scale.connect = </Mat.inputs:scale>
        float4 outputs:cycle.connect = </Mat/Graph.outputs:cycle>
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        float4 inputs:scale.connect = </Mat/Graph.outputs:scale>
        float4 inputs:bias.connect = </Mat.inputs:bias>
    }
}
"#.as_slice()).unwrap();
    let stage = source.open_stage().unwrap();
    let prim = openusd::sdf::path("/Mat/Tex").unwrap();
    let before = stage.root_layer().export_to_string().unwrap();
    assert_eq!(texture_value_transform(&stage, &prim, 0, None).unwrap(), [1.0, 0.1]);
    for (time, first) in [(0.0, 2.0), (5.0, 3.0), (10.0, 4.0), (0.0, 2.0)] {
        for (channel, bias) in [0.1, 0.2, 0.3, 0.4].into_iter().enumerate() {
            assert_eq!(texture_value_transform(&stage, &prim, channel, Some(time)).unwrap(), [first + channel as f32, bias]);
        }
    }
    assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    let scale = stage.attribute("/Mat/Tex.inputs:scale").unwrap();
    scale.clone().set_connections([openusd::sdf::path("/Mat/Graph.outputs:cycle").unwrap()]).unwrap();
    assert!(texture_value_transform(&stage, &prim, 0, Some(0.0)).unwrap_err().to_string().contains("no readable value"));
    scale.clone().set(Value::Vec4f([-9.0; 4].into())).unwrap();
    assert_eq!(texture_value_transform(&stage, &prim, 0, Some(0.0)).unwrap(), [-9.0, 0.1]);
    scale.clone().set_connections([openusd::sdf::path("/Mat.inputs:scale").unwrap(), openusd::sdf::path("/Mat.inputs:bias").unwrap()]).unwrap();
    assert!(texture_value_transform(&stage, &prim, 0, Some(0.0)).unwrap_err().to_string().contains("multiple texture scale/bias"));
    scale.set_connections([openusd::sdf::path("/Mat.inputs:scale").unwrap()]).unwrap();
    assert_eq!(texture_value_transform(&stage, &prim, 0, Some(0.0)).unwrap(), [2.0, 0.1]);
}

fn read_uv_transform(stage: &Stage, textures: &[Path], time: Option<f64>) -> anyhow::Result<Option<bevy::math::Affine2>> {
    let mut common = None;
    let mut authored = false;
    for texture in textures {
        let mut current = texture.append_property("inputs:st")?;
        let mut transform = bevy::math::Affine2::IDENTITY;
        for depth in 0..=16 {
            let connections = connections_at(stage, &current)?;
            anyhow::ensure!(connections.len() <= 1, "multiple texture-coordinate connections at {current}");
            let Some(next) = connections.into_iter().next() else {
                if let Some(value) = sampled_value(stage, &current, time)? {
                    let value = match value {
                        Value::Vec2f(v) => bevy::math::Vec2::new(v.x,v.y),
                        Value::Vec2d(v) => bevy::math::Vec2::new(v.x as f32,v.y as f32),
                        _ => anyhow::bail!("constant texture coordinates must be a two-component vector at {current}"),
                    };
                    transform *= bevy::math::Affine2::from_scale_angle_translation(bevy::math::Vec2::ZERO,0.0,value);
                    authored = true;
                }
                break;
            };
            anyhow::ensure!(depth < 16, "texture-coordinate graph exceeds 16 connections");
            let node = next.prim_path();
            if read_token_or_string(stage, &node, "info:id")?.as_deref() == Some("UsdTransform2d") {
                transform *= read_uv_node(stage, &node, time)?;
                authored = true;
                current = node.append_property("inputs:in")?;
            } else {
                current = next;
            }
        }
        anyhow::ensure!(transform.is_finite(), "non-finite composed UV transform");
        if let Some(common) = common { anyhow::ensure!(transform == common, "different per-texture UV transforms are unsupported"); }
        else { common = Some(transform); }
    }
    Ok(if authored { common } else { None })
}

#[test]
fn constant_uv_coordinates_follow_interfaces_and_transforms() {
    use bevy::math::Vec2;
    let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("constant-uv.usda").unwrap();
    for name in ["/Tex","/Mat","/Transform"] { stage.define_prim(name).unwrap(); }
    let textures = [Path::new("/Tex").unwrap()];
    let st = stage.create_attribute("/Tex.inputs:st", "float2").unwrap();
    st.clone().set(Value::Vec2f([0.25,0.75].into())).unwrap();
    let constant = read_uv_transform(&stage,&textures,Some(0.0)).unwrap().unwrap();
    for uv in [Vec2::ZERO,Vec2::ONE,Vec2::new(-10.0,40.0)] {
        assert_eq!(constant.transform_point2(uv),Vec2::new(0.25,0.75));
    }
    let input = stage.create_attribute("/Mat.inputs:uv", "float2").unwrap();
    input.clone().set_at(Value::Vec2f([0.0,0.0].into()),openusd::usd::TimeCode::new(0.0)).unwrap();
    input.set_at(Value::Vec2f([1.0,0.5].into()),openusd::usd::TimeCode::new(10.0)).unwrap();
    st.clone().set_connections([Path::new("/Mat.inputs:uv").unwrap()]).unwrap();
    assert_eq!(read_uv_transform(&stage,&textures,Some(5.0)).unwrap().unwrap().translation,Vec2::new(0.5,0.25));
    stage.create_attribute("/Transform.info:id", "token").unwrap().set(Value::Token("UsdTransform2d".into())).unwrap();
    stage.create_attribute("/Transform.inputs:translation", "float2").unwrap().set(Value::Vec2f([2.0,3.0].into())).unwrap();
    stage.create_attribute("/Transform.inputs:in", "float2").unwrap().set_connections([Path::new("/Mat.inputs:uv").unwrap()]).unwrap();
    st.set_connections([Path::new("/Transform.outputs:result").unwrap()]).unwrap();
    assert_eq!(read_uv_transform(&stage,&textures,Some(5.0)).unwrap().unwrap().translation,Vec2::new(2.5,3.25));
    stage.define_prim("/Bad").unwrap();
    stage.create_attribute("/Bad.inputs:st", "token").unwrap().set(Value::Token("wrong".into())).unwrap();
    assert!(read_uv_transform(&stage,&[Path::new("/Bad").unwrap()],None).unwrap_err().to_string().contains("two-component"));
    stage.define_prim("/Double").unwrap();
    let double = stage.create_attribute("/Double.inputs:st", "double2").unwrap();
    double.clone().set(Value::Vec2d([0.5,0.25].into())).unwrap();
    assert_eq!(read_uv_transform(&stage,&[Path::new("/Double").unwrap()],None).unwrap().unwrap().translation,Vec2::new(0.5,0.25));
    double.set(Value::Vec2d([f64::MAX,0.0].into())).unwrap();
    assert!(read_uv_transform(&stage,&[Path::new("/Double").unwrap()],None).unwrap_err().to_string().contains("non-finite"));
}

fn read_uv_node(stage: &Stage, node: &Path, time: Option<f64>) -> anyhow::Result<bevy::math::Affine2> {
        let mut scale = bevy::math::Vec2::ONE;
        let mut translation = bevy::math::Vec2::ZERO;
        let mut rotation_deg = 0.0_f32;
        let vector = |name| -> anyhow::Result<Option<[f32; 2]>> {
            Ok(match uv_input_value(stage, node, name, time)? {
                Some(Value::Vec2f(value)) => Some([value.x, value.y]),
                Some(Value::Vec2d(value)) => Some([value.x as f32, value.y as f32]),
                None => None,
                _ => anyhow::bail!("UV transform {name} must be a two-component vector at {node}"),
            })
        };
        if let Some(s) = vector("inputs:scale")? {
            scale = s.into();
        }
        if let Some(tr) = vector("inputs:translation")? {
            translation = tr.into();
        }
        match uv_input_value(stage, node, "inputs:rotation", time)? {
            Some(Value::Float(r)) => rotation_deg = r,
            Some(Value::Double(r)) => rotation_deg = r as f32,
            None => (),
            _ => anyhow::bail!("UV transform rotation must be a scalar at {node}"),
        }
        let matrix = bevy::math::Affine2::from_scale_angle_translation(scale, rotation_deg.to_radians(), translation);
        anyhow::ensure!(matrix.is_finite(), "non-finite UV transform at {node}");
        Ok(matrix)
}

fn uv_input_value(stage: &Stage, node: &Path, name: &str, time: Option<f64>) -> anyhow::Result<Option<Value>> {
    let input = stage.prim(node)?.attribute(name);
    let connected = !input.connections()?.is_empty();
    let Some(attribute) = texture_input_attribute(stage, node, name.strip_prefix("inputs:").unwrap())? else { return Ok(None); };
    let value = attribute.get_at::<Value>(time.map(openusd::usd::TimeCode::new))?;
    let sampled_default = time.is_none() && !attribute.time_sample_times()?.is_empty();
    anyhow::ensure!(!connected || value.is_some() || sampled_default,
        "connected UV transform input has no readable value at {}", input.path());
    Ok(value)
}

#[test]
fn uv_transform_reads_sampled_material_interfaces() {
    let source = crate::UsdSource::new("uv-interface.usda", br#"#usda 1.0
def Material "Mat" {
    float2 inputs:move.timeSamples = {0: (0,0.5), 10: (0.75,0.25)}
    float2 inputs:size.timeSamples = {0: (1,1), 10: (0.5,1.5)}
    float inputs:angle.timeSamples = {0: 0, 10: 90}
    def Shader "Transform" {
        uniform token info:id = "UsdTransform2d"
        float2 inputs:translation.connect = </Mat.inputs:move>
        float2 inputs:scale.connect = </Mat.inputs:size>
        float inputs:rotation.connect = </Mat.inputs:angle>
    }
}
"#.as_slice()).unwrap();
    let stage = source.open_stage().unwrap();
    let node = Path::new("/Mat/Transform").unwrap();
    let before = stage.root_layer().export_to_string().unwrap();
    for time in [0.0, 5.0, 10.0, 0.0] {
        let w = time as f32 / 10.0;
        let expected = bevy::math::Affine2::from_scale_angle_translation(
            bevy::math::Vec2::new(1.0 - 0.5*w, 1.0 + 0.5*w),
            (90.0*w).to_radians(), bevy::math::Vec2::new(0.75*w, 0.5 - 0.25*w));
        assert!(read_uv_node(&stage, &node, Some(time)).unwrap().abs_diff_eq(expected, 1e-6), "time {time}");
    }
    assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    assert_eq!(read_uv_node(&stage, &node, None).unwrap(), bevy::math::Affine2::IDENTITY);
    let scale = stage.attribute("/Mat/Transform.inputs:scale").unwrap();
    scale.clone().set_connections([Path::new("/Mat.inputs:angle").unwrap()]).unwrap();
    assert!(read_uv_node(&stage, &node, Some(0.0)).unwrap_err().to_string().contains("two-component"));
    scale.clone().set_connections([Path::new("/Mat.inputs:size").unwrap(), Path::new("/Mat.inputs:move").unwrap()]).unwrap();
    assert!(read_uv_node(&stage, &node, Some(0.0)).unwrap_err().to_string().contains("multiple"));
    scale.clone().set_connections([Path::new("/Mat.inputs:missing").unwrap()]).unwrap();
    assert!(read_uv_node(&stage, &node, Some(0.0)).is_err());
    scale.set_connections([Path::new("/Mat.inputs:size").unwrap()]).unwrap();
    stage.attribute("/Mat.inputs:size").unwrap().set_at(Value::Vec2f([f32::INFINITY, 1.0].into()),
        openusd::usd::TimeCode::new(0.0)).unwrap();
    assert!(read_uv_node(&stage, &node, Some(0.0)).unwrap_err().to_string().contains("non-finite"));
}

#[derive(Copy, Clone, Debug)]
enum SurfaceDialect {
    Preview,
    MaterialX,
    Mdl,
}

fn resolve_surface_shader(
    stage: &Stage,
    material: &Path,
) -> anyhow::Result<Option<(Path, SurfaceDialect)>> {
    let outputs = [
        ("outputs:surface", SurfaceDialect::Preview),
        ("outputs:mtlx:surface", SurfaceDialect::MaterialX),
        ("outputs:mdl:surface", SurfaceDialect::Mdl),
    ];
    for (attr_name, dialect) in outputs {
        let attr_path = material.append_property(attr_name)?;
        if let Some(t) = connections_at(stage, &attr_path)?.into_iter().next() {
            return Ok(Some((t.prim_path(), dialect)));
        }
    }
    // Fallback: scan child Shader prims and infer the dialect.
    for child in stage
        .prim(material.clone()).expect("validated USD path")
        .child_names()
        .unwrap_or_default()
    {
        let shader = material.append_path(child.as_str())?;
        if stage.prim(shader.clone()).expect("validated USD path").type_name()?.as_deref() != Some("Shader") {
            continue;
        }
        let shader_id = read_token_or_string(stage, &shader, "info:id")?;
        let mdl_subid = read_token_or_string(stage, &shader, "info:mdl:sourceAsset:subIdentifier")?;
        let mdl_source = read_asset_path(stage, &shader, "info:mdl:sourceAsset")?;
        if mdl_subid.is_some() || mdl_source.is_some() {
            return Ok(Some((shader, SurfaceDialect::Mdl)));
        }
        if matches!(
            shader_id.as_deref(),
            Some("UsdPreviewSurface")
                | Some("ND_UsdPreviewSurface_surfaceshader")
                | Some("OmniPBR")
                | Some("OmniPBR_Opacity")
                | Some("OmniPBR_ClearCoat")
        ) {
            return Ok(Some((shader, SurfaceDialect::Preview)));
        }
        if matches!(
            shader_id.as_deref(),
            Some("ND_standard_surface_surfaceshader")
        ) {
            return Ok(Some((shader, SurfaceDialect::MaterialX)));
        }
    }
    Ok(None)
}

type ColourSetter = fn(&mut ReadPreviewMaterial, [f32; 3]);
type ScalarSetter = fn(&mut ReadPreviewMaterial, f32);
type TextureInput = (String, usize, Option<bool>, Path, [f32; 2], [[f32; 3]; 2], bool);
type TextureSetter = fn(&mut ReadPreviewMaterial, TextureInput);

impl ReadPreviewMaterial {
    pub fn color_texture_transform(&self, semantic: &str) -> [[f32; 3]; 2] {
        self.color_texture_transforms.get(semantic).copied().unwrap_or([[1.0; 3], [0.0; 3]])
    }

    pub fn scalar_texture_transform(&self, semantic: &str) -> [f32; 2] {
        self.scalar_texture_transforms.get(semantic).copied().unwrap_or([1.0, 0.0])
    }

    pub fn texture_srgb(&self, semantic: &str) -> bool {
        self.texture_color_spaces.get(semantic).copied().unwrap_or(matches!(semantic, "diffuse" | "emissive"))
    }
}

fn set_color_space(o: &mut ReadPreviewMaterial, semantic: &str, srgb: Option<bool>) {
    if let Some(srgb) = srgb { o.texture_color_spaces.insert(semantic.into(), srgb); }
    else { o.texture_color_spaces.remove(semantic); }
}

fn set_diffuse_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.diffuse_color = Some(c);
}
fn set_diffuse_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_diffuse_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.color_texture_transforms.insert("diffuse".into(), s.5);
    set_color_space(o, "diffuse", s.2);
    o.diffuse_texture = Some(s.0);
}
fn set_opacity_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_opacity_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.opacity = Some(s);
}
fn set_opacity_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.scalar_texture_transforms.insert("opacity".into(), s.4);
    set_color_space(o, "opacity", s.2);
    o.opacity_channel = s.1;
    o.opacity_texture = Some(s.0);
}
fn set_opacity_threshold_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_opacity_threshold_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.opacity_threshold = Some(s);
}
fn set_opacity_threshold_tex(_: &mut ReadPreviewMaterial, _: TextureInput) {}
fn set_rough_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_coat_c(o: &mut ReadPreviewMaterial, _: [f32; 3]) { o.warnings.push("clearcoat requires a scalar".into()); }
fn set_coat_s(o: &mut ReadPreviewMaterial, s: f32) { o.clearcoat = Some(s); }
fn set_coat_tex(o: &mut ReadPreviewMaterial, _: TextureInput) { o.warnings.push("clearcoat textures are unsupported".into()); }
fn set_coat_rough_c(o: &mut ReadPreviewMaterial, _: [f32; 3]) { o.warnings.push("clearcoat roughness requires a scalar".into()); }
fn set_coat_rough_s(o: &mut ReadPreviewMaterial, s: f32) { o.clearcoat_roughness = Some(s); }
fn set_coat_rough_tex(o: &mut ReadPreviewMaterial, _: TextureInput) { o.warnings.push("clearcoat roughness textures are unsupported".into()); }
fn set_rough_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.roughness = Some(s);
}
fn set_rough_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.scalar_texture_transforms.insert("roughness".into(), s.4);
    set_color_space(o, "roughness", s.2);
    o.roughness_texture = Some(s.0);
    o.roughness_channel = s.1;
}
fn set_metal_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_metal_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.metallic = Some(s);
}
fn set_metal_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.scalar_texture_transforms.insert("metallic".into(), s.4);
    set_color_space(o, "metallic", s.2);
    o.metallic_texture = Some(s.0);
    o.metallic_channel = s.1;
}
fn set_emissive_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.emissive_color = Some(c);
}
fn set_emissive_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_emissive_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.color_texture_transforms.insert("emissive".into(), s.5);
    set_color_space(o, "emissive", s.2);
    o.emissive_texture = Some(s.0);
}
fn set_ior_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_ior_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.ior = Some(s);
}
fn set_ior_tex(_: &mut ReadPreviewMaterial, _: TextureInput) {}
fn set_normal_c(o: &mut ReadPreviewMaterial, value: [f32; 3]) { o.normal = Some(value); }
fn set_normal_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_normal_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.normal_texture_transform = s.6.then_some(s.5);
    set_color_space(o, "normal", s.2);
    o.normal_texture = Some(s.0);
}
fn set_occlusion_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_occlusion_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_occlusion_tex(o: &mut ReadPreviewMaterial, s: TextureInput) {
    o.scalar_texture_transforms.insert("occlusion".into(), s.4);
    set_color_space(o, "occlusion", s.2);
    o.occlusion_channel = s.1;
    o.occlusion_texture = Some(s.0);
}

/// MaterialX `opacity` is a `color3`; fold to a luminance scalar.
fn set_opacity_mtlx_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.opacity = Some(0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]);
}

const PREVIEW_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuseColor",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    ("opacity", set_opacity_c, set_opacity_s, set_opacity_tex),
    (
        "opacityThreshold",
        set_opacity_threshold_c,
        set_opacity_threshold_s,
        set_opacity_threshold_tex,
    ),
    ("roughness", set_rough_c, set_rough_s, set_rough_tex),
    ("clearcoat", set_coat_c, set_coat_s, set_coat_tex),
    ("clearcoatRoughness", set_coat_rough_c, set_coat_rough_s, set_coat_rough_tex),
    ("metallic", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emissiveColor",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    ("ior", set_ior_c, set_ior_s, set_ior_tex),
    ("normal", set_normal_c, set_normal_s, set_normal_tex),
    (
        "occlusion",
        set_occlusion_c,
        set_occlusion_s,
        set_occlusion_tex,
    ),
];

const MATERIALX_STD_SURFACE_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    ("base_color", set_diffuse_c, set_diffuse_s, set_diffuse_tex),
    ("metalness", set_metal_c, set_metal_s, set_metal_tex),
    (
        "specular_roughness",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    (
        "emission_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "opacity",
        set_opacity_mtlx_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    ("normal", set_normal_c, set_normal_s, set_normal_tex),
];

const OMNIPBR_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuse_color_constant",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "diffuse_texture",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "reflection_roughness_constant",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    (
        "reflectionroughness_texture",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    ("metallic_constant", set_metal_c, set_metal_s, set_metal_tex),
    ("metallic_texture", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emissive_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "emissive_color_texture",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "opacity_constant",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "opacity_texture",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "normalmap_texture",
        set_normal_c,
        set_normal_s,
        set_normal_tex,
    ),
];

const OMNISURFACE_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuse_reflection_color",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "diffuse_reflection_color_image",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "geometry_normal_image",
        set_normal_c,
        set_normal_s,
        set_normal_tex,
    ),
    (
        "geometry_opacity_image",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "geometry_opacity",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    ("roughness", set_rough_c, set_rough_s, set_rough_tex),
    ("metalness", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emission_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "emission_color_image",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
];

#[derive(Debug, Clone, PartialEq)]
enum ResolvedValue {
    Color3([f32; 3]),
    Scalar(f32),
}

fn resolve_channel(
    stage: &Stage,
    material: &Path,
    shader: &Path,
    channel: &str,
    warnings: &mut Vec<String>,
    time: Option<f64>,
) -> anyhow::Result<(Option<ResolvedValue>, Option<TextureInput>)> {
    let mat_attr = format!("inputs:{channel}");
    let mat_path = material.append_property(&mat_attr)?;
    let (v, t) = resolve_attr_chain(stage, &mat_path, warnings, time)?;
    if v.is_some() || t.is_some() {
        return Ok((v, t));
    }
    let sh_path = shader.append_property(&mat_attr)?;
    resolve_attr_chain(stage, &sh_path, warnings, time)
}

fn resolve_attr_chain(
    stage: &Stage,
    attr_path: &Path,
    warnings: &mut Vec<String>,
    time: Option<f64>,
) -> anyhow::Result<(Option<ResolvedValue>, Option<TextureInput>)> {
    resolve_attr_chain_inner(stage, attr_path, &mut 256, warnings, time, 0)
}

fn resolve_attr_chain_inner(
    stage: &Stage,
    attr_path: &Path,
    remaining: &mut usize,
    warnings: &mut Vec<String>,
    time: Option<f64>,
    depth: usize,
) -> anyhow::Result<(Option<ResolvedValue>, Option<TextureInput>)> {
    anyhow::ensure!(depth < 32, "material graph exceeds the 32-level recursive traversal budget");
    let mut cur = attr_path.clone();
    for _ in 0..16 {
        anyhow::ensure!(*remaining > 0, "material graph exceeds the 256-input traversal budget");
        *remaining -= 1;
        if let Some(next) = connections_at(stage, &cur)?.into_iter().next() {
            let prim = next.prim_path();
            let kind = shader_kind(stage, &prim)?;
            match kind {
                ShaderKind::Texture => {
                    let channel = match next.as_str().rsplit(':').next() { Some("g") => 1, Some("b") => 2, Some("a") => 3, _ => 0 };
                    let srgb = read_texture_color_space(stage, &prim, time)?;
                    let rgba = texture_rgba_transform(stage, &prim, time)?;
                    let transform = [rgba[0][channel], rgba[1][channel]];
                    let rgb = rgba.map(|value| [value[0], value[1], value[2]]);
                    let signed_normal = read_token_or_string(stage, &prim, "info:id")?.as_deref() == Some("UsdUVTexture");
                    return Ok((None, read_texture_file(stage, &prim, time)?.map(|path| (path, channel, srgb, prim, transform, rgb, signed_normal))));
                }
                ShaderKind::NormalMap => {
                    let (value, texture) = resolve_attr_chain_inner(stage, &prim.append_property("inputs:in")?, remaining, warnings, time, depth + 1)?;
                    return Ok((value, texture.map(|mut texture| { texture.6 = false; texture })));
                }
                ShaderKind::Constant => {
                    let v_path = prim.append_property("inputs:value")?;
                    return resolve_attr_chain_inner(stage, &v_path, remaining, warnings, time, depth + 1);
                }
                ShaderKind::Multiply | ShaderKind::Add | ShaderKind::Subtract => {
                    let a = resolve_attr_chain_inner(stage, &prim.append_property("inputs:in1")?, remaining, warnings, time, depth + 1)?;
                    let b = resolve_attr_chain_inner(stage, &prim.append_property("inputs:in2")?, remaining, warnings, time, depth + 1)?;
                    if let (Some(a), Some(b)) = (&a.0, &b.0) {
                        let op = match kind { ShaderKind::Multiply => |a, b| a * b, ShaderKind::Add => |a, b| a + b, _ => |a, b| a - b };
                        return Ok((Some(combine(a, b, op)?), None));
                    }
                    warnings.push(format!("{prim}: textured or unresolved arithmetic is approximated by inputs:in1"));
                    return Ok(a);
                }
                ShaderKind::Mix => {
                    let fg = resolve_attr_chain_inner(stage, &prim.append_property("inputs:fg")?, remaining, warnings, time, depth + 1)?;
                    let bg = resolve_attr_chain_inner(stage, &prim.append_property("inputs:bg")?, remaining, warnings, time, depth + 1)?;
                    let weight = resolve_attr_chain_inner(stage, &prim.append_property("inputs:mix")?, remaining, warnings, time, depth + 1)?;
                    if let (Some(fg), Some(bg), Some(weight)) = (&fg.0, &bg.0, &weight.0) {
                        let difference = combine(fg, bg, |a, b| a - b)?;
                        let weighted = combine(&difference, weight, |a, b| a * b)?;
                        return Ok((Some(combine(bg, &weighted, |a, b| a + b)?), None));
                    }
                    warnings.push(format!("{prim}: textured or unresolved mix is approximated by inputs:bg"));
                    return Ok(bg);
                }
                ShaderKind::Unknown => {
                    cur = next;
                    continue;
                }
            }
        }
        let default = sampled_value(stage, &cur, time)?;
        match default.clone() {
            Some(Value::AssetPath(s)) => return Ok((None, Some((s.resolved_path().unwrap_or(s.as_str()).to_string(), 0, None, cur.prim_path(), [1.0, 0.0], [[1.0; 3], [0.0; 3]], false)))),
            Some(Value::String(s)) => return Ok((None, Some((s, 0, None, cur.prim_path(), [1.0, 0.0], [[1.0; 3], [0.0; 3]], false)))),
            _ => {}
        }
        return Ok((default.and_then(value_to_preview), None));
    }
    anyhow::bail!("material connection chain exceeds 16 links")
}

fn combine(a: &ResolvedValue, b: &ResolvedValue, op: fn(f32, f32) -> f32) -> anyhow::Result<ResolvedValue> {
    let result = match (a, b) {
        (ResolvedValue::Scalar(a), ResolvedValue::Scalar(b)) => ResolvedValue::Scalar(op(*a, *b)),
        _ => {
            let channels = |value: &ResolvedValue| match value { ResolvedValue::Scalar(v) => [*v; 3], ResolvedValue::Color3(v) => *v };
            let (a, b) = (channels(a), channels(b));
            ResolvedValue::Color3(std::array::from_fn(|i| op(a[i], b[i])))
        }
    };
    anyhow::ensure!(match &result { ResolvedValue::Scalar(v) => v.is_finite(), ResolvedValue::Color3(v) => v.iter().all(|v| v.is_finite()) },
        "nonfinite material graph result");
    Ok(result)
}

#[derive(Clone, Copy)]
enum ShaderKind {
    Texture,
    NormalMap,
    Constant,
    Multiply,
    Add,
    Subtract,
    Mix,
    Unknown,
}

fn shader_kind(stage: &Stage, prim: &Path) -> anyhow::Result<ShaderKind> {
    let id = read_token_or_string(stage, prim, "info:id")?;
    Ok(match id.as_deref() {
        Some("UsdUVTexture") => ShaderKind::Texture,
        Some(s) if s.starts_with("ND_image_") => ShaderKind::Texture,
        Some("ND_normalmap") => ShaderKind::NormalMap,
        Some(s) if s.starts_with("ND_constant_") => ShaderKind::Constant,
        Some(s) if s.starts_with("ND_multiply_") => ShaderKind::Multiply,
        Some(s) if s.starts_with("ND_add_") => ShaderKind::Add,
        Some(s) if s.starts_with("ND_subtract_") => ShaderKind::Subtract,
        Some(s) if s.starts_with("ND_mix_") => ShaderKind::Mix,
        _ => ShaderKind::Unknown,
    })
}

fn texture_input_attribute(stage: &Stage, tex_prim: &Path, name: &str) -> anyhow::Result<Option<openusd::usd::Attribute>> {
    use openusd_schemas::shade::{Connectable, ProducerFilter, Shader};
    let input = Shader::new(stage.prim(tex_prim)?).input(name);
    let connections = input.connected_sources()?;
    anyhow::ensure!(connections.invalid_source_paths().is_empty(), "invalid texture {name} connection at {}", input.path());
    let producers = input.value_producing_attributes(ProducerFilter::Any)?;
    anyhow::ensure!(producers.len() <= 1, "multiple texture {name} value producers at {}", input.path());
    anyhow::ensure!(connections.sources().is_empty() || !producers.is_empty(), "texture {name} connection has no value producer at {}", input.path());
    Ok(producers.first().map(|source| source.attribute().clone()))
}

fn read_texture_file(stage: &Stage, tex_prim: &Path, time: Option<f64>) -> anyhow::Result<Option<String>> {
    let Some(attribute) = texture_input_attribute(stage, tex_prim, "file")? else { return Ok(None); };
    Ok(match attribute.get_at::<Value>(time.map(openusd::usd::TimeCode::new))? {
        Some(Value::AssetPath(path)) => Some(path.resolved_path().unwrap_or(path.as_str()).to_owned()),
        Some(Value::String(path)) => Some(path),
        Some(Value::Token(path)) => Some(path.as_str().to_owned()),
        None if time.is_none() && !attribute.time_sample_times()?.is_empty() => None,
        _ => anyhow::bail!("texture file producer has no readable asset value at {}", attribute.path()),
    })
}

fn read_texture_color_space(stage: &Stage, tex_prim: &Path, time: Option<f64>) -> anyhow::Result<Option<bool>> {
    let Some(attribute) = texture_input_attribute(stage, tex_prim, "sourceColorSpace")? else { return Ok(None); };
    let value = match attribute.get_at::<Value>(time.map(openusd::usd::TimeCode::new))? {
        Some(Value::Token(value)) => value.as_str().to_owned(),
        Some(Value::String(value)) => value,
        None if time.is_none() && !attribute.time_sample_times()?.is_empty() => return Ok(None),
        _ => anyhow::bail!("texture sourceColorSpace producer has no readable token at {}", attribute.path()),
    };
    match value.as_str() {
        "raw" => Ok(Some(false)),
        "sRGB" => Ok(Some(true)),
        "auto" => Ok(None),
        _ => anyhow::bail!("unsupported texture sourceColorSpace: {value}"),
    }
}

#[test]
fn color_space_interfaces_discover_and_resolve_sampled_tokens() {
    let source = crate::UsdSource::snapshot("spaces.usda", br#"#usda 1.0
def Material "Mat" {
    token inputs:encoding.timeSamples = {0: "raw", 10: "sRGB", 20: "auto"}
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Tex.outputs:rgb>
        token outputs:surface
    }
    def NodeGraph "Graph" {
        token outputs:encoding.connect = </Mat.inputs:encoding>
        token outputs:cycle.connect = </Mat/Graph.outputs:cycle>
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @pixel.png@
        token inputs:sourceColorSpace.connect = </Mat/Graph.outputs:encoding>
    }
}
"#.as_slice()).unwrap();
    let stage = source.open_stage().unwrap();
    let material = openusd::sdf::path("/Mat").unwrap();
    let texture = openusd::sdf::path("/Mat/Tex").unwrap();
    assert_eq!(material_texture_sample_times(&stage, &material).unwrap(), [0.0, 10.0, 20.0]);
    assert_eq!(read_texture_color_space(&stage, &texture, None).unwrap(), None);
    for (time, expected) in [(0.0, Some(false)), (5.0, Some(false)), (10.0, Some(true)), (20.0, None), (0.0, Some(false))] {
        assert_eq!(read_texture_color_space(&stage, &texture, Some(time)).unwrap(), expected);
    }
    stage.attribute("/Mat.inputs:encoding").unwrap().set_at(Value::Token("acescg".into()), openusd::usd::TimeCode::new(10.0)).unwrap();
    assert!(read_texture_color_space(&stage, &texture, Some(10.0)).unwrap_err().to_string().contains("unsupported texture sourceColorSpace"));
    stage.attribute("/Mat/Tex.inputs:sourceColorSpace").unwrap().set_connections([openusd::sdf::path("/Mat/Graph.outputs:cycle").unwrap()]).unwrap();
    assert!(read_texture_color_space(&stage, &texture, Some(0.0)).unwrap_err().to_string().contains("no value producer"));
}

#[test]
fn texture_file_interfaces_discover_arbitrarily_named_samples() {
    let source = crate::UsdSource::snapshot("files.usda", br#"#usda 1.0
def Material "Mat" {
    asset inputs:image.timeSamples = {0: @a.png@, 10: @b.png@}
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Tex.outputs:rgb>
        token outputs:surface
    }
    def NodeGraph "Graph" {
        asset outputs:image.connect = </Mat.inputs:image>
        asset outputs:cycle.connect = </Mat/Graph.outputs:cycle>
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file.connect = </Mat/Graph.outputs:image>
    }
}
"#.as_slice()).unwrap();
    let stage = source.open_stage().unwrap();
    let material = openusd::sdf::path("/Mat").unwrap();
    let texture = openusd::sdf::path("/Mat/Tex").unwrap();
    assert_eq!(material_texture_sample_times(&stage, &material).unwrap(), [0.0, 10.0]);
    assert!(read_texture_file(&stage, &texture, None).unwrap().is_none());
    for (time, name) in [(0.0, "a.png"), (5.0, "a.png"), (10.0, "b.png"), (0.0, "a.png")] {
        assert!(read_texture_file(&stage, &texture, Some(time)).unwrap().unwrap().ends_with(name));
    }
    let file = stage.attribute("/Mat/Tex.inputs:file").unwrap();
    file.clone().set_connections([openusd::sdf::path("/Mat/Graph.outputs:cycle").unwrap()]).unwrap();
    assert!(read_texture_file(&stage, &texture, Some(0.0)).unwrap_err().to_string().contains("no value producer"));
    file.clone().set_connections([openusd::sdf::path("/Mat.inputs:missing").unwrap()]).unwrap();
    assert!(read_texture_file(&stage, &texture, Some(0.0)).unwrap_err().to_string().contains("invalid texture file connection"));
    stage.create_attribute("/Mat.inputs:other", "asset").unwrap().set(Value::AssetPath(openusd::sdf::AssetPath::new("c.png"))).unwrap();
    file.set_connections([openusd::sdf::path("/Mat.inputs:image").unwrap(), openusd::sdf::path("/Mat.inputs:other").unwrap()]).unwrap();
    assert!(read_texture_file(&stage, &texture, Some(0.0)).unwrap_err().to_string().contains("multiple texture file value producers"));
}

fn value_to_preview(v: Value) -> Option<ResolvedValue> {
    match v {
        Value::Float(f) => Some(ResolvedValue::Scalar(f)),
        Value::Double(d) => Some(ResolvedValue::Scalar(d as f32)),
        Value::Vec3f(c) => Some(ResolvedValue::Color3([c.x, c.y, c.z])),
        Value::Vec3d(c) => Some(ResolvedValue::Color3([c.x as f32, c.y as f32, c.z as f32])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preview_clearcoat_textures_report_unsupported_inputs() {
        let source = crate::UsdSource::snapshot("coat-texture.usda", br#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:clearcoat.connect = </Mat/Texture.outputs:r>
        float inputs:clearcoatRoughness.connect = </Mat/Texture.outputs:g>
        token outputs:surface
    }
    def Shader "Texture" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @coat.png@
        float outputs:r
        float outputs:g
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let read = super::read_preview_material(&stage, &openusd::sdf::path("/Mat").unwrap()).unwrap().unwrap();
        assert!(read.warnings.iter().any(|warning| warning == "clearcoat textures are unsupported"));
        assert!(read.warnings.iter().any(|warning| warning == "clearcoat roughness textures are unsupported"));
    }

    #[test]
    fn texture_times_follow_each_material_graph_and_surface_fallback() {
        use openusd::sdf::{Path, Value};
        let stage = openusd::usd::Stage::builder().in_memory("texture-graphs.usda").unwrap();
        for (name, times) in [("A", [0.0, 10.0]), ("B", [10.0, 20.0])] {
            stage.define_prim(format!("/{name}")).unwrap().set_type_name("Material").unwrap();
            stage.define_prim(format!("/{name}/Surface")).unwrap().set_type_name("Shader").unwrap();
            stage.create_attribute(format!("/{name}/Surface.info:id"), "token").unwrap()
                .set(Value::Token("UsdPreviewSurface".into())).unwrap();
            stage.define_prim(format!("/Texture{name}")).unwrap().set_type_name("Shader").unwrap();
            stage.create_attribute(format!("/{name}/Surface.inputs:diffuseColor"), "color3f").unwrap()
                .set_connections([Path::new(&format!("/Texture{name}.outputs:rgb")).unwrap()]).unwrap();
            let mut file = stage.create_attribute(format!("/Texture{name}.inputs:file"), "asset").unwrap();
            for time in times {
                file = file.set_at(Value::AssetPath(openusd::sdf::AssetPath::new("pixel.png")), openusd::usd::TimeCode::new(time)).unwrap();
            }
        }
        stage.define_prim("/A/Unused").unwrap().set_type_name("Shader").unwrap();
        stage.create_attribute("/A/Unused.inputs:file", "asset").unwrap()
            .set_at(Value::AssetPath(openusd::sdf::AssetPath::new("unused.png")), openusd::usd::TimeCode::new(999.0)).unwrap();
        stage.create_attribute("/TextureA.inputs:cycle", "float").unwrap()
            .set_connections([Path::new("/A/Surface.outputs:surface").unwrap()]).unwrap();
        let a = Path::new("/A").unwrap();
        let b = Path::new("/B").unwrap();
        assert_eq!(super::material_texture_sample_times(&stage, &a).unwrap(), [0.0, 10.0]);
        assert_eq!(super::material_texture_sample_times(&stage, &b).unwrap(), [10.0, 20.0]);
        stage.create_attribute("/A/Surface.inputs:emissiveColor", "color3f").unwrap()
            .set_connections([Path::new("/TextureB.outputs:rgb").unwrap()]).unwrap();
        assert_eq!(super::material_texture_sample_times(&stage, &a).unwrap(), [0.0, 10.0, 20.0]);
    }

    use super::*;

    #[test]
    fn material_animation_detection_follows_external_connections_and_cycles() {
        let source = crate::UsdSource::new("material-dependencies.usda", &br#"#usda 1.0
def Cube "Box" { rel material:binding = </Mat> }
def Material "Mat" {
    token outputs:surface.connect = </Surface.outputs:surface>
}
def Shader "Surface" {
    uniform token info:id = "UsdPreviewSurface"
    float inputs:roughness.connect = </External.outputs:out>
    token outputs:surface
}
def Shader "External" {
    uniform token info:id = "ND_constant_float"
    float inputs:value = 0.5
    float inputs:cycle.connect = </Surface.outputs:surface>
    float outputs:out
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let path = Path::new("/Box").unwrap();
        assert!(!bound_material_is_time_varying(&stage, &path));
        stage.prim("/External").unwrap().attribute("inputs:value")
            .set_at(Value::Float(0.25), openusd::usd::TimeCode::new(0.0)).unwrap()
            .set_at(Value::Float(0.75), openusd::usd::TimeCode::new(10.0)).unwrap();
        assert!(bound_material_is_time_varying(&stage, &path));
        let read = read_preview_material_at(&stage, &Path::new("/Mat").unwrap(), Some(5.0)).unwrap().unwrap();
        assert_eq!(read.roughness, Some(0.5));
    }
    #[test]
    fn disconnected_shader_samples_do_not_animate_bound_geometry() {
        let source = crate::UsdSource::new("disconnected-animation.usda", &br#"#usda 1.0
def Cube "Box" { rel material:binding = </Mat> }
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:roughness = 0.5
        token outputs:surface
    }
    def Shader "Unused" {
        uniform token info:id = "ND_constant_float"
        float inputs:value.timeSamples = {0: 0.25, 10: 0.75}
        float outputs:out
    }
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let path = Path::new("/Box").unwrap();
        assert!(!bound_material_is_time_varying(&stage, &path));
        assert!(!crate::live::prim_is_animated(&stage, &path));
        stage.prim("/Mat/Surface").unwrap().attribute("inputs:roughness")
            .set_connections([Path::new("/Mat/Unused.outputs:out").unwrap()]).unwrap();
        assert!(bound_material_is_time_varying(&stage, &path));
        assert!(crate::live::prim_is_animated(&stage, &path));
        stage.remove_property(Path::new("/Mat.outputs:surface").unwrap()).unwrap();
        assert!(bound_material_is_time_varying(&stage, &path));
    }

    use openusd::usd::Stage;

    #[test]
    fn bindings_resolve_inheritance_strength_and_preview_without_authoring() {
        use openusd_schemas::shade::{MaterialBindingAPI, BindingStrength};
        let source = crate::UsdSource::new("binding.usda", &b"#usda 1.0\ndef Xform \"Set\" { def Cube \"Child\" {} }\n"[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let parent = MaterialBindingAPI::new(stage.prim("/Set").unwrap());
        let child = MaterialBindingAPI::new(stage.prim("/Set/Child").unwrap());
        let path = Path::new("/Set/Child").unwrap();
        parent.bind("/Inherited").unwrap();
        assert_eq!(read_material_binding(&stage, &path).unwrap().unwrap().as_str(), "/Inherited");
        child.bind("/Local").unwrap();
        assert_eq!(read_material_binding(&stage, &path).unwrap().unwrap().as_str(), "/Local");
        parent.bind_for_purpose("", "/Strong", BindingStrength::StrongerThanDescendants).unwrap();
        assert_eq!(read_material_binding(&stage, &path).unwrap().unwrap().as_str(), "/Strong");
        parent.bind_for_purpose("preview", "/Preview", BindingStrength::WeakerThanDescendants).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        assert_eq!(read_material_binding(&stage, &path).unwrap().unwrap().as_str(), "/Preview");
        assert_eq!(read_material_binding_for_purpose(&stage, &path, "full").unwrap().unwrap().as_str(), "/Strong");
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn collection_binding_applies_only_to_members() {
        use openusd_schemas::shade::{MaterialBindingAPI, BindingStrength};
        let source = crate::UsdSource::new("collection.usda", &b"#usda 1.0\ndef Xform \"Set\" { def Cube \"A\" {} def Cube \"B\" {} }\n"[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let collection = openusd::usd::apply_collection(&stage, Path::new("/Set").unwrap(), "selected").unwrap();
        collection.include_path(&stage, Path::new("/Set/A").unwrap()).unwrap();
        let binding = MaterialBindingAPI::new(stage.prim("/Set").unwrap());
        binding.bind("/Default").unwrap();
        binding.bind_collection("selected", Path::new("/Set.collection:selected").unwrap(), Path::new("/Collection").unwrap(), "", BindingStrength::WeakerThanDescendants).unwrap();
        assert_eq!(read_material_binding(&stage, &Path::new("/Set/A").unwrap()).unwrap().unwrap().as_str(), "/Collection");
        assert_eq!(read_material_binding(&stage, &Path::new("/Set/B").unwrap()).unwrap().unwrap().as_str(), "/Default");
    }

    #[test]
    fn constant_math_graphs_evaluate_and_cycles_are_bounded() {
        let source = crate::UsdSource::new("math.usda", &br#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Sub.outputs:out>
        float inputs:roughness.connect = </Mat/Mix.outputs:out>
        token outputs:surface
    }
    def Shader "Mul" {
        uniform token info:id = "ND_multiply_color3FA"
        color3f inputs:in1 = (0.2, 0.4, 0.6)
        float inputs:in2 = 2
        color3f outputs:out
    }
    def Shader "Add" {
        uniform token info:id = "ND_add_color3FA"
        color3f inputs:in1.connect = </Mat/Mul.outputs:out>
        float inputs:in2 = 0.1
        color3f outputs:out
    }
    def Shader "Sub" {
        uniform token info:id = "ND_subtract_color3FA"
        color3f inputs:in1.connect = </Mat/Add.outputs:out>
        float inputs:in2 = 0.2
        color3f outputs:out
    }
    def Shader "Mix" {
        uniform token info:id = "ND_mix_float"
        float inputs:fg = 0.8
        float inputs:bg = 0.2
        float inputs:mix = 0.25
        float outputs:out
    }
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let material = read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap().unwrap();
        for (actual, expected) in material.diffuse_color.unwrap().into_iter().zip([0.3, 0.7, 1.1]) {
            assert!((actual - expected).abs() < 0.00001);
        }
        assert!((material.roughness.unwrap() - 0.35).abs() < 0.00001);
        assert!(material.warnings.is_empty());
        stage.define_prim("/Mat/Tex").unwrap().set_type_name("Shader").unwrap();
        stage.create_attribute("/Mat/Tex.info:id", "token").unwrap().set(Value::Token("UsdUVTexture".into())).unwrap();
        stage.create_attribute("/Mat/Tex.inputs:file", "asset").unwrap().set(Value::AssetPath(openusd::sdf::AssetPath::new("pixel.png"))).unwrap();
        stage.create_attribute("/Mat/Mul.inputs:in1", "color3f").unwrap()
            .set_connections([Path::new("/Mat/Tex.outputs:rgb").unwrap()]).unwrap();
        let approximate = read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap().unwrap();
        assert_eq!(approximate.warnings.len(), 3);
        assert!(approximate.warnings.iter().any(|warning| warning.contains("/Mat/Mul") && warning.contains("inputs:in1")));
        assert!(approximate.diffuse_texture.is_some());
        let mut editor = crate::editor::EditorSession::new(stage.clone());
        editor.select(Some("/Mat".into())).unwrap();
        assert_eq!(editor.snapshot().unwrap().material_warnings, approximate.warnings);
        stage.create_attribute("/Mat/Mul.inputs:in1", "color3f").unwrap()
            .set_connections([Path::new("/Mat/Sub.outputs:out").unwrap()]).unwrap();
        let error = read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap_err();
        assert!(error.to_string().contains("traversal budget"));
        assert!(editor.snapshot().unwrap().material_warnings[0].contains("Material read failed"));
    }

    #[test]
    fn graph_arithmetic_rejects_nonfinite_outputs() {
        assert!(combine(&ResolvedValue::Scalar(f32::MAX), &ResolvedValue::Scalar(2.0), |a, b| a * b).is_err());
        assert_eq!(combine(&ResolvedValue::Scalar(2.0), &ResolvedValue::Color3([1.0, 2.0, 3.0]), |a, b| a - b).unwrap(),
            ResolvedValue::Color3([1.0, 0.0, -1.0]));
    }

    #[test]
    fn reads_uv_transform_from_transform2d() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("uv.usda").unwrap();
        // Material with a preview surface + a UsdTransform2d node on the st chain.
        stage.define_prim("/Mat").unwrap().set_type_name("Material").unwrap();
        stage
            .create_attribute("/Mat.outputs:surface", "token")
            .unwrap()
            .set_connections([Path::new("/Mat/Surface.outputs:surface").unwrap()])
            .unwrap();
        let surf = stage.define_prim("/Mat/Surface").unwrap();
        surf.set_type_name("Shader").unwrap();
        stage
            .create_attribute("/Mat/Surface.info:id", "token")
            .unwrap()
            .set(Value::Token("UsdPreviewSurface".into()))
            .unwrap();
        stage.define_prim("/Mat/Xf").unwrap().set_type_name("Shader").unwrap();
        stage
            .create_attribute("/Mat/Xf.info:id", "token")
            .unwrap()
            .set(Value::Token("UsdTransform2d".into()))
            .unwrap();
        stage
            .create_attribute("/Mat/Xf.inputs:scale", "float2")
            .unwrap()
            .set(Value::Vec2f([2.0f32, 3.0f32].into()))
            .unwrap();
        stage
            .create_attribute("/Mat/Xf.inputs:translation", "float2")
            .unwrap()
            .set(Value::Vec2f([0.5f32, 0.25f32].into()))
            .unwrap();
        stage
            .create_attribute("/Mat/Xf.inputs:rotation", "float")
            .unwrap()
            .set(Value::Float(90.0))
            .unwrap();

        assert!(read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap().unwrap().uv_transform.is_none());
        stage.define_prim("/Mat/Tex").unwrap().set_type_name("Shader").unwrap();
        stage.create_attribute("/Mat/Tex.info:id", "token").unwrap().set(Value::Token("UsdUVTexture".into())).unwrap();
        stage.create_attribute("/Mat/Tex.inputs:file", "asset").unwrap().set(Value::AssetPath(openusd::sdf::AssetPath::new("pixel.png"))).unwrap();
        stage.create_attribute("/Mat/Surface.inputs:diffuseColor", "color3f").unwrap()
            .set_connections([Path::new("/Mat/Tex.outputs:rgb").unwrap()]).unwrap();
        stage.create_attribute("/Mat/Tex.inputs:st", "float2").unwrap()
            .set_connections([Path::new("/Mat/Xf.outputs:result").unwrap()]).unwrap();

        let read = read_preview_material(&stage, &Path::new("/Mat").unwrap())
            .unwrap()
            .expect("material");
        let uv = read.uv_transform.expect("uv transform");
        let expected = bevy::math::Affine2::from_scale_angle_translation([2.0, 3.0].into(), 90_f32.to_radians(), [0.5, 0.25].into());
        assert!(uv.abs_diff_eq(expected, 1e-6));

        stage.define_prim("/Elsewhere").unwrap().set_type_name("Shader").unwrap();
        stage.create_attribute("/Elsewhere.info:id", "token").unwrap().set(Value::Token("UsdTransform2d".into())).unwrap();
        stage.create_attribute("/Elsewhere.inputs:translation", "float2").unwrap().set(Value::Vec2f([0.1, 0.2].into())).unwrap();
        stage.attribute("/Mat/Tex.inputs:st").unwrap().set_connections([Path::new("/Elsewhere.outputs:result").unwrap()]).unwrap();
        assert_eq!(read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap().unwrap().uv_transform.unwrap().translation, bevy::math::Vec2::new(0.1, 0.2));
        stage.create_attribute("/Elsewhere.inputs:in", "float2").unwrap().set_connections([Path::new("/Mat/Xf.outputs:result").unwrap()]).unwrap();
        let chain = read_preview_material(&stage, &Path::new("/Mat").unwrap()).unwrap().unwrap().uv_transform.unwrap();
        assert!(chain.transform_point2(bevy::math::Vec2::splat(0.25)).abs_diff_eq(bevy::math::Vec2::new(-0.15, 0.95), 1e-6));
    }

    #[test]
    fn coordinate_graph_rejects_cycles_and_conflicting_texture_transforms() {
        let stage = Stage::builder().in_memory("uv-graphs.usda").unwrap();
        for path in ["/A", "/B", "/Transform", "/Graph"] { stage.define_prim(path).unwrap(); }
        stage.create_attribute("/Transform.info:id", "token").unwrap().set(Value::Token("UsdTransform2d".into())).unwrap();
        stage.create_attribute("/Transform.inputs:translation", "float2").unwrap().set(Value::Vec2f([0.0, 0.5].into())).unwrap();
        stage.create_attribute("/A.inputs:st", "float2").unwrap().set_connections([Path::new("/Transform.outputs:result").unwrap()]).unwrap();
        let textures = [Path::new("/A").unwrap(), Path::new("/B").unwrap()];
        assert!(read_uv_transform(&stage, &textures, None).unwrap_err().to_string().contains("per-texture"));
        stage.create_attribute("/B.inputs:st", "float2").unwrap().set_connections([Path::new("/Transform.outputs:result").unwrap()]).unwrap();
        assert_eq!(read_uv_transform(&stage, &textures, None).unwrap().unwrap().translation, bevy::math::Vec2::new(0.0, 0.5));
        stage.attribute("/Transform.inputs:translation").unwrap().set(Value::Vec2f([f32::NAN, 0.0].into())).unwrap();
        assert!(read_uv_transform(&stage, &textures, None).unwrap_err().to_string().contains("non-finite"));
        stage.attribute("/A.inputs:st").unwrap().set_connections([Path::new("/Graph.outputs:st").unwrap()]).unwrap();
        stage.create_attribute("/Graph.outputs:st", "float2").unwrap().set_connections([Path::new("/A.inputs:st").unwrap()]).unwrap();
        assert!(read_uv_transform(&stage, &textures[..1], None).unwrap_err().to_string().contains("16 connections"));
    }

    #[test]
    fn uv_transform_chains_preserve_nonorthogonal_affine_axes() {
        use bevy::math::{Affine2, Vec2};
        let stage = Stage::builder().in_memory("uv-shear.usda").unwrap();
        for path in ["/Texture", "/Outer", "/Inner"] { stage.define_prim(path).unwrap(); }
        for path in ["/Outer", "/Inner"] {
            stage.create_attribute(format!("{path}.info:id"), "token").unwrap().set(Value::Token("UsdTransform2d".into())).unwrap();
        }
        stage.create_attribute("/Outer.inputs:scale", "float2").unwrap().set(Value::Vec2f([2.0, 3.0].into())).unwrap();
        stage.create_attribute("/Inner.inputs:rotation", "float").unwrap().set(Value::Float(37.0)).unwrap();
        stage.create_attribute("/Texture.inputs:st", "float2").unwrap().set_connections([Path::new("/Outer.outputs:result").unwrap()]).unwrap();
        stage.create_attribute("/Outer.inputs:in", "float2").unwrap().set_connections([Path::new("/Inner.outputs:result").unwrap()]).unwrap();
        let actual = read_uv_transform(&stage, &[Path::new("/Texture").unwrap()], None).unwrap().unwrap();
        let expected = Affine2::from_scale(Vec2::new(2.0, 3.0)) * Affine2::from_angle(37_f32.to_radians());
        assert!(actual.abs_diff_eq(expected, 1e-6));
        assert!(actual.matrix2.x_axis.dot(actual.matrix2.y_axis).abs() > 1.0);
        for point in [Vec2::ZERO, Vec2::ONE, Vec2::new(-0.5, 0.25)] {
            let (sin, cos) = 37_f32.to_radians().sin_cos();
            let expected = Vec2::new((cos * point.x - sin * point.y) * 2.0, (sin * point.x + cos * point.y) * 3.0);
            assert!(actual.transform_point2(point).abs_diff_eq(expected, 1e-6));
        }
    }

    #[test]
    fn no_transform2d_yields_none() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("uv2.usda").unwrap();
        stage.define_prim("/Mat").unwrap().set_type_name("Material").unwrap();
        stage
            .create_attribute("/Mat.outputs:surface", "token")
            .unwrap()
            .set_connections([Path::new("/Mat/Surface.outputs:surface").unwrap()])
            .unwrap();
        stage.define_prim("/Mat/Surface").unwrap().set_type_name("Shader").unwrap();
        stage
            .create_attribute("/Mat/Surface.info:id", "token")
            .unwrap()
            .set(Value::Token("UsdPreviewSurface".into()))
            .unwrap();

        let read = read_preview_material(&stage, &Path::new("/Mat").unwrap())
            .unwrap()
            .expect("material");
        assert!(read.uv_transform.is_none());
    }
}
