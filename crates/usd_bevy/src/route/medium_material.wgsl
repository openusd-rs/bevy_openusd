#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    view_transformations::{frag_coord_to_ndc, position_ndc_to_world},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(input, is_front);
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    // The medium a view ray crosses, from this surface to the scene behind,
    // absorbs the light passing through it. Refraction keeps the material's
    // own thickness, so the floor stays in place.
#ifdef DEPTH_PREPASS
    let behind = position_ndc_to_world(vec3(frag_coord_to_ndc(input.position).xy, prepass_depth(input.position, 0u)));
    let crossed = min(distance(behind, input.world_position.xyz), 8.0 * pbr.material.attenuation_distance);
#else
    let crossed = 2.0 * pbr.material.attenuation_distance;
#endif
    let absorption = pow(1.0 - pbr.material.attenuation_color.rgb, vec3(2.718282)) / pbr.material.attenuation_distance;
    pbr.material.base_color = vec4(pbr.material.base_color.rgb * exp(-crossed * absorption), pbr.material.base_color.a);
    var output: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        output.color = apply_pbr_lighting(pbr);
    } else {
        output.color = pbr.material.base_color;
    }
    output.color = main_pass_post_lighting_processing(pbr, output.color);
    return output;
}
