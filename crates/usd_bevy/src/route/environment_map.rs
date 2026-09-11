//! Linear HDR cubemap conversion for Y-pole USD latitude-longitude environments.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

/// Converts a top-row-first, 2:1 latitude-longitude image into a linear cubemap.
///
/// The pole is +Y and longitude zero is +Z. Longitude decreases left to right.
/// Source sRGB pixels are linearized before bilinear filtering. Tint is linear;
/// exposure and world rotation belong on the environment-light component.
/// Sampling uses pixel centers, periodic longitude and clamped latitude.
/// Faces are power-of-two, at most 1024 pixels; sources are at most 16M pixels.
/// The output is suitable for Bevy's `GeneratedEnvironmentMapLight`.
pub fn latlong_cubemap(source: &Image, face_size: u32, tint: [f32; 3]) -> anyhow::Result<Image> {
    let size = source.texture_descriptor.size;
    anyhow::ensure!(source.texture_descriptor.dimension == TextureDimension::D2
        && size.depth_or_array_layers == 1 && size.height > 0
        && u64::from(size.width) == 2 * u64::from(size.height), "environment must be a 2:1 2D latitude-longitude image");
    anyhow::ensure!(face_size.is_power_of_two() && face_size <= 1024, "environment face size must be power-of-two in 1..=1024");
    anyhow::ensure!(tint.iter().all(|v| v.is_finite() && *v >= 0.0), "environment tint must be finite and nonnegative");
    let count = u64::from(size.width) * u64::from(size.height);
    anyhow::ensure!(count <= 16_777_216, "environment source exceeds 16M pixels");
    let stride = match source.texture_descriptor.format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => 4,
        TextureFormat::Rgba16Float => 8,
        TextureFormat::Rgba32Float => 16,
        format => anyhow::bail!("unsupported environment pixel format: {format:?}"),
    };
    anyhow::ensure!(source.texture_descriptor.mip_level_count == 1
        && source.data.as_ref().is_some_and(|data| data.len() == count as usize * stride),
        "environment requires one complete CPU-resident mip level");
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count as usize)?;
    for y in 0..size.height {
        for x in 0..size.width {
            let c = source.get_color_at(x, y)?.to_linear();
            let rgb = [c.red, c.green, c.blue];
            anyhow::ensure!(rgb.iter().all(|v| v.is_finite() && *v >= 0.0), "environment contains invalid radiance");
            pixels.push(Vec3::from_array(rgb));
        }
    }
    let mut data = Vec::new();
    data.try_reserve_exact(face_size as usize * face_size as usize * 6 * 8)?;
    for face in 0..6 {
        for y in 0..face_size {
            for x in 0..face_size {
                let u = 2.0 * (x as f32 + 0.5) / face_size as f32 - 1.0;
                let v = 2.0 * (y as f32 + 0.5) / face_size as f32 - 1.0;
                let direction = face_direction(face, u, v).normalize();
                let rgb = sample(&pixels, size.width, size.height, direction) * Vec3::from_array(tint);
                for value in [rgb.x, rgb.y, rgb.z, 1.0] {
                    anyhow::ensure!(value.is_finite() && value <= 65504.0, "environment radiance exceeds finite RGBA16Float range");
                    data.extend_from_slice(&openusd::gf::f16::from_f32(value).to_bits().to_le_bytes());
                }
            }
        }
    }
    let mut image = Image::new(Extent3d { width: face_size, height: face_size, depth_or_array_layers: 6 },
        TextureDimension::D2, data, TextureFormat::Rgba16Float, bevy::asset::RenderAssetUsages::all());
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    Ok(image)
}

fn face_direction(face: usize, u: f32, v: f32) -> Vec3 {
    match face {
        0 => Vec3::new(1.0, -v, -u),
        1 => Vec3::new(-1.0, -v, u),
        2 => Vec3::new(u, 1.0, v),
        3 => Vec3::new(u, -1.0, -v),
        4 => Vec3::new(u, -v, 1.0),
        _ => Vec3::new(-u, -v, -1.0),
    }
}

fn sample(pixels: &[Vec3], width: u32, height: u32, direction: Vec3) -> Vec3 {
    let longitude = direction.x.atan2(direction.z);
    let latitude = direction.y.clamp(-1.0, 1.0).asin();
    let x = (0.5 - longitude / std::f32::consts::TAU) * width as f32 - 0.5;
    let y = (0.5 - latitude / std::f32::consts::PI) * height as f32 - 0.5;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let texel = |x: i64, y: i64| pixels[(y.clamp(0, height as i64 - 1) * width as i64 + x.rem_euclid(width as i64)) as usize];
    let (tx, ty) = (x - x.floor(), y - y.floor());
    texel(x0, y0).lerp(texel(x0 + 1, y0), tx)
        .lerp(texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), tx), ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant(color: [f32; 4]) -> Image {
        Image::new_fill(Extent3d { width: 8, height: 4, depth_or_array_layers: 1 }, TextureDimension::D2,
            &color.into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>(), TextureFormat::Rgba32Float,
            bevy::asset::RenderAssetUsages::all())
    }

    #[test]
    fn directional_hdr_fixture_places_blue_between_positive_x_and_z() {
        let source = Image::from_buffer(include_bytes!("../../../../assets/dome_directional.hdr"),
            bevy::image::ImageType::Extension("hdr"), bevy::image::CompressedImageFormats::NONE,
            false, bevy::image::ImageSampler::default(), bevy::asset::RenderAssetUsages::all()).unwrap();
        assert_eq!((source.width(), source.height()), (4, 2));
        let cube = latlong_cubemap(&source, 1, [1.0; 3]).unwrap();
        let positive = cube.get_color_at_3d(0, 0, 0).unwrap().to_linear();
        let negative = cube.get_color_at_3d(0, 0, 1).unwrap().to_linear();
        assert!((positive.blue - positive.red).abs() < 0.001);
        assert!(negative.red > negative.blue);
        let cube = latlong_cubemap(&source, 4, [1.0; 3]).unwrap();
        let diagonal = cube.get_color_at_3d(3, 1, 4).unwrap().to_linear();
        assert!(diagonal.blue > diagonal.red);
    }

    #[test]
    fn preserves_hdr_and_linear_tint_in_all_faces() {
        let cube = latlong_cubemap(&constant([8.0, 2.0, 0.5, 0.0]), 4, [0.5, 2.0, 1.0]).unwrap();
        assert_eq!(cube.texture_view_descriptor.as_ref().unwrap().dimension, Some(TextureViewDimension::Cube));
        assert_eq!(cube.data.as_ref().unwrap().len(), 4 * 4 * 6 * 8);
        for face in 0..6 {
            let c = cube.get_color_at_3d(2, 2, face).unwrap().to_linear();
            assert_eq!([c.red, c.green, c.blue, c.alpha], [4.0, 4.0, 0.5, 1.0]);
        }
    }

    #[test]
    fn srgb_is_linearized_before_filtering() {
        let mut source = Image::new_fill(Extent3d { width: 2, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2,
            &[128, 128, 128, 255], TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::all());
        let c = latlong_cubemap(&source, 1, [1.0; 3]).unwrap().get_color_at_3d(0, 0, 0).unwrap().to_linear();
        assert!((c.red - 0.21586).abs() < 0.0002);
        source.data = Some(vec![0, 0, 0, 255, 255, 255, 255, 255]);
        let c = latlong_cubemap(&source, 1, [1.0; 3]).unwrap().get_color_at_3d(0, 0, 4).unwrap().to_linear();
        assert_eq!(c.red, 0.5);
    }

    #[test]
    fn usd_axes_poles_and_longitude_seam() {
        let pixels: Vec<_> = (0..4).flat_map(|y| (0..8).map(move |x| Vec3::new(x as f32, y as f32, 0.0))).collect();
        assert_eq!(sample(&pixels, 8, 4, Vec3::Z), Vec3::new(3.5, 1.5, 0.0));
        assert_eq!(sample(&pixels, 8, 4, Vec3::X), Vec3::new(1.5, 1.5, 0.0));
        assert_eq!(sample(&pixels, 8, 4, -Vec3::X), Vec3::new(5.5, 1.5, 0.0));
        assert_eq!(sample(&pixels, 8, 4, Vec3::Y).y, 0.0);
        assert_eq!(sample(&pixels, 8, 4, -Vec3::Y).y, 3.0);
        for (face, axis) in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z].into_iter().enumerate() {
            assert_eq!(face_direction(face, 0.0, 0.0), axis);
        }
        let periodic: Vec<_> = (0..32).map(|i| Vec3::splat((std::f32::consts::TAU * (i % 8) as f32 / 7.0).cos())).collect();
        let left = sample(&periodic, 8, 4, Vec3::new(-1e-6, 0.0, -1.0).normalize());
        let right = sample(&periodic, 8, 4, Vec3::new(1e-6, 0.0, -1.0).normalize());
        assert!((left - right).length() < 1e-5);
    }

    #[test]
    fn longitude_seam_blends_distinct_edge_texels() {
        let pixels: Vec<_> = (0..32).map(|i| Vec3::splat((i % 8) as f32)).collect();
        for x in [-1e-6, 0.0, 1e-6] {
            let value = sample(&pixels, 8, 4, Vec3::new(x, 0.0, -1.0).normalize());
            assert!((value.x - 3.5).abs() < 1e-4, "seam value: {value:?}");
        }
    }

    #[test]
    fn latlong_pixel_centers_recover_source_texels() {
        let pixels: Vec<_> = (0..32).map(|i| Vec3::new((i % 8) as f32, (i / 8) as f32, 0.0)).collect();
        for y in 0..4 {
            for x in 0..8 {
                let longitude = (0.5 - (x as f32 + 0.5) / 8.0) * std::f32::consts::TAU;
                let latitude = (0.5 - (y as f32 + 0.5) / 4.0) * std::f32::consts::PI;
                let direction = Vec3::new(latitude.cos() * longitude.sin(), latitude.sin(), latitude.cos() * longitude.cos());
                let value = sample(&pixels, 8, 4, direction);
                assert!((value - pixels[y * 8 + x]).length() < 1e-5, "pixel ({x}, {y}): {value:?}");
            }
        }
    }

    #[test]
    fn rejects_invalid_sources_sizes_and_radiance() {
        for size in [0, 3, 2048] { assert!(latlong_cubemap(&constant([1.0; 4]), size, [1.0; 3]).is_err()); }
        for value in [f32::NAN, f32::INFINITY, -1.0, 70000.0] {
            assert!(latlong_cubemap(&constant([value; 4]), 1, [1.0; 3]).is_err());
        }
        let mut source = constant([1.0; 4]);
        source.data = Some(vec![0]);
        assert!(latlong_cubemap(&source, 1, [1.0; 3]).is_err());
        source.data = None;
        assert!(latlong_cubemap(&source, 1, [1.0; 3]).is_err());
        source.texture_descriptor.size.depth_or_array_layers = 6;
        assert!(latlong_cubemap(&source, 1, [1.0; 3]).is_err());
    }
}
