#import bevy_pbr::{mesh_view_bindings::view, view_transformations::position_world_to_clip}

struct Ribbon {
    world_position: vec3<f32>,
    // The strand's projected width in pixels.
    pixels: f32,
    // The authored normal, else the side of a round strand this edge spans.
    normal: vec3<f32>,
}

fn toward_eye(position: vec3<f32>) -> vec3<f32> {
    let orthographic = view.clip_from_view[3][3] == 1.0;
    return select(normalize(view.world_position - position), view.world_from_view[2].xyz, orthographic);
}

// An authored ribbon normal turned to the side the eye sees, as thin
// two-sided strands such as leaflets are lit.
fn facing_normal(normal: vec3<f32>, position: vec3<f32>) -> vec3<f32> {
    let unit = normalize(normal);
    return select(unit, -unit, dot(unit, toward_eye(position)) < 0.0);
}

// The normal of a round strand across its ribbon: the interpolated `side`
// grows from the center to the rims and the rest bends toward the eye.
fn round_normal(side: vec3<f32>, position: vec3<f32>) -> vec3<f32> {
    return normalize(side + toward_eye(position) * sqrt(max(1.0 - dot(side, side), 0.0)));
}

// Spreads a centerline sample to its side of the ribbon: across the view, or
// across an authored world `normal`, by the strand width but never narrower
// than `min_pixels`.
fn ribbon(
    world_from_local: mat4x4<f32>,
    position: vec3<f32>,
    tangent: vec4<f32>,
    normal: vec3<f32>,
    oriented: bool,
    width: f32,
    min_pixels: f32,
) -> Ribbon {
    let center = (world_from_local * vec4(position, 1.0)).xyz;
    let along = normalize((world_from_local * vec4(tangent.xyz, 0.0)).xyz);
    let eye = toward_eye(center);
    var across = select(cross(along, eye), cross(normal, along), oriented);
    let fallback = cross(along, select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(along.y) > 0.9));
    across = normalize(select(across, fallback, dot(across, across) < 1e-12));
    let scale = (length(world_from_local[0].xyz) + length(world_from_local[1].xyz) + length(world_from_local[2].xyz)) / 3.0;
    let pixels_per_unit = view.clip_from_view[1][1] * 0.5 * view.viewport.w / position_world_to_clip(center).w;
    let world_width = width * scale;
    let drawn = max(world_width, min_pixels / pixels_per_unit);
    var out: Ribbon;
    out.world_position = center + across * (0.5 * drawn * sign(tangent.w));
    out.pixels = world_width * pixels_per_unit;
    out.normal = select(across * sign(tangent.w), normal, oriented);
    return out;
}
