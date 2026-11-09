import usd_bevy::route::flat_functions::surface_normal;
import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::pbr_prepass_functions;
@if(NORMAL_PREPASS) import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
@if(VISIBILITY_RANGE_DITHER) import bevy_pbr::render::pbr_functions::visibility_range_dither;

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var vertex = input;
    @if(NORMAL_PREPASS) vertex.world_normal = surface_normal(vertex.world_position.xyz, is_front);
    @if(NORMAL_PREPASS) let pbr = pbr_input_from_standard_material(vertex, is_front);
    @if(VISIBILITY_RANGE_DITHER) visibility_range_dither(vertex.position, vertex.visibility_range_dither);
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(vertex);
    var output: FragmentOutput;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) output.frag_depth = vertex.unclipped_depth;
    @if(NORMAL_PREPASS) output.normal = vec4(pbr.N * 0.5 + vec3(0.5), 1.0);
    @if(MOTION_VECTOR_PREPASS) output.motion_vector = pbr_prepass_functions::calculate_motion_vector(vertex.world_position, vertex.previous_world_position);
    return output;
}

@else
@fragment
fn fragment(vertex: VertexOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(vertex);
}
