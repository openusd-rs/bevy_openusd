#import bevy_pbr::{mesh_functions, forward_io::VertexOutput, view_transformations::position_world_to_clip}

struct Point {
    translation: vec4<f32>,
    rotation: vec4<f32>,
    scale: vec4<f32>,
}

fn rotate(rotation: vec4<f32>, value: vec3<f32>) -> vec3<f32> {
    return value + 2.0 * cross(rotation.xyz, cross(rotation.xyz, value) + rotation.w * value);
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> vertices: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> points: array<Point>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> vertices_per_point: u32;

@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) root: u32) -> VertexOutput {
    let point = points[index / vertices_per_point];
    let offset = (index % vertices_per_point) * 2u;
    let position = vertices[offset];
    let normal = vertices[offset + 1u];
    let world_from_local = mesh_functions::get_world_from_local(root);
    var out: VertexOutput;
    let local_position = rotate(point.rotation, position.xyz * point.scale.xyz) + point.translation.xyz;
    out.world_position = world_from_local * vec4<f32>(local_position, 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(rotate(point.rotation, normal.xyz / point.scale.xyz), root);
#ifdef VERTEX_UVS_A
    out.uv = vec2<f32>(position.w, normal.w);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = root;
#endif
    return out;
}
