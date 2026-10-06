#import bevy_pbr::view_transformations::position_world_to_clip
#import "embedded://usd_bevy/route/gpu_instancing_types.wgsl"::place

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> @builtin(position) vec4<f32> {
    var position = position_world_to_clip(place(vertex.position, vertex.instance_index).world_position.xyz);
#ifdef CLAMP_DEPTH
    // Without unclipped depth, casters before the cascade flatten onto it.
    position.z = min(position.z, 1.0);
#endif
    return position;
}
