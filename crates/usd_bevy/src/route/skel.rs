//! Skinning route (PLAN in-repo): a skinned `Mesh` → its CPU-deformed geometry
//! at the current [`StageTime`](super::StageTime).
//!
//! Runs after the mesh route (which bakes the rest mesh) and replaces the
//! entity's `Mesh3d` with the skinned points computed by [`read::skel`]. Being
//! a normal route, it re-runs on reproject and — because a skinned mesh is
//! flagged animated (see `crate::live::prim_is_animated`) — it resamples as
//! `StageTime` moves.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};
use crate::read::skel::{has_blend_shapes, is_skinned};

/// Replaces a skinned / blend-shaped mesh's geometry with its deformed points.
pub struct SkinRoute;

/// Deformation could not be evaluated; generated geometry is suppressed.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct UsdDeformationError(pub String);

#[derive(Component)]
pub(crate) struct CpuSubsetGeometry(pub crate::read::geom::ReadMesh);

/// Poses a mesh whose deformation never changes once, and shares the result
/// like any static mesh.
fn bake_static(ctx: &RouteCtx, world: &mut World, entity: Entity, skinned: bool, blended: bool) {
    super::gpu_morph::clear(world, entity);
    if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
        super::gpu_skin::clear(world, entity);
    }
    world
        .entity_mut(entity)
        .remove::<super::gpu_skin::UsdCpuSkinFallback>();
    let read = match deformed_mesh_with_kinds(ctx, skinned, blended) {
        Ok(Some(read)) => read,
        result => {
            let error = result.err().map_or_else(
                || "deformation produced no mesh".into(),
                |error| error.to_string(),
            );
            world
                .entity_mut(entity)
                .remove::<Mesh3d>()
                .insert(UsdDeformationError(error));
            return;
        }
    };
    let assembly = if crate::mesh::uses_flat_normals(&read) && super::flat_material::enabled(world)
    {
        super::cache::Assembly::FlatShared
    } else {
        super::cache::Assembly::Standard
    };
    let handle = super::cache::intern_assembled_mesh(world, &read, assembly);
    world.entity_mut(entity).insert(Mesh3d(handle));
}

pub(crate) fn deformed_mesh(ctx: &RouteCtx) -> anyhow::Result<Option<crate::read::geom::ReadMesh>> {
    let (skinned, blended) = ctx.prepared_kinds.get().map_or_else(
        || {
            (
                is_skinned(ctx.stage, ctx.path),
                has_blend_shapes(ctx.stage, ctx.path),
            )
        },
        |kinds| (kinds.skinned, kinds.blended),
    );
    deformed_mesh_with_kinds(ctx, skinned, blended)
}

fn deformed_mesh_with_kinds(
    ctx: &RouteCtx,
    skinned: bool,
    blended: bool,
) -> anyhow::Result<Option<crate::read::geom::ReadMesh>> {
    let Some(source) = ctx.read_mesh()? else {
        return Ok(None);
    };
    let sample = if skinned {
        crate::read::skel::cpu_skin_sample(ctx.stage, ctx.path, ctx.time, Some(source))?
            .map(|sample| (sample.points, Some(sample.normals)))
    } else {
        crate::read::skel::blend_shape_deform(ctx.stage, ctx.path, &source.points, ctx.time)
            .map(|points| (points, None))
    };
    let Some((points, skin_normals)) = sample else {
        return Ok(None);
    };
    let mut read = source.clone();
    read.triangulation_points = Some(std::mem::replace(&mut read.points, points));
    read.normals = if read.normals.is_some() {
        let sample = if blended {
            crate::read::skel::morph_sample_with_mesh(ctx.stage, ctx.path, ctx.time, source)?
        } else {
            crate::read::skel::MorphSample {
                targets: Vec::new(),
                normal_targets: Vec::new(),
                weights: Vec::new(),
            }
        };
        let (mut normals, points) = crate::read::skel::morph_normals(&read, &sample)?;
        if let Some(skin) = skin_normals {
            skin.apply(&mut normals, &points, read.points.len())?;
        }
        Some(normals)
    } else {
        None
    };
    Ok(Some(read))
}

impl PrimRoute for SkinRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        super::gpu_skin::clear(world, entity);
        world.entity_mut(entity).remove::<(
            CpuSubsetGeometry,
            UsdDeformationError,
            super::gpu_skin::UsdCpuSkinFallback,
        )>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        matches!(ctx.type_name.as_deref(), Some("Mesh"))
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        world
            .entity_mut(entity)
            .remove::<(CpuSubsetGeometry, UsdDeformationError)>();
        if world.get_resource::<Assets<Mesh>>().is_none() {
            return;
        }
        if super::subdivision::enabled(ctx, world) {
            return;
        }
        let kinds = ctx
            .prepared_kinds
            .get()
            .unwrap_or_else(|| super::preparation::DeformationKinds::read(ctx));
        let (skinned, blended) = (kinds.skinned, kinds.blended);
        if !skinned && !blended {
            super::gpu_morph::clear(world, entity);
            if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
                super::gpu_skin::clear(world, entity);
            }
            world
                .entity_mut(entity)
                .remove::<super::gpu_skin::UsdCpuSkinFallback>();
            return;
        }
        if !kinds.varying {
            bake_static(ctx, world, entity, skinned, blended);
            return;
        }
        if world.contains_resource::<super::gpu_skin::GpuSkinningEnabled>() {
            let result = if !skinned && blended {
                if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
                    super::gpu_skin::clear(world, entity);
                }
                super::gpu_morph::attach(ctx, world, entity)
            } else {
                if !blended {
                    super::gpu_morph::clear(world, entity);
                }
                super::gpu_skin::attach(ctx, world, entity, blended)
            };
            match result {
                Ok(()) => return,
                Err(error) => {
                    super::gpu_skin::clear(world, entity);
                    world
                        .entity_mut(entity)
                        .insert(super::gpu_skin::UsdCpuSkinFallback(error.to_string()));
                }
            }
        } else if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
            super::gpu_skin::clear(world, entity);
        }
        super::gpu_morph::clear(world, entity);
        let prepared = ctx
            .prepared_deformation
            .borrow_mut()
            .take()
            .unwrap_or_else(|| {
                deformed_mesh_with_kinds(ctx, skinned, blended)
                    .map(|read| read.map(super::preparation::PreparedDeformation::new))
            });
        let mut prepared = match prepared {
            Ok(Some(prepared)) => prepared,
            result => {
                let error = result.err().map_or_else(
                    || "deformation produced no mesh".into(),
                    |error| error.to_string(),
                );
                world
                    .entity_mut(entity)
                    .remove::<Mesh3d>()
                    .insert(UsdDeformationError(error));
                return;
            }
        };
        prepared.generate_tangents(world);
        if !prepared.read.subsets.is_empty() {
            world
                .entity_mut(entity)
                .insert(CpuSubsetGeometry(prepared.read));
        }
        let handle = world.resource_mut::<Assets<Mesh>>().add(prepared.mesh);
        if let Some(mut m) = world.get_mut::<Mesh3d>(entity) {
            m.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(Mesh3d(handle));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::UsdInstances;

    #[test]
    fn cached_cpu_tangents_follow_unique_clocks_and_mesh_edits() {
        use super::super::cache::{MeshCacheMetrics, MeshTangentCache};
        use super::super::gpu_skin::{GpuSkinningEnabled, UsdCpuSkinFallback};
        use openusd::vt::Value;
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_morph_blended_animated.usda"
        );
        for (gpu, budget) in [(false, 0), (false, 4096), (true, 0), (true, 4096)] {
            let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap())
                .unwrap()
                .open_stage()
                .unwrap();
            let path = openusd::sdf::path("/Test/Face").unwrap();
            stage
                .attribute("/Test/Face.primvars:skel:jointWeights")
                .unwrap()
                .set(Value::FloatVec(vec![0.8; 8]))
                .unwrap();
            let mut world = World::new();
            world.init_resource::<Assets<Mesh>>();
            world.init_resource::<MeshCacheMetrics>();
            world.insert_resource(MeshTangentCache::with_byte_budget(budget));
            if gpu {
                world.init_resource::<GpuSkinningEnabled>();
            }
            let entity = world.spawn_empty().id();
            for (iteration, time) in (0..=40)
                .map(|step| step as f64 * 0.25)
                .chain([5.0, 0.0, 10.0, 5.0])
                .enumerate()
            {
                let edit = match iteration {
                    10 => Some((
                        "primvars:st",
                        Value::Vec2fVec(
                            [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]
                                .map(Into::into)
                                .to_vec(),
                        ),
                    )),
                    20 => Some(("normals", Value::Vec3fVec(vec![[0.0, 1.0, 1.0].into(); 4]))),
                    30 => Some(("orientation", Value::Token("leftHanded".into()))),
                    _ => None,
                };
                if let Some((name, value)) = edit {
                    stage
                        .prim(path.clone())
                        .unwrap()
                        .attribute(name)
                        .set(value)
                        .unwrap();
                }
                let ctx = RouteCtx::at(&stage, &path, Some(time));
                let read = deformed_mesh(&ctx).unwrap().unwrap();
                let expected = crate::mesh::mesh_from_usd(&read);
                for _ in 0..3 {
                    SkinRoute.project(&ctx, &mut world, entity);
                    assert_eq!(world.get::<UsdCpuSkinFallback>(entity).is_some(), gpu);
                    let handle = &world.get::<Mesh3d>(entity).unwrap().0;
                    let actual = world.resource::<Assets<Mesh>>().get(handle).unwrap();
                    assert_eq!(
                        actual.get_index_buffer_bytes(),
                        expected.get_index_buffer_bytes()
                    );
                    assert_eq!(actual.attributes().count(), expected.attributes().count());
                    for (attribute, values) in expected.attributes() {
                        assert_eq!(
                            actual.attribute(attribute.id).unwrap().get_bytes(),
                            values.get_bytes(),
                            "gpu={gpu}, budget={budget}, time={time}, attribute={}",
                            attribute.name
                        );
                    }
                }
                assert!(
                    world
                        .resource::<MeshTangentCache>()
                        .retained_payload_bytes()
                        <= budget
                );
            }
            let metrics = &world.resource::<MeshCacheMetrics>().0;
            assert!(metrics["tangent_misses"] > 0);
            assert_eq!(
                metrics.get("tangent_hits").copied().unwrap_or(0) > 0,
                budget > 0
            );
        }
    }

    #[test]
    fn unanimated_skins_bake_once_and_share_their_mesh() {
        let mesh = |name: &str| {
            format!(
                r#"
    def Mesh "{name}" (prepend apiSchemas = ["SkelBindingAPI"]) {{
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        int[] primvars:skel:jointIndices = [0, 0, 0] (interpolation = "vertex" elementSize = 1)
        float[] primvars:skel:jointWeights = [1, 1, 1] (interpolation = "vertex" elementSize = 1)
        rel skel:skeleton = </Rig/Skel>
    }}"#
            )
        };
        let text = format!(
            r#"#usda 1.0
def SkelRoot "Rig" {{
    def Skeleton "Skel" {{
        uniform token[] joints = ["a"]
        uniform matrix4d[] bindTransforms = [((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1))]
        uniform matrix4d[] restTransforms = [((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 2, 0, 1))]
    }}{}{}
}}
"#,
            mesh("A"),
            mesh("B")
        );
        let source = crate::UsdSource::new("static_skin.usda", text.into_bytes()).unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::UsdPlugin,
            crate::UsdAssetPlugin,
        ));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        app.init_resource::<super::super::gpu_skin::GpuSkinningEnabled>();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source,
                textures: default(),
            });
        let root = app.world_mut().spawn(crate::UsdSceneRoot(handle)).id();
        app.update();
        let world = app.world();
        let instances = world.get_non_send::<UsdInstances>().unwrap();
        let a = instances.entity(root, "/Rig/A").unwrap();
        let b = instances.entity(root, "/Rig/B").unwrap();
        assert!(world.get::<super::super::gpu_skin::UsdGpuSkin>(a).is_none());
        assert!(world.get::<bevy::mesh::skinning::SkinnedMesh>(a).is_none());
        let handle = &world.get::<Mesh3d>(a).unwrap().0;
        assert_eq!(handle, &world.get::<Mesh3d>(b).unwrap().0);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = world
            .resource::<Assets<Mesh>>()
            .get(handle)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("baked skin has no positions")
        };
        assert!(positions.contains(&[0.0, 2.0, 0.0]));
        assert!(!positions.contains(&[0.0, 0.0, 0.0]));
    }

    #[test]
    fn singular_authored_normals_suppress_subsets_and_recover_for_both_backends() {
        for gpu in [false, true] {
            let file = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/skel_morph_normals.usda"
            );
            let source = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap();
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::asset::AssetPlugin::default(),
                crate::UsdPlugin,
                crate::UsdAssetPlugin,
            ));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<Assets<StandardMaterial>>();
            app.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
            app.init_resource::<Assets<super::super::flat_material::FlatMaterial>>();
            if gpu {
                app.init_resource::<super::super::gpu_skin::GpuSkinningEnabled>();
            }
            let handle = app
                .world_mut()
                .resource_mut::<Assets<crate::UsdScene>>()
                .add(crate::UsdScene {
                    source,
                    textures: default(),
                });
            let root = app.world_mut().spawn(crate::UsdSceneRoot(handle)).id();
            app.update();
            let entity = app
                .world()
                .get_non_send::<UsdInstances>()
                .unwrap()
                .entity(root, "/Test/Face")
                .unwrap();
            let stage = app
                .world()
                .get_non_send::<UsdInstances>()
                .unwrap()
                .stage(root)
                .unwrap()
                .clone();
            let runtime_child = app.world_mut().spawn(ChildOf(entity)).id();
            let children = |world: &World| {
                world
                    .get::<Children>(entity)
                    .into_iter()
                    .flat_map(|children| children.iter())
                    .filter(|child| {
                        world
                            .get::<super::super::subset::UsdSubset>(*child)
                            .is_some()
                    })
                    .collect::<Vec<_>>()
            };
            let original_subsets = children(app.world());
            assert!(!original_subsets.is_empty());
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            let scales = stage.attribute("/Test/Skel/Anim.scales").unwrap();
            scales
                .clone()
                .set(openusd::vt::Value::Vec3hVec(vec![
                    [0.0, 1.0, 1.0].map(openusd::gf::f16::from_f32).into(),
                ]))
                .unwrap();
            app.update();
            assert!(
                app.world()
                    .get::<UsdDeformationError>(entity)
                    .unwrap()
                    .0
                    .contains("singular")
            );
            assert!(app.world().get::<Mesh3d>(entity).is_none());
            assert!(
                app.world()
                    .get::<super::super::gpu_skin::UsdGpuSkin>(entity)
                    .is_none()
            );
            assert!(children(app.world()).is_empty());
            assert!(
                original_subsets
                    .iter()
                    .all(|child| app.world().get_entity(*child).is_err())
            );
            assert!(app.world().get_entity(runtime_child).is_ok());
            scales
                .set(openusd::vt::Value::Vec3hVec(vec![
                    [2.0, 1.0, 0.5].map(openusd::gf::f16::from_f32).into(),
                ]))
                .unwrap();
            app.update();
            assert!(app.world().get::<UsdDeformationError>(entity).is_none());
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(!children(app.world()).is_empty());
            assert_eq!(
                app.world()
                    .get::<super::super::gpu_skin::UsdGpuSkin>(entity)
                    .is_some(),
                gpu
            );
            assert!(app.world().get_entity(runtime_child).is_ok());
        }
    }
}
