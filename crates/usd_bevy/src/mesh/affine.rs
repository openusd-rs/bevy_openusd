use bevy::{math::DMat3, mesh::VertexAttributeValues, prelude::*};

/// Bakes an invertible affine matrix into positions, normals and tangent frames.
pub(crate) fn bake(mesh: &mut Mesh, matrix: Mat4) -> Option<()> {
    if !matrix.is_finite() || matrix.row(3) != Vec4::W { return None; }
    let linear = DMat3::from_cols(matrix.x_axis.truncate().as_dvec3(), matrix.y_axis.truncate().as_dvec3(), matrix.z_axis.truncate().as_dvec3());
    let determinant = linear.determinant();
    if !determinant.is_finite() || determinant == 0.0 { return None; }
    let normal_matrix = linear.inverse().transpose();
    if !normal_matrix.is_finite() { return None; }
    if let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
        for position in positions {
            let transformed = matrix.transform_point3(Vec3::from(*position));
            if !transformed.is_finite() { return None; }
            *position = transformed.to_array();
        }
    }
    if let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL) {
        for normal in normals { *normal = direction(normal_matrix, Vec3::from(*normal))?.to_array(); }
    }
    if let Some(VertexAttributeValues::Float32x4(tangents)) = mesh.attribute_mut(Mesh::ATTRIBUTE_TANGENT) {
        for tangent in tangents {
            let vector = direction(linear, Vec3::new(tangent[0], tangent[1], tangent[2]))?;
            if !tangent[3].is_finite() { return None; }
            *tangent = vector.extend(tangent[3] * determinant.signum() as f32).to_array();
        }
    }
    Some(())
}

fn direction(matrix: DMat3, vector: Vec3) -> Option<Vec3> {
    if !vector.is_finite() { return None; }
    if vector == Vec3::ZERO { return Some(Vec3::ZERO); }
    Some((matrix * vector.as_dvec3()).try_normalize()?.as_vec3())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::PrimitiveTopology;

    #[test]
    fn shear_preserves_points_and_orthogonal_tangent_frames() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0,0.0,1.0];3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0,0.0,0.0,1.0];3]);
        for scale in [1.0, -1.0, 1e-20] {
            let matrix = Mat4::from_cols(Vec4::new(scale,0.0,0.5*scale,0.0),
                Vec4::new(0.75*scale,scale,0.0,0.0), Vec4::new(0.0,0.0,scale,0.0), Vec4::new(4.0,2.0,1.0,1.0));
            let mut transformed = mesh.clone();
            bake(&mut transformed, matrix).unwrap();
            let VertexAttributeValues::Float32x3(positions) = transformed.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            assert_eq!(positions[1], matrix.transform_point3(Vec3::X).to_array());
            assert_eq!(positions[2], matrix.transform_point3(Vec3::Y).to_array());
            let VertexAttributeValues::Float32x3(normals) = transformed.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!() };
            let VertexAttributeValues::Float32x4(tangents) = transformed.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap() else { panic!() };
            let normal = Vec3::from(normals[0]);
            let tangent = Vec3::from_slice(&tangents[0][..3]);
            assert!(normal.dot(tangent).abs() < 1e-6);
            assert!((normal.length()-1.0).abs() < 1e-6);
            assert_eq!(tangents[0][3], scale.signum());
        }
        assert!(bake(&mut mesh.clone(), Mat4::from_scale(Vec3::new(1.0,0.0,1.0))).is_none());
        assert!(bake(&mut mesh.clone(), Mat4::perspective_rh(1.0,1.0,0.1,10.0)).is_none());
        assert!(bake(&mut mesh, Mat4::from_cols_array(&[f32::NAN;16])).is_none());
    }
}
