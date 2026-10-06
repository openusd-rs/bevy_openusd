#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
    forward_io::VertexOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import "embedded://usd_bevy/route/strand_functions.wgsl"::{facing_normal, ribbon, round_normal}
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
    @location(8) width: f32,
    @location(9) tangent: vec4<f32>,
#ifdef STRAND_NORMALS
    @location(10) normal: vec4<f32>,
#endif
};

struct StrandOutput {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) samples: u32,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#ifdef STRAND_NORMALS
    let normal = mesh_functions::mesh_normal_local_to_world(vertex.normal.xyz, vertex.instance_index);
    let strand = ribbon(world_from_local, vertex.position, vertex.tangent, normal, true, vertex.width, 1.0);
#else
    let strand = ribbon(world_from_local, vertex.position, vertex.tangent, vec3(0.0), false, vertex.width, 1.0);
#endif
    out.world_normal = strand.normal;
    out.position = position_world_to_clip(strand.world_position);
    // The projected width rides in the unused w for the fragment's coverage.
    out.world_position = vec4(strand.world_position, strand.pixels);
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
    return out;
}

fn hash(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// Keeps each of four samples with probability `coverage`, seeded by the
// fragment's position so overlapping strands draw independently.
fn coverage_mask(coverage: f32, world: vec3<f32>, pixel: vec2<f32>) -> u32 {
    let seed = hash(bitcast<u32>(world.x) ^ hash(bitcast<u32>(world.y) ^ hash(bitcast<u32>(world.z)
        ^ hash(u32(pixel.x) | (u32(pixel.y) << 16u)))));
    var mask = 0u;
    for (var index = 0u; index < 4u; index += 1u) {
        if f32(hash(seed + index) >> 8u) < coverage * 16777216.0 {
            mask |= 1u << index;
        }
    }
    return mask;
}

@fragment
fn fragment(input: VertexOutput) -> StrandOutput {
    var vertex = input;
    let coverage = clamp(vertex.world_position.w, 0.0, 1.0);
    vertex.world_position.w = 1.0;
#ifdef STRAND_NORMALS
    vertex.world_normal = facing_normal(vertex.world_normal, vertex.world_position.xyz);
#else
    vertex.world_normal = round_normal(vertex.world_normal, vertex.world_position.xyz);
#endif
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(vertex.position, vertex.visibility_range_dither);
#endif
    // The normal already faces the eye, whichever way the ribbon winds.
    var pbr = pbr_input_from_standard_material(vertex, true);
    var output: StrandOutput;
    output.samples = coverage_mask(coverage * pbr.material.base_color.a, vertex.world_position.xyz, vertex.position.xy);
    if output.samples == 0u {
        discard;
    }
    pbr.material.base_color.a = 1.0;
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        output.color = apply_pbr_lighting(pbr);
    } else {
        output.color = pbr.material.base_color;
    }
    output.color = main_pass_post_lighting_processing(pbr, output.color);
    return output;
}
