//! Inspect a PNG region and reject predominantly near-black captures.

use std::{io::Read, process::ExitCode};

const MAX_INPUT: usize = 32 * 1024 * 1024;
const MAX_DECODED: usize = 64 * 1024 * 1024;

fn inspect(bytes: &[u8], region: [u32; 4], threshold: u8) -> Result<(usize, usize), String> {
    if bytes.len() > MAX_INPUT { return Err("PNG exceeds input budget".into()); }
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: MAX_DECODED });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    if reader.info().animation_control.is_some() { return Err("animated PNG captures are unsupported".into()); }
    let [x, y, width, height] = region;
    if width == 0 || height == 0
        || x.checked_add(width).is_none_or(|right| right > reader.info().width)
        || y.checked_add(height).is_none_or(|bottom| bottom > reader.info().height)
    { return Err("region must be nonempty and inside the PNG".into()); }
    let size = reader.output_buffer_size().filter(|size| *size <= MAX_DECODED).ok_or("PNG exceeds decoded budget")?;
    let mut pixels = vec![0; size];
    let info = reader.next_frame(&mut pixels).map_err(|error| error.to_string())?;
    if info.bit_depth != png::BitDepth::Eight { return Err("unsupported decoded bit depth".into()); }
    let (channels, color_channels) = match info.color_type {
        png::ColorType::Rgb => (3, 3),
        png::ColorType::Rgba => (4, 3),
        png::ColorType::Grayscale => (1, 1),
        png::ColorType::GrayscaleAlpha => (2, 1),
        _ => return Err("unsupported decoded color type".into()),
    };
    let mut nonblack = 0;
    for row in y..y + height {
        for column in x..x + width {
            let offset = (row as usize * info.width as usize + column as usize) * channels;
            nonblack += usize::from(pixels[offset..offset + color_channels].iter().any(|value| *value > threshold));
        }
    }
    Ok((width as usize * height as usize, nonblack))
}

fn run(args: &[String]) -> Result<bool, String> {
    if !(5..=7).contains(&args.len()) { return Err("usage: capture_inspect IMAGE.png X Y WIDTH HEIGHT [BLACK_THRESHOLD [MIN_NONBLACK_PERCENT]]".into()); }
    let region: Vec<_> = args[1..5].iter().map(|value| value.parse::<u32>().map_err(|_| "invalid region integer")).collect::<Result<_, _>>()?;
    let threshold = args.get(5).map(|value| value.parse::<u8>().map_err(|_| "invalid black threshold")).transpose()?.unwrap_or(8);
    let minimum = args.get(6).map(|value| value.parse::<f64>().map_err(|_| "invalid nonblack percentage")).transpose()?.unwrap_or(1.0);
    if !minimum.is_finite() || !(0.0..=100.0).contains(&minimum) { return Err("nonblack percentage must be finite and 0..=100".into()); }
    let mut bytes = Vec::new();
    std::fs::File::open(&args[0]).map_err(|error| error.to_string())?.take(MAX_INPUT as u64 + 1)
        .read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let (pixels, nonblack) = inspect(&bytes, region.try_into().unwrap(), threshold)?;
    let percentage = nonblack as f64 * 100.0 / pixels as f64;
    println!("pixels={pixels} nonblack={nonblack} nonblack_percent={percentage:.6} black_threshold={threshold} minimum_percent={minimum}");
    Ok(percentage >= minimum)
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => { eprintln!("capture region is predominantly near-black"); ExitCode::from(1) }
        Err(error) => { eprintln!("{error}"); ExitCode::from(2) }
    }
}

#[test]
fn region_excludes_wallpaper_and_ignores_alpha() {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 4, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut pixels = vec![255; 32];
        for index in [1, 2, 5, 6] { pixels[index * 4..index * 4 + 3].fill(0); }
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    assert_eq!(inspect(&bytes, [0,0,4,2], 8).unwrap(), (8,4));
    assert_eq!(inspect(&bytes, [1,0,2,2], 8).unwrap(), (4,0));
    for region in [[0,0,0,2], [3,0,2,2], [0,u32::MAX,1,2]] { assert!(inspect(&bytes, region, 8).is_err()); }
    assert!(inspect(b"invalid PNG", [0,0,1,1], 8).is_err());
}

#[test]
fn inspection_handles_png_formats_and_threshold_boundary() {
    for (color, depth, pixels) in [
        (png::ColorType::Grayscale, png::BitDepth::Eight, vec![8,9]),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, vec![8,255,9,255]),
        (png::ColorType::Rgb, png::BitDepth::Eight, vec![8,8,8,9,0,0]),
        (png::ColorType::Grayscale, png::BitDepth::Sixteen, vec![8,255,9,0]),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(color);
            encoder.set_depth(depth);
            encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        }
        assert_eq!(inspect(&bytes, [0,0,2,1], 8).unwrap(), (2,1));
    }
    assert!(run(&[]).is_err());
    for percent in ["NaN", "inf", "-1", "101"] {
        let args = ["missing.png", "0", "0", "1", "1", "8", percent].map(str::to_owned);
        assert!(run(&args).unwrap_err().contains("percentage"));
    }
}
