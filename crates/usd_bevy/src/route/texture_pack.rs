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
    images: std::collections::HashMap<(u32, u32, Vec<u8>), Handle<Image>>,
    bytes: usize,
}

pub(crate) fn base_color_alpha(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    let Some(alpha) = plane(world, &read.opacity_texture, read.opacity_channel, read.texture_srgb("opacity"))? else { return Ok(None) };
    let color = if let Some(path) = &read.diffuse_texture {
        let textures = world.get_resource::<SnapshotTextures>().ok_or_else(|| anyhow::anyhow!("alpha packing requires loaded snapshot textures"))?;
        let handle = textures.0.get(&(path.clone(), read.texture_srgb("diffuse"))).ok_or_else(|| anyhow::anyhow!("missing diffuse texture: {path}"))?;
        let image = world.resource::<Assets<Image>>().get(handle).ok_or_else(|| anyhow::anyhow!("diffuse image is not ready: {path}"))?;
        let size = image.texture_descriptor.size;
        anyhow::ensure!(image.texture_descriptor.dimension == TextureDimension::D2 && size.depth_or_array_layers == 1,
            "alpha packing requires a 2D diffuse image");
        anyhow::ensure!((size.width, size.height) == (alpha.width, alpha.height), "diffuse and opacity textures have different resolutions");
        anyhow::ensure!(matches!(image.sampler, bevy::image::ImageSampler::Default), "alpha packing requires a shared default sampler");
        Some(image)
    } else { None };
    let mut data = Vec::with_capacity(alpha.values.len() * 4);
    for y in 0..alpha.height {
        for x in 0..alpha.width {
            let rgb = if let Some(image) = color {
                let color = image.get_color_at(x, y)?.to_srgba();
                [color.red, color.green, color.blue].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            } else { [255; 3] };
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha.values[(y * alpha.width + x) as usize]]);
        }
    }
    let key = (alpha.width, alpha.height, data);
    if let Some(handle) = world.get_resource::<AlphaTextures>().and_then(|cache| cache.images.get(&key)) {
        if world.resource::<Assets<Image>>().contains(handle) { return Ok(Some(handle.clone())); }
    }
    let image = Image::new(Extent3d { width: alpha.width, height: alpha.height, depth_or_array_layers: 1 },
        TextureDimension::D2, key.2.clone(), TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default());
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    world.init_resource::<AlphaTextures>();
    let mut cache = world.resource_mut::<AlphaTextures>();
    let bytes = key.2.len() * 2;
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

fn plane(world: &World, path: &Option<String>, channel: usize, srgb: bool) -> anyhow::Result<Option<Plane>> {
    let Some(path) = path else { return Ok(None) };
    anyhow::ensure!(channel < 4, "invalid scalar texture channel");
    let textures = world.get_resource::<SnapshotTextures>().ok_or_else(|| anyhow::anyhow!("scalar packing requires loaded snapshot textures"))?;
    let handle = textures.0.get(&(path.clone(), srgb)).ok_or_else(|| anyhow::anyhow!("missing scalar texture: {path}"))?;
    let image = world.resource::<Assets<Image>>().get(handle).ok_or_else(|| anyhow::anyhow!("scalar image is not ready: {path}"))?;
    let size = image.texture_descriptor.size;
    anyhow::ensure!(size.depth_or_array_layers == 1 && image.texture_descriptor.dimension == TextureDimension::D2,
        "scalar packing requires a 2D image");
    anyhow::ensure!(matches!(image.sampler, bevy::image::ImageSampler::Default), "scalar packing requires a shared default sampler");
    let count = u64::from(size.width) * u64::from(size.height);
    anyhow::ensure!(count > 0 && count <= 16_777_216, "scalar image exceeds the 16M pixel packing limit");
    let mut values = Vec::with_capacity(count as usize);
    for y in 0..size.height {
        for x in 0..size.width {
            let color = image.get_color_at(x, y)?.to_linear();
            let value = [color.red, color.green, color.blue, color.alpha][channel];
            anyhow::ensure!(value.is_finite(), "nonfinite scalar texture value");
            values.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok(Some(Plane { width: size.width, height: size.height, values }))
}

pub(crate) fn metallic_roughness(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    if read.roughness_texture.is_none() && read.metallic_texture.is_none() { return Ok(None); }
    let rough = plane(world, &read.roughness_texture, read.roughness_channel, read.texture_srgb("roughness"))?;
    let metal = plane(world, &read.metallic_texture, read.metallic_channel, read.texture_srgb("metallic"))?;
    pack(world, rough, metal, None)
}

pub(crate) fn occlusion(world: &mut World, read: &ReadPreviewMaterial) -> anyhow::Result<Option<Handle<Image>>> {
    let occlusion = plane(world, &read.occlusion_texture, read.occlusion_channel, read.texture_srgb("occlusion"))?;
    if occlusion.is_none() { return Ok(None); }
    pack(world, None, None, occlusion)
}

fn pack(world: &mut World, rough: Option<Plane>, metal: Option<Plane>, occlusion: Option<Plane>) -> anyhow::Result<Option<Handle<Image>>> {
    let first = rough.as_ref().or(metal.as_ref()).or(occlusion.as_ref()).unwrap();
    let (width, height) = (first.width, first.height);
    if let (Some(a), Some(b)) = (&rough, &metal) {
        anyhow::ensure!((a.width, a.height) == (b.width, b.height), "scalar textures have different resolutions");
    }
    let key = (rough, metal, occlusion);
    if let Some(handle) = world.get_resource::<PackedTextures>().and_then(|cache| cache.images.get(&key)) {
        if world.resource::<Assets<Image>>().contains(handle) { return Ok(Some(handle.clone())); }
    }
    let count = width as usize * height as usize;
    let mut data = Vec::with_capacity(count * 4);
    for i in 0..count {
        data.extend_from_slice(&[key.2.as_ref().map_or(255, |p| p.values[i]), key.0.as_ref().map_or(255, |p| p.values[i]), key.1.as_ref().map_or(255, |p| p.values[i]), 255]);
    }
    let image = Image::new(Extent3d { width, height, depth_or_array_layers: 1 }, TextureDimension::D2,
        data, TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default());
    let handle = world.resource_mut::<Assets<Image>>().add(image);
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

#[cfg(test)]
mod tests {
    use super::*;

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
