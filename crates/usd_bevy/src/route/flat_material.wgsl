#import "embedded://usd_bevy/route/flat_functions.wgsl"::surface_normal
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    decal::clustered::apply_decals,
}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::{prepass_io::{VertexOutput, FragmentOutput}, pbr_deferred_functions::deferred_output}
#else
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#endif
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif
#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#import bevy_pbr::pbr_types
#endif

@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var vertex = input;
    vertex.world_normal = surface_normal(vertex.world_position.xyz, is_front);
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(vertex.position, vertex.visibility_range_dither);
#endif
    var pbr = pbr_input_from_standard_material(vertex, is_front);
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    apply_decals(&pbr);
#ifdef PREPASS_PIPELINE
    let output = deferred_output(vertex, pbr);
#else
    var output: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        output.color = apply_pbr_lighting(pbr);
    } else {
        output.color = pbr.material.base_color;
    }
    output.color = main_pass_post_lighting_processing(pbr, output.color);
#ifdef OIT_ENABLED
    let alpha_mode = pbr.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        oit_draw(vertex.position, output.color);
        discard;
    }
#endif
#endif
    return output;
}
