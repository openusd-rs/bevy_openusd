use bevy::{prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat}};
use crate::{asset::SnapshotTextures, read::shade::ReadPreviewMaterial};

#[derive(Resource, Default)]
struct ColorTextures {
    images: std::collections::HashMap<(u32, u32, Vec<u8>), Handle<Image>>,
    bytes: usize,
}

pub(super) fn configure(app: &mut App) { app.add_systems(Last, prune_cache); }

fn prune_cache(cache: Option<ResMut<ColorTextures>>, assets: Option<Res<Assets<Image>>>) {
    let (Some(mut cache), Some(assets)) = (cache, assets) else { return };
    cache.images.retain(|_, handle| assets.contains(handle.id()) && super::cache::externally_owned(handle));
    cache.bytes = cache.images.keys().map(|key| key.2.len() * 2).sum();
}

pub(super) fn append_rgba(data: &mut Vec<u8>, rgba: [f32; 4]) -> anyhow::Result<()> {
    for value in rgba {
        let value = half::f16::from_f32(value);
        anyhow::ensure!(value.is_finite(), "color texture value exceeds finite float16 range");
        data.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

pub(super) fn transformed(world: &mut World, read: &ReadPreviewMaterial, semantic: &str) -> anyhow::Result<Option<Handle<Image>>> {
    if semantic == "normal" && read.normal_texture.is_none() && let Some(normal) = read.normal {
        anyhow::ensure!(normal.iter().all(|v| v.is_finite() && (-1.0..=1.0).contains(v))
            && normal.iter().any(|v| *v != 0.0), "constant normal must be finite, nonzero and within [-1,1]");
        if normal == [0.0, 0.0, 1.0] { return Ok(None); }
        let mut data = Vec::with_capacity(8);
        append_rgba(&mut data, [normal[0] * 0.5 + 0.5, normal[1] * 0.5 + 0.5, normal[2] * 0.5 + 0.5, 1.0])?;
        anyhow::ensure!(data[..6].chunks_exact(2).any(|bytes| half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32() != 0.5),
            "constant normal becomes zero at float16 precision");
        return cached(world, (1, 1, data)).map(Some);
    }
    let [scale, bias] = if semantic == "normal" {
        let Some([scale, bias]) = read.normal_texture_transform else { return Ok(None); };
        [scale.map(|v| v * 0.5), bias.map(|v| v * 0.5 + 0.5)]
    } else { read.color_texture_transform(semantic) };
    if scale == [1.0; 3] && bias == [0.0; 3] { return Ok(None); }
    let path = match semantic { "diffuse" => &read.diffuse_texture, "emissive" => &read.emissive_texture, "normal" => &read.normal_texture,
        _ => anyhow::bail!("unsupported color texture semantic: {semantic}") };
    let Some(path) = path else { return Ok(None); };
    anyhow::ensure!(scale.iter().chain(&bias).all(|v| v.is_finite()), "nonfinite color texture scale/bias");
    let textures = world.get_resource::<SnapshotTextures>().ok_or_else(|| anyhow::anyhow!("color transform requires loaded snapshot textures"))?;
    let handle = textures.0.get(&(path.clone(), read.texture_srgb(semantic))).ok_or_else(|| anyhow::anyhow!("missing color texture: {path}"))?;
    let image = world.resource::<Assets<Image>>().get(handle).ok_or_else(|| anyhow::anyhow!("color image is not ready: {path}"))?;
    let size = image.texture_descriptor.size;
    anyhow::ensure!(image.texture_descriptor.dimension == TextureDimension::D2 && size.depth_or_array_layers == 1,
        "color transform requires a 2D image");
    anyhow::ensure!(matches!(image.sampler, bevy::image::ImageSampler::Default), "color transform requires a shared default sampler");
    let count = u64::from(size.width) * u64::from(size.height);
    anyhow::ensure!(count > 0 && count <= 16_777_216, "color image exceeds the 16M pixel transform limit");
    if let Some(data) = rgba8_transformed(image, count as usize, scale, bias) {
        return cached(world, (size.width, size.height, data)).map(Some);
    }
    let mut data = Vec::with_capacity(count as usize * 8);
    for y in 0..size.height {
        for x in 0..size.width {
            let color = image.get_color_at(x, y)?.to_linear();
            append_rgba(&mut data, [color.red * scale[0] + bias[0], color.green * scale[1] + bias[1],
                color.blue * scale[2] + bias[2], color.alpha])?;
        }
    }
    cached(world, (size.width, size.height, data)).map(Some)
}

fn rgba8_transformed(image: &Image, count: usize, scale: [f32; 3], bias: [f32; 3]) -> Option<Vec<u8>> {
    let srgb = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return None,
    };
    let pixels = image.data.as_ref()?.get(..count.checked_mul(4)?)?;
    let mut table = [[[0; 2]; 256]; 4];
    for byte in 0..256 {
        let value = byte as f32 / 255.0;
        let linear = if srgb { Color::srgb(value, value, value).to_linear().red } else { value };
        for channel in 0..4 {
            let value = if channel == 3 { value } else { linear * scale[channel] + bias[channel] };
            let value = half::f16::from_f32(value);
            if !value.is_finite() { return None; }
            table[channel][byte] = value.to_le_bytes();
        }
    }
    let mut data = Vec::with_capacity(count.checked_mul(8)?);
    for pixel in pixels.chunks_exact(4) {
        for channel in 0..4 { data.extend_from_slice(&table[channel][pixel[channel] as usize]); }
    }
    Some(data)
}

fn cached(world: &mut World, key: (u32, u32, Vec<u8>)) -> anyhow::Result<Handle<Image>> {
    if let Some(handle) = world.get_resource::<ColorTextures>().and_then(|cache| cache.images.get(&key)) {
        if world.resource::<Assets<Image>>().contains(handle) { return Ok(handle.clone()); }
    }
    let image = Image::new(Extent3d { width: key.0, height: key.1, depth_or_array_layers: 1 },
        TextureDimension::D2, key.2.clone(), TextureFormat::Rgba16Float, bevy::asset::RenderAssetUsages::default());
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    world.init_resource::<ColorTextures>();
    let mut cache = world.resource_mut::<ColorTextures>();
    let bytes = key.2.len() * 2;
    if cache.bytes + bytes > 64 * 1024 * 1024 { cache.images.clear(); cache.bytes = 0; }
    if bytes <= 64 * 1024 * 1024 { cache.images.insert(key, handle.clone()); cache.bytes += bytes; }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba8_transfer_tables_match_float_conversion_and_source_edits() {
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
            let data = (0..=255_u8).flat_map(|v| [v, 255-v, v.rotate_left(2), v.rotate_right(3)]).collect();
            let mut image = Image::new(Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                TextureDimension::D2, data, format, bevy::asset::RenderAssetUsages::default());
            for edited in [false, true] {
                if edited { image.data.as_mut().unwrap().reverse(); }
                for [scale, bias] in [[[2.0,3.0,4.0], [0.1,-0.8,2.0]], [[0.5;3], [0.5;3]], [[-2.0;3], [1.0;3]]] {
                    let mut expected = Vec::new();
                    for i in 0..256 {
                        let c = image.get_color_at(i % 16, i / 16).unwrap().to_linear();
                        append_rgba(&mut expected, [c.red*scale[0]+bias[0], c.green*scale[1]+bias[1],
                            c.blue*scale[2]+bias[2], c.alpha]).unwrap();
                    }
                    assert_eq!(rgba8_transformed(&image, 256, scale, bias).unwrap(), expected);
                }
            }
            assert!(rgba8_transformed(&image, 257, [1.0;3], [0.0;3]).is_none());
            assert!(rgba8_transformed(&image, 256, [1e10;3], [0.0;3]).is_none());
            image.data = None;
            assert!(rgba8_transformed(&image, 256, [1.0;3], [0.0;3]).is_none());
        }
    }

    #[test]
    fn pruning_preserves_shared_images_and_recounts_payload() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let read = ReadPreviewMaterial { normal: Some([0.0, 0.5, 0.5]), ..default() };
        let handle = transformed(&mut world, &read, "normal").unwrap().unwrap();
        world.run_system_once(prune_cache).unwrap();
        assert_eq!(world.resource::<ColorTextures>().bytes, 16);
        assert_eq!(transformed(&mut world, &read, "normal").unwrap().unwrap(), handle);
        world.resource_mut::<Assets<Image>>().remove(handle.id());
        world.run_system_once(prune_cache).unwrap();
        assert_eq!(world.resource::<ColorTextures>().bytes, 0);
        drop(transformed(&mut world, &read, "normal").unwrap());
        world.run_system_once(prune_cache).unwrap();
        assert!(world.resource::<ColorTextures>().images.is_empty());
        assert_eq!(world.resource::<ColorTextures>().bytes, 0);
    }

    #[test]
    fn animated_conversions_release_images_after_material_owners_disappear() {
        for flat in [false, true] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::UsdPlugin));
            app.init_asset::<StandardMaterial>().init_asset::<Image>();
            if flat { app.add_plugins(super::super::gpu_skin::UsdGpuSkinningPlugin); }
            let entity = app.world_mut().spawn_empty().id();
            for step in 0..1000 {
                let read = ReadPreviewMaterial { normal: Some([step as f32 / 1000.0, 0.5, 0.5]), ..default() };
                let image = transformed(app.world_mut(), &read, "normal").unwrap().unwrap();
                let material = super::super::cache::intern_material(app.world_mut(),
                    StandardMaterial { normal_map_texture: Some(image), ..default() });
                app.world_mut().entity_mut(entity).insert(MeshMaterial3d(material));
                if flat { super::super::flat_material::attach(app.world_mut(), entity); }
                app.update();
                assert!(app.world().resource::<Assets<Image>>().len() <= 8);
            }
            app.world_mut().despawn(entity);
            for _ in 0..8 { app.update(); }
            assert_eq!(app.world().resource::<Assets<Image>>().len(), 0);
            assert_eq!(app.world().resource::<ColorTextures>().bytes, 0);
        }
    }

    #[test]
    fn constant_normals_encode_cache_and_reject_invalid_vectors() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let mut read = ReadPreviewMaterial { normal: Some([0.0, 0.5, 0.5]), ..Default::default() };
        let first = transformed(&mut world, &read, "normal").unwrap().unwrap();
        assert_eq!(pixel(&world, &first), [0.5, 0.75, 0.75, 1.0]);
        read.normal = Some([0.0, -0.5, 0.5]);
        let other = transformed(&mut world, &read, "normal").unwrap().unwrap();
        assert_ne!(first, other);
        assert_eq!(pixel(&world, &other), [0.5, 0.25, 0.75, 1.0]);
        read.normal = Some([0.0, 0.5, 0.5]);
        assert_eq!(transformed(&mut world, &read, "normal").unwrap(), Some(first));
        read.normal = Some([0.0, 0.0, 1.0]);
        assert!(transformed(&mut world, &read, "normal").unwrap().is_none());
        for normal in [[0.0; 3], [f32::NAN, 0.0, 1.0], [f32::INFINITY, 0.0, 1.0], [2.0, 0.0, 1.0], [1e-9, 0.0, 0.0]] {
            read.normal = Some(normal);
            assert!(transformed(&mut world, &read, "normal").is_err());
        }
    }

    fn world_with_pixel(srgb: bool, bytes: [u8; 4]) -> World {
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, bytes.to_vec(), if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
            bevy::asset::RenderAssetUsages::default()));
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("color.png".into(), srgb), handle);
        world.insert_resource(images);
        world.insert_resource(textures);
        world
    }

    fn pixel(world: &World, handle: &Handle<Image>) -> [f32; 4] {
        let image = world.resource::<Assets<Image>>().get(handle).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
        std::array::from_fn(|i| {
            let data = image.data.as_ref().unwrap();
            half::f16::from_le_bytes([data[i * 2], data[i * 2 + 1]]).to_f32()
        })
    }

    #[test]
    fn rgb_transform_is_linear_hdr_and_content_cached() {
        let mut world = world_with_pixel(true, [128, 128, 128, 64]);
        let mut read = ReadPreviewMaterial { emissive_texture: Some("color.png".into()), ..default() };
        assert!(transformed(&mut world, &read, "emissive").unwrap().is_none());
        let transform = [[2.0, 3.0, 4.0], [0.1, -0.8, 2.0]];
        read.color_texture_transforms.insert("emissive".into(), transform);
        let first = transformed(&mut world, &read, "emissive").unwrap().unwrap();
        let actual = pixel(&world, &first);
        let linear = ((128.0_f32 / 255.0 + 0.055) / 1.055).powf(2.4);
        for i in 0..3 { assert!((actual[i] - (linear * transform[0][i] + transform[1][i])).abs() < 0.002); }
        assert!(actual[1] < 0.0 && actual[2] > 1.0);
        assert!((actual[3] - 64.0 / 255.0).abs() < 0.001);
        assert_eq!(transformed(&mut world, &read, "emissive").unwrap(), Some(first.clone()));
        read.color_texture_transforms.insert("emissive".into(), [[1.0; 3], [1.0; 3]]);
        assert_ne!(transformed(&mut world, &read, "emissive").unwrap(), Some(first.clone()));
        read.color_texture_transforms.insert("emissive".into(), transform);
        assert_eq!(transformed(&mut world, &read, "emissive").unwrap(), Some(first));
        for invalid in [f32::NAN, f32::INFINITY, 1.0e10] {
            read.color_texture_transforms.insert("emissive".into(), [[invalid; 3], [0.0; 3]]);
            assert!(transformed(&mut world, &read, "emissive").is_err());
        }
    }

    #[test]
    fn signed_normal_transforms_encode_for_bevy_without_double_decoding() {
        let mut world = world_with_pixel(false, [255; 4]);
        let mut read = ReadPreviewMaterial { normal_texture: Some("color.png".into()), ..default() };
        assert!(transformed(&mut world, &read, "normal").unwrap().is_none());
        read.normal_texture_transform = Some([[2.0; 3], [-1.0; 3]]);
        assert!(transformed(&mut world, &read, "normal").unwrap().is_none());
        read.normal_texture_transform = Some([[0.0, 0.5, 0.5], [0.0; 3]]);
        let handle = transformed(&mut world, &read, "normal").unwrap().unwrap();
        assert_eq!(pixel(&world, &handle), [0.5, 0.75, 0.75, 1.0]);
        read.normal_texture_transform = Some([[1.0; 3], [0.0; 3]]);
        let handle = transformed(&mut world, &read, "normal").unwrap().unwrap();
        assert_eq!(pixel(&world, &handle), [1.0; 4]);
    }

    #[test]
    fn opacity_packing_preserves_transformed_hdr_rgb() {
        let mut world = world_with_pixel(false, [255,255,255,64]);
        let mut read = ReadPreviewMaterial { diffuse_texture: Some("color.png".into()),
            opacity_texture: Some("color.png".into()), opacity_channel: 3, ..default() };
        read.texture_color_spaces.insert("diffuse".into(), false);
        read.color_texture_transforms.insert("diffuse".into(), [[2.0, 1.0, 0.5], [0.0; 3]]);
        let handle = super::super::texture_pack::base_color_alpha(&mut world, &read).unwrap().unwrap();
        let rgba = pixel(&world, &handle);
        assert_eq!(&rgba[..3], &[2.0, 1.0, 0.5]);
        assert!((rgba[3] - 64.0 / 255.0).abs() < 0.001);
        assert_eq!(super::super::texture_pack::base_color_alpha(&mut world, &read).unwrap(), Some(handle));
    }

    #[test]
    fn sampled_rgb_interfaces_reach_both_color_semantics() {
        let source = crate::UsdSource::snapshot("rgb.usda", br#"#usda 1.0
def Material "Mat" {
    float4 inputs:gain.timeSamples = {0: (1,2,3,4), 10: (3,4,5,6)}
    float4 inputs:offset = (0.1,0.2,0.3,0.4)
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Tex.outputs:rgb>
        color3f inputs:emissiveColor.connect = </Mat/Tex.outputs:rgb>
        normal3f inputs:normal.connect = </Mat/Tex.outputs:rgb>
        token outputs:surface
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @color.png@
        float4 inputs:scale.connect = </Mat.inputs:gain>
        float4 inputs:bias.connect = </Mat.inputs:offset>
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        for (time, gain) in [(0.0, 1.0), (5.0, 2.0), (10.0, 3.0), (0.0, 1.0)] {
            let read = crate::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Mat").unwrap(), Some(time)).unwrap().unwrap();
            for semantic in ["diffuse", "emissive"] {
                assert_eq!(read.color_texture_transform(semantic), [[gain, gain + 1.0, gain + 2.0], [0.1,0.2,0.3]]);
            }
            assert_eq!(read.normal_texture_transform, Some([[gain, gain + 1.0, gain + 2.0], [0.1,0.2,0.3]]));
        }
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }
}
