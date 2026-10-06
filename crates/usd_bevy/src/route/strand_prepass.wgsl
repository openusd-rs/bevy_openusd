#import bevy_pbr::{
    mesh_functions,
    prepass_io::VertexOutput,
    view_transformations::position_world_to_clip,
}
#import "embedded://usd_bevy/route/strand_functions.wgsl"::ribbon

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(8) width: f32,
    @location(9) tangent: vec4<f32>,
#ifdef STRAND_NORMALS
    @location(10) normal: vec4<f32>,
#endif
};

// Shadow and depth views see strands at their true width.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#ifdef STRAND_NORMALS
    let normal = mesh_functions::mesh_normal_local_to_world(vertex.normal.xyz, vertex.instance_index);
    let strand = ribbon(world_from_local, vertex.position, vertex.tangent, normal, true, vertex.width, 0.0);
#else
    let strand = ribbon(world_from_local, vertex.position, vertex.tangent, vec3(0.0), false, vertex.width, 0.0);
#endif
    out.world_position = vec4(strand.world_position, 1.0);
    out.position = position_world_to_clip(strand.world_position);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = strand.normal;
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.previous_world_position = out.world_position;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
    return out;
}
