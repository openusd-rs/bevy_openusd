#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
    forward_io::VertexOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
    @location(8) width: f32,
};

struct StrandOutput {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) samples: u32,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
    // Projected strand width in pixels, carried in the unused w.
    let scale = (length(world_from_local[0].xyz) + length(world_from_local[1].xyz) + length(world_from_local[2].xyz)) / 3.0;
    let pixels_per_unit = view.clip_from_view[1][1] * 0.5 * view.viewport.w / out.position.w;
    out.world_position.w = vertex.width * scale * pixels_per_unit;
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
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> StrandOutput {
    var vertex = input;
    let coverage = clamp(vertex.world_position.w, 0.0, 1.0);
    vertex.world_position.w = 1.0;
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(vertex.position, vertex.visibility_range_dither);
#endif
    var pbr = pbr_input_from_standard_material(vertex, is_front);
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
