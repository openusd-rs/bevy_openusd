fn surface_normal(position: vec3<f32>, is_front: bool) -> vec3<f32> {
    let normal = normalize(cross(dpdy(position), dpdx(position)));
    return select(-normal, normal, is_front);
}
