#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import "embedded://usd_bevy/route/sheen_functions.wgsl"::sheen_light

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> sheen: vec4<f32>;

@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(input, is_front);
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    var output: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        output.color = apply_pbr_lighting(pbr);
        output.color = vec4(output.color.rgb + sheen_light(pbr, sheen.x, sheen.y), output.color.a);
    } else {
        output.color = pbr.material.base_color;
    }
    output.color = main_pass_post_lighting_processing(pbr, output.color);
    return output;
}
