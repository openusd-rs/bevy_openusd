//! GPU morph targets with generated face normals or authored vertex normals.

use bevy::{mesh::morph::{MeshMorphWeights, MorphAttributes, MAX_MORPH_WEIGHTS, MAX_TEXTURE_WIDTH}, prelude::*};
use super::RouteCtx;

#[derive(Component)]
pub struct UsdGpuMorph {
    weights: Vec<f32>,
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    let Some(owned) = world.entity_mut(entity).take::<UsdGpuMorph>() else { return };
    if matches!(world.get::<MeshMorphWeights>(entity), Some(MeshMorphWeights::Value { weights }) if *weights == owned.weights) {
        world.entity_mut(entity).remove::<MeshMorphWeights>();
    }
    super::deformation::release(world, entity, super::deformation::MORPH);
    super::flat_material::clear(world, entity);
}

pub(crate) fn inherit(world: &mut World, parent: Entity, child: Entity, flat: bool) {
    let Some(weights) = world.get::<UsdGpuMorph>(parent).map(|morph| morph.weights.clone()) else { return };
    set_weights(world, child, weights);
    if flat { super::flat_material::attach(world, child); }
    else { super::flat_material::clear(world, child); }
}

pub(crate) fn set_weights(world: &mut World, entity: Entity, weights: Vec<f32>) {
    super::deformation::acquire(world, entity, super::deformation::MORPH);
    world.entity_mut(entity).insert((
        MeshMorphWeights::Value { weights: weights.clone() },
        UsdGpuMorph { weights }));
}

pub(crate) fn attach(ctx: &RouteCtx, world: &mut World, entity: Entity) -> anyhow::Result<()> {
    let read = crate::read::geom::read_mesh_at(ctx.stage, ctx.path, ctx.time)?.ok_or_else(|| anyhow::anyhow!("missing morph mesh"))?;
    let mut mesh = crate::mesh::mesh_from_usd(&read);
    let weights = prepare(ctx, &read, &mut mesh)?;
    let handle = super::cache::intern_mesh(world, mesh);
    set_weights(world, entity, weights);
    world.entity_mut(entity).insert(Mesh3d(handle))
        .remove::<super::gpu_skin::UsdCpuSkinFallback>();
    if crate::mesh::uses_flat_normals(&read) { super::flat_material::attach(world, entity); }
    else { super::flat_material::clear(world, entity); }
    Ok(())
}

pub(crate) fn prepare(ctx: &RouteCtx, read: &crate::read::geom::ReadMesh, mesh: &mut Mesh) -> anyhow::Result<Vec<f32>> {
    let authored = read.normals.is_some();
    anyhow::ensure!(crate::mesh::uses_flat_normals(&read) || authored,
        "GPU morph normal mode is unsupported");
    let sample = crate::read::skel::morph_sample(ctx.stage, ctx.path, ctx.time)?;
    anyhow::ensure!(!sample.targets.is_empty() && sample.targets.len() <= MAX_MORPH_WEIGHTS, "unsupported morph target count");
    let mapping = crate::mesh::vertex_point_indices(&read);
    anyhow::ensure!(!mapping.is_empty() && mapping.len() == mesh.count_vertices(), "invalid morph vertex mapping");
    anyhow::ensure!(mapping.len() <= (MAX_TEXTURE_WIDTH as usize).pow(2) / MorphAttributes::COMPONENT_COUNT,
        "morph target exceeds Bevy texture-backend capacity");
    let normals = if authored { Some(crate::read::skel::morph_normals(read, &sample)?.0) } else { None };
    if read.uvs.is_some() {
        let mut deformed = read.clone();
        deformed.triangulation_points = Some(read.points.clone());
        for (point, position) in deformed.points.iter_mut().enumerate() {
            let mut value = Vec3::from(*position);
            for (target, weight) in sample.targets.iter().zip(&sample.weights) {
                value += Vec3::from(target[point]) * *weight;
            }
            anyhow::ensure!(value.is_finite(), "morphed tangent position is nonfinite");
            *position = value.to_array();
        }
        deformed.normals = normals;
        let tangent_mesh = crate::mesh::mesh_from_usd(&deformed);
        anyhow::ensure!(tangent_mesh.count_vertices() == mesh.count_vertices(), "morphed tangent vertex mapping changed");
        if let Some(tangents) = tangent_mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents.clone());
        } else { mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT); }
    }
    let attributes = sample.targets.iter().zip(&sample.normal_targets).flat_map(|(target, normals)| mapping.iter().map(move |&point|
        MorphAttributes::new(target[point].into(), if authored { normals[point].into() } else { Vec3::ZERO }, Vec3::ZERO))).collect();
    mesh.set_morph_targets(attributes);
    Ok(sample.weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::visibility::NoFrustumCulling;

    #[test]
    fn combined_normal_fixture_keeps_skin_morph_uvs_and_materials() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_tangent_normals.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<StandardMaterial>>();
        for time in [0.0, 5.0, 10.0] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            assert!(read.double_sided);
            assert_eq!(read.subsets.len(), 1);
            let mut mesh = crate::mesh::mesh_from_usd(&read);
            let weights = prepare(&ctx, &read, &mut mesh).unwrap();
            assert_eq!(weights, [time as f32 / 10.0]);
            assert!(mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
            assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
            assert!(mesh.get_morph_targets().unwrap().iter().any(|target| target.normal != Vec3::ZERO));
            let skin = crate::read::skel::gpu_skin_sample(&stage, &path, Some(time)).unwrap();
            assert!((skin.matrices[0].x_axis.truncate().length() - 2.0).abs() < 1e-5);
            assert!((skin.matrices[0].z_axis.truncate().length() - 0.5).abs() < 1e-5);
            for path in [path.clone(), path.append_path("Part").unwrap()] {
                let (handle, warnings) = super::super::material::resolve_material(
                    &RouteCtx::at(&stage, &path, Some(time)), &mut world).unwrap().unwrap();
                assert!(warnings.is_empty(), "{warnings:?}");
                assert!(world.resource::<Assets<StandardMaterial>>().get(&handle).unwrap().normal_map_texture.is_some());
            }
        }
    }

    #[test]
    fn sampled_morph_tangents_match_cpu_geometry() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_tangent_normals.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let mut changed = false;
        for time in [0.0, 5.0, 10.0, 0.0] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            let mut gpu = crate::mesh::mesh_from_usd(&read);
            let rest = gpu.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap().clone();
            prepare(&ctx, &read, &mut gpu).unwrap();
            let cpu = crate::mesh::mesh_from_usd(&super::super::skel::deformed_mesh(&ctx).unwrap().unwrap());
            let bevy::mesh::VertexAttributeValues::Float32x4(actual) = gpu.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap() else { panic!("gpu tangents") };
            let bevy::mesh::VertexAttributeValues::Float32x4(expected) = cpu.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap() else { panic!("cpu tangents") };
            let bevy::mesh::VertexAttributeValues::Float32x4(rest) = rest else { panic!("rest tangents") };
            assert_eq!(actual.len(), expected.len());
            for ((actual, expected), rest) in actual.iter().zip(expected).zip(rest) {
                changed |= !Vec4::from(*expected).abs_diff_eq(Vec4::from(rest), 1e-5);
                assert!(Vec4::from(*actual).abs_diff_eq(Vec4::from(*expected), 1e-5), "tangent at {time}: {actual:?} != {expected:?}");
            }
        }
        assert!(changed, "fixture must distinguish sampled and rest tangents");
    }

    #[test]
    fn corner_normals_use_the_source_points_skin_influences() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_corner_influences.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        for time in [0.0, 5.0, 10.0] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let cpu = crate::mesh::mesh_from_usd(&super::super::skel::deformed_mesh(&ctx).unwrap().unwrap());
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            let base = crate::mesh::mesh_from_usd(&read);
            let mapping = crate::mesh::vertex_point_indices(&read);
            assert_eq!(mapping, vec![0,1,2,0,2,3]);
            let sample = crate::read::skel::gpu_skin_sample(&stage, &path, Some(time)).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(normals) = base.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!() };
            let bevy::mesh::VertexAttributeValues::Float32x3(expected) = cpu.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!() };
            for (corner, &point) in mapping.iter().enumerate() {
                let matrix = (0..4).fold(Mat4::ZERO, |m, slot| m + sample.matrices[sample.indices[point][slot] as usize] * sample.weights[point][slot]);
                let normal = (Mat3::from_mat4(matrix).inverse().transpose() * Vec3::from(normals[corner])).normalize();
                assert!(normal.abs_diff_eq(Vec3::from(expected[corner]), 1e-5));
            }
            assert_ne!(expected[0], expected[3]);
        }
    }

    #[test]
    fn indexed_normal_interpolations_match_cpu_after_morph_and_skin() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_corner_normals.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let attribute = stage.prim(&path).unwrap().attribute("primvars:normals");
        for (mode, indices) in [("constant", vec![1]), ("uniform", vec![0,1]),
            ("faceVarying", vec![0,0,0,1,1,1]), ("vertex", vec![0,1,0,1]), ("varying", vec![0,1,0,1])] {
            attribute.clone().set_metadata("interpolation", openusd::sdf::Value::Token(mode.into())).unwrap();
            stage.prim(&path).unwrap().attribute("primvars:normals:indices").set(openusd::sdf::Value::IntVec(indices)).unwrap();
            for time in [0.0, 5.0, 10.0] {
                let ctx = RouteCtx::at(&stage, &path, Some(time));
                let cpu = super::super::skel::deformed_mesh(&ctx).unwrap().unwrap();
                let cpu_mesh = crate::mesh::mesh_from_usd(&cpu);
                let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
                let mut gpu_mesh = crate::mesh::mesh_from_usd(&read);
                let weights = prepare(&ctx, &read, &mut gpu_mesh).unwrap();
                let sample = crate::read::skel::gpu_skin_sample(&stage, &path, Some(time)).unwrap();
                let bevy::mesh::VertexAttributeValues::Float32x3(base) = gpu_mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!() };
                let bevy::mesh::VertexAttributeValues::Float32x3(expected) = cpu_mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!() };
                assert_eq!(base.len(), expected.len(), "{mode}");
                let targets = gpu_mesh.get_morph_targets().unwrap();
                let normal_matrix = Mat3::from_mat4(sample.matrices[0]).inverse().transpose();
                for (vertex, (base, expected)) in base.iter().zip(expected).enumerate() {
                    let morphed = Vec3::from(*base) + targets[vertex].normal * weights[0];
                    assert!((normal_matrix * morphed).normalize().abs_diff_eq(Vec3::from(*expected), 1e-5), "{mode} at {time} vertex {vertex}");
                }
            }
        }
    }

    #[test]
    fn combined_normals_use_inverse_transpose_after_morphing() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_normals.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        for time in [0.0, 5.0, 10.0] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let cpu = super::super::skel::deformed_mesh(&ctx).unwrap().unwrap();
            let normals = cpu.normals.unwrap().values;
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            let mut mesh = crate::mesh::mesh_from_usd(&read);
            let weights = prepare(&ctx, &read, &mut mesh).unwrap();
            let targets = mesh.get_morph_targets().unwrap();
            for point in 0..4 {
                let morphed = Vec3::Z + targets[point].normal * weights[0];
                let expected = (rotation * (morphed / Vec3::new(2.0,1.0,0.5))).normalize();
                assert!(Vec3::from(normals[point]).distance(expected) < 1e-5);
            }
        }
        stage.attribute("/Test/Skel/Anim.scales").unwrap().set(openusd::sdf::Value::Vec3hVec(vec![[0.0,1.0,1.0].map(openusd::gf::f16::from_f32).into()])).unwrap();
        assert!(super::super::skel::deformed_mesh(&RouteCtx::at(&stage, &path, Some(10.0))).is_err());
        assert!(crate::read::skel::gpu_skin_sample(&stage, &path, Some(10.0)).err().unwrap().to_string().contains("singular"));
    }

    #[test]
    fn authored_normal_targets_match_cpu_and_keep_standard_material() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_normals.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<super::super::flat_material::FlatMaterial>>();
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(material.clone())).id();
        for time in [0.0, 5.0, 10.0] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            attach(&ctx, &mut world, entity).unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let targets = mesh.get_morph_targets().unwrap();
            assert_eq!(targets.len(), 4);
            assert_eq!(targets[0].normal, Vec3::new(0.4,0.4,-0.3));
            assert!(targets[1..].iter().all(|target| target.normal == Vec3::ZERO));
            let cpu = super::super::skel::deformed_mesh(&ctx).unwrap().unwrap();
            let normals = cpu.normals.unwrap();
            assert!(Vec3::from(normals.values[0]).distance(Vec3::Z + targets[0].normal * (time as f32 / 10.0)) < 1e-6);
            assert!(normals.values[1..].iter().all(|normal| *normal == [0.0,0.0,1.0]));
            assert!(world.get::<MeshMaterial3d<super::super::flat_material::FlatMaterial>>(entity).is_none());
            assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0, material);
        }
    }

    #[test]
    fn morph_weights_reuse_geometry_and_expand_sparse_seams() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_animation.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<super::super::flat_material::FlatMaterial>>();
        world.init_resource::<super::super::cache::ProjectionCache>();
        let base = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(base.clone())).id();
        attach(&RouteCtx::at(&stage, &path, Some(0.0)), &mut world, entity).unwrap();
        let handle = world.get::<Mesh3d>(entity).unwrap().0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        let read = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mapping = crate::mesh::vertex_point_indices(&read);
        for (attribute, point) in mesh.get_morph_targets().unwrap().iter().zip(mapping) {
            assert_eq!(attribute.position, if point == 0 { Vec3::Z } else { Vec3::ZERO });
        }
        for time in [5.0, 10.0] {
            attach(&RouteCtx::at(&stage, &path, Some(time)), &mut world, entity).unwrap();
            assert_eq!(world.get::<Mesh3d>(entity).unwrap().0, handle);
            assert!(matches!(world.get::<MeshMorphWeights>(entity), Some(MeshMorphWeights::Value { weights }) if weights == &vec![time as f32 / 10.0]));
        }
        clear(&mut world, entity);
        assert!(world.get::<MeshMorphWeights>(entity).is_none());
        assert!(world.get::<NoFrustumCulling>(entity).is_none());
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0, base);
        world.entity_mut(entity).insert(NoFrustumCulling);
        attach(&RouteCtx::at(&stage, &path, Some(0.0)), &mut world, entity).unwrap();
        world.entity_mut(entity).insert(MeshMorphWeights::Value { weights: vec![0.3] });
        clear(&mut world, entity);
        assert!(world.get::<MeshMorphWeights>(entity).is_some());
        assert!(world.get::<NoFrustumCulling>(entity).is_some());
    }
}
