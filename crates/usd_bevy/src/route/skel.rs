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
use crate::read::skel::{blend_shaped_points_at, has_blend_shapes, is_skinned, skinned_points_at};

/// Replaces a skinned / blend-shaped mesh's geometry with its deformed points.
pub struct SkinRoute;

/// Deformation could not be evaluated; generated geometry is suppressed.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct UsdDeformationError(pub String);

#[derive(Component)]
pub(crate) struct CpuSubsetGeometry(pub crate::read::geom::ReadMesh);

pub(crate) fn deformed_mesh(ctx: &RouteCtx) -> anyhow::Result<Option<crate::read::geom::ReadMesh>> {
    let points = if is_skinned(ctx.stage, ctx.path) {
        skinned_points_at(ctx.stage, ctx.path, ctx.time)?
    } else {
        blend_shaped_points_at(ctx.stage, ctx.path, ctx.time)?
    };
    let Some(points) = points else { return Ok(None) };
    let Some(mut read) = crate::read::geom::read_mesh_at(ctx.stage, ctx.path, ctx.time)? else { return Ok(None) };
    read.triangulation_points = Some(std::mem::replace(&mut read.points, points));
    read.normals = if read.normals.is_some() {
        let sample = if has_blend_shapes(ctx.stage, ctx.path) {
            crate::read::skel::morph_sample(ctx.stage, ctx.path, ctx.time)?
        } else { crate::read::skel::MorphSample { targets: Vec::new(), normal_targets: Vec::new(), weights: Vec::new() } };
        let (mut normals, points) = crate::read::skel::morph_normals(&read, &sample)?;
        if is_skinned(ctx.stage, ctx.path) { crate::read::skel::skin_normals(ctx.stage, ctx.path, ctx.time, &mut normals, &points, read.points.len())?; }
        Some(normals)
    } else { None };
    Ok(Some(read))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::UsdInstances;

    #[test]
    fn singular_authored_normals_suppress_subsets_and_recover_for_both_backends() {
        for gpu in [false, true] {
            let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_normals.usda");
            let source = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap();
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<Assets<StandardMaterial>>();
            app.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
            app.init_resource::<Assets<super::super::flat_material::FlatMaterial>>();
            if gpu { app.init_resource::<super::super::gpu_skin::GpuSkinningEnabled>(); }
            let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
            let root = app.world_mut().spawn(crate::UsdSceneRoot(handle)).id();
            app.update();
            let entity = app.world().get_non_send::<UsdInstances>().unwrap().entity(root, "/Test/Face").unwrap();
            let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(root).unwrap().clone();
            let runtime_child = app.world_mut().spawn(ChildOf(entity)).id();
            let children = |world: &World| world.get::<Children>(entity).into_iter().flat_map(|children| children.iter())
                .filter(|child| world.get::<super::super::subset::UsdSubset>(*child).is_some()).collect::<Vec<_>>();
            let original_subsets = children(app.world());
            assert!(!original_subsets.is_empty());
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            let scales = stage.attribute("/Test/Skel/Anim.scales").unwrap();
            scales.clone().set(openusd::sdf::Value::Vec3hVec(vec![[0.0, 1.0, 1.0].map(openusd::gf::f16::from_f32).into()])).unwrap();
            app.update();
            assert!(app.world().get::<UsdDeformationError>(entity).unwrap().0.contains("singular"));
            assert!(app.world().get::<Mesh3d>(entity).is_none());
            assert!(app.world().get::<super::super::gpu_skin::UsdGpuSkin>(entity).is_none());
            assert!(children(app.world()).is_empty());
            assert!(original_subsets.iter().all(|child| app.world().get_entity(*child).is_err()));
            assert!(app.world().get_entity(runtime_child).is_ok());
            scales.set(openusd::sdf::Value::Vec3hVec(vec![[2.0, 1.0, 0.5].map(openusd::gf::f16::from_f32).into()])).unwrap();
            app.update();
            assert!(app.world().get::<UsdDeformationError>(entity).is_none());
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(!children(app.world()).is_empty());
            assert_eq!(app.world().get::<super::super::gpu_skin::UsdGpuSkin>(entity).is_some(), gpu);
            assert!(app.world().get_entity(runtime_child).is_ok());
        }
    }
}

impl PrimRoute for SkinRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        super::gpu_skin::clear(world, entity);
        world.entity_mut(entity).remove::<(CpuSubsetGeometry, UsdDeformationError, super::gpu_skin::UsdCpuSkinFallback)>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        matches!(ctx.type_name.as_deref(), Some("Mesh"))
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<(CpuSubsetGeometry, UsdDeformationError)>();
        if world.get_resource::<Assets<Mesh>>().is_none() {
            return;
        }
        if super::subdivision::enabled(ctx, world) { return; }
        if !is_skinned(ctx.stage, ctx.path) && !has_blend_shapes(ctx.stage, ctx.path) {
            super::gpu_morph::clear(world, entity);
            if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
                super::gpu_skin::clear(world, entity);
            }
            world.entity_mut(entity).remove::<super::gpu_skin::UsdCpuSkinFallback>();
            return;
        }
        if world.contains_resource::<super::gpu_skin::GpuSkinningEnabled>() {
            let result = if !is_skinned(ctx.stage, ctx.path) && has_blend_shapes(ctx.stage, ctx.path) {
                if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() { super::gpu_skin::clear(world, entity); }
                super::gpu_morph::attach(ctx, world, entity)
            } else {
                if !has_blend_shapes(ctx.stage, ctx.path) { super::gpu_morph::clear(world, entity); }
                super::gpu_skin::attach(ctx, world, entity)
            };
            match result {
                Ok(()) => return,
                Err(error) => {
                    super::gpu_skin::clear(world, entity);
                    world.entity_mut(entity).insert(super::gpu_skin::UsdCpuSkinFallback(error.to_string()));
                }
            }
        } else if world.get::<super::gpu_skin::UsdGpuSkin>(entity).is_some() {
            super::gpu_skin::clear(world, entity);
        }
        super::gpu_morph::clear(world, entity);
        let read = match deformed_mesh(ctx) {
            Ok(Some(read)) => read,
            result => {
                let error = result.err().map_or_else(|| "deformation produced no mesh".into(), |error| error.to_string());
                world.entity_mut(entity).remove::<Mesh3d>().insert(UsdDeformationError(error));
                return;
            }
        };
        let mesh = crate::mesh::mesh_from_usd(&read);
        if !read.subsets.is_empty() {
            world.entity_mut(entity).insert(CpuSubsetGeometry(read));
        }
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        if let Some(mut m) = world.get_mut::<Mesh3d>(entity) {
            m.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(Mesh3d(handle));
        }
    }
}
