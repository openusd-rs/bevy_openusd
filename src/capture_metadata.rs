use bevy::prelude::*;

pub(crate) fn environment_report(map: Option<&bevy::light::EnvironmentMapLight>,
    state: Option<&usd_bevy::route::dome_environment::UsdDomeEnvironmentState>,
    recorded: u64, active: usize) -> String {
    let mut report = format!("environment_metadata_phase=Last-at-request\nenvironment_state={state:?}\nrecorded_environment_generations={recorded}\nactive_environment_generators={active}\n");
    if let Some(map) = map {
        report.push_str(&format!("environment_intensity={}\nenvironment_rotation={:?}\n", map.intensity, map.rotation));
    }
    report
}

pub(crate) fn camera_report(transform: &GlobalTransform, clip_from_view: Mat4) -> String {
    format!("camera_metadata_phase=Last-at-request\ncamera_eye={:?}\ncamera_forward={:?}\ncamera_up={:?}\ncamera_world_from_view_cols={:?}\ncamera_clip_from_view_cols={:?}\n",
        transform.translation(), transform.forward(), transform.up(),
        transform.to_matrix().to_cols_array(), clip_from_view.to_cols_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_metadata_distinguishes_attachment_and_filtering() {
        use usd_bevy::route::dome_environment::UsdDomeEnvironmentState;
        let absent = environment_report(None, None, 0, 0);
        assert!(absent.contains("environment_state=None\n"));
        assert!(!absent.contains("environment_intensity="));
        let map = bevy::light::EnvironmentMapLight { intensity: 0.0,
            rotation: Quat::from_rotation_y(std::f32::consts::PI), ..Default::default() };
        let report = environment_report(Some(&map), Some(&UsdDomeEnvironmentState::Attached), 2, 1);
        for expected in ["environment_state=Some(Attached)\n", "environment_intensity=0\n",
            "recorded_environment_generations=2\n", "active_environment_generators=1\n"] {
            assert!(report.contains(expected), "{report}");
        }
        assert!(report.contains(&format!("environment_rotation={:?}\n", map.rotation)));
    }

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
