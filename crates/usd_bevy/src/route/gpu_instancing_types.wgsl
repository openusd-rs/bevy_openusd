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

struct Draw {
    world_from_instancer: mat4x4<f32>,
    part: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    roughness: f32,
    metallic: f32,
}

@group(2) @binding(0) var<storage, read> instances: array<Instance>;
@group(2) @binding(1) var<storage, read> visible: array<u32>;
@group(2) @binding(2) var<uniform> draw: Draw;

struct Placement {
    world_position: vec4<f32>,
    rotation: vec4<f32>,
    scale: vec3<f32>,
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

// Places a prototype vertex at the `instance_index`th visible instance.
fn place(position: vec3<f32>, instance_index: u32) -> Placement {
    let instance = instances[visible[instance_index]];
    var out: Placement;
    out.rotation = normalize(vec4<f32>(
        unpack2x16snorm(instance.rotation_xy),
        unpack2x16snorm(instance.rotation_zw),
    ));
    out.scale = vec3<f32>(instance.scale_x, instance.scale_y, instance.scale_z);
    let translation = vec3<f32>(instance.translation_x, instance.translation_y, instance.translation_z);
    let in_prototype = (draw.part * vec4<f32>(position, 1.0)).xyz;
    let in_instancer = rotate(out.rotation, in_prototype * out.scale) + translation;
    out.world_position = draw.world_from_instancer * vec4<f32>(in_instancer, 1.0);
    return out;
}
