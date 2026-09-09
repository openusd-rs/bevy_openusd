use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::Path;

fn save_readback(image: &Image, output: &Path) -> Result<(), String> {
    if output.extension().and_then(|value| value.to_str()) != Some("png") {
        return Err("USD_SCREENSHOT must end in .png".into());
    }
    let rgba = image.clone().try_into_dynamic().map_err(|error| error.to_string())?.to_rgba8();
    let (width, height) = rgba.dimensions();
    std::fs::write(output.with_extension("rgba"), rgba.as_raw()).map_err(|error| error.to_string())?;
    let report = format!("source=embedded-viewer\nwidth={width}\nheight={height}\nlayout=rgba8\nsource_format={:?}\nrow_bytes={}\nbytes={}\n", image.texture_descriptor.format, u64::from(width) * 4, rgba.len());
    std::fs::write(output.with_extension("capture.txt"), report).map_err(|error| error.to_string())?;
    rgba.save(output).map_err(|error| error.to_string())
}

pub fn configure(app: &mut App) {
    let Ok(output) = std::env::var("USD_SCREENSHOT") else { return };
    if let Ok(time) = std::env::var("USD_CAPTURE_TIME") {
        if let Ok(current) = time.parse::<f64>() {
            if current.is_finite() { app.insert_resource(usd_bevy::route::StageTime { current }); }
        }
    }
    app.add_systems(Update, move |mut commands: Commands, cameras: Query<&RenderTarget, With<Camera3d>>, mut frames: Local<u32>, mut requested: Local<bool>| {
        *frames += 1;
        if *frames >= 120 && !*requested {
            if let Some(target) = cameras.iter().next() {
                let output = output.clone();
                commands.spawn(Screenshot(target.clone())).observe(move |event: On<ScreenshotCaptured>| {
                    match save_readback(&event.image, Path::new(&output)) {
                        Ok(()) => eprintln!("VIEWPORT_CAPTURE_OK {output}"),
                        Err(error) => eprintln!("VIEWPORT_CAPTURE_ERROR {output}: {error}"),
                    }
                });
                *requested = true;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::TextureFormat;

    #[test]
    fn embedded_readback_is_tightly_packed_and_reports_dimensions() {
        let directory = std::env::temp_dir().join(format!("usd-embedded-capture-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("viewport.png");
        let mut image = Image::new_target_texture(3, 2, TextureFormat::Rgba8UnormSrgb, None);
        image.data = Some((0..24).collect());
        save_readback(&image, &output).unwrap();
        assert_eq!(std::fs::read(output.with_extension("rgba")).unwrap(), image.data.unwrap());
        let report = std::fs::read_to_string(output.with_extension("capture.txt")).unwrap();
        assert!(report.contains("width=3\nheight=2\n"));
        assert!(report.contains("row_bytes=12\nbytes=24\n"));
        assert!(output.is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn embedded_readback_reports_invalid_output_and_io_errors() {
        let image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
        assert!(save_readback(&image, Path::new("invalid.jpg")).is_err());
        let directory = std::env::temp_dir().join(format!("usd-capture-missing-{}", std::process::id()));
        assert!(save_readback(&image, &directory.join("missing/viewport.png")).is_err());
    }
}
