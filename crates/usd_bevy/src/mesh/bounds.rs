//! World-space bounds for static and GPU-deformed mesh instances.

use bevy::{camera::primitives::Aabb, ecs::system::SystemParam, prelude::*};
use bevy::mesh::{VertexAttributeValues, morph::{MeshMorphWeights, MorphWeights}, skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}};

/// Samples current mesh bounds after global transforms and joint palettes update.
#[derive(SystemParam)]
pub struct MeshBounds<'w, 's> {
    meshes: Option<Res<'w, Assets<Mesh>>>,
    inverse: Option<Res<'w, Assets<SkinnedMeshInverseBindposes>>>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    morphs: Query<'w, 's, &'static MorphWeights>,
}

impl MeshBounds<'_, '_> {
    /// Returns world bounds, or None for empty, unavailable or malformed geometry.
    pub fn get(&self, handle: &Mesh3d, transform: &GlobalTransform, bounds: Option<&Aabb>,
        skin: Option<&SkinnedMesh>, morph: Option<&MeshMorphWeights>) -> Option<(Vec3, Vec3)> {
        let mesh = self.meshes.as_ref().and_then(|meshes| meshes.get(&handle.0));
        if mesh.is_some_and(|mesh| mesh.count_vertices() == 0 || mesh.indices().is_some_and(|indices| indices.is_empty())) { return None; }
        if skin.is_none() && morph.is_none() && mesh.is_none() {
            let bounds = bounds?;
            let points = (0..8).map(|i| {
                let sign = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 });
                transform.transform_point(Vec3::from(bounds.center) + Vec3::from(bounds.half_extents) * sign)
            });
            return enclose(points);
        }
        let weights = match morph {
            Some(MeshMorphWeights::Value { weights }) => weights.as_slice(),
            Some(MeshMorphWeights::Reference(entity)) => self.morphs.get(*entity).ok()?.weights(),
            None => &[],
        };
        let palette = if let Some(skin) = skin {
            let inverse = self.inverse.as_ref()?.get(&skin.inverse_bindposes)?;
            if inverse.len() != skin.joints.len() { return None; }
            Some(skin.joints.iter().zip(inverse.iter()).map(|(joint, inverse)|
                Some(self.globals.get(*joint).ok()?.to_matrix() * *inverse)).collect::<Option<Vec<_>>>()?)
        } else { None };
        deformed(mesh?, transform.to_matrix(), weights, palette.as_deref())
    }
}

fn enclose(points: impl Iterator<Item = Vec3>) -> Option<(Vec3, Vec3)> {
    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for point in points {
        if !point.is_finite() { return None; }
        low = low.min(point); high = high.max(point);
    }
    low.is_finite().then_some((low, high))
}

fn deformed(mesh: &Mesh, transform: Mat4, weights: &[f32], palette: Option<&[Mat4]>) -> Option<(Vec3, Vec3)> {
    let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)? else { return None };
    let targets = mesh.get_morph_targets().unwrap_or(&[]);
    if !weights.is_empty() && targets.len() != positions.len().checked_mul(weights.len())? { return None; }
    let skin = if palette.is_some() {
        let VertexAttributeValues::Uint16x4(indices) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)? else { return None };
        let VertexAttributeValues::Float32x4(weights) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)? else { return None };
        Some((indices, weights))
    } else { None };
    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut visit = |index: usize| -> Option<()> {
        let mut point = Vec3::from(*positions.get(index)?);
        for (target, weight) in weights.iter().enumerate() { point += targets[target*positions.len()+index].position * *weight; }
        let point = if let Some((indices, weights)) = skin {
            let mut matrix = Mat4::ZERO;
            for (&joint, &weight) in indices.get(index)?.iter().zip(weights.get(index)?) {
                matrix += *palette?.get(joint as usize)? * weight;
            }
            (matrix * point.extend(1.0)).truncate()
        } else { transform.transform_point3(point) };
        if !point.is_finite() { return None; }
        low = low.min(point); high = high.max(point);
        Some(())
    };
    if let Some(indices) = mesh.indices() { for index in indices.iter() { visit(index)?; } }
    else { for index in 0..positions.len() { visit(index)?; } }
    low.is_finite().then_some((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::{Indices, PrimitiveTopology, morph::MorphAttributes};

    #[test]
    fn morph_precedes_skin_and_bounds_ignore_undrawn_vertices() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[1.0,0.0,0.0], [1000.0;3]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0;4];2]));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0,0.0,0.0,0.0];2]);
        mesh.set_morph_targets(vec![MorphAttributes::new(Vec3::X, Vec3::ZERO, Vec3::ZERO);2]);
        mesh.insert_indices(Indices::U16(vec![0]));
        let palette = [Mat4::from_scale_rotation_translation(Vec3::splat(2.0), Quat::IDENTITY, Vec3::Y)];
        let result = deformed(&mesh, Mat4::from_translation(Vec3::splat(50.0)), &[0.5], Some(&palette)).unwrap();
        assert_eq!(result, (Vec3::new(3.0,1.0,0.0), Vec3::new(3.0,1.0,0.0)));
        assert!(deformed(&mesh, Mat4::IDENTITY, &[f32::NAN], Some(&palette)).is_none());
        assert!(deformed(&mesh, Mat4::IDENTITY, &[0.5], Some(&[])).is_none());
        assert!(deformed(&mesh, Mat4::IDENTITY, &[0.5,0.5], None).is_none());
        mesh.insert_indices(Indices::U16(vec![2]));
        assert!(deformed(&mesh, Mat4::IDENTITY, &[], None).is_none());
        mesh.insert_indices(Indices::U16(vec![]));
        assert!(deformed(&mesh, Mat4::IDENTITY, &[], None).is_none());
    }

    #[test]
    fn combined_usd_deformation_bounds_match_cpu_at_multiple_times() {
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_subsets.usda")).unwrap();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        let entity = world.spawn_empty().id();
        let transform = Mat4::from_scale_rotation_translation(Vec3::new(2.0,0.5,3.0),
            Quat::from_rotation_y(0.7), Vec3::new(4.0,-2.0,8.0));
        for time in [0.0,15.0,30.0,45.0,60.0] {
            let ctx = crate::route::RouteCtx::at(&stage, &path, Some(time));
            crate::route::gpu_skin::attach(&ctx, &mut world, entity, crate::read::skel::has_blend_shapes(ctx.stage, ctx.path)).unwrap();
            let handle = world.get::<Mesh3d>(entity).unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&handle.0).unwrap();
            let skin = world.get::<crate::route::gpu_skin::UsdGpuSkin>(entity).unwrap();
            let palette: Vec<_> = skin.matrices.iter().map(|matrix| transform * *matrix).collect();
            let MeshMorphWeights::Value { weights } = world.get::<MeshMorphWeights>(entity).unwrap() else { panic!() };
            let actual = deformed(mesh, transform, weights, Some(&palette)).unwrap();
            let cpu = crate::read::skel::skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            let expected = enclose(cpu.iter().map(|point| transform.transform_point3(Vec3::from(*point)))).unwrap();
            assert!(actual.0.abs_diff_eq(expected.0, 1e-4), "time={time} {actual:?} {expected:?}");
            assert!(actual.1.abs_diff_eq(expected.1, 1e-4));
        }
    }

    #[test]
    fn instance_bounds_resolve_shared_morphs_and_inverse_bindposes() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[1.0,0.0,0.0]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0;4]]));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0,0.0,0.0,0.0]]);
        mesh.set_morph_targets(vec![MorphAttributes::new(Vec3::X, Vec3::ZERO, Vec3::ZERO)]);
        let handle = Mesh3d(world.resource_mut::<Assets<Mesh>>().add(mesh));
        let weights = world.spawn(MorphWeights::new(vec![0.5], None).unwrap()).id();
        let joint = world.spawn(GlobalTransform::from_translation(Vec3::new(10.0,0.0,0.0))).id();
        let inverse = world.resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
            .add(SkinnedMeshInverseBindposes::from(vec![Mat4::from_scale(Vec3::splat(2.0))]));
        let skin = SkinnedMesh { inverse_bindposes: inverse, joints: vec![joint] };
        let mut state = bevy::ecs::system::SystemState::<MeshBounds>::new(&mut world);
        let get = |world: &World, state: &mut bevy::ecs::system::SystemState<MeshBounds>| state.get(world).unwrap()
            .get(&handle, &GlobalTransform::IDENTITY, None, Some(&skin), Some(&MeshMorphWeights::Reference(weights)));
        assert_eq!(get(&world, &mut state), Some((Vec3::new(13.0,0.0,0.0), Vec3::new(13.0,0.0,0.0))));
        world.despawn(joint);
        assert!(get(&world, &mut state).is_none());
    }
}
