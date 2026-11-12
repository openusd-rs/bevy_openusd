use bevy::render::{
    render_resource::{
        BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
        TexelCopyBufferInfo, TexelCopyBufferLayout, TextureDimension, TextureFormat, TextureUsages,
    },
    renderer::{RenderDevice, RenderQueue},
    view::ViewTarget,
};
use std::{path::Path, sync::mpsc};

const BYTE_LIMIT: u64 = 64 * 1024 * 1024;

struct Layout {
    row_bytes: u32,
    padded_row_bytes: u32,
    buffer_bytes: u64,
}

fn layout(
    size: Extent3d,
    format: TextureFormat,
    dimension: TextureDimension,
    samples: u32,
    usage: TextureUsages,
) -> Result<Layout, String> {
    let pixel_bytes = match format {
        TextureFormat::Rgba8Unorm
        | TextureFormat::Rgba8UnormSrgb
        | TextureFormat::Bgra8Unorm
        | TextureFormat::Bgra8UnormSrgb => 4,
        TextureFormat::Rgba16Float => 8,
        TextureFormat::Rgba32Float => 16,
        _ => return Err("unsupported main target readback format".into()),
    };
    if size.width == 0
        || size.height == 0
        || size.depth_or_array_layers != 1
        || dimension != TextureDimension::D2
        || samples != 1
        || !usage.contains(TextureUsages::COPY_SRC)
    {
        return Err("invalid main target readback texture".into());
    }
    let row_bytes = u64::from(size.width) * pixel_bytes;
    let padded_row_bytes = row_bytes.div_ceil(256) * 256;
    let buffer_bytes = padded_row_bytes
        .checked_mul(u64::from(size.height))
        .filter(|bytes| *bytes <= BYTE_LIMIT)
        .ok_or("main target readback exceeds 64 MiB")?;
    Ok(Layout {
        row_bytes: row_bytes as u32,
        padded_row_bytes: padded_row_bytes as u32,
        buffer_bytes,
    })
}

fn unpad(bytes: &[u8], layout: &Layout) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 != layout.buffer_bytes {
        return Err("main target readback length mismatch".into());
    }
    Ok(bytes
        .chunks_exact(layout.padded_row_bytes as usize)
        .flat_map(|row| row[..layout.row_bytes as usize].iter().copied())
        .collect())
}

pub fn save(
    device: &RenderDevice,
    queue: &RenderQueue,
    target: &ViewTarget,
    output: &Path,
    frame: u32,
) -> Result<String, String> {
    let texture = target.main_texture();
    let size = texture.size();
    let format = texture.format();
    let layout = layout(
        size,
        format,
        texture.dimension(),
        texture.sample_count(),
        texture.usage(),
    )?;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("capture_main_target_readback"),
        size: layout.buffer_bytes,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.padded_row_bytes),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    let submitted = queue.submit([encoder.finish()]);
    let (tx, rx) = mpsc::sync_channel(1);
    buffer.slice(..).map_async(MapMode::Read, move |result| {
        let _ = tx.send(result.map_err(|error| error.to_string()));
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .map_err(|error| format!("main target polling failed: {error}"))?;
    rx.try_recv()
        .map_err(|error| format!("main target mapping incomplete: {error}"))??;
    let bytes = unpad(
        &buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| format!("main target range unavailable: {error}"))?,
        &layout,
    )?;
    buffer.unmap();
    std::fs::write(output.with_extension("gpu-main.bin"), &bytes)
        .map_err(|error| format!("main target write failed: {error}"))?;
    let report = format!(
        "gpu_main_scope=same-extraction-after-render-before-cleanup\ngpu_main_frame={frame}\ngpu_main_width={}\ngpu_main_height={}\ngpu_main_format={format:?}\ngpu_main_row_bytes={}\ngpu_main_bytes={}\n",
        size.width,
        size.height,
        layout.row_bytes,
        bytes.len(),
    );
    std::fs::write(output.with_extension("gpu-main.txt"), &report)
        .map_err(|error| format!("main target metadata write failed: {error}"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(width: u32, height: u32, format: TextureFormat) -> Result<Layout, String> {
        layout(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            format,
            TextureDimension::D2,
            1,
            TextureUsages::COPY_SRC,
        )
    }

    #[test]
    fn target_rows_are_aligned_and_bounded() {
        let layout = checked(1440, 920, TextureFormat::Rgba8UnormSrgb).unwrap();
        assert_eq!(layout.row_bytes, 5760);
        assert_eq!(layout.padded_row_bytes, 5888);
        assert_eq!(layout.buffer_bytes, 5_416_960);
        assert_eq!(
            checked(3, 2, TextureFormat::Rgba16Float).unwrap().row_bytes,
            24
        );
        for (w, h) in [
            (0, 1),
            (1, 0),
            (u32::MAX, 1),
            (1, u32::MAX),
            (u32::MAX, u32::MAX),
        ] {
            assert!(checked(w, h, TextureFormat::Rgba32Float).is_err());
        }
        assert!(checked(1, 1, TextureFormat::Depth32Float).is_err());
    }

    #[test]
    fn target_readback_rejects_incompatible_textures() {
        let size = Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        };
        for (dimension, samples, usage) in [
            (TextureDimension::D3, 1, TextureUsages::COPY_SRC),
            (TextureDimension::D2, 4, TextureUsages::COPY_SRC),
            (TextureDimension::D2, 1, TextureUsages::RENDER_ATTACHMENT),
        ] {
            assert!(layout(size, TextureFormat::Rgba8Unorm, dimension, samples, usage).is_err());
        }
        assert!(
            layout(
                Extent3d {
                    depth_or_array_layers: 2,
                    ..size
                },
                TextureFormat::Rgba8Unorm,
                TextureDimension::D2,
                1,
                TextureUsages::COPY_SRC
            )
            .is_err()
        );
    }

    #[test]
    fn target_readback_removes_only_row_padding() {
        let layout = checked(3, 2, TextureFormat::Rgba8UnormSrgb).unwrap();
        let mut bytes = vec![255; layout.buffer_bytes as usize];
        bytes[..12].copy_from_slice(&[1; 12]);
        bytes[256..268].copy_from_slice(&[2; 12]);
        assert_eq!(
            unpad(&bytes, &layout).unwrap(),
            [vec![1; 12], vec![2; 12]].concat()
        );
        assert!(unpad(&bytes[..bytes.len() - 1], &layout).is_err());
    }
}
