use bevy::render::settings::WgpuLimits;

pub fn configure(supported: &WgpuLimits, requested: &mut WgpuLimits) {
    requested.max_storage_buffers_per_shader_stage = requested
        .max_storage_buffers_per_shader_stage
        .max(supported.max_storage_buffers_per_shader_stage.min(16));
}

#[test]
fn deformation_storage_limits_fit_the_adapter_and_preserve_host_requirements() {
    for (available, existing, expected) in [
        (0, 0, 0),
        (4, 4, 4),
        (8, 8, 8),
        (9, 8, 9),
        (16, 8, 16),
        (32, 8, 16),
        (32, 24, 24),
    ] {
        let supported = WgpuLimits {
            max_storage_buffers_per_shader_stage: available,
            ..Default::default()
        };
        let mut requested = WgpuLimits {
            max_storage_buffers_per_shader_stage: existing,
            max_sampled_textures_per_shader_stage: 32,
            ..Default::default()
        };
        let mut wanted = requested.clone();
        wanted.max_storage_buffers_per_shader_stage = expected;
        <super::UsdApp as super::WindowApp>::configure_gpu_limits(&supported, &mut requested);
        assert_eq!(requested, wanted);
        assert!(requested.max_storage_buffers_per_shader_stage <= available);
    }
}
