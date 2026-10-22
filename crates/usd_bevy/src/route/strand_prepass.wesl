import bevy_pbr::prepass::io::VertexOutput;
import bevy_pbr::render::{
    mesh_functions,
    view_transformations::position_world_to_clip,
};
import usd_bevy::route::strand_functions::ribbon;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(8) width: f32,
    @location(9) tangent: vec4<f32>,
    @if(STRAND_NORMALS) @location(10) normal: vec4<f32>,
};

// Shadow and depth views see strands at their true width.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    @if(STRAND_NORMALS) let normal = mesh_functions::mesh_normal_local_to_world(vertex.normal.xyz, vertex.instance_index);
    @if(STRAND_NORMALS) let strand = ribbon(world_from_local, vertex.position, vertex.tangent, normal, true, vertex.width, 0.0);
    @else let strand = ribbon(world_from_local, vertex.position, vertex.tangent, vec3(0.0), false, vertex.width, 0.0);
    out.world_position = vec4(strand.world_position, 1.0);
    out.position = position_world_to_clip(strand.world_position);
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) out.unclipped_depth = out.position.z;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) out.position.z = min(out.position.z, 1.0);
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) out.world_normal = strand.normal;
    @if(MOTION_VECTOR_PREPASS) out.previous_world_position = out.world_position;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) out.instance_index = vertex.instance_index;
    @if(VISIBILITY_RANGE_DITHER) out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
    return out;
}
