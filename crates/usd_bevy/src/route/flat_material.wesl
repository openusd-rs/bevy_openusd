import usd_bevy::route::flat_functions::surface_normal;
import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    pbr_types,
};
import bevy_pbr::decal::clustered::apply_decals;
@if(PREPASS_PIPELINE) import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
@if(PREPASS_PIPELINE) import bevy_pbr::deferred::functions::deferred_output;
@if(!PREPASS_PIPELINE) import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
@if(!PREPASS_PIPELINE) import bevy_pbr::render::pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing};
@if(VISIBILITY_RANGE_DITHER) import bevy_pbr::render::pbr_functions::visibility_range_dither;
@if(MATERIAL_OIT_ENABLED) import bevy_core_pipeline::oit::draw::oit_draw;

@if(PREPASS_PIPELINE)
@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var vertex = input;
    vertex.world_normal = surface_normal(vertex.world_position.xyz, is_front);
    @if(VISIBILITY_RANGE_DITHER) visibility_range_dither(vertex.position, vertex.visibility_range_dither);
    var pbr = pbr_input_from_standard_material(vertex, is_front);
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    apply_decals(&pbr);
    return deferred_output(vertex, pbr);
}

@if(!PREPASS_PIPELINE)
@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var vertex = input;
    vertex.world_normal = surface_normal(vertex.world_position.xyz, is_front);
    @if(VISIBILITY_RANGE_DITHER) visibility_range_dither(vertex.position, vertex.visibility_range_dither);
    var pbr = pbr_input_from_standard_material(vertex, is_front);
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    apply_decals(&pbr);
    var output: FragmentOutput;
    if (pbr.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        output.color = apply_pbr_lighting(pbr);
    } else {
        output.color = pbr.material.base_color;
    }
    output.color = main_pass_post_lighting_processing(pbr, output.color);
    @if(MATERIAL_OIT_ENABLED) {
        let alpha_mode = pbr.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
        if alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND {
            oit_draw(vertex.position, vec4(output.color.rgb * output.color.a, output.color.a));
            discard;
        }
        if alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_PREMULTIPLIED
            || alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_ADD {
            oit_draw(vertex.position, output.color);
            discard;
        }
    }
    return output;
}
