#import bevy_pbr::{
    forward_io::Vertex,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings::view,
    pbr_functions,
    pbr_types,
    view_transformations::position_world_to_clip,
}

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

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
#ifdef VERTEX_COLORS
    @location(2) color: vec4<f32>,
#endif
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let instance = instances[visible[vertex.instance_index]];
    let rotation = normalize(vec4<f32>(
        unpack2x16snorm(instance.rotation_xy),
        unpack2x16snorm(instance.rotation_zw),
    ));
    let scale = vec3<f32>(instance.scale_x, instance.scale_y, instance.scale_z);
    let translation = vec3<f32>(instance.translation_x, instance.translation_y, instance.translation_z);
    let in_prototype = (draw.part * vec4<f32>(vertex.position, 1.0)).xyz;
    let in_instancer = rotate(rotation, in_prototype * scale) + translation;
    var out: VertexOutput;
    out.world_position = draw.world_from_instancer * vec4<f32>(in_instancer, 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef VERTEX_NORMALS
    let normal = rotate(rotation, (draw.part * vec4<f32>(vertex.normal, 0.0)).xyz / scale);
    out.world_normal = normalize((draw.world_from_instancer * vec4<f32>(normal, 0.0)).xyz);
#else
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
#ifdef VERTEX_NORMALS
    let facing = normalize(in.world_normal);
#else
    let facing = normalize(cross(dpdy(in.world_position.xyz), dpdx(in.world_position.xyz)));
#endif
    let normal = select(-facing, facing, is_front);
    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = draw.base_color;
#ifdef VERTEX_COLORS
    pbr.material.base_color = pbr.material.base_color * in.color;
#endif
    pbr.material.emissive = draw.emissive;
    pbr.material.perceptual_roughness = draw.roughness;
    pbr.material.metallic = draw.metallic;
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = normal;
    pbr.N = normal;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr.V = pbr_functions::calculate_view(in.world_position, pbr.is_orthographic);
    pbr.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    let color = pbr_functions::apply_pbr_lighting(pbr);
    return pbr_functions::main_pass_post_lighting_processing(pbr, color);
}
