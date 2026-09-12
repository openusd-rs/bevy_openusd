//! Bevy GPU skinning with stage-evaluated joint palettes.

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use super::RouteCtx;

#[derive(Resource, Default)]
pub struct GpuSkinningEnabled;

#[derive(Component)]
pub struct UsdGpuSkin {
    pub matrices: Vec<Mat4>,
    pub joints: Vec<Entity>,
}

#[derive(Component, Debug)]
pub struct UsdCpuSkinFallback(pub String);

/// Enables supported GPU skinning and morph targets; other bindings use the CPU route.
pub struct UsdGpuSkinningPlugin;

impl Plugin for UsdGpuSkinningPlugin {
    fn build(&self, app: &mut App) {
        super::flat_material::configure(app);
        if !app.world().contains_resource::<Assets<SkinnedMeshInverseBindposes>>() {
            app.init_asset::<SkinnedMeshInverseBindposes>();
        }
        app.init_resource::<GpuSkinningEnabled>().add_systems(
            PostUpdate,
            update_joint_globals.after(bevy::transform::TransformSystems::Propagate)
                .after(super::xform::UsdTransformSystems::Propagate),
        );
    }
}

fn update_joint_globals(world: &mut World) {
    let updates: Vec<_> = world.query::<(&GlobalTransform, &UsdGpuSkin)>().iter(world)
        .flat_map(|(transform, skin)| skin.joints.iter().zip(&skin.matrices)
            .map(move |(&joint, matrix)| (joint, GlobalTransform::from(transform.to_matrix() * *matrix))))
        .collect();
    for (joint, transform) in updates {
        if let Some(mut global) = world.get_mut::<GlobalTransform>(joint) { *global = transform; }
    }
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::gpu_morph::clear(world, entity);
    super::flat_material::clear(world, entity);
    if let Some(skin) = world.entity_mut(entity).take::<UsdGpuSkin>() {
        for joint in skin.joints { world.despawn(joint); }
        super::deformation::release(world, entity, super::deformation::SKIN);
    }
    world.entity_mut(entity).remove::<SkinnedMesh>();
}

pub(crate) fn attach(ctx: &RouteCtx, world: &mut World, entity: Entity) -> anyhow::Result<()> {
    let read = crate::read::geom::read_mesh_at(ctx.stage, ctx.path, ctx.time)?.ok_or_else(|| anyhow::anyhow!("missing mesh"))?;
    anyhow::ensure!(!read.points.is_empty(), "cannot skin an empty point array");
    let sample = crate::read::skel::gpu_skin_sample(ctx.stage, ctx.path, ctx.time)?;
    let skinned_tangents = read.uvs.is_some() && sample.normal_corrections.iter().any(|matrix| *matrix != Mat3::IDENTITY);
    let mut mesh = crate::mesh::assemble_mesh(&read, None, !skinned_tangents);
    let morph_weights = if crate::read::skel::has_blend_shapes(ctx.stage, ctx.path) {
        Some(super::gpu_morph::prepare(ctx, &read, &mut mesh, !skinned_tangents)?)
    } else { None };
    let source_points = crate::mesh::vertex_point_indices(&read);
    anyhow::ensure!(source_points.len() == mesh.count_vertices(), "skin vertex map does not match render mesh");
    correct_normals(&mut mesh, &source_points, &sample.normal_corrections, morph_weights.as_deref().unwrap_or(&[]))?;
    if skinned_tangents {
        correct_tangents(ctx, &mut mesh, &source_points, &sample)?;
    }
    let indices: Vec<_> = source_points.iter().map(|&point| sample.indices[point]).collect();
    let weights: Vec<_> = source_points.iter().map(|&point| sample.weights[point]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, bevy::mesh::VertexAttributeValues::Uint16x4(indices));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    let handle = super::cache::intern_mesh(world, mesh);
    let reusable = world.get::<UsdGpuSkin>(entity).is_some_and(|skin| skin.joints.len() == sample.matrices.len()
        && skin.joints.iter().all(|joint| world.get_entity(*joint).is_ok()));
    if !reusable {
        clear(world, entity);
        let joints: Vec<_> = sample.matrices.iter().map(|_| world.spawn((GlobalTransform::IDENTITY, ChildOf(entity))).id()).collect();
        let inverse = world.resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
            .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; joints.len()]));
        world.entity_mut(entity).insert((
            SkinnedMesh { inverse_bindposes: inverse, joints: joints.clone() },
            UsdGpuSkin { matrices: sample.matrices, joints },
        ));
    } else {
        world.get_mut::<UsdGpuSkin>(entity).unwrap().matrices = sample.matrices;
    }
    super::deformation::acquire(world, entity, super::deformation::SKIN);
    if let Some(weights) = morph_weights { super::gpu_morph::set_weights(world, entity, weights); }
    world.entity_mut(entity).insert(Mesh3d(handle))
        .remove::<UsdCpuSkinFallback>();
    if crate::mesh::uses_flat_normals(&read) { super::flat_material::attach(world, entity); }
    else { super::flat_material::clear(world, entity); }
    Ok(())
}

fn correct_normals(mesh: &mut Mesh, source_points: &[usize], corrections: &[Mat3], weights: &[f32]) -> anyhow::Result<()> {
    if corrections.iter().all(|matrix| *matrix == Mat3::IDENTITY) { return Ok(()); }
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL) {
        for (normal, &point) in normals.iter_mut().zip(source_points) {
            let value = corrections[point] * Vec3::from(*normal);
            anyhow::ensure!(value.is_finite(), "nonfinite corrected normal");
            *normal = value.to_array();
        }
    }
    if let Some(targets) = mesh.get_morph_targets() {
        let mut targets = targets.to_vec();
        for (index, target) in targets.iter_mut().enumerate() {
            target.normal = corrections[source_points[index % source_points.len()]] * target.normal;
            anyhow::ensure!(target.normal.is_finite(), "nonfinite corrected morph normal");
        }
        mesh.set_morph_targets(targets);
    }
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        for (vertex, normal) in normals.iter().enumerate() {
            let mut value = Vec3::from(*normal);
            if let Some(targets) = mesh.get_morph_targets() {
                for (target, weight) in weights.iter().enumerate() { value += targets[target * normals.len() + vertex].normal * *weight; }
            }
            anyhow::ensure!(value.is_finite() && value.length_squared() > 1e-20, "invalid corrected skinned normal");
        }
    }
    Ok(())
}

fn correct_tangents(ctx: &RouteCtx, mesh: &mut Mesh, mapping: &[usize], skin: &crate::read::skel::GpuSkinSample) -> anyhow::Result<()> {
    let read = super::skel::deformed_mesh(ctx)?.ok_or_else(|| anyhow::anyhow!("missing tangent deformation"))?;
    let deformed = crate::mesh::mesh_from_usd(&read);
    anyhow::ensure!(deformed.count_vertices() == mesh.count_vertices(), "skinned tangent mapping changed");
    let Some(bevy::mesh::VertexAttributeValues::Float32x4(tangents)) = deformed.attribute(Mesh::ATTRIBUTE_TANGENT) else {
        mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
        return Ok(());
    };
    let mut corrected = Vec::with_capacity(tangents.len());
    for (tangent, &point) in tangents.iter().zip(mapping) {
        let matrix = skin.indices[point].iter().zip(skin.weights[point]).fold(Mat4::ZERO,
            |matrix, (&index, weight)| matrix + skin.matrices[index as usize] * weight);
        let value = Mat3::from_mat4(matrix).inverse() * Vec3::from_slice(&tangent[..3]);
        anyhow::ensure!(value.is_finite(), "nonfinite skinned tangent correction");
        corrected.push([value.x, value.y, value.z, tangent[3]]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, corrected);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrected_normals_validate_the_active_morph_result() {
        let mut source = Mesh::from(Rectangle::default());
        let count = source.count_vertices();
        source.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![Vec3::X.to_array(); count]);
        source.set_morph_targets(vec![bevy::mesh::morph::MorphAttributes::new(Vec3::ZERO, Vec3::Y, Vec3::ZERO); count]);
        let mapping = (0..count).collect::<Vec<_>>();
        let corrections = vec![Mat3::from_diagonal(Vec3::new(0.0, 1.0, 1.0)); count];
        assert!(correct_normals(&mut source.clone(), &mapping, &corrections, &[0.0]).is_err());
        assert!(correct_normals(&mut source, &mapping, &corrections, &[1.0]).is_ok());
    }

    #[test]
    fn blended_gpu_normal_inputs_reproduce_native_cpu_normals() {
        check_blended_gpu_inputs(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_blended_normals.usda"));
    }

    #[test]
    fn animated_blended_gpu_inputs_reproduce_cpu_normals_and_tangents() {
        check_blended_gpu_inputs(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_blended_animated.usda"));
    }

    fn check_blended_gpu_inputs(file: &str) {
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        for (joint_weights, time) in [[0.5; 8], [1.0,0.0,0.75,0.25,0.25,0.75,0.0,1.0]].into_iter()
            .flat_map(|weights| [0.0,2.5,5.0,7.5,10.0].into_iter().map(move |time| (weights, time))) {
            stage.attribute("/Test/Face.primvars:skel:jointWeights").unwrap()
                .set(openusd::sdf::Value::FloatVec(joint_weights.to_vec())).unwrap();
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            let skin = crate::read::skel::gpu_skin_sample(&stage, &path, Some(time)).unwrap();
            let mapping = crate::mesh::vertex_point_indices(&read);
            let mut gpu = crate::mesh::assemble_mesh(&read, None, false);
            assert!(gpu.attribute(Mesh::ATTRIBUTE_TANGENT).is_none());
            let weights = super::super::gpu_morph::prepare(&ctx, &read, &mut gpu, false).unwrap();
            correct_normals(&mut gpu, &mapping, &skin.normal_corrections, &weights).unwrap();
            correct_tangents(&ctx, &mut gpu, &mapping, &skin).unwrap();
            let morph = crate::read::skel::morph_sample(&stage, &path, Some(time)).unwrap();
            let (mut normals, points) = crate::read::skel::morph_normals(&read, &morph).unwrap();
            crate::read::skel::skin_normals(&stage, &path, Some(time), &mut normals, &points, read.points.len()).unwrap();
            let mut deformed = read.clone();
            deformed.normals = Some(normals);
            let cpu = crate::mesh::mesh_from_usd(&deformed);
            let tangent_cpu = crate::mesh::mesh_from_usd(&super::super::skel::deformed_mesh(&ctx).unwrap().unwrap());
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(actual_tangents)) = gpu.attribute(Mesh::ATTRIBUTE_TANGENT) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(expected_tangents)) = tangent_cpu.attribute(Mesh::ATTRIBUTE_TANGENT) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(actual)) = gpu.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(expected)) = cpu.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!() };
            assert!(skin.normal_corrections.iter().any(|matrix| !matrix.abs_diff_eq(Mat3::IDENTITY, 1e-3)));
            for (vertex, &point) in mapping.iter().enumerate() {
                let mut normal = Vec3::from(actual[vertex]);
                for (target, weight) in weights.iter().enumerate() {
                    normal += gpu.get_morph_targets().unwrap()[target * mapping.len() + vertex].normal * *weight;
                }
                let matrix = skin.indices[point].iter().zip(skin.weights[point]).fold(Mat4::ZERO,
                    |matrix, (&index, weight)| matrix + skin.matrices[index as usize] * weight);
                normal = (Mat3::from_mat4(matrix).inverse().transpose() * normal).normalize();
                assert!(normal.abs_diff_eq(Vec3::from(expected[vertex]), 1e-5), "time {time}, vertex {vertex}");
                let tangent = (Mat3::from_mat4(matrix) * Vec3::from_slice(&actual_tangents[vertex][..3])).normalize();
                assert!(tangent.abs_diff_eq(Vec3::from_slice(&expected_tangents[vertex][..3]), 1e-5));
                assert_eq!(actual_tangents[vertex][3], expected_tangents[vertex][3]);
            }
        }
    }

    #[test]
    fn influence_only_animation_updates_independent_gpu_instances() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("influences.usda", include_bytes!("../../../../assets/skel_influences.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        app.init_resource::<GpuSkinningEnabled>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let a = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let b = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let entity = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/Test/Bar").unwrap();
        let (ea, eb) = (entity(app.world(), a), entity(app.world(), b));
        let weights = |world: &World, entity| {
            let handle = &world.get::<Mesh3d>(entity).unwrap().0;
            world.resource::<Assets<Mesh>>().get(handle).unwrap().attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).unwrap().clone()
        };
        let expected = |weight| bevy::mesh::VertexAttributeValues::Float32x4(vec![[weight, 1.0-weight, 0.0, 0.0]; 6]);
        assert_eq!(weights(app.world(), ea), expected(1.0));
        assert_eq!(weights(app.world(), eb), expected(0.0));
        let joints = app.world().get::<UsdGpuSkin>(ea).unwrap().joints.clone();
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 5.0;
        app.update();
        assert_eq!(entity(app.world(), a), ea);
        assert_eq!(entity(app.world(), b), eb);
        assert_eq!(weights(app.world(), ea), expected(0.5));
        assert_eq!(weights(app.world(), eb), expected(0.0));
        assert_eq!(app.world().get::<UsdGpuSkin>(ea).unwrap().joints, joints);
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(a).unwrap().clone();
        stage.attribute("/Test/Bar.primvars:skel:jointWeights").unwrap()
            .set_at(openusd::sdf::Value::FloatVec([0.25,0.75].repeat(4)), openusd::usd::TimeCode::new(5.0)).unwrap();
        app.update();
        assert_eq!(weights(app.world(), ea), expected(0.25));
        assert_eq!(weights(app.world(), eb), expected(0.0));
        assert!(app.world().get::<UsdCpuSkinFallback>(ea).is_none());
    }

    fn stage() -> openusd::usd::Stage {
        openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_test_simple.usda")).unwrap()
    }

    #[test]
    fn sampled_base_points_feed_cpu_and_gpu_skinning() {
        use openusd::sdf::Value;
        use openusd::usd::TimeCode;
        let stage = stage();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let rest = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap().points;
        for (time, offset) in [(0.0, 0.0), (30.0, 2.0)] {
            let points = rest.iter().map(|point| [point[0] + offset, point[1], point[2]].into()).collect();
            stage.prim(path.clone()).unwrap().attribute("points").set_at(Value::Vec3fVec(points), TimeCode::new(time)).unwrap();
        }
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        let entity = world.spawn_empty().id();
        for time in [0.0, 15.0, 30.0] {
            let read = crate::read::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            assert!((read.points[0][0] - rest[0][0] - time as f32 / 15.0).abs() < 1e-6);
            let cpu = crate::read::skel::skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            attach(&RouteCtx::at(&stage, &path, Some(time)), &mut world, entity).unwrap();
            let skin = world.get::<UsdGpuSkin>(entity).unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            let bevy::mesh::VertexAttributeValues::Uint16x4(indices) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).unwrap() else { panic!("indices") };
            let bevy::mesh::VertexAttributeValues::Float32x4(weights) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).unwrap() else { panic!("weights") };
            let mapping = crate::mesh::vertex_point_indices(&read);
            for (vertex, &point) in mapping.iter().enumerate() {
                assert_eq!(points[vertex], read.points[point]);
                let gpu: Vec3 = (0..4).map(|lane| skin.matrices[indices[vertex][lane] as usize]
                    .transform_point3(Vec3::from_array(points[vertex])) * weights[vertex][lane]).sum();
                assert!(gpu.distance(Vec3::from_array(cpu[point])) < 1e-5);
            }
            use crate::route::PrimRoute;
            crate::route::skel::SkinRoute.project(&RouteCtx::at(&stage, &path, Some(time)), &mut world, entity);
            assert!(world.get::<UsdGpuSkin>(entity).is_none());
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("CPU positions") };
            assert_eq!(points.len(), mapping.len());
            for (vertex, &source) in mapping.iter().enumerate() {
                assert_eq!(points[vertex], cpu[source]);
            }
        }
        stage.prim(path.clone()).unwrap().attribute("points").set_at(Value::Vec3fVec(vec![[0.0,0.0,0.0].into()]), TimeCode::new(40.0)).unwrap();
        assert!(crate::read::skel::gpu_skin_sample(&stage, &path, Some(40.0)).err().unwrap().to_string().contains("influence count"));
        assert!(crate::read::skel::skinned_points_at(&stage, &path, Some(40.0)).unwrap().is_none());
    }

    #[test]
    fn seam_vertices_keep_their_point_influences() {
        use openusd::sdf::Value;
        let stage = stage();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let mut read = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        read.points[8] = read.points[0];
        crate::authoring::set_attribute(&stage, "/Test/Bar", "points", "point3f[]",
            Value::Vec3fVec(read.points.iter().copied().map(Into::into).collect())).unwrap();
        let corners = read.face_vertex_indices.len();
        stage.create_attribute("/Test/Bar.primvars:st", "texCoord2f[]").unwrap()
            .set_metadata("interpolation", Value::Token("faceVarying".into())).unwrap()
            .set(Value::Vec2fVec(vec![[0.0, 0.0].into(); corners])).unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<SkinnedMeshInverseBindposes>::default());
        let entity = world.spawn_empty().id();
        attach(&RouteCtx::at(&stage, &path, Some(30.0)), &mut world, entity).unwrap();
        let skin = world.get::<UsdGpuSkin>(entity).unwrap();
        let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
        let mapping = crate::mesh::vertex_point_indices(&read);
        assert_eq!(mesh.count_vertices(), mapping.len());
        assert!(mapping.len() > corners);
        let bevy::mesh::VertexAttributeValues::Uint16x4(indices) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).unwrap() else { panic!("joint indices") };
        let bevy::mesh::VertexAttributeValues::Float32x4(weights) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).unwrap() else { panic!("joint weights") };
        let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
        let cpu = crate::read::skel::skinned_points_at(&stage, &path, Some(30.0)).unwrap().unwrap();
        for (corner, &point) in mapping.iter().enumerate() {
            let gpu: Vec3 = (0..4).map(|lane| skin.matrices[indices[corner][lane] as usize]
                .transform_point3(Vec3::from_array(positions[corner])) * weights[corner][lane]).sum();
            assert!(gpu.distance(Vec3::from_array(cpu[point])) < 1e-5);
        }
    }

    #[test]
    fn reordered_animation_keeps_cpu_and_gpu_poses() {
        let original = stage();
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_test_simple.usda")).unwrap();
        let (prefix, animation) = source.split_once("def SkelAnimation").unwrap();
        let animation = animation.replace("[\"Root\", \"Root/Tip\"]", "[\"Root/Tip\", \"Root\"]")
            .replace("[(0, 0, 0), (0, 1, 0)]", "[(0, 1, 0), (0, 0, 0)]")
            .replace("[(1, 0, 0, 0), (0.7071, 0, 0, 0.7071)]", "[(0.7071, 0, 0, 0.7071), (1, 0, 0, 0)]");
        let text = format!("{prefix}def SkelAnimation{animation}");
        let reordered = crate::UsdSource::new("reordered.usda", text.into_bytes()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        for time in [0.0, 15.0, 30.0, 60.0] {
            let expected = crate::read::skel::skinned_points_at(&original, &path, Some(time)).unwrap().unwrap();
            let actual = crate::read::skel::skinned_points_at(&reordered, &path, Some(time)).unwrap().unwrap();
            assert_eq!(expected, actual);
            let expected = crate::read::skel::gpu_skin_sample(&original, &path, Some(time)).unwrap();
            let actual = crate::read::skel::gpu_skin_sample(&reordered, &path, Some(time)).unwrap();
            assert_eq!(expected.matrices, actual.matrices);
        }
    }

    #[test]
    fn gpu_palette_matches_cpu_positions_with_geom_bind() {
        let stage = stage();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let bind = Mat4::from_translation(Vec3::new(0.25, -0.5, 0.75));
        crate::authoring::set_attribute(&stage, "/Test/Bar", "primvars:skel:geomBindTransform", "matrix4d",
            openusd::sdf::Value::Matrix4d(openusd::gf::Matrix4d(bind.to_cols_array().map(f64::from)))).unwrap();
        let points = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap().points;
        for time in [0.0, 15.0, 30.0, 60.0] {
            let sample = crate::read::skel::gpu_skin_sample(&stage, &path, Some(time)).unwrap();
            let cpu = crate::read::skel::skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            for (i, point) in points.iter().enumerate() {
                let gpu = (0..4).map(|lane| sample.matrices[sample.indices[i][lane] as usize]
                    .transform_point3(Vec3::from_array(*point)) * sample.weights[i][lane]).sum::<Vec3>();
                assert!(gpu.distance(Vec3::from_array(cpu[i])) < 1e-5, "time {time}, vertex {i}: {gpu:?} != {:?}", cpu[i]);
            }
        }
    }

    #[test]
    fn gpu_projection_reuses_mesh_and_updates_world_palette() {
        let live = crate::live::LiveStage::new(stage());
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(Assets::<SkinnedMeshInverseBindposes>::default());
        world.insert_resource(GpuSkinningEnabled);
        world.insert_resource(super::super::cache::ProjectionCache::default());
        world.insert_resource(crate::SchemaRegistry::builtin());
        world.insert_resource(super::super::StageTime { current: 0.0 });
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Test/Bar").unwrap();
        let mesh = world.get::<Mesh3d>(entity).unwrap().0.clone();
        let joints = world.get::<UsdGpuSkin>(entity).expect("GPU skin attached").joints.clone();
        assert!(world.get::<SkinnedMesh>(entity).is_some());
        let registry = world.resource::<crate::SchemaRegistry>().clone();
        world.resource_mut::<super::super::StageTime>().current = 30.0;
        registry.patch_prim(&live.stage, &openusd::sdf::path("/Test/Bar").unwrap(), &mut world, entity, &[]);
        assert_eq!(world.get::<Mesh3d>(entity).unwrap().0, mesh);
        assert_eq!(world.get::<UsdGpuSkin>(entity).unwrap().joints, joints);
        let placement = Mat4::from_translation(Vec3::new(10.0, 2.0, 3.0));
        world.entity_mut(entity).insert(GlobalTransform::from(placement));
        update_joint_globals(&mut world);
        let skin = world.get::<UsdGpuSkin>(entity).unwrap();
        for (&joint, matrix) in joints.iter().zip(&skin.matrices) {
            assert!(world.get::<GlobalTransform>(joint).unwrap().to_matrix().abs_diff_eq(placement * *matrix, 1e-5));
        }
        crate::authoring::set_attribute(&live.stage, "/Test/Bar", "skel:blendShapes", "token[]",
            openusd::sdf::Value::TokenVec(vec!["Blend".into()])).unwrap();
        registry.patch_prim(&live.stage, &openusd::sdf::path("/Test/Bar").unwrap(), &mut world, entity, &[]);
        assert!(world.get::<UsdCpuSkinFallback>(entity).unwrap().0.contains("name/target count mismatch"));
        assert!(joints.iter().all(|joint| world.get_entity(*joint).is_err()));
        assert!(world.get::<SkinnedMesh>(entity).is_none());
    }
}
