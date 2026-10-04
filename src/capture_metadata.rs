use bevy::prelude::*;

pub(crate) fn mesh_residency_report(assets: &Assets<Mesh>, references: impl IntoIterator<Item = (AssetId<Mesh>, bool)>) -> String {
    let mut usage = std::collections::HashMap::<AssetId<Mesh>, u8>::new();
    for (id, visible) in references { *usage.entry(id).or_default() |= if visible { 1 } else { 2 }; }
    let mut buckets = [[0usize; 7]; 4];
    for (id, mesh) in assets.iter() {
        let row = &mut buckets[usize::from(*usage.get(&id).unwrap_or(&0))];
        row[0] += 1;
        let render_world = mesh.asset_usage.contains(bevy::asset::RenderAssetUsages::RENDER_WORLD);
        row[4] += usize::from(render_world);
        match (mesh.try_attributes(), mesh.try_indices_option(), mesh.try_has_morph_targets()) {
            (Ok(attributes), Ok(indices), Ok(_)) => {
                let vertices = attributes.map(|(_, values)| values.get_bytes().len()).sum::<usize>();
                let indices = indices.map_or(0, |indices| match indices {
                    bevy::mesh::Indices::U16(values) => size_of_val(values.as_slice()),
                    bevy::mesh::Indices::U32(values) => size_of_val(values.as_slice()),
                });
                let morph = mesh.get_morph_targets().map_or(0, size_of_val);
                row[1] += vertices;
                row[2] += indices;
                row[3] += morph;
                if render_world { row[5] += vertices + indices + morph; }
            }
            _ => row[6] += 1,
        }
    }
    let missing = usage.keys().filter(|id| !assets.contains(**id)).count();
    let mut report = format!("mesh_residency_scope=unique-retained-cpu-payload-by-hierarchy-visibility\nmesh_residency_excludes=frustum-culling,gpu-allocation,textures,skin-palettes,allocator-overhead\nmesh_residency_missing_assets={missing}\n");
    for (name, row) in ["unreferenced", "visible_only", "hidden_only", "shared"].into_iter().zip(buckets) {
        for (field, value) in ["assets", "vertex_bytes", "index_bytes", "morph_bytes", "render_world_assets", "render_world_cpu_bytes", "unavailable_cpu_assets"].into_iter().zip(row) {
            report.push_str(&format!("mesh_residency_{name}_{field}={value}\n"));
        }
    }
    report
}

pub(crate) fn environment_report(map: Option<&bevy::light::EnvironmentMapLight>,
    state: Option<&usd_bevy::route::dome_environment::UsdDomeEnvironmentState>,
    recorded: u64, active: usize, phase: &str) -> String {
    let mut report = format!("environment_metadata_phase={phase}\nenvironment_state={state:?}\nrecorded_environment_generations={recorded}\nactive_environment_generators={active}\n");
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
    fn residency_counts_shared_assets_once_and_separates_hidden_payload() {
        let mut assets = Assets::<Mesh>::default();
        let shared = assets.add(Mesh::from(Rectangle::default()));
        let hidden = assets.add(Mesh::from(Rectangle::default()));
        let unreferenced = assets.add(Mesh::from(Rectangle::default()));
        let missing = assets.add(Mesh::from(Rectangle::default()));
        assets.remove(missing.id());
        let report = mesh_residency_report(&assets, [(shared.id(), true), (shared.id(), false),
            (shared.id(), true), (hidden.id(), false), (missing.id(), true)]);
        for (field, value) in [("shared_assets", 1), ("hidden_only_assets", 1), ("visible_only_assets", 0),
            ("unreferenced_assets", 1), ("missing_assets", 1), ("shared_vertex_bytes", 128),
            ("hidden_only_vertex_bytes", 128), ("shared_render_world_cpu_bytes", 152)] {
            assert!(report.contains(&format!("mesh_residency_{field}={value}\n")), "{report}");
        }
        assert!(assets.contains(unreferenced.id()));
        assets.get_mut(&hidden).unwrap().take_gpu_data().unwrap();
        let report = mesh_residency_report(&assets, [(hidden.id(), false)]);
        assert!(report.contains("mesh_residency_hidden_only_unavailable_cpu_assets=1\n"));
        assert!(report.contains("mesh_residency_hidden_only_vertex_bytes=0\n"));
    }

    #[test]
    fn environment_metadata_distinguishes_attachment_and_filtering() {
        use usd_bevy::route::dome_environment::UsdDomeEnvironmentState;
        let absent = environment_report(None, None, 0, 0, "test");
        assert!(absent.contains("environment_state=None\n"));
        assert!(!absent.contains("environment_intensity="));
        let map = bevy::light::EnvironmentMapLight { intensity: 0.0,
            rotation: Quat::from_rotation_y(std::f32::consts::PI), ..Default::default() };
        let report = environment_report(Some(&map), Some(&UsdDomeEnvironmentState::Attached), 2, 1, "test");
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
