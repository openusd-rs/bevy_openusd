//! How material textures are sampled: the `wrapS`/`wrapT` their texture
//! nodes author, trilinear and anisotropic filtering, and a mip chain built
//! on the CPU, so tiled textures repeat and stay calm at a distance.
//!
//! Textures are finalised once their materials are built: the packing and
//! colour transforms read the decoded images first, at their top level.

use crate::read::shade::{ReadPreviewMaterial, TextureWrap};
use bevy::asset::AssetId;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureDimension, TextureFormat};

/// Anisotropy every USD material texture samples with.
const ANISOTROPY: u16 = 16;

#[derive(Resource, Default)]
struct TextureSampling {
    pending: HashMap<AssetId<Image>, [TextureWrap; 2]>,
    done: HashSet<AssetId<Image>>,
}

pub(super) fn configure(app: &mut App) {
    app.init_resource::<TextureSampling>()
        .add_systems(PostUpdate, finalise);
}

/// Notes how each of a built material's textures samples, from the wraps of
/// the texture nodes it came from.
pub(super) fn note(world: &mut World, material: &StandardMaterial, read: &ReadPreviewMaterial) {
    let wrap = |paths: &[&Option<String>]| {
        paths
            .iter()
            .find_map(|path| path.as_ref().and_then(|path| read.texture_wraps.get(path)))
            .copied()
            .unwrap_or_default()
    };
    let slots = [
        (
            &material.base_color_texture,
            wrap(&[&read.diffuse_texture, &read.opacity_texture]),
        ),
        (&material.normal_map_texture, wrap(&[&read.normal_texture])),
        (&material.emissive_texture, wrap(&[&read.emissive_texture])),
        (
            &material.metallic_roughness_texture,
            wrap(&[&read.roughness_texture, &read.metallic_texture]),
        ),
        (
            &material.occlusion_texture,
            wrap(&[&read.occlusion_texture]),
        ),
        (
            &material.clearcoat_texture,
            wrap(&[&read.clearcoat_texture]),
        ),
        (
            &material.clearcoat_roughness_texture,
            wrap(&[&read.clearcoat_roughness_texture]),
        ),
    ];
    let Some(mut sampling) = world.get_resource_mut::<TextureSampling>() else {
        return;
    };
    for (handle, wrap) in slots {
        if let Some(handle) = handle
            && !sampling.done.contains(&handle.id())
        {
            sampling.pending.insert(handle.id(), wrap);
        }
    }
}

fn finalise(mut sampling: ResMut<TextureSampling>, images: Option<ResMut<Assets<Image>>>) {
    let Some(mut images) = images else { return };
    if sampling.pending.is_empty() {
        sampling.done.retain(|id| images.contains(*id));
        return;
    }
    let pending: Vec<_> = sampling.pending.drain().collect();
    for (id, wrap) in pending {
        let Some(mut image) = images.get_mut(id) else {
            continue;
        };
        add_mips(&mut image);
        image.sampler = sampler(wrap);
        sampling.done.insert(id);
    }
}

/// The sampler of a texture wrapping as authored.
pub(crate) fn sampler(wrap: [TextureWrap; 2]) -> ImageSampler {
    let mode = |wrap| match wrap {
        TextureWrap::Repeat => ImageAddressMode::Repeat,
        TextureWrap::Mirror => ImageAddressMode::MirrorRepeat,
        TextureWrap::Clamp => ImageAddressMode::ClampToEdge,
        TextureWrap::Black => ImageAddressMode::ClampToBorder,
    };
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: mode(wrap[0]),
        address_mode_v: mode(wrap[1]),
        border_color: matches!(wrap, [TextureWrap::Black, _] | [_, TextureWrap::Black])
            .then_some(bevy::image::ImageSamplerBorderColor::TransparentBlack),
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: ANISOTROPY,
        ..default()
    })
}

/// Whether derived textures may be built from an image sampled this way:
/// the default sampler, or one a material texture was finalised with.
pub(crate) fn derivable(sampler: &ImageSampler) -> bool {
    match sampler {
        ImageSampler::Default => true,
        ImageSampler::Descriptor(d) => {
            d.anisotropy_clamp == ANISOTROPY && d.mipmap_filter == ImageFilterMode::Linear
        }
    }
}

/// Appends a box-filtered mip chain to a 2D image of 8-bit or half-float
/// texels; sRGB texels average as light, not as code values.
fn add_mips(image: &mut Image) {
    let desc = &image.texture_descriptor;
    if desc.mip_level_count != 1
        || desc.dimension != TextureDimension::D2
        || desc.size.depth_or_array_layers != 1
    {
        return;
    }
    let (channels, kind) = match desc.format {
        TextureFormat::Rgba8UnormSrgb => (4, Kind::Srgb8),
        TextureFormat::Rgba8Unorm => (4, Kind::Unorm8),
        TextureFormat::Rg8Unorm => (2, Kind::Unorm8),
        TextureFormat::R8Unorm => (1, Kind::Unorm8),
        TextureFormat::Rgba16Float => (4, Kind::Half),
        _ => return,
    };
    let (mut w, mut h) = (desc.size.width as usize, desc.size.height as usize);
    let size = kind.bytes() * channels;
    let Some(data) = image.data.as_mut() else {
        return;
    };
    if w < 2 && h < 2 || data.len() != w * h * size {
        return;
    }
    let mut level: Vec<f32> = data
        .chunks_exact(kind.bytes())
        .enumerate()
        .map(|(i, v)| kind.decode(v, i % channels == 3))
        .collect();
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0.0f32; nw * nh * channels];
        for y in 0..nh {
            for x in 0..nw {
                let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
                let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
                for c in 0..channels {
                    let at = |px: usize, py: usize| level[(py * w + px) * channels + c];
                    next[(y * nw + x) * channels + c] =
                        0.25 * (at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1));
                }
            }
        }
        for (i, v) in next.iter().enumerate() {
            kind.encode(*v, i % channels == 3, data);
        }
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    image.texture_descriptor.mip_level_count = levels;
}

#[derive(Clone, Copy)]
enum Kind {
    Srgb8,
    Unorm8,
    Half,
}

impl Kind {
    fn bytes(self) -> usize {
        match self {
            Kind::Half => 2,
            _ => 1,
        }
    }

    fn decode(self, v: &[u8], alpha: bool) -> f32 {
        match self {
            Kind::Srgb8 if !alpha => srgb_to_linear(v[0] as f32 / 255.0),
            Kind::Srgb8 | Kind::Unorm8 => v[0] as f32 / 255.0,
            Kind::Half => half::f16::from_le_bytes([v[0], v[1]]).to_f32(),
        }
    }

    fn encode(self, v: f32, alpha: bool, out: &mut Vec<u8>) {
        match self {
            Kind::Srgb8 if !alpha => {
                out.push((linear_to_srgb(v).clamp(0.0, 1.0) * 255.0).round() as u8)
            }
            Kind::Srgb8 | Kind::Unorm8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
            Kind::Half => out.extend_from_slice(&half::f16::from_f32(v).to_le_bytes()),
        }
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::Extent3d;

    fn image(w: u32, h: u32, format: TextureFormat, data: Vec<u8>) -> Image {
        Image::new(
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            format,
            RenderAssetUsages::default(),
        )
    }

    #[test]
    fn a_texture_gains_every_level_down_to_one_texel() {
        let mut checker = image(
            4,
            2,
            TextureFormat::Rgba8Unorm,
            (0..8)
                .flat_map(|i| if i % 2 == 0 { [255; 4] } else { [0, 0, 0, 255] })
                .collect(),
        );
        add_mips(&mut checker);
        assert_eq!(checker.texture_descriptor.mip_level_count, 3);
        let data = checker.data.unwrap();
        assert_eq!(data.len(), (8 + 2 + 1) * 4);
        // Black and white average to grey a level down, alpha stays opaque.
        assert_eq!(&data[32..36], &[128, 128, 128, 255]);
    }

    #[test]
    fn srgb_texels_average_as_light() {
        let mut texels = image(
            2,
            1,
            TextureFormat::Rgba8UnormSrgb,
            vec![255, 255, 255, 255, 0, 0, 0, 255],
        );
        add_mips(&mut texels);
        // Half the light of white is code value 188, not 128.
        assert_eq!(texels.data.unwrap()[8], 188);
    }

    #[test]
    fn a_wrap_picks_its_address_mode_and_stays_derivable() {
        let ImageSampler::Descriptor(d) = sampler([TextureWrap::Repeat, TextureWrap::Clamp]) else {
            panic!()
        };
        assert_eq!(
            (d.address_mode_u, d.address_mode_v),
            (ImageAddressMode::Repeat, ImageAddressMode::ClampToEdge)
        );
        assert!(derivable(&sampler([TextureWrap::Mirror; 2])) && derivable(&ImageSampler::Default));
        assert!(!derivable(&ImageSampler::nearest()));
    }
}
