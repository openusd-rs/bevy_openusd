#import bevy_pbr::{
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    pbr_types::PbrInput,
    shadows,
}

// Disney's sheen from each directional light: the light's color, tinted
// toward the base color's hue, strongest where it grazes past the view.
// Unlike the diffuse lobe it is not divided by pi.
fn sheen_light(pbr: PbrInput, weight: f32, tint: f32) -> vec3<f32> {
    let base = pbr.material.base_color.rgb;
    let luminance = dot(base, vec3(0.3, 0.6, 0.1));
    let hue = select(vec3(1.0), base / luminance, luminance > 0.0);
    let color = weight * (1.0 - pbr.material.metallic) * mix(vec3(1.0), hue, tint);
    let view_z = dot(vec4(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), pbr.world_position);
    var light = vec3(0.0);
    for (var i = 0u; i < view_bindings::lights.n_directional_lights; i++) {
        let directional = &view_bindings::lights.directional_lights[i];
        let to_light = (*directional).direction_to_light;
        let lit = saturate(dot(pbr.N, to_light));
        let grazing = pow(1.0 - saturate(dot(to_light, normalize(to_light + pbr.V))), 5.0);
        var shadow = 1.0;
        if (pbr.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
                && ((*directional).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            shadow = shadows::fetch_directional_shadow(i, pbr.world_position, pbr.world_normal, view_z, pbr.frag_coord.xy);
        }
        light += color * grazing * lit * shadow * (*directional).color.rgb;
    }
    return light * view_bindings::view.exposure;
}
