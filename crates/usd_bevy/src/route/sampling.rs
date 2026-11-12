//! How material textures are sampled: the `wrapS`/`wrapT` their texture
//! nodes author, trilinear and anisotropic filtering, and a mip chain built
//! on the CPU, so tiled textures repeat and stay calm at a distance.
//!
//! Textures are finalised once their materials are built: the packing and
//! colour transforms read the decoded images first, at their top level.

use crate::read::shade::{ReadPreviewMaterial, TextureWrap};
use bevy::asset::AssetId;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureDimension, TextureFormat};

/// Anisotropy every USD material texture samples with.
const ANISOTROPY: u16 = 16;

#[derive(Resource, Default)]
struct TextureSampling {
    pending: HashMap<AssetId<Image>, [TextureWrap; 2]>,
    done: HashMap<AssetId<Image>, [TextureWrap; 2]>,
}

/// Opt-in cumulative CPU texture-finalization counters, including mip generation.
#[derive(Resource, Default)]
pub struct TextureSamplingTiming {
    pub images: usize,
    pub input_bytes: usize,
    pub appended_bytes: usize,
    pub elapsed: std::time::Duration,
    pub slowest: std::time::Duration,
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
            && sampling.done.get(&handle.id()) != Some(&wrap)
        {
            sampling.pending.insert(handle.id(), wrap);
        }
    }
}

fn finalise(
    mut sampling: ResMut<TextureSampling>,
    images: Option<ResMut<Assets<Image>>>,
    mut generated: Option<ResMut<super::generated_image::GeneratedImages>>,
    mut timing: Option<ResMut<TextureSamplingTiming>>,
) {
    let Some(mut images) = images else { return };
    if sampling.pending.is_empty() {
        sampling.done.retain(|id, _| images.contains(*id));
        return;
    }
    let pending: Vec<_> = sampling.pending.drain().collect();
    for (id, wrap) in pending {
        let Some(mut image) = images.get_mut(id) else {
            continue;
        };
        let started = timing
            .as_ref()
            .map(|_| bevy::platform::time::Instant::now());
        let source = super::generated_image::layout(&image);
        let base_len = image.data.as_ref().map_or(0, Vec::len);
        add_mips(&mut image);
        image.sampler = sampler(wrap);
        if let Some(generated) = &mut generated {
            generated.record_finalized(id, source, base_len, &image);
        }
        sampling.done.insert(id, wrap);
        if let (Some(started), Some(timing)) = (started, timing.as_deref_mut()) {
            let elapsed = started.elapsed();
            timing.images += 1;
            timing.input_bytes += base_len;
            timing.appended_bytes += image.data.as_ref().map_or(0, Vec::len) - base_len;
            timing.elapsed += elapsed;
            timing.slowest = timing.slowest.max(elapsed);
        }
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
    if w == 0
        || h == 0
        || w < 2 && h < 2
        || w.checked_mul(h).and_then(|pixels| pixels.checked_mul(size)) != Some(data.len())
    {
        return;
    }
    let Some(total_bytes) = mip_chain_bytes(w, h, size) else {
        return;
    };
    data.reserve_exact(total_bytes - data.len());
    let mut level = Vec::new();
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let next = if levels == 1 {
            downsample(w, h, channels, |i| {
                kind.decode(&data[i * kind.bytes()..], i % channels == 3)
            })
        } else {
            downsample(w, h, channels, |i| level[i])
        };
        for (i, v) in next.iter().enumerate() {
            kind.encode(*v, i % channels == 3, data);
        }
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    image.texture_descriptor.mip_level_count = levels;
}

fn mip_chain_bytes(mut w: usize, mut h: usize, texel_bytes: usize) -> Option<usize> {
    if w == 0 || h == 0 || texel_bytes == 0 {
        return None;
    }
    let mut bytes = w.checked_mul(h)?.checked_mul(texel_bytes)?;
    while w > 1 || h > 1 {
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
        bytes = bytes.checked_add(w.checked_mul(h)?.checked_mul(texel_bytes)?)?;
    }
    Some(bytes)
}

fn downsample(w: usize, h: usize, channels: usize, read: impl Fn(usize) -> f32) -> Vec<f32> {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut next = vec![0.0; nw * nh * channels];
    for y in 0..nh {
        for x in 0..nw {
            let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
            let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
            for c in 0..channels {
                let at = |px: usize, py: usize| read((py * w + px) * channels + c);
                next[(y * nw + x) * channels + c] =
                    0.25 * (at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1));
            }
        }
    }
    next
}

static SRGB_TO_LINEAR: std::sync::LazyLock<[f32; 256]> =
    std::sync::LazyLock::new(|| std::array::from_fn(|value| srgb_to_linear(value as f32 / 255.0)));

static SRGB_ENCODING: std::sync::LazyLock<Option<SrgbEncoding>> =
    std::sync::LazyLock::new(|| SrgbEncoding::new(scalar_srgb_byte));

struct SrgbEncoding {
    bins: [u8; 4097],
    thresholds: [f32; 256],
}

impl SrgbEncoding {
    fn new(quantize: impl Fn(f32) -> u8) -> Option<Self> {
        if quantize(0.0) != 0 || quantize(1.0) != 255 {
            return None;
        }
        let mut thresholds = [f32::INFINITY; 256];
        for (code, threshold) in thresholds[..255].iter_mut().enumerate() {
            let (mut low, mut high) = (0, 1.0f32.to_bits());
            while low < high {
                let middle = low + (high - low) / 2;
                if usize::from(quantize(f32::from_bits(middle))) <= code {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            *threshold = f32::from_bits(low);
            for offset in -8..=8 {
                let bits = low.saturating_add_signed(offset);
                let expected = code + usize::from(bits >= low);
                if usize::from(quantize(f32::from_bits(bits))) != expected {
                    return None;
                }
            }
        }
        if thresholds.windows(2).any(|pair| pair[0] >= pair[1]) {
            return None;
        }
        let table = Self {
            bins: std::array::from_fn(|bin| quantize(bin as f32 / 4096.0)),
            thresholds,
        };
        for bin in 0..4096 {
            let first = bin as f32 / 4096.0;
            let last = f32::from_bits(((bin + 1) as f32 / 4096.0).to_bits() - 1);
            if quantize(last).checked_sub(table.bins[bin])? > 1
                || table.encode(first) != quantize(first)
                || table.encode(last) != quantize(last)
            {
                return None;
            }
        }
        Some(table)
    }

    fn encode(&self, value: f32) -> u8 {
        let value = value.clamp(0.0, 1.0);
        let code = self.bins[(value * 4096.0) as usize];
        code + u8::from(value >= self.thresholds[usize::from(code)])
    }
}

fn scalar_srgb_byte(value: f32) -> u8 {
    (linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn srgb_byte(value: f32) -> u8 {
    SRGB_ENCODING
        .as_ref()
        .map_or_else(|| scalar_srgb_byte(value), |table| table.encode(value))
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
            Kind::Srgb8 if !alpha => SRGB_TO_LINEAR[v[0] as usize],
            Kind::Srgb8 | Kind::Unorm8 => v[0] as f32 / 255.0,
            Kind::Half => half::f16::from_le_bytes([v[0], v[1]]).to_f32(),
        }
    }

    fn encode(self, v: f32, alpha: bool, out: &mut Vec<u8>) {
        match self {
            Kind::Srgb8 if !alpha => out.push(srgb_byte(v)),
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
    fn finalization_metrics_count_only_loaded_pending_images() {
        let mut app = App::new();
        configure(&mut app);
        app.init_resource::<Assets<Image>>()
            .init_resource::<TextureSamplingTiming>();
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image(
            4,
            2,
            TextureFormat::Rgba8Unorm,
            vec![127; 32],
        ));
        let missing = Handle::<Image>::default().id();
        app.world_mut()
            .resource_mut::<TextureSampling>()
            .pending
            .extend([
                (handle.id(), [TextureWrap::Repeat; 2]),
                (missing, [TextureWrap::Repeat; 2]),
            ]);
        app.update();
        let timing = app.world().resource::<TextureSamplingTiming>();
        assert_eq!(
            (timing.images, timing.input_bytes, timing.appended_bytes),
            (1, 32, 12)
        );
        assert_eq!(timing.elapsed, timing.slowest);
        let elapsed = timing.elapsed;
        app.update();
        let timing = app.world().resource::<TextureSamplingTiming>();
        assert_eq!(timing.images, 1);
        assert_eq!(timing.elapsed, elapsed);
        app.world_mut().remove_resource::<TextureSamplingTiming>();
        app.world_mut()
            .resource_mut::<TextureSampling>()
            .pending
            .insert(handle.id(), [TextureWrap::Clamp; 2]);
        app.update();
        let image = app
            .world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap();
        assert_eq!(image.sampler, sampler([TextureWrap::Clamp; 2]));
        assert_eq!(image.data.as_ref().unwrap().len(), 44);
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
    fn decoded_srgb_table_matches_every_code_and_keeps_alpha_linear() {
        for code in 0..=255u8 {
            assert_eq!(
                Kind::Srgb8.decode(&[code], false).to_bits(),
                srgb_to_linear(code as f32 / 255.0).to_bits()
            );
            assert_eq!(
                Kind::Srgb8.decode(&[code], true).to_bits(),
                (code as f32 / 255.0).to_bits()
            );
        }
    }

    #[test]
    fn encoded_srgb_table_matches_transitions_bins_and_special_values() {
        let table = SrgbEncoding::new(scalar_srgb_byte).unwrap();
        assert!(std::mem::size_of::<SrgbEncoding>() <= 5124);
        let mut encoded = Vec::new();
        let mut check = |value: f32| {
            let expected = scalar_srgb_byte(value);
            assert_eq!(table.encode(value), expected, "value={value:?}");
            encoded.clear();
            Kind::Srgb8.encode(value, false, &mut encoded);
            assert_eq!(encoded, [expected]);
            encoded.clear();
            Kind::Srgb8.encode(value, true, &mut encoded);
            assert_eq!(encoded, [(value.clamp(0.0, 1.0) * 255.0).round() as u8]);
        };
        for threshold in &table.thresholds[..255] {
            for offset in -16..=16 {
                check(f32::from_bits(
                    threshold.to_bits().saturating_add_signed(offset),
                ));
            }
        }
        for bin in 0..=4096 {
            let boundary = (bin as f32 / 4096.0).to_bits();
            for offset in -2..=2 {
                check(f32::from_bits(boundary.saturating_add_signed(offset)));
            }
        }
        for value in [
            f32::NEG_INFINITY,
            -f32::MAX,
            -1.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            0.003_130_8,
            1.0,
            2.0,
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
            f32::from_bits(0xffc0_0011),
        ] {
            check(value);
        }
        let mut seed = 0x1234_5678u32;
        for _ in 0..250_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            check(f32::from_bits(seed));
            check(seed as f32 / u32::MAX as f32);
        }
    }

    #[test]
    fn encoded_srgb_table_rejects_missing_or_skipped_transitions() {
        assert!(SrgbEncoding::new(|_| 0).is_none());
        assert!(SrgbEncoding::new(|_| 255).is_none());
        assert!(SrgbEncoding::new(|value| if value < 0.5 { 0 } else { 255 }).is_none());
    }

    #[test]
    #[ignore = "release exhaustive f32 sRGB quantization check"]
    fn srgb_encoding_exhaustive() {
        if cfg!(debug_assertions) {
            panic!("use oslo make test-texture-encoding");
        }
        let table = SrgbEncoding::new(scalar_srgb_byte).unwrap();
        let first = table.thresholds[0].to_bits() - 32;
        let last = 1.0f32.to_bits();
        let started = std::time::Instant::now();
        for bits in first..=last {
            let value = f32::from_bits(bits);
            assert_eq!(
                table.encode(value),
                scalar_srgb_byte(value),
                "bits={bits:#x}"
            );
        }
        eprintln!(
            "SRGB_ENCODING_EXHAUSTIVE values={} elapsed_ms={:.3}",
            last - first + 1,
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }

    #[test]
    #[ignore = "release paired sRGB encoder wall-time diagnostic"]
    fn srgb_encoding_kernel() {
        if cfg!(debug_assertions) {
            panic!("use oslo make test-texture-encoding");
        }
        assert!(SRGB_ENCODING.is_some());
        let mut seed = 0x1234_5678u32;
        let mut channel = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            SRGB_TO_LINEAR[(seed >> 24) as usize]
        };
        let values: Vec<f32> = (0..1_048_576)
            .map(|_| 0.25 * (channel() + channel() + channel() + channel()))
            .collect();
        let measure = |encode: fn(f32) -> u8| {
            let started = std::time::Instant::now();
            let mut checksum = 0u64;
            for _ in 0..4 {
                for value in &values {
                    checksum +=
                        u64::from(std::hint::black_box(encode(std::hint::black_box(*value))));
                }
            }
            (started.elapsed(), checksum)
        };
        let mut expected = None;
        for (round, candidate) in [false, true, true, false, false, true, true, false]
            .into_iter()
            .enumerate()
        {
            let (elapsed, checksum) = measure(if candidate {
                srgb_byte
            } else {
                scalar_srgb_byte
            });
            assert_eq!(*expected.get_or_insert(checksum), checksum);
            eprintln!(
                "SRGB_ENCODING_KERNEL round={round} candidate={candidate} values={} elapsed_ms={:.6} checksum={checksum}",
                values.len() * 4,
                elapsed.as_secs_f64() * 1000.0,
            );
        }
    }

    fn scalar_mips(
        mut w: usize,
        mut h: usize,
        channels: usize,
        kind: Kind,
        mut data: Vec<u8>,
    ) -> (Vec<u8>, u32) {
        let mut level: Vec<f32> = data
            .chunks_exact(kind.bytes())
            .enumerate()
            .map(|(i, value)| match kind {
                Kind::Srgb8 if i % channels != 3 => srgb_to_linear(value[0] as f32 / 255.0),
                Kind::Srgb8 | Kind::Unorm8 => value[0] as f32 / 255.0,
                Kind::Half => half::f16::from_le_bytes([value[0], value[1]]).to_f32(),
            })
            .collect();
        let mut levels = 1;
        while w > 1 || h > 1 {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![0.0; nw * nh * channels];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..channels {
                        let at = |x: usize, y: usize| {
                            level[(y.min(h - 1) * w + x.min(w - 1)) * channels + c]
                        };
                        next[(y * nw + x) * channels + c] = 0.25
                            * (at(2 * x, 2 * y)
                                + at(2 * x + 1, 2 * y)
                                + at(2 * x, 2 * y + 1)
                                + at(2 * x + 1, 2 * y + 1));
                    }
                }
            }
            for (i, value) in next.iter().enumerate() {
                match kind {
                    Kind::Srgb8 if i % channels != 3 => {
                        data.push((linear_to_srgb(*value).clamp(0.0, 1.0) * 255.0).round() as u8)
                    }
                    Kind::Srgb8 | Kind::Unorm8 => {
                        data.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
                    }
                    Kind::Half => {
                        data.extend_from_slice(&half::f16::from_f32(*value).to_le_bytes())
                    }
                }
            }
            level = next;
            (w, h) = (nw, nh);
            levels += 1;
        }
        (data, levels)
    }

    #[test]
    fn streamed_mips_match_full_precision_scalar_reference() {
        let dimensions = [
            (1, 1),
            (1, 9),
            (9, 1),
            (2, 2),
            (3, 5),
            (4, 8),
            (7, 16),
            (64, 37),
            (127, 129),
        ];
        for (format, channels, kind) in [
            (TextureFormat::Rgba8UnormSrgb, 4, Kind::Srgb8),
            (TextureFormat::Rgba8Unorm, 4, Kind::Unorm8),
            (TextureFormat::Rg8Unorm, 2, Kind::Unorm8),
            (TextureFormat::R8Unorm, 1, Kind::Unorm8),
            (TextureFormat::Rgba16Float, 4, Kind::Half),
        ] {
            for (w, h) in dimensions {
                let mut seed = 0x1234_5678u32;
                let mut data: Vec<u8> = (0..w * h * channels * kind.bytes())
                    .map(|_| {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        (seed >> 24) as u8
                    })
                    .collect();
                if let Kind::Half = kind {
                    for value in data.as_chunks_mut::<2>().0 {
                        let bits = u16::from_le_bytes([value[0], value[1]]);
                        if !half::f16::from_bits(bits).is_finite() {
                            value.copy_from_slice(&(bits & !0x0400).to_le_bytes());
                        }
                    }
                }
                let (expected, levels) = scalar_mips(w, h, channels, kind, data.clone());
                let mut texture = image(w as u32, h as u32, format, data);
                add_mips(&mut texture);
                let actual = texture.data.as_ref().unwrap();
                assert_eq!(actual.len(), expected.len());
                assert_eq!(
                    actual.iter().zip(&expected).position(|(a, b)| a != b),
                    None,
                    "{format:?} {w}x{h}"
                );
                assert_eq!(texture.texture_descriptor.mip_level_count, levels);
                add_mips(&mut texture);
                assert_eq!(texture.data.as_ref().unwrap(), &expected);
                assert_eq!(texture.texture_descriptor.mip_level_count, levels);
            }
        }
    }

    #[test]
    fn half_mips_preserve_nonfinite_values_and_original_texels() {
        let data: Vec<u8> = (0..64 * 37 * 4u32)
            .flat_map(|i| (i.wrapping_mul(7_919) as u16).to_le_bytes())
            .collect();
        let (expected, levels) = scalar_mips(64, 37, 4, Kind::Half, data.clone());
        let mut texture = image(64, 37, TextureFormat::Rgba16Float, data.clone());
        add_mips(&mut texture);
        let actual = texture.data.unwrap();
        assert_eq!(&actual[..data.len()], &data);
        assert_eq!(actual.len(), expected.len());
        assert_eq!(texture.texture_descriptor.mip_level_count, levels);
        for (index, (a, b)) in actual
            .as_chunks::<2>()
            .0
            .iter()
            .zip(expected.as_chunks::<2>().0)
            .enumerate()
        {
            let a = half::f16::from_le_bytes([a[0], a[1]]);
            let b = half::f16::from_le_bytes([b[0], b[1]]);
            assert!(
                a.to_bits() == b.to_bits() || a.is_nan() && b.is_nan(),
                "half texel {index}: {a:?} != {b:?}"
            );
        }
    }

    #[test]
    fn mip_chain_sizes_include_narrow_levels_and_reject_overflow() {
        for (w, h, size, expected) in [
            (1, 1, 8, 8),
            (1, 9, 4, 64),
            (9, 1, 4, 64),
            (3, 5, 4, 72),
            (4, 2, 4, 44),
            (4096, 4096, 4, 89_478_484),
        ] {
            assert_eq!(mip_chain_bytes(w, h, size), Some(expected));
        }
        for (w, h, size) in [
            (0, 1, 4),
            (1, 0, 4),
            (1, 1, 0),
            (usize::MAX, 2, 1),
            (usize::MAX, 1, 2),
            (usize::MAX, 1, 1),
        ] {
            assert_eq!(mip_chain_bytes(w, h, size), None);
        }
    }

    #[test]
    fn mip_output_capacity_tracks_payload_or_existing_reservation() {
        for (format, channels, kind) in [
            (TextureFormat::Rgba8UnormSrgb, 4, Kind::Srgb8),
            (TextureFormat::Rgba8Unorm, 4, Kind::Unorm8),
            (TextureFormat::Rg8Unorm, 2, Kind::Unorm8),
            (TextureFormat::R8Unorm, 1, Kind::Unorm8),
            (TextureFormat::Rgba16Float, 4, Kind::Half),
        ] {
            for (w, h) in [(1, 1), (1, 9), (9, 1), (3, 5), (64, 64), (64, 37)] {
                let source = vec![51; w * h * channels * kind.bytes()];
                let (expected, levels) = scalar_mips(w, h, channels, kind, source.clone());
                for extra in [0, expected.len()] {
                    let mut data = source.clone();
                    data.reserve_exact(extra);
                    let initial_capacity = data.capacity();
                    let mut texture = image(w as u32, h as u32, format, data);
                    add_mips(&mut texture);
                    let data = texture.data.as_ref().unwrap();
                    assert_eq!(data, &expected);
                    assert_eq!(texture.texture_descriptor.mip_level_count, levels);
                    assert_eq!(
                        data.capacity(),
                        initial_capacity.max(expected.len()),
                        "{format:?} {w}x{h} extra={extra}"
                    );
                    let capacity = data.capacity();
                    add_mips(&mut texture);
                    assert_eq!(texture.data.as_ref().unwrap().capacity(), capacity);
                }
            }
        }
    }

    #[test]
    fn malformed_and_unsupported_mip_inputs_are_unchanged() {
        let template = image(4, 2, TextureFormat::Rgba8Unorm, vec![128; 32]);
        for case in 0..8 {
            let mut texture = template.clone();
            match case {
                0 => texture.data = None,
                1 => {
                    texture.data.as_mut().unwrap().pop();
                }
                2 => {
                    texture.texture_descriptor.size.width = 0;
                    texture.data.as_mut().unwrap().clear();
                }
                3 => texture.texture_descriptor.size.height = 0,
                4 => {
                    texture.texture_descriptor.size.width = u32::MAX;
                    texture.texture_descriptor.size.height = u32::MAX;
                }
                5 => texture.texture_descriptor.size.depth_or_array_layers = 2,
                6 => texture.texture_descriptor.dimension = TextureDimension::D3,
                _ => texture.texture_descriptor.format = TextureFormat::Rgba32Float,
            }
            let expected = texture.clone();
            add_mips(&mut texture);
            assert_eq!(texture.data, expected.data);
            assert_eq!(texture.texture_descriptor, expected.texture_descriptor);
            assert_eq!(texture.sampler, expected.sampler);
        }
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
