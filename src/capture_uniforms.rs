use bevy::{
    diagnostic::FrameCount,
    pbr::{GpuLights, LightMeta, ViewLightsUniformOffset},
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        init_gpu_resource,
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, MapMode, PollType,
            ShaderType, encase,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};
use std::sync::{Arc, Mutex, mpsc};

const BYTE_LIMIT: u64 = 64 * 1024;

#[derive(Component)]
pub struct UniformCapture(pub Entity);

struct Captured {
    screenshot: Entity,
    report: Result<String, String>,
}

#[derive(Clone, Default, Resource)]
pub struct UniformProbe(Arc<Mutex<Option<Captured>>>);

impl UniformProbe {
    pub fn report(&self, screenshot: Entity) -> Result<String, String> {
        let captured = self.0.lock().unwrap();
        let Some(captured) = captured.as_ref() else {
            return Err("GPU uniform readback missing".into());
        };
        if captured.screenshot != screenshot {
            return Err("GPU uniform readback belongs to another screenshot".into());
        }
        captured.report.clone()
    }
}

struct Request {
    screenshot: Entity,
    camera: MainEntity,
    frame: u32,
}

#[derive(Default, Resource)]
struct Pending(Option<Request>);

pub fn configure(app: &mut App) {
    let probe = UniformProbe::default();
    app.insert_resource(probe.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(probe)
        .init_resource::<Pending>()
        .add_systems(
            RenderStartup,
            enable_copy
                .after(init_gpu_resource::<ViewUniforms>)
                .after(init_gpu_resource::<LightMeta>),
        )
        .add_systems(ExtractSchedule, extract_request)
        .add_systems(
            Render,
            capture
                .after(RenderSystems::Prepare)
                .before(RenderSystems::Render),
        );
}

fn enable_copy(mut views: ResMut<ViewUniforms>, mut lights: ResMut<LightMeta>) {
    views.uniforms.add_usages(BufferUsages::COPY_SRC);
    lights.view_gpu_lights.add_usages(BufferUsages::COPY_SRC);
}

fn extract_request(
    captures: Extract<Query<(Entity, &UniformCapture)>>,
    frame: Extract<Res<FrameCount>>,
    mut pending: ResMut<Pending>,
    mut seen: Local<Option<Entity>>,
) {
    for (screenshot, capture) in &captures {
        if *seen != Some(screenshot) {
            pending.0 = Some(Request {
                screenshot,
                camera: capture.0.into(),
                frame: frame.0,
            });
            *seen = Some(screenshot);
        }
    }
}

type CaptureView = (
    &'static MainEntity,
    &'static ViewUniformOffset,
    &'static ViewLightsUniformOffset,
);

fn capture(
    mut pending: ResMut<Pending>,
    probe: Res<UniformProbe>,
    views: Query<CaptureView, With<ExtractedCamera>>,
    uniforms: Res<ViewUniforms>,
    lights: Res<LightMeta>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(request) = pending.0.take() else {
        return;
    };
    let report = (|| {
        let (_, view, light) = views
            .iter()
            .find(|(entity, ..)| **entity == request.camera)
            .ok_or("captured camera has no prepared view/light offsets")?;
        let view_buffer = uniforms.uniforms.buffer().ok_or("view buffer missing")?;
        let light_buffer = lights
            .view_gpu_lights
            .buffer()
            .ok_or("light buffer missing")?;
        let view_size = ViewUniform::min_size().get();
        let light_size = GpuLights::min_size().get();
        validate_range(view_buffer.size(), u64::from(view.offset), view_size)?;
        validate_range(light_buffer.size(), u64::from(light.offset), light_size)?;
        let bytes = read_buffers(
            &device,
            &queue,
            [
                (view_buffer, u64::from(view.offset), view_size),
                (light_buffer, u64::from(light.offset), light_size),
            ],
        )?;
        decode_report(request.frame, view.offset, light.offset, &bytes)
    })();
    *probe.0.lock().unwrap() = Some(Captured {
        screenshot: request.screenshot,
        report,
    });
}

fn validate_range(capacity: u64, offset: u64, size: u64) -> Result<(), String> {
    if size == 0
        || size > BYTE_LIMIT
        || !offset.is_multiple_of(4)
        || !size.is_multiple_of(4)
        || offset.checked_add(size).is_none_or(|end| end > capacity)
    {
        return Err("invalid or oversized uniform readback range".into());
    }
    Ok(())
}

fn read_buffers(
    device: &RenderDevice,
    queue: &RenderQueue,
    inputs: [(&Buffer, u64, u64); 2],
) -> Result<Vec<u8>, String> {
    let size = inputs.iter().map(|(_, _, size)| size).sum();
    validate_range(BYTE_LIMIT, 0, size)?;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("capture_view_light_uniforms"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    let mut destination = 0;
    for (source, offset, size) in inputs {
        encoder.copy_buffer_to_buffer(source, offset, &buffer, destination, size);
        destination += size;
    }
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
        .map_err(|error| format!("GPU uniform polling failed: {error}"))?;
    rx.try_recv()
        .map_err(|error| format!("GPU uniform mapping incomplete: {error}"))??;
    let bytes = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|error| format!("GPU uniform range unavailable: {error}"))?
        .to_vec();
    buffer.unmap();
    Ok(bytes)
}

fn decode_report(
    frame: u32,
    view_offset: u32,
    light_offset: u32,
    bytes: &[u8],
) -> Result<String, String> {
    let view_size = ViewUniform::min_size().get() as usize;
    if bytes.len() != view_size + GpuLights::min_size().get() as usize {
        return Err("GPU uniform readback length mismatch".into());
    }
    let view: ViewUniform = encase::UniformBuffer::new(&bytes[..view_size])
        .create()
        .map_err(|error| format!("GPU view decode failed: {error}"))?;
    let lights: GpuLights = encase::UniformBuffer::new(&bytes[view_size..])
        .create()
        .map_err(|error| format!("GPU light decode failed: {error}"))?;
    if view.frame_count != frame {
        return Err(format!(
            "GPU uniform frame mismatch: extracted {frame}, uploaded {}",
            view.frame_count
        ));
    }
    Ok(format!(
        "gpu_uniform_scope=same-extraction-before-render-GPU-buffer-readback\ngpu_uniform_frame={frame}\ngpu_uniform_view_offset={view_offset}\ngpu_uniform_light_offset={light_offset}\ngpu_uniform_bytes={}\ngpu_view clip_from_world={:?} unjittered_clip_from_world={:?} world_from_clip={:?} world_from_view={:?} view_from_world={:?} clip_from_view={:?} view_from_clip={:?} world_position={:?} exposure={:?} viewport={:?} main_pass_viewport={:?} frustum={:?} lod_view_world_position={:?} color_grading={:?} mip_bias={:?}\ngpu_lights {lights:?}\n",
        bytes.len(),
        view.clip_from_world,
        view.unjittered_clip_from_world,
        view.world_from_clip,
        view.world_from_view,
        view.view_from_world,
        view.clip_from_view,
        view.view_from_clip,
        view.world_position,
        view.exposure,
        view.viewport,
        view.main_pass_viewport,
        view.frustum,
        view.lod_view_world_position,
        view.color_grading,
        view.mip_bias,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_ranges_are_aligned_bounded_and_overflow_safe() {
        assert!(validate_range(1024, 256, 768).is_ok());
        for (capacity, offset, size) in [
            (1024, 0, 0),
            (1024, 1, 4),
            (1024, 0, 3),
            (1024, 512, 516),
            (u64::MAX, u64::MAX - 3, 4),
            (u64::MAX, 0, BYTE_LIMIT + 4),
        ] {
            assert!(validate_range(capacity, offset, size).is_err());
        }
    }

    #[test]
    fn report_requires_the_corresponding_screenshot() {
        let mut world = World::new();
        let screenshot = world.spawn_empty().id();
        let other = world.spawn_empty().id();
        let probe = UniformProbe::default();
        assert!(probe.report(screenshot).is_err());
        *probe.0.lock().unwrap() = Some(Captured {
            screenshot,
            report: Ok("measured".into()),
        });
        assert_eq!(probe.report(screenshot), Ok("measured".into()));
        assert!(probe.report(other).is_err());
        probe.0.lock().unwrap().as_mut().unwrap().report = Err("failed mapping".into());
        assert_eq!(probe.report(screenshot), Err("failed mapping".into()));
    }

    #[test]
    fn decode_rejects_incomplete_data_and_different_frame() {
        assert!(decode_report(10, 0, 0, &[]).is_err());
        let bytes = vec![0; (ViewUniform::min_size().get() + GpuLights::min_size().get()) as usize];
        assert!(
            decode_report(10, 0, 0, &bytes)
                .unwrap_err()
                .contains("frame mismatch")
        );
        let report = decode_report(0, 256, 512, &bytes).unwrap();
        assert!(report.contains("gpu_uniform_frame=0\n"));
        assert!(report.contains("gpu_uniform_view_offset=256\n"));
        assert!(report.contains("gpu_uniform_light_offset=512\n"));
        assert!(report.contains("gpu_view clip_from_world="));
        assert!(report.contains("gpu_lights GpuLights"));
    }

    #[test]
    fn decoded_view_preserves_small_changes_and_capture_frame() {
        let view_size = ViewUniform::min_size().get() as usize;
        let mut bytes = vec![0; view_size + GpuLights::min_size().get() as usize];
        let mut view: ViewUniform = encase::UniformBuffer::new(&bytes[..view_size])
            .create()
            .unwrap();
        view.frame_count = 42;
        view.exposure = 1.0;
        encase::UniformBuffer::new(&mut bytes[..view_size])
            .write(&view)
            .unwrap();
        let first = decode_report(42, 0, 0, &bytes).unwrap();
        view.exposure = f32::from_bits(1.0_f32.to_bits() + 1);
        encase::UniformBuffer::new(&mut bytes[..view_size])
            .write(&view)
            .unwrap();
        let second = decode_report(42, 0, 0, &bytes).unwrap();
        assert_ne!(first, second);
        assert!(second.contains(&format!("exposure={:?}", view.exposure)));
        assert!(second.contains("gpu_uniform_frame=42\n"));
    }
}
