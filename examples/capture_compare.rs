//! Compare tightly packed RGBA8 viewport captures using RGB channel errors.

use std::process::ExitCode;

#[derive(Debug, PartialEq)]
struct Difference {
    pixels: usize,
    changed: usize,
    maximum: u8,
    mean: f64,
}

fn compare(a: &[u8], b: &[u8], tolerance: u8) -> Result<Difference, String> {
    if a.is_empty() || a.len() != b.len() || a.len() % 4 != 0 {
        return Err("captures must have equal nonzero RGBA8 byte lengths".into());
    }
    let mut changed = 0;
    let mut maximum = 0;
    let mut total = 0_u64;
    for (a,b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let errors = [a[0].abs_diff(b[0]), a[1].abs_diff(b[1]), a[2].abs_diff(b[2])];
        changed += usize::from(errors.iter().any(|&error| error > tolerance));
        maximum = maximum.max(*errors.iter().max().unwrap());
        total += errors.iter().map(|&error| u64::from(error)).sum::<u64>();
    }
    let pixels = a.len() / 4;
    Ok(Difference { pixels, changed, maximum, mean: total as f64 / (pixels * 3) as f64 })
}

fn run(args: &[String]) -> Result<bool, String> {
    if !matches!(args.len(), 2 | 3 | 5) {
        return Err("usage: capture_compare BEFORE.rgba AFTER.rgba [TOLERANCE [WIDTH DIFF.png]]".into());
    }
    let tolerance = args.get(2).map(|arg| arg.parse::<u8>().map_err(|_| "invalid tolerance"))
        .transpose()?.unwrap_or(0);
    let a = std::fs::read(&args[0]).map_err(|error| format!("{}: {error}", args[0]))?;
    let b = std::fs::read(&args[1]).map_err(|error| format!("{}: {error}", args[1]))?;
    let difference = compare(&a, &b, tolerance)?;
    if args.len() == 5 {
        let width = args[3].parse::<u32>().map_err(|_| "invalid width")?;
        let (pixels, height, bounds) = difference_image(&a, &b, tolerance, width)?;
        let mut image = bevy::prelude::Image::new_target_texture(width, height,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some(pixels);
        image.try_into_dynamic().map_err(|error| error.to_string())?.save(&args[4])
            .map_err(|error| format!("difference image: {error}"))?;
        println!("changed_bounds_inclusive={bounds:?}");
    }
    println!("pixels={} changed={} changed_percent={:.6} max_rgb_error={} mean_rgb_error={:.6} tolerance={tolerance}",
        difference.pixels, difference.changed, difference.changed as f64 * 100.0 / difference.pixels as f64,
        difference.maximum, difference.mean);
    Ok(difference.changed == 0)
}

fn difference_image(a: &[u8], b: &[u8], tolerance: u8, width: u32)
    -> Result<(Vec<u8>, u32, Option<[u32; 4]>), String> {
    compare(a, b, tolerance)?;
    let count = a.len() / 4;
    if width == 0 || count % width as usize != 0 {
        return Err("width must be nonzero and divide the pixel count".into());
    }
    let height = u32::try_from(count / width as usize).map_err(|_| "image is too tall")?;
    let mut bounds: Option<[u32; 4]> = None;
    let mut pixels = Vec::with_capacity(a.len());
    for (index, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let maximum = (0..3).map(|channel| a[channel].abs_diff(b[channel])).max().unwrap();
        if maximum > tolerance {
            let x = (index % width as usize) as u32;
            let y = (index / width as usize) as u32;
            bounds = Some(match bounds {
                Some([left, top, right, bottom]) => [left.min(x), top.min(y), right.max(x), bottom.max(y)],
                None => [x, y, x, y],
            });
            pixels.extend_from_slice(&[255, maximum, 0, 255]);
        } else {
            let gray = ((u16::from(a[0]) + u16::from(a[1]) + u16::from(a[2])) / 12) as u8;
            pixels.extend_from_slice(&[gray, gray, gray, 255]);
        }
    }
    Ok((pixels, height, bounds))
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => { eprintln!("{error}"); ExitCode::from(2) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_marks_thresholded_pixels_and_reports_bounds() {
        let a = [0; 16];
        let mut b = a;
        b[4] = 3;
        b[12] = 8;
        let (pixels, height, bounds) = difference_image(&a, &b, 3, 2).unwrap();
        assert_eq!(height, 2);
        assert_eq!(bounds, Some([1, 1, 1, 1]));
        assert_eq!(&pixels[4..8], &[0, 0, 0, 255]);
        assert_eq!(&pixels[12..16], &[255, 8, 0, 255]);
        assert_eq!(difference_image(&a, &a, 0, 2).unwrap().2, None);
        assert!(difference_image(&a, &b, 0, 0).is_err());
        assert!(difference_image(&a, &b, 0, 3).is_err());
    }

    #[test]
    fn compares_rgb_with_explicit_tolerance_and_ignores_alpha() {
        let a = [0,10,20,0, 100,100,100,255];
        let b = [0,10,20,255, 104,100,98,0];
        let diff = compare(&a, &b, 0).unwrap();
        assert_eq!(diff, Difference { pixels: 2, changed: 1, maximum: 4, mean: 1.0 });
        assert_eq!(compare(&a, &b, 4).unwrap().changed, 0);
        assert_eq!(compare(&a, &a, 0).unwrap().changed, 0);
    }

    #[test]
    fn rejects_empty_truncated_and_mismatched_data() {
        for (a,b) in [(&[][..], &[][..]), (&[0,0,0][..], &[0,0,0][..]), (&[0;4][..], &[0;8][..])] {
            assert!(compare(a,b,0).is_err());
        }
    }
}
