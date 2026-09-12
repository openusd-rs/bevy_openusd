//! Scalar texture packing for Bevy's metallic/roughness and occlusion layouts.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use crate::asset::SnapshotTextures;
use crate::read::shade::ReadPreviewMaterial;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Plane {
    width: u32,
    height: u32,
    values: Vec<u8>,
}

#[derive(Resource, Default)]
pub(crate) struct PackedTextures {
    images: std::collections::HashMap<(Option<Plane>, Option<Plane>, Option<Plane>), Handle<Image>>,
    bytes: usize,
}

#[derive(Resource, Default)]
struct AlphaTextures {
    images: std::collections::HashMap<(u32, u32, bool, Vec<u8>), Handle<Image>>,
    bytes: usize,
}

#[derive(Resource, Default)]
struct PackedInputs(std::collections::VecDeque<(blake3::Hash, bevy::asset::AssetId<Image>)>);

pub(super) fn configure(app: &mut App) { app.add_systems(Last, prune_caches); }

fn prune_caches(packed: Option<ResMut<PackedTextures>>, alpha: Option<ResMut<AlphaTextures>>,
    inputs: Option<ResMut<PackedInputs>>, assets: Option<Res<Assets<Image>>>) {
    let Some(assets) = assets else { return };
    if let Some(mut inputs) = inputs { inputs.0.retain(|(_, id)| assets.contains(*id)); }
    if let Some(mut cache) = packed {
        cache.images.retain(|_, handle| assets.contains(handle.id()) && super::cache::externally_owned(handle));
        cache.bytes = cache.images.keys().map(|key| {
            let planes = [key.0.as_ref(), key.1.as_ref(), key.2.as_ref()];
            let image_bytes = planes.iter().flatten().next().map_or(0, |plane| plane.values.len() * 4);
            image_bytes + planes.iter().flatten().map(|plane| plane.values.len()).sum::<usize>()
        }).sum();
    }
    if let Some(mut cache) = alpha {
        cache.images.retain(|_, handle| assets.contains(handle.id()) && super::cache::externally_owned(handle));
        cache.bytes = cache.images.keys().map(|key| key.3.len() * 2).sum();
    }
}

pub(crate) fn base_color_alpha(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    let Some(alpha) = plane(world, &read.opacity_texture, read.opacity_channel, read.texture_srgb("opacity"), read.scalar_texture_transform("opacity"))? else { return Ok(None) };
    let transformed = super::color_texture::transformed(world, read, "diffuse")?;
    let color = if let Some(path) = &read.diffuse_texture {
        let textures = world.get_resource::<SnapshotTextures>().ok_or_else(|| anyhow::anyhow!("alpha packing requires loaded snapshot textures"))?;
        let handle = transformed.as_ref().or_else(|| textures.0.get(&(path.clone(), read.texture_srgb("diffuse"))))
            .ok_or_else(|| anyhow::anyhow!("missing diffuse texture: {path}"))?;
        let image = world.resource::<Assets<Image>>().get(handle).ok_or_else(|| anyhow::anyhow!("diffuse image is not ready: {path}"))?;
        let size = image.texture_descriptor.size;
        anyhow::ensure!(image.texture_descriptor.dimension == TextureDimension::D2 && size.depth_or_array_layers == 1,
            "alpha packing requires a 2D diffuse image");
        anyhow::ensure!((size.width, size.height) == (alpha.width, alpha.height), "diffuse and opacity textures have different resolutions");
        anyhow::ensure!(matches!(image.sampler, bevy::image::ImageSampler::Default), "alpha packing requires a shared default sampler");
        Some(image)
    } else { None };
    let float = transformed.is_some();
    let data = alpha_pixels(color, float, &alpha)?;
    let key = (alpha.width, alpha.height, float, data);
    if let Some(handle) = world.get_resource::<AlphaTextures>().and_then(|cache| cache.images.get(&key)) {
        if world.resource::<Assets<Image>>().contains(handle) { return Ok(Some(handle.clone())); }
    }
    let image = Image::new(Extent3d { width: alpha.width, height: alpha.height, depth_or_array_layers: 1 },
        TextureDimension::D2, key.3.clone(), if float { TextureFormat::Rgba16Float } else { TextureFormat::Rgba8UnormSrgb }, bevy::asset::RenderAssetUsages::default());
    let handle = super::generated_image::intern(world, image);
    world.init_resource::<AlphaTextures>();
    let mut cache = world.resource_mut::<AlphaTextures>();
    let bytes = key.3.len() * 2;
    if cache.bytes + bytes > 64 * 1024 * 1024 {
        cache.images.clear();
        cache.bytes = 0;
    }
    if bytes <= 64 * 1024 * 1024 {
        cache.images.insert(key, handle.clone());
        cache.bytes += bytes;
    }
    Ok(Some(handle))
}

fn alpha_pixels(color: Option<&Image>, float: bool, alpha: &Plane) -> anyhow::Result<Vec<u8>> {
    if !float && let Some(data) = rgba8_alpha(color, &alpha.values) { return Ok(data); }
    let mut data = Vec::with_capacity(alpha.values.len() * if float { 8 } else { 4 });
    for y in 0..alpha.height {
        for x in 0..alpha.width {
            if float {
                let color = color.unwrap().get_color_at(x, y)?.to_linear();
                super::color_texture::append_rgba(&mut data, [color.red, color.green, color.blue,
                    alpha.values[(y * alpha.width + x) as usize] as f32 / 255.0])?;
                continue;
            }
            let rgb = if let Some(image) = color {
                let color = image.get_color_at(x, y)?.to_srgba();
                [color.red, color.green, color.blue].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            } else { [255; 3] };
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha.values[(y * alpha.width + x) as usize]]);
        }
    }
    Ok(data)
}

fn rgba8_alpha(color: Option<&Image>, alpha: &[u8]) -> Option<Vec<u8>> {
    let Some(image) = color else {
        return Some(alpha.iter().flat_map(|&value| [255, 255, 255, value]).collect());
    };
    let srgb = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return None,
    };
    let pixels = image.data.as_ref()?.get(..alpha.len().checked_mul(4)?)?;
    let table: [u8; 256] = std::array::from_fn(|byte| {
        if srgb { return byte as u8; }
        let value = byte as f32 / 255.0;
        (Color::linear_rgb(value, value, value).to_srgba().red.clamp(0.0, 1.0) * 255.0).round() as u8
    });
    Some(pixels.chunks_exact(4).zip(alpha).flat_map(|(pixel, &alpha)|
        [table[pixel[0] as usize], table[pixel[1] as usize], table[pixel[2] as usize], alpha]).collect())
}

fn plane(world: &World, path: &Option<String>, channel: usize, srgb: bool, [scale, bias]: [f32; 2]) -> anyhow::Result<Option<Plane>> {
    let Some(path) = path else { return Ok(None) };
    anyhow::ensure!(channel < 4, "invalid scalar texture channel");
    anyhow::ensure!(scale.is_finite() && bias.is_finite(), "nonfinite scalar texture scale/bias");
    let textures = world.get_resource::<SnapshotTextures>().ok_or_else(|| anyhow::anyhow!("scalar packing requires loaded snapshot textures"))?;
    let handle = textures.0.get(&(path.clone(), srgb)).ok_or_else(|| anyhow::anyhow!("missing scalar texture: {path}"))?;
    let image = world.resource::<Assets<Image>>().get(handle).ok_or_else(|| anyhow::anyhow!("scalar image is not ready: {path}"))?;
    let size = image.texture_descriptor.size;
    anyhow::ensure!(size.depth_or_array_layers == 1 && image.texture_descriptor.dimension == TextureDimension::D2,
        "scalar packing requires a 2D image");
    anyhow::ensure!(matches!(image.sampler, bevy::image::ImageSampler::Default), "scalar packing requires a shared default sampler");
    let count = u64::from(size.width) * u64::from(size.height);
    anyhow::ensure!(count > 0 && count <= 16_777_216, "scalar image exceeds the 16M pixel packing limit");
    if let Some(values) = rgba8_plane(image, count as usize, channel, scale, bias)? {
        return Ok(Some(Plane { width: size.width, height: size.height, values }));
    }
    let mut values = Vec::with_capacity(count as usize);
    for y in 0..size.height {
        for x in 0..size.width {
            let color = image.get_color_at(x, y)?.to_linear();
            let value = [color.red, color.green, color.blue, color.alpha][channel] * scale + bias;
            anyhow::ensure!(value.is_finite(), "nonfinite scalar texture value");
            values.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok(Some(Plane { width: size.width, height: size.height, values }))
}

fn rgba8_plane(image: &Image, count: usize, channel: usize, scale: f32, bias: f32) -> anyhow::Result<Option<Vec<u8>>> {
    let srgb = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return Ok(None),
    };
    let Some(data) = image.data.as_ref().and_then(|data| data.get(..count.checked_mul(4)?)) else { return Ok(None) };
    let mut table = [0; 256];
    for (byte, output) in table.iter_mut().enumerate() {
        let value = byte as f32 / 255.0;
        let linear = if srgb && channel < 3 { Color::srgb(value, value, value).to_linear().red } else { value };
        let value = linear * scale + bias;
        if !value.is_finite() { return Ok(None); }
        *output = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    Ok(Some(data.chunks_exact(4).map(|pixel| table[pixel[channel] as usize]).collect()))
}

pub(crate) fn metallic_roughness(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    if read.roughness_texture.is_none() && read.metallic_texture.is_none() { return Ok(None); }
    let rough = plane(world, &read.roughness_texture, read.roughness_channel, read.texture_srgb("roughness"), read.scalar_texture_transform("roughness"))?;
    let metal = plane(world, &read.metallic_texture, read.metallic_channel, read.texture_srgb("metallic"), read.scalar_texture_transform("metallic"))?;
    pack(world, rough, metal, None)
}

pub(crate) fn occlusion(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    let occlusion = plane(world, &read.occlusion_texture, read.occlusion_channel, read.texture_srgb("occlusion"), read.scalar_texture_transform("occlusion"))?;
    if occlusion.is_none() { return Ok(None); }
    pack(world, None, None, occlusion)
}

pub(crate) fn clearcoat(world: &mut World, read: &ReadPreviewMaterial, roughness: bool) -> anyhow::Result<Option<Handle<Image>>> {
    let (path, channel, semantic) = if roughness {
        (&read.clearcoat_roughness_texture, read.clearcoat_roughness_channel, "clearcoat_roughness")
    } else { (&read.clearcoat_texture, read.clearcoat_channel, "clearcoat") };
    let value = plane(world, path, channel, read.texture_srgb(semantic), read.scalar_texture_transform(semantic))?;
    if value.is_none() { return Ok(None); }
    if roughness { pack(world, value, None, None) } else { pack(world, None, None, value) }
}

fn pack(world: &mut World, rough: Option<Plane>, metal: Option<Plane>, occlusion: Option<Plane>) -> anyhow::Result<Option<Handle<Image>>> {
    let first = rough.as_ref().or(metal.as_ref()).or(occlusion.as_ref()).unwrap();
    let (width, height) = (first.width, first.height);
    if let (Some(a), Some(b)) = (&rough, &metal) {
        anyhow::ensure!((a.width, a.height) == (b.width, b.height), "scalar textures have different resolutions");
    }
    let key = (rough, metal, occlusion);
    let mut fingerprint = blake3::Hasher::new();
    fingerprint.update(&width.to_le_bytes()).update(&height.to_le_bytes());
    for plane in [&key.0, &key.1, &key.2] {
        if let Some(plane) = plane {
            anyhow::ensure!((plane.width, plane.height) == (width, height), "scalar textures have different resolutions");
            anyhow::ensure!(plane.values.len() == width as usize * height as usize, "scalar plane payload has the wrong length");
            fingerprint.update(&[1]).update(&plane.values);
        } else { fingerprint.update(&[0]); }
    }
    let fingerprint = fingerprint.finalize();
    let template = Image::new_uninit(Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2, TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default());
    let candidate = world.get_resource::<PackedInputs>().and_then(|cache|
        cache.0.iter().find_map(|(hash, id)| (*hash == fingerprint).then_some(*id)));
    if let Some(id) = candidate {
        if let Some(handle) = super::generated_image::reuse_if(world, id, &template,
            |data| packed_pixels_match(data, width as usize * height as usize, &key)) {
            return Ok(Some(handle));
        }
    }
    if let Some(handle) = world.get_resource::<PackedTextures>().and_then(|cache| cache.images.get(&key)) {
        let id = handle.id();
        if let Some(handle) = super::generated_image::reuse_if(world, id, &template,
            |data| packed_pixels_match(data, width as usize * height as usize, &key)) {
            return Ok(Some(handle));
        }
    }
    let count = width as usize * height as usize;
    let mut data = Vec::with_capacity(count * 4);
    for i in 0..count {
        data.extend_from_slice(&[key.2.as_ref().map_or(255, |p| p.values[i]), key.0.as_ref().map_or(255, |p| p.values[i]), key.1.as_ref().map_or(255, |p| p.values[i]), 255]);
    }
    let image = Image::new(Extent3d { width, height, depth_or_array_layers: 1 }, TextureDimension::D2,
        data, TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default());
    let handle = super::generated_image::intern(world, image);
    world.init_resource::<PackedInputs>();
    let mut inputs = world.resource_mut::<PackedInputs>();
    inputs.0.retain(|(hash, _)| *hash != fingerprint);
    if inputs.0.len() == 4096 { inputs.0.pop_front(); }
    inputs.0.push_back((fingerprint, handle.id()));
    world.init_resource::<PackedTextures>();
    let mut cache = world.resource_mut::<PackedTextures>();
    let bytes = count * 4 + key.0.as_ref().map_or(0, |p| p.values.len()) + key.1.as_ref().map_or(0, |p| p.values.len()) + key.2.as_ref().map_or(0, |p| p.values.len());
    if cache.bytes + bytes > 64 * 1024 * 1024 {
        cache.images.clear();
        cache.bytes = 0;
    }
    if bytes <= 64 * 1024 * 1024 {
        cache.images.insert(key, handle.clone());
        cache.bytes += bytes;
    }
    Ok(Some(handle))
}

fn packed_pixels_match(data: &[u8], count: usize, planes: &(Option<Plane>, Option<Plane>, Option<Plane>)) -> bool {
    data.len() == count * 4 && data.chunks_exact(4).enumerate().all(|(i, pixel)| {
        pixel == [planes.2.as_ref().map_or(255, |p| p.values[i]),
            planes.0.as_ref().map_or(255, |p| p.values[i]),
            planes.1.as_ref().map_or(255, |p| p.values[i]), 255]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_input_reuse_checks_live_output_and_source_changes() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let mut plane = Plane { width: 1, height: 1, values: vec![60] };
        let a = pack(&mut world, Some(plane.clone()), None, None).unwrap().unwrap();
        world.resource_mut::<Assets<Image>>().get_mut(&a).unwrap().data = Some(vec![0; 4]);
        let b = pack(&mut world, Some(plane.clone()), None, None).unwrap().unwrap();
        assert_ne!(a.id(), b.id());
        assert_eq!(pack(&mut world, Some(plane.clone()), None, None).unwrap().unwrap().id(), b.id());
        world.resource_mut::<Assets<Image>>().get_mut(&b).unwrap().sampler = bevy::image::ImageSampler::nearest();
        let c = pack(&mut world, Some(plane.clone()), None, None).unwrap().unwrap();
        assert_ne!(b.id(), c.id());
        plane.values[0] = 61;
        let d = pack(&mut world, Some(plane), None, None).unwrap().unwrap();
        assert_ne!(c.id(), d.id());
        assert_eq!(world.resource::<Assets<Image>>().get(&d).unwrap().data.as_deref(), Some([255, 61, 255, 255].as_slice()));
    }

    #[test]
    fn large_packed_images_share_beyond_the_pixel_cache_budget() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let plane = Plane { width: 4096, height: 4096, values: vec![255; 4096 * 4096] };
        let a = pack(&mut world, Some(plane.clone()), None, None).unwrap().unwrap();
        let b = pack(&mut world, Some(plane), None, None).unwrap().unwrap();
        assert_eq!(a.id(), b.id());
        assert_eq!(world.resource::<Assets<Image>>().len(), 1);
        assert_eq!(world.resource::<PackedTextures>().bytes, 0);
    }

    #[test]
    fn rgba8_alpha_lookup_matches_pixel_conversion_and_live_edits() {
        let alpha: Vec<_> = (0..=255_u8).rev().collect();
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
            let data = (0..=255_u8).flat_map(|value| [value, value.rotate_left(2), 255-value, 17]).collect();
            let mut image = Image::new(Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                TextureDimension::D2, data, format, bevy::asset::RenderAssetUsages::default());
            for edited in [false, true] {
                if edited { image.data.as_mut().unwrap().reverse(); }
                let actual = rgba8_alpha(Some(&image), &alpha).unwrap();
                let expected: Vec<_> = (0..256).flat_map(|i| {
                    let color = image.get_color_at(i % 16, i / 16).unwrap().to_srgba();
                    let [r, g, b] = [color.red, color.green, color.blue].map(|value|
                        (value.clamp(0.0, 1.0) * 255.0).round() as u8);
                    [r, g, b, alpha[i as usize]]
                }).collect();
                assert_eq!(actual, expected, "{format:?} edited={edited}");
            }
            assert!(rgba8_alpha(Some(&image), &[0; 257]).is_none());
            image.data = None;
            assert!(rgba8_alpha(Some(&image), &alpha).is_none());
        }
        assert_eq!(rgba8_alpha(None, &[0, 127, 255]).unwrap(), [255,255,255,0,255,255,255,127,255,255,255,255]);
        let image = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![0; 8], TextureFormat::Rgba16Float, bevy::asset::RenderAssetUsages::default());
        assert!(rgba8_alpha(Some(&image), &[255]).is_none());
    }

    #[test]
    fn rgba8_scalar_lookup_matches_pixel_conversion() {
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
            let mut data = Vec::new();
            for byte in 0..=255_u8 { data.extend_from_slice(&[byte, 255-byte, byte.rotate_left(3), byte.rotate_right(2)]); }
            let image = Image::new(Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                TextureDimension::D2, data, format, bevy::asset::RenderAssetUsages::default());
            for channel in 0..4 {
                for [scale, bias] in [[1.0, 0.0], [2.0, -1.0], [-0.7, 0.8], [0.0, 0.5]] {
                    let actual = rgba8_plane(&image, 256, channel, scale, bias).unwrap().unwrap();
                    let expected: Vec<_> = (0..256).map(|i| {
                        let color = image.get_color_at(i % 16, i / 16).unwrap().to_linear();
                        let value = [color.red, color.green, color.blue, color.alpha][channel] * scale + bias;
                        (value.clamp(0.0, 1.0) * 255.0).round() as u8
                    }).collect();
                    assert_eq!(actual, expected, "{format:?} channel={channel} scale={scale} bias={bias}");
                }
            }
        }
    }

    #[test]
    fn rgba8_scalar_lookup_defers_unsupported_inputs() {
        let mut image = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![0; 4], TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default());
        assert!(rgba8_plane(&image, 1, 0, f32::MAX, f32::MAX).unwrap().is_none());
        assert!(rgba8_plane(&image, 2, 0, 1.0, 0.0).unwrap().is_none());
        image.data = None;
        assert!(rgba8_plane(&image, 1, 0, 1.0, 0.0).unwrap().is_none());
        image.texture_descriptor.format = TextureFormat::Rgba16Float;
        assert!(rgba8_plane(&image, 1, 0, 1.0, 0.0).unwrap().is_none());
    }

    #[test]
    fn pruning_packed_and_alpha_images_preserves_owners_and_byte_counts() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let plane = Plane { width: 1, height: 1, values: vec![128] };
        let packed = pack(&mut world, Some(plane.clone()), Some(plane.clone()), None).unwrap().unwrap();
        let source = world.resource_mut::<Assets<Image>>().add(Image::new(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2,
            vec![128; 4], TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default()));
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("alpha.png".into(), false), source);
        world.insert_resource(textures);
        let read = ReadPreviewMaterial { opacity_texture: Some("alpha.png".into()), ..default() };
        let alpha = base_color_alpha(&mut world, &read).unwrap().unwrap();
        world.run_system_once(prune_caches).unwrap();
        assert_eq!(world.resource::<PackedTextures>().bytes, 6);
        assert_eq!(world.resource::<AlphaTextures>().bytes, 8);
        assert_eq!(pack(&mut world, Some(plane.clone()), Some(plane), None).unwrap().unwrap(), packed);
        assert_eq!(base_color_alpha(&mut world, &read).unwrap().unwrap(), alpha);
        world.resource_mut::<Assets<Image>>().remove(packed.id());
        world.run_system_once(prune_caches).unwrap();
        assert_eq!(world.resource::<PackedTextures>().bytes, 0);
        drop(alpha);
        world.run_system_once(prune_caches).unwrap();
        assert_eq!(world.resource::<AlphaTextures>().bytes, 0);
        assert!(world.resource::<AlphaTextures>().images.is_empty());
    }

    #[test]
    fn scalar_scale_bias_uses_linear_values_before_quantization() {
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![128; 4], TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default()));
        world.insert_resource(images);
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("srgb.png".into(), true), handle);
        world.insert_resource(textures);
        let path = Some("srgb.png".into());
        assert_eq!(plane(&world, &path, 0, true, [2.0, 0.0]).unwrap().unwrap().values, [110]);
        assert_eq!(plane(&world, &path, 3, true, [0.5, 0.0]).unwrap().unwrap().values, [64]);
        assert_eq!(plane(&world, &path, 0, true, [-1.0, 0.0]).unwrap().unwrap().values, [0]);
        assert_eq!(plane(&world, &path, 0, true, [10.0, 1.0]).unwrap().unwrap().values, [255]);
        assert!(plane(&world, &path, 0, true, [1.0, f32::INFINITY]).is_err());
    }

    #[test]
    fn scalar_scale_bias_flows_from_usd_into_packed_channels() {
        let source = crate::UsdSource::snapshot("scalar-transform.usda", br#"#usda 1.0
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:roughness.connect = </Mat/Tex.outputs:r>
        float inputs:metallic.connect = </Mat/Tex.outputs:g>
        float inputs:occlusion.connect = </Mat/Tex.outputs:b>
        float inputs:opacity.connect = </Mat/Tex.outputs:a>
        token outputs:surface
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @scalar.png@
        token inputs:sourceColorSpace = "raw"
        float4 inputs:scale.timeSamples = {0: (-1, 0.5, 2, 0.25), 10: (0, 0, 0, 0)}
        float4 inputs:bias = (1, 0, -1, 0.1)
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let read_at = |time| crate::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Mat").unwrap(), Some(time)).unwrap().unwrap();
        let read = read_at(0.0);
        assert_eq!(read.scalar_texture_transform("roughness"), [-1.0, 1.0]);
        assert_eq!(read.scalar_texture_transform("metallic"), [0.5, 0.0]);
        assert_eq!(read.scalar_texture_transform("occlusion"), [2.0, -1.0]);
        assert_eq!(read.scalar_texture_transform("opacity"), [0.25, 0.1]);
        assert_eq!(read_at(5.0).scalar_texture_transform("roughness"), [-0.5, 1.0]);
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![64, 128, 192, 255], TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default()));
        world.insert_resource(images);
        let mut textures = SnapshotTextures::default();
        textures.0.insert((read.roughness_texture.clone().unwrap(), false), handle);
        world.insert_resource(textures);
        let packed = metallic_roughness(&mut world, &read).unwrap().unwrap();
        assert_eq!(world.resource::<Assets<Image>>().get(&packed).unwrap().data.as_deref(), Some([255, 191, 64, 255].as_slice()));
        let occ = occlusion(&mut world, &read).unwrap().unwrap();
        assert_eq!(world.resource::<Assets<Image>>().get(&occ).unwrap().data.as_deref(), Some([129, 255, 255, 255].as_slice()));
        let alpha = base_color_alpha(&mut world, &read).unwrap().unwrap();
        assert_eq!(world.resource::<Assets<Image>>().get(&alpha).unwrap().data.as_deref(), Some([255, 255, 255, 89].as_slice()));
        let later = metallic_roughness(&mut world, &read_at(10.0)).unwrap().unwrap();
        assert_ne!(packed, later);
        assert_eq!(world.resource::<Assets<Image>>().get(&later).unwrap().data.as_deref(), Some([255, 255, 0, 255].as_slice()));
        assert_eq!(metallic_roughness(&mut world, &read_at(0.0)).unwrap(), Some(packed));
        let mut invalid = read;
        invalid.scalar_texture_transforms.insert("roughness".into(), [f32::NAN, 0.0]);
        assert!(metallic_roughness(&mut world, &invalid).is_err());
        stage.attribute("/Mat/Tex.inputs:scale").unwrap().set_connections([openusd::sdf::path("/Mat.inputs:scale").unwrap()]).unwrap();
        assert!(crate::read::shade::read_preview_material_at(&stage, &openusd::sdf::path("/Mat").unwrap(), Some(0.0)).unwrap_err().to_string().contains("connected texture scale/bias"));
    }

    #[test]
    fn alpha_packing_preserves_rgb_replaces_alpha_and_reuses_content() {
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let alpha = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![48, 96, 144, 192], TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default()));
        let rgb = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![32, 64, 128, 16], TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default()));
        world.insert_resource(images);
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("alpha.png".into(), false), alpha);
        textures.0.insert(("rgb.png".into(), true), rgb.clone());
        world.insert_resource(textures);
        for (channel, expected) in [48, 96, 144, 192].into_iter().enumerate() {
            let read = ReadPreviewMaterial { diffuse_texture: Some("rgb.png".into()), opacity_texture: Some("alpha.png".into()), opacity_channel: channel, ..Default::default() };
            let packed = base_color_alpha(&mut world, &read).unwrap().unwrap();
            let image = world.resource::<Assets<Image>>().get(&packed).unwrap();
            assert_eq!(image.data.as_deref(), Some([32, 64, 128, expected].as_slice()));
            assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
            assert_eq!(base_color_alpha(&mut world, &read).unwrap(), Some(packed));
        }
        let read = ReadPreviewMaterial { opacity_texture: Some("alpha.png".into()), ..Default::default() };
        let packed = base_color_alpha(&mut world, &read).unwrap().unwrap();
        assert_eq!(world.resource::<Assets<Image>>().get(&packed).unwrap().data.as_deref(), Some([255, 255, 255, 48].as_slice()));
        world.resource_mut::<Assets<Image>>().get_mut(&rgb).unwrap().texture_descriptor.size.width = 2;
        let read = ReadPreviewMaterial { diffuse_texture: Some("rgb.png".into()), ..read };
        assert!(base_color_alpha(&mut world, &read).is_err());
    }

    #[test]
    fn absent_channel_is_neutral_and_packed_assets_are_reused() {
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![48, 96, 144, 255], TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default()));
        world.insert_resource(images);
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("metal.png".into(), false), handle);
        world.insert_resource(textures);
        let read = ReadPreviewMaterial { metallic_texture: Some("metal.png".into()), ..Default::default() };
        let packed = metallic_roughness(&mut world, &read).unwrap().unwrap();
        assert_eq!(world.resource::<Assets<Image>>().get(&packed).unwrap().data.as_deref(), Some([255, 255, 48, 255].as_slice()));
        assert_eq!(metallic_roughness(&mut world, &read).unwrap(), Some(packed));
        assert_eq!(world.resource::<Assets<Image>>().len(), 2);
        let invalid = ReadPreviewMaterial { metallic_channel: 5, ..read };
        assert!(metallic_roughness(&mut world, &invalid).is_err());
    }

    #[test]
    fn occlusion_channels_repack_to_red_and_cache_separately() {
        let mut world = World::new();
        let mut images = Assets::<Image>::default();
        let handle = images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![48, 96, 144, 192], TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default()));
        world.insert_resource(images);
        let mut textures = SnapshotTextures::default();
        textures.0.insert(("scalar.png".into(), false), handle);
        world.insert_resource(textures);
        for (channel, expected) in [48, 96, 144, 192].into_iter().enumerate() {
            let read = ReadPreviewMaterial { occlusion_texture: Some("scalar.png".into()), occlusion_channel: channel, ..Default::default() };
            let packed = occlusion(&mut world, &read).unwrap().unwrap();
            assert_eq!(world.resource::<Assets<Image>>().get(&packed).unwrap().data.as_deref(), Some([expected, 255, 255, 255].as_slice()));
            assert_eq!(occlusion(&mut world, &read).unwrap(), Some(packed));
        }
        assert!(occlusion(&mut world, &ReadPreviewMaterial::default()).unwrap().is_none());
        let missing = ReadPreviewMaterial { occlusion_texture: Some("missing.png".into()), ..Default::default() };
        assert!(occlusion(&mut world, &missing).is_err());
    }
}
