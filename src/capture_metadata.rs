use bevy::prelude::*;

pub(crate) fn camera_report(transform: &GlobalTransform, clip_from_view: Mat4) -> String {
    format!("camera_metadata_phase=Last-at-request\ncamera_eye={:?}\ncamera_forward={:?}\ncamera_up={:?}\ncamera_world_from_view_cols={:?}\ncamera_clip_from_view_cols={:?}\n",
        transform.translation(), transform.forward(), transform.up(),
        transform.to_matrix().to_cols_array(), clip_from_view.to_cols_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_metadata_uses_effective_world_transform_and_projection() {
        let transform = GlobalTransform::from(Transform::from_xyz(1.6, 0.8, 1.8)
            .looking_at(Vec3::new(0.0, -0.15, 0.0), Vec3::Y));
        let projection = Mat4::perspective_infinite_reverse_rh(0.7, 16.0 / 9.0, 0.2);
        let report = camera_report(&transform, projection);
        assert!(report.contains("camera_eye=Vec3(1.6, 0.8, 1.8)\n"));
        assert!(report.contains(&format!("camera_world_from_view_cols={:?}\n", transform.to_matrix().to_cols_array())));
        assert!(report.contains(&format!("camera_clip_from_view_cols={:?}\n", projection.to_cols_array())));
        assert!(report.contains(&format!("camera_forward={:?}\n", transform.forward())));
        assert!(report.contains(&format!("camera_up={:?}\n", transform.up())));
        assert!(!report.contains("Vec3(6.0, 4.0, 8.0)"));
    }
}
