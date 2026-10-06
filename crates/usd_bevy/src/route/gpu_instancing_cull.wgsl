struct Instance {
    translation_x: f32,
    translation_y: f32,
    translation_z: f32,
    rotation_xy: u32,
    rotation_zw: u32,
    scale_x: f32,
    scale_y: f32,
    scale_z: f32,
}

struct Chunk {
    world_from_instancer: mat4x4<f32>,
    sphere: vec4<f32>,
    len: u32,
    world_scale: f32,
}

struct CullView {
    planes: array<vec4<f32>, 6>,
    camera: vec4<f32>,
    min_pixels: f32,
}

@group(0) @binding(0) var<storage, read> instances: array<Instance>;
@group(0) @binding(1) var<storage, read_write> visible: array<u32>;
@group(0) @binding(2) var<storage, read_write> count: atomic<u32>;
@group(0) @binding(3) var<uniform> chunk: Chunk;
@group(0) @binding(4) var<uniform> cull: CullView;

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= chunk.len {
        return;
    }
    let instance = instances[index];
    let rotation = normalize(vec4<f32>(
        unpack2x16snorm(instance.rotation_xy),
        unpack2x16snorm(instance.rotation_zw),
    ));
    let scale = vec3<f32>(instance.scale_x, instance.scale_y, instance.scale_z);
    let translation = vec3<f32>(instance.translation_x, instance.translation_y, instance.translation_z);
    let local = rotate(rotation, chunk.sphere.xyz * scale) + translation;
    let center = (chunk.world_from_instancer * vec4<f32>(local, 1.0)).xyz;
    let radius = chunk.sphere.w * max(abs(scale.x), max(abs(scale.y), abs(scale.z))) * chunk.world_scale;
    for (var plane = 0u; plane < 6u; plane++) {
        let half_space = cull.planes[plane];
        if dot(half_space.xyz, center) + half_space.w < -radius {
            return;
        }
    }
    let distance = max(length(center - cull.camera.xyz), 1e-4);
    if radius * cull.camera.w < cull.min_pixels * distance {
        return;
    }
    visible[atomicAdd(&count, 1u)] = index;
}
