#import "embedded://usd_bevy/route/flat_functions.wgsl"::surface_normal
#import bevy_pbr::{prepass_io, pbr_prepass_functions}
#ifdef NORMAL_PREPASS
#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#endif
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(input: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
    var vertex = input;
#ifdef NORMAL_PREPASS
    vertex.world_normal = surface_normal(vertex.world_position.xyz, is_front);
    let pbr = pbr_input_from_standard_material(vertex, is_front);
#endif
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(vertex.position, vertex.visibility_range_dither);
#endif
    pbr_prepass_functions::prepass_alpha_discard(vertex);
    var output: prepass_io::FragmentOutput;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    output.frag_depth = vertex.unclipped_depth;
#endif
#ifdef NORMAL_PREPASS
    output.normal = vec4(pbr.N * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    output.motion_vector = pbr_prepass_functions::calculate_motion_vector(vertex.world_position, vertex.previous_world_position);
#endif
    return output;
}
#else
@fragment
fn fragment(vertex: prepass_io::VertexOutput) {
    pbr_prepass_functions::prepass_alpha_discard(vertex);
}
#endif
