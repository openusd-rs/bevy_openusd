use bevy::mesh::{Indices, Mesh, VertexAttributeValues};

/// Builds a mesh from selected indices, remapping attributes and target-major morph data.
pub(crate) fn compact(mesh: &Mesh, indices: &Indices) -> Option<Mesh> {
    let count = mesh.count_vertices();
    if indices.iter().any(|index| index >= count)
        || mesh.attributes().any(|(_, values)| values.len() != count)
        || mesh.get_morph_targets().is_some_and(|targets|
            if count == 0 { !targets.is_empty() } else { targets.len() % count != 0 })
    { return None; }
    let sparse = indices.len() <= count / 8;
    let retained: Vec<_> = if sparse {
        let mut retained: Vec<_> = indices.iter().collect();
        retained.sort_unstable();
        retained.dedup();
        retained
    } else {
        let mut used = vec![false; count];
        for index in indices.iter() { used[index] = true; }
        used.iter().enumerate().filter_map(|(index, used)| used.then_some(index)).collect()
    };
    if retained.len() == count {
        let mut output = mesh.clone();
        output.insert_indices(indices.clone());
        return Some(output);
    }
    let indices = if sparse {
        Indices::U32(indices.iter().map(|old| retained.binary_search(&old).unwrap() as u32).collect())
    } else {
        let mut remap = vec![0_u32; count];
        for (new, &old) in retained.iter().enumerate() { remap[old] = new as u32; }
        Indices::U32(indices.iter().map(|old| remap[old]).collect())
    };
    let morph = mesh.get_morph_targets().map(|targets| targets.chunks_exact(count)
        .flat_map(|target| retained.iter().map(|&old| target[old])).collect());
    let mut output = Mesh::new(mesh.primitive_topology(), mesh.asset_usage);
    if retained.is_empty() { output.asset_usage.remove(bevy::asset::RenderAssetUsages::RENDER_WORLD); }
    output.enable_raytracing = mesh.enable_raytracing;
    output.final_aabb = mesh.final_aabb;
    for (attribute, values) in mesh.attributes() { output.insert_attribute(*attribute, select(values, &retained)); }
    output.insert_indices(indices);
    if !retained.is_empty() && let Some(morph) = morph { output.set_morph_targets(morph); }
    if let Some(names) = mesh.morph_target_names() { output.set_morph_target_names(names.to_vec()); }
    Some(output)
}

fn select(values: &VertexAttributeValues, retained: &[usize]) -> VertexAttributeValues {
    macro_rules! select_variants {
        ($($variant:ident),+ $(,)?) => {
            match values { $(VertexAttributeValues::$variant(values) =>
                VertexAttributeValues::$variant(retained.iter().map(|&index| values[index]).collect()),)+ }
        };
    }
    select_variants!(Uint8, Uint8x2, Uint8x4, Sint8, Sint8x2, Sint8x4, Unorm8, Unorm8x2, Unorm8x4,
        Snorm8, Snorm8x2, Snorm8x4, Uint16, Uint16x2, Uint16x4, Sint16, Sint16x2, Sint16x4,
        Unorm16, Unorm16x2, Unorm16x4, Snorm16, Snorm16x2, Snorm16x4, Float16, Float16x2, Float16x4,
        Float32, Float32x2, Float32x3, Float32x4, Uint32, Uint32x2, Uint32x3, Uint32x4,
        Sint32, Sint32x2, Sint32x3, Sint32x4, Float64, Float64x2, Float64x3, Float64x4,
        Unorm10_10_10_2, Unorm8x4Bgra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::RenderAssetUsages, math::Vec3, mesh::{PrimitiveTopology, morph::MorphAttributes}};

    fn compact(mesh: &mut Mesh) -> bool {
        let Some(indices) = mesh.indices().cloned() else { return false };
        let Some(output) = super::compact(mesh, &indices) else { return false };
        *mesh = output;
        true
    }

    #[test]
    fn sparse_selection_preserves_order_attributes_and_target_major_morphs() {
        let mut source = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        source.insert_attribute(Mesh::ATTRIBUTE_POSITION, (0..1024).map(|i| [i as f32;3]).collect::<Vec<_>>());
        source.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4((0..1024).map(|i| [i;4]).collect()));
        source.set_morph_targets((0..2048).map(|i| MorphAttributes {
            position: Vec3::splat(i as f32), ..Default::default()
        }).collect());
        for indices in [Indices::U16(vec![900, 3, 500, 3, 900, 500]), Indices::U32(vec![900, 3, 500, 3, 900, 500])] {
            let result = super::compact(&source, &indices).unwrap();
            assert_eq!(result.count_vertices(), 3);
            assert_eq!(result.indices().unwrap().iter().collect::<Vec<_>>(), [2, 0, 1, 0, 2, 1]);
            for (attribute, values) in source.attributes() {
                assert_eq!(result.attribute(attribute.id).unwrap(), &select(values, &[3, 500, 900]));
            }
            for target in 0..2 {
                for (new, old) in [3, 500, 900].into_iter().enumerate() {
                    assert_eq!(result.get_morph_targets().unwrap()[target*3+new],
                        source.get_morph_targets().unwrap()[target*1024+old]);
                }
            }
        }
        assert_eq!(source.count_vertices(), 1024);
        assert!(source.indices().is_none());
    }

    #[test]
    fn borrowed_source_supports_independent_selections_without_mutation() {
        let mut source = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        source.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0;3], [1.0;3], [2.0;3], [3.0;3]]);
        source.set_morph_targets((0..8).map(|index| MorphAttributes::new(Vec3::splat(index as f32), Vec3::ZERO, Vec3::ZERO)).collect());
        let before = source.clone();
        for selected in [[3,1,3], [0,2,0]] {
            let indices = Indices::U32(selected.to_vec());
            let result = super::compact(&source, &indices).unwrap();
            assert_eq!(result.count_vertices(), 2);
            let VertexAttributeValues::Float32x3(positions) = result.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            for (&old, new) in selected.iter().zip(result.indices().unwrap().iter()) {
                assert_eq!(positions[new], [old as f32;3]);
                for target in 0..2 {
                    assert_eq!(result.get_morph_targets().unwrap()[target*2+new], before.get_morph_targets().unwrap()[target*4+old as usize]);
                }
            }
        }
        assert_eq!(source.attribute(Mesh::ATTRIBUTE_POSITION), before.attribute(Mesh::ATTRIBUTE_POSITION));
        assert_eq!(source.get_morph_targets(), before.get_morph_targets());
        assert!(source.indices().is_none());
        let all = super::compact(&source, &Indices::U16(vec![3,2,1,0])).unwrap();
        assert_eq!(all.count_vertices(), 4);
        assert_eq!(all.indices().unwrap().iter().collect::<Vec<_>>(), [3,2,1,0]);
        assert_eq!(all.get_morph_targets(), source.get_morph_targets());
    }

    #[test]
    fn compaction_preserves_indexed_attributes_and_each_morph_target() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0;3], [1.0;3], [2.0;3], [3.0;3]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0;4], [1;4], [2;4], [3;4]]));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[0.25;4];4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0,1.0,0.0];4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0;2], [0.25;2], [0.5;2], [1.0;2]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0,0.0,0.0,-1.0];4]);
        mesh.insert_indices(Indices::U16(vec![3,1,3]));
        mesh.set_morph_targets((0..8).map(|index| MorphAttributes::new(Vec3::splat(index as f32), Vec3::Y, Vec3::X)).collect());
        mesh.set_morph_target_names(vec!["first".into(), "second".into()]);
        let before = mesh.clone();
        assert!(compact(&mut mesh));
        assert_eq!(mesh.count_vertices(), 2);
        assert_eq!(mesh.indices().unwrap().iter().collect::<Vec<_>>(), [1,0,1]);
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX), Some(&VertexAttributeValues::Uint16x4(vec![[1;4], [3;4]])));
        let old = before.get_morph_targets().unwrap();
        assert_eq!(mesh.get_morph_targets().unwrap(), [old[1], old[3], old[5], old[7]]);
        assert_eq!(mesh.morph_target_names(), before.morph_target_names());
        for (attribute, values) in before.attributes() {
            let selected = mesh.attribute(*attribute).unwrap();
            let stride = values.get_bytes().len() / values.len();
            for (old, new) in before.indices().unwrap().iter().zip(mesh.indices().unwrap().iter()) {
                assert_eq!(&values.get_bytes()[old*stride..(old+1)*stride], &selected.get_bytes()[new*stride..(new+1)*stride]);
            }
        }
        assert_eq!(mesh.asset_usage, before.asset_usage);
        assert_eq!(mesh.enable_raytracing, before.enable_raytracing);
        assert_eq!(mesh.final_aabb, before.final_aabb);
        mesh.insert_indices(Indices::U32(vec![]));
        assert!(compact(&mut mesh));
        assert_eq!(mesh.count_vertices(), 0);
        assert!(mesh.get_morph_targets().is_none());
        assert!(mesh.asset_usage.contains(RenderAssetUsages::MAIN_WORLD));
        assert!(!mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD));
        assert!(compact(&mut mesh));
    }

    #[test]
    fn invalid_compaction_inputs_are_unchanged() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0;3];4]);
        assert!(!compact(&mut mesh));
        mesh.insert_indices(Indices::U32(vec![9]));
        assert!(!compact(&mut mesh));
        assert_eq!(mesh.count_vertices(), 4);
        mesh.insert_indices(Indices::U32(vec![1]));
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0;2];3]);
        assert!(!compact(&mut mesh));
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().len(), 4);
        mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0);
        mesh.set_morph_targets(vec![MorphAttributes::default();3]);
        assert!(!compact(&mut mesh));
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().unwrap().iter().collect::<Vec<_>>(), [1]);
    }
}
