#import bevy_pbr::{
    forward_io::Vertex,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings::view,
    pbr_functions,
    pbr_types,
    view_transformations::position_world_to_clip,
}
#import "embedded://usd_bevy/route/gpu_instancing_types.wgsl"::{draw, place, rotate}
#import "embedded://usd_bevy/route/sheen_functions.wgsl"::sheen_light

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
#ifdef VERTEX_COLORS
    @location(2) color: vec4<f32>,
#endif
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let placed = place(vertex.position, vertex.instance_index);
    var out: VertexOutput;
    out.world_position = placed.world_position;
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef VERTEX_NORMALS
    let normal = rotate(placed.rotation, (draw.part * vec4<f32>(vertex.normal, 0.0)).xyz / placed.scale);
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
    var color = pbr_functions::apply_pbr_lighting(pbr);
    if draw.sheen > 0.0 {
        color = vec4(color.rgb + sheen_light(pbr, draw.sheen, draw.sheen_tint), color.a);
    }
    return pbr_functions::main_pass_post_lighting_processing(pbr, color);
}
