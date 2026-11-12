import bevy_pbr::render::view_transformations::position_world_to_clip;
import usd_bevy::route::gpu_instancing_types::place;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> @builtin(position) vec4<f32> {
    var position = position_world_to_clip(place(vertex.position, vertex.instance_index).world_position.xyz);
    // Without unclipped depth, casters before the cascade flatten onto it.
    @if(CLAMP_DEPTH) position.z = min(position.z, 1.0);
    return position;
}
