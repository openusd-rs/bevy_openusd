//! Finite CPU subdivision after control-point deformation and before subsets.

use bevy::prelude::*;
use super::{PrimRoute, RouteCtx};

#[derive(Resource, Clone, Copy, Debug)]
pub struct UsdSubdivisionSettings { levels: u32 }

impl UsdSubdivisionSettings {
    pub fn new(levels: u32) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=6).contains(&levels), "subdivision levels must be 1..=6");
        Ok(Self { levels })
    }
    pub fn levels(self) -> u32 { self.levels }
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct UsdSubdivisionError(pub String);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsdSubdivisionApplied { pub levels: u32 }

pub struct SubdivisionRoute;

pub(crate) fn current_levels(world: &World) -> Option<u32> {
    world.get_resource::<UsdSubdivisionSettings>().map(|settings| settings.levels)
}

pub(crate) fn refresh_geometry(world: &mut World, stage: &openusd::usd::Stage, map: &crate::live::PrimEntities) {
    let registry = world.resource::<super::SchemaRegistry>().clone();
    for (path, entity) in map.iter() {
        let Ok(path) = openusd::sdf::path(path) else { continue };
        let kind = stage.prim(path.clone()).ok().and_then(|prim| prim.type_name().ok().flatten());
        if matches!(kind.as_deref(), Some("Mesh" | "PointInstancer")) {
            registry.patch_prim(stage, &path, world, entity, &[]);
        }
    }
}

pub(crate) fn enabled(ctx: &RouteCtx, world: &World) -> bool {
    world.contains_resource::<UsdSubdivisionSettings>() && ctx.type_name.as_deref() == Some("Mesh")
        && !matches!(ctx.stage.prim(ctx.path.clone()).ok().and_then(|prim|
            prim.attribute("subdivisionScheme").get_at::<openusd::sdf::Value>(ctx.time.map(openusd::usd::TimeCode::new)).ok().flatten()),
            Some(openusd::sdf::Value::Token(token)) if token.as_str() == "none")
}

pub(crate) fn refined_mesh(ctx: &RouteCtx, levels: u32) -> anyhow::Result<crate::read::geom::ReadMesh> {
    let mut mesh = ctx.read_mesh()?.cloned()
        .ok_or_else(|| anyhow::anyhow!("missing subdivision mesh"))?;
    let points = if crate::read::skel::is_skinned(ctx.stage, ctx.path) {
        crate::read::skel::skinned_points_at(ctx.stage, ctx.path, ctx.time)?
    } else if crate::read::skel::has_blend_shapes(ctx.stage, ctx.path) {
        crate::read::skel::blend_shaped_points_at(ctx.stage, ctx.path, ctx.time)?
    } else { Some(mesh.points.clone()) };
    let points = points.ok_or_else(|| anyhow::anyhow!("missing deformed control points"))?;
    mesh.triangulation_points = Some(std::mem::replace(&mut mesh.points, points));
    let rules = crate::read::subdivision::read_subdivision_at(ctx.stage, ctx.path, ctx.time)?;
    crate::subdivision::refine_mesh(&mesh, &rules, levels)
}

impl PrimRoute for SubdivisionRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool { ctx.type_name.as_deref() == Some("Mesh") }

    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<(UsdSubdivisionError, UsdSubdivisionApplied)>();
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<(UsdSubdivisionError, UsdSubdivisionApplied)>();
        if !enabled(ctx, world) || !world.contains_resource::<Assets<Mesh>>() { return; }
        let levels = world.resource::<UsdSubdivisionSettings>().levels;
        super::gpu_skin::clear(world, entity);
        super::gpu_morph::clear(world, entity);
        world.entity_mut(entity).remove::<(super::skel::CpuSubsetGeometry, super::skel::UsdDeformationError, super::gpu_skin::UsdCpuSkinFallback)>();
        let result = refined_mesh(ctx, levels);
        match result {
            Ok(mesh) => {
                let handle = super::cache::intern_mesh(world, crate::mesh::mesh_from_usd(&mesh));
                world.entity_mut(entity).remove::<bevy::camera::primitives::Aabb>()
                    .insert((Mesh3d(handle), super::skel::CpuSubsetGeometry(mesh), UsdSubdivisionApplied { levels }));
            }
            Err(error) => {
                world.entity_mut(entity).remove::<(Mesh3d, bevy::camera::primitives::Aabb)>()
                    .insert(UsdSubdivisionError(error.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{UsdInstances, UsdInstanceTime};

    #[test]
    fn animated_sharpness_rebuilds_normals_and_subsets_per_instance() {
        use super::super::{instancer::UsdInstance, subset::UsdSubset};
        #[derive(Component)]
        struct RuntimeOnly;
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/subdivision_creases.usda");
        let source = crate::UsdSource::new("sharp-layout.usda", format!(r#"#usda 1.0
( subLayers = [@{fixture}@] )
over "RoundedCube" {{
    def GeomSubset "Part" {{
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices = [0]
    }}
}}
def PointInstancer "PI" {{
    point3f[] positions = [(0,0,0),(3,0,0)]
    int[] protoIndices = [0,0]
    rel prototypes = [</RoundedCube>]
}}
"#).into_bytes()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.insert_resource(UsdSubdivisionSettings::new(2).unwrap());
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let lookup = |world: &World, root, path| world.get_non_send::<UsdInstances>().unwrap().entity(root, path).unwrap();
        let parents = roots.map(|root| lookup(app.world(), root, "/RoundedCube"));
        let part = |world: &World, parent| world.get::<Children>(parent).unwrap().iter()
            .find(|child| world.get::<UsdSubset>(*child).is_some_and(|subset| subset.0 == "Part")).unwrap();
        let parts = parents.map(|parent| part(app.world(), parent));
        let runtime = app.world_mut().spawn((RuntimeOnly, ChildOf(parents[0]))).id();
        for times in [[0.0,0.0], [0.0,3.0], [3.0,0.0], [0.0,0.0]] {
            for (root, time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
            app.update();
            assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), parents[0]);
            for (index, root) in roots.into_iter().enumerate() {
                assert_eq!(lookup(app.world(), root, "/RoundedCube"), parents[index]);
                assert_eq!(part(app.world(), parents[index]), parts[index]);
                let instancer = lookup(app.world(), root, "/PI");
                let instances: Vec<_> = app.world().get::<Children>(instancer).unwrap().iter()
                    .filter(|entity| app.world().get::<UsdInstance>(*entity).is_some()).collect();
                assert_eq!(instances.len(), 2);
                assert_eq!(app.world().get::<Mesh3d>(instances[0]).unwrap().0, app.world().get::<Mesh3d>(instances[1]).unwrap().0);
                for parent in std::iter::once(parents[index]).chain(instances) {
                    for entity in [parent, part(app.world(), parent)] {
                        let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
                        assert_eq!(mesh.count_vertices(), mesh.indices().unwrap().iter().collect::<std::collections::HashSet<_>>().len());
                        let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
                        assert!(normals.iter().all(|normal| (Vec3::from_array(*normal).length_squared() - 1.0).abs() < 1e-5));
                        if times[index] == 3.0 {
                            assert!(normals.iter().all(|normal| normal.iter().filter(|lane| lane.abs() > 1e-6).count() == 1));
                        }
                        if entity != parent { assert_eq!(mesh.indices().unwrap().len(), 96); }
                    }
                }
            }
            let handles = parents.map(|parent| app.world().get::<Mesh3d>(parent).unwrap().0.clone());
            assert_eq!(handles[0] == handles[1], times[0] == times[1]);
        }
        let unaffected = app.world().get::<Mesh3d>(parents[1]).unwrap().0.clone();
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(roots[0]).unwrap().clone();
        let sharpness = stage.attribute("/RoundedCube.creaseSharpnesses").unwrap();
        sharpness.clone().set_at(openusd::sdf::Value::FloatVec(vec![-1.0;12]), openusd::usd::TimeCode::new(3.0)).unwrap();
        app.world_mut().get_mut::<UsdInstanceTime>(roots[0]).unwrap().current = 3.0;
        app.update();
        assert!(app.world().get::<Mesh3d>(parents[0]).is_none());
        assert!(app.world().get::<UsdSubdivisionError>(parents[0]).is_some());
        assert!(app.world().get::<Children>(parents[0]).unwrap().iter().all(|entity| app.world().get::<UsdSubset>(entity).is_none()));
        let pi = lookup(app.world(), roots[0], "/PI");
        assert!(app.world().get::<super::super::instancer::UsdInstancerWarning>(pi).is_some());
        assert_eq!(app.world().get::<Mesh3d>(parents[1]).unwrap().0, unaffected);
        sharpness.set_at(openusd::sdf::Value::FloatVec(vec![10.0;12]), openusd::usd::TimeCode::new(3.0)).unwrap();
        app.update();
        assert!(app.world().get::<UsdSubdivisionError>(parents[0]).is_none());
        assert!(app.world().get::<super::super::instancer::UsdInstancerWarning>(pi).is_none());
        let recovered: Vec<_> = app.world().get::<Children>(pi).unwrap().iter()
            .filter(|entity| app.world().get::<UsdInstance>(*entity).is_some()).collect();
        assert_eq!(recovered.len(), 2);
        for entity in recovered {
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            assert_eq!(mesh.count_vertices(), 320);
        }
        assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), parents[0]);
        let restored = part(app.world(), parents[0]);
        let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(restored).unwrap().0).unwrap();
        assert_eq!(mesh.count_vertices(), 64);
        assert_eq!(app.world().get::<Mesh3d>(parents[1]).unwrap().0, unaffected);
    }

    #[test]
    fn settings_changes_refresh_live_and_independent_instance_geometry() {
        #[derive(Component)]
        struct RuntimeOnly;
        let source = crate::UsdSource::new("settings.usda", br#"#usda 1.0
def Mesh "M" {
    point3f[] points.timeSamples = { 0: [(0,0,0),(1,0,0),(1,1,0),(0,1,0)], 10: [(0,0,0),(2,0,0),(2,2,0),(0,2,0)] }
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
}
def PointInstancer "PI" {
    point3f[] positions = [(0,0,0)]
    int[] protoIndices = [0]
    rel prototypes = [</M>]
}
"#.as_slice()).unwrap();
        for instanced in [false, true] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin));
            app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
            let mut roots = Vec::new();
            if instanced {
                app.add_plugins(crate::UsdAssetPlugin);
                let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>()
                    .add(crate::UsdScene { source: source.clone(), textures: default() });
                for current in [0.0, 10.0] {
                    roots.push(app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current })).id());
                }
            } else {
                app.add_plugins(crate::live::LiveStagePlugin);
                app.insert_non_send(crate::live::LiveStage::new(source.open_stage().unwrap()));
            }
            app.update();
            let lookup = |world: &World, path: &str| -> Vec<Entity> {
                if instanced { roots.iter().map(|root| world.get_non_send::<UsdInstances>().unwrap().entity(*root, path).unwrap()).collect() }
                else { vec![world.resource::<crate::live::PrimEntities>().entity(path).unwrap()] }
            };
            let meshes = lookup(app.world(), "/M");
            let children: Vec<_> = meshes.iter().map(|entity| {
                app.world_mut().entity_mut(*entity).insert(RuntimeOnly);
                app.world_mut().spawn((RuntimeOnly, ChildOf(*entity))).id()
            }).collect();
            for (levels, vertices) in [(Some(1),9), (Some(2),25), (None,4), (Some(1),9)] {
                if let Some(levels) = levels { app.insert_resource(UsdSubdivisionSettings::new(levels).unwrap()); }
                else { app.world_mut().remove_resource::<UsdSubdivisionSettings>(); }
                app.update();
                assert_eq!(lookup(app.world(), "/M"), meshes);
                let mut rendered = meshes.clone();
                for parent in lookup(app.world(), "/PI") {
                    rendered.extend(app.world().get::<Children>(parent).unwrap().iter()
                        .filter(|child| app.world().get::<super::super::instancer::UsdInstance>(*child).is_some()));
                }
                for entity in rendered {
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
                    assert_eq!(mesh.count_vertices(), vertices);
                }
                for (index, entity) in meshes.iter().enumerate() {
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(*entity).unwrap().0).unwrap();
                    let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("positions") };
                    assert_eq!(points[1][0], if index == 1 { 2.0 } else { 1.0 });
                    assert!(app.world().get::<RuntimeOnly>(*entity).is_some());
                    assert_eq!(app.world().get::<ChildOf>(children[index]).unwrap().parent(), *entity);
                    assert_eq!(app.world().get::<UsdSubdivisionApplied>(*entity).map(|applied| applied.levels), levels);
                }
                for (root, time) in roots.iter().zip([0.0,10.0]) {
                    assert_eq!(app.world().get::<UsdInstanceTime>(*root).unwrap().current, time);
                }
            }
        }
    }

    #[test]
    fn morph_control_points_are_deformed_before_refinement() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_animation.usda");
        let source = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap();
        let stage = source.open_stage().unwrap();
        stage.attribute("/Test/Face.subdivisionScheme").unwrap()
            .set(openusd::sdf::Value::Token("catmullClark".into())).unwrap();
        let path = openusd::sdf::Path::new("/Test/Face").unwrap();
        for (time, height) in [(0.0,0.0), (5.0,0.5), (10.0,1.0)] {
            let ctx = RouteCtx::at(&stage, &path, Some(time));
            let mesh = refined_mesh(&ctx, 1).unwrap();
            assert_eq!(mesh.points.len(), 9);
            assert_eq!(mesh.points[0][2], height);
            assert_eq!(mesh.points[4], [0.5,0.5,height * 0.25]);
            assert!(mesh.triangulation_points.as_ref().unwrap().iter().all(|point| point[2] == 0.0));
        }
    }

    #[test]
    fn refined_routes_follow_clocks_subsets_and_failure_recovery() {
        let source = crate::UsdSource::new("refined-route.usda", br#"#usda 1.0
def Mesh "M" {
    point3f[] points.timeSamples = { 0: [(0,0,0),(1,0,0),(1,1,0),(0,1,0)], 10: [(0,0,0),(2,0,0),(2,2,0),(0,2,0)] }
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0,1,2,3]
    def GeomSubset "Part" {
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices = [0]
    }
}
def PointInstancer "PI" {
    point3f[] positions = [(0,0,0),(3,0,0)]
    int[] protoIndices = [0,0]
    rel prototypes = [</M>]
}
"#.as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>()
            .insert_resource(UsdSubdivisionSettings::new(1).unwrap());
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let lookup = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/M").unwrap();
        let a = lookup(app.world(), first);
        let b = lookup(app.world(), second);
        let instances = |world: &World, root| {
            let parent = world.get_non_send::<UsdInstances>().unwrap().entity(root, "/PI").unwrap();
            world.get::<Children>(parent).unwrap().iter()
                .filter(|child| world.get::<super::super::instancer::UsdInstance>(*child).is_some()).collect::<Vec<_>>()
        };
        let check_instances = |world: &World, root, width| {
            let children = instances(world, root);
            assert_eq!(children.len(), 2);
            let handles: Vec<_> = children.iter().map(|&child| world.get::<Mesh3d>(child).unwrap().0.clone()).collect();
            assert_eq!(handles[0], handles[1]);
            let mesh = world.resource::<Assets<Mesh>>().get(&handles[0]).unwrap();
            assert_eq!(mesh.count_vertices(), 0);
            for child in children {
                let subset = world.get::<Children>(child).unwrap().iter()
                    .find(|child| world.get::<super::super::subset::UsdSubset>(*child).is_some()).unwrap();
                let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(subset).unwrap().0).unwrap();
                assert_eq!(mesh.indices().unwrap().len(), 24);
                assert_eq!(mesh.count_vertices(), 9);
                let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("points") };
                assert_eq!(points[1][0], width);
            }
        };
        let check = |world: &World, entity, width| {
            assert_eq!(world.get::<UsdSubdivisionApplied>(entity).unwrap().levels, 1);
            let read = &world.get::<super::super::skel::CpuSubsetGeometry>(entity).unwrap().0;
            assert_eq!(read.points.len(), 9);
            assert_eq!(read.face_vertex_counts, [4;4]);
            assert_eq!(read.points[1][0], width);
            let child = world.get::<Children>(entity).unwrap().iter()
                .find(|child| world.get::<super::super::subset::UsdSubset>(*child).is_some()).unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(child).unwrap().0).unwrap();
            assert_eq!(mesh.indices().unwrap().len(), 24);
        };
        check(app.world(), a, 1.0);
        check(app.world(), b, 2.0);
        check_instances(app.world(), first, 1.0);
        check_instances(app.world(), second, 2.0);
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.update();
        check(app.world(), a, 1.5);
        check(app.world(), b, 2.0);
        check_instances(app.world(), first, 1.5);
        check_instances(app.world(), second, 2.0);
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(first).unwrap().clone();
        let holes = stage.create_attribute("/M.holeIndices", "int[]").unwrap();
        holes.clone().set(openusd::sdf::Value::IntVec(vec![1])).unwrap();
        let runtime = app.world_mut().spawn(ChildOf(a)).id();
        app.update();
        assert!(app.world().get::<UsdSubdivisionError>(a).unwrap().0.contains("hole"));
        assert!(app.world().get::<Mesh3d>(a).is_none());
        assert!(app.world().get::<Children>(a).unwrap().iter()
            .all(|child| app.world().get::<super::super::subset::UsdSubset>(child).is_none()));
        assert!(app.world().get_entity(runtime).is_ok());
        assert!(instances(app.world(), first).iter().all(|&child| app.world().get::<Mesh3d>(child).is_none()));
        check(app.world(), b, 2.0);
        holes.set(openusd::sdf::Value::IntVec(vec![])).unwrap();
        app.update();
        assert_eq!(lookup(app.world(), first), a);
        assert!(app.world().get::<UsdSubdivisionError>(a).is_none());
        check(app.world(), a, 1.5);
        check_instances(app.world(), first, 1.5);
        assert!(app.world().get_entity(runtime).is_ok());
        let scheme = stage.attribute("/M.subdivisionScheme").unwrap();
        scheme.clone().set(openusd::sdf::Value::Token("unknown".into())).unwrap();
        app.update();
        assert!(app.world().get::<UsdSubdivisionError>(a).unwrap().0.contains("subdivisionScheme"));
        assert!(app.world().get::<Mesh3d>(a).is_none());
        scheme.set(openusd::sdf::Value::Token("none".into())).unwrap();
        app.update();
        assert!(app.world().get::<UsdSubdivisionError>(a).is_none());
        assert!(app.world().get::<UsdSubdivisionApplied>(a).is_none());
        assert!(app.world().get::<Mesh3d>(a).is_some());
        check(app.world(), b, 2.0);
        assert!(UsdSubdivisionSettings::new(0).is_err());
        assert!(UsdSubdivisionSettings::new(7).is_err());
    }
}
