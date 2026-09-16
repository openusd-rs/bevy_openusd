//! Material-bound face subsets projected as mesh children.

use bevy::prelude::*;
use super::{PrimRoute, RouteCtx};

pub struct SubsetRoute;

#[derive(Component)]
pub struct UsdSubset(pub String);

#[derive(Component, Debug)]
pub struct UsdSubsetWarning(pub String);

#[derive(Component)]
struct SubsetSkin {
    skin: bevy::mesh::skinning::SkinnedMesh,
}

fn update_skin(world: &mut World, parent: Entity, flat: bool) {
    use bevy::mesh::skinning::SkinnedMesh;
    let skin = world.get::<super::gpu_skin::UsdGpuSkin>(parent)
        .and_then(|_| world.get::<SkinnedMesh>(parent)).cloned();
    let morph = world.get::<super::gpu_morph::UsdGpuMorph>(parent).is_some();
    for (child, _) in children(world, parent) {
        if !morph { super::gpu_morph::clear(world, child); }
        if let Some(skin) = &skin {
            super::deformation::acquire(world, child, super::deformation::SKIN);
            world.entity_mut(child).insert((skin.clone(), SubsetSkin { skin: skin.clone() }));
            if flat { super::flat_material::attach(world, child); }
            else { super::flat_material::clear(world, child); }
        } else if let Some(owned) = world.entity_mut(child).take::<SubsetSkin>() {
            let unchanged = world.get::<SkinnedMesh>(child).is_some_and(|current|
                current.joints == owned.skin.joints && current.inverse_bindposes == owned.skin.inverse_bindposes);
            if unchanged { world.entity_mut(child).remove::<SkinnedMesh>(); }
            super::deformation::release(world, child, super::deformation::SKIN);
            super::flat_material::clear(world, child);
        }
        if morph { super::gpu_morph::inherit(world, parent, child, flat); }
    }
}

#[derive(Clone, Default)]
pub(crate) struct PreparedSubsets {
    remainder: Option<Handle<Mesh>>,
    parts: Vec<(String, Handle<Mesh>, Handle<StandardMaterial>, Vec<String>)>,
    warning: Option<String>,
}

pub(crate) fn prepare(
    ctx: &RouteCtx,
    world: &mut World,
    read: &crate::read::geom::ReadMesh,
    source: &Mesh,
    default_material: &Handle<StandardMaterial>,
) -> PreparedSubsets {
    let mut prepared = PreparedSubsets::default();
    if read.subsets.is_empty() { return prepared; }
    let mut assigned = vec![false; read.face_vertex_counts.len()];
    for subset in &read.subsets {
        for &face in &subset.indices {
            let valid = usize::try_from(face).ok().and_then(|face| assigned.get_mut(face))
                .is_some_and(|assigned| { let fresh = !*assigned; *assigned = true; fresh });
            if !valid {
                prepared.warning = Some("invalid or overlapping material subset faces; rendering whole mesh".into());
                return prepared;
            }
        }
    }
    let face_indices = crate::mesh::MeshFaceIndices::new(read);
    for subset in &read.subsets {
        let Ok(path) = ctx.path.append_path(subset.name.as_str()) else { continue };
        let subset_ctx = RouteCtx::at(ctx.stage, &path, ctx.time);
        let (material, mut warnings) = match super::material::resolve_material(&subset_ctx, world) {
            Ok(Some((handle, warnings))) => {
                let mut material = world.resource::<Assets<StandardMaterial>>().get(&handle).unwrap().clone();
                super::material::apply_sidedness(ctx, &mut material);
                (super::cache::intern_material(world, material), warnings)
            }
            Ok(None) => (default_material.clone(), Vec::new()),
            Err(error) => (default_material.clone(), vec![error.to_string()]),
        };
        let mesh = subset_mesh(source, face_indices.for_faces(&subset.indices));
        if let Some(material) = world.resource::<Assets<StandardMaterial>>().get(&material) {
            super::material::warn_geometry_inputs(&mesh, material, &mut warnings);
        }
        prepared.parts.push((subset.name.clone(), super::cache::intern_mesh(world, mesh), material, warnings));
    }
    let remaining: Vec<i32> = assigned.iter().enumerate().filter_map(|(face, assigned)| (!assigned).then_some(face as i32)).collect();
    let mesh = subset_mesh(source, face_indices.for_faces(&remaining));
    prepared.remainder = Some(super::cache::intern_mesh(world, mesh));
    prepared
}

fn subset_mesh(source: &Mesh, indices: bevy::mesh::Indices) -> Mesh {
    crate::mesh::compact::compact(source, &indices).unwrap_or_else(|| {
        let mut mesh = source.clone();
        mesh.insert_indices(indices);
        mesh
    })
}

pub(crate) fn apply(world: &mut World, entity: Entity, prepared: &PreparedSubsets) {
    let old = children(world, entity);
    let mut retained = Vec::new();
    for (name, mesh, material, warnings) in &prepared.parts {
        let child = old.iter().find(|(_, old_name)| old_name == name).map(|(entity, _)| *entity)
            .unwrap_or_else(|| world.spawn((UsdSubset(name.clone()), Transform::default(), Visibility::default(), ChildOf(entity))).id());
        world.entity_mut(child).insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
        world.entity_mut(child).remove::<bevy::camera::primitives::Aabb>();
        if warnings.is_empty() { world.entity_mut(child).remove::<super::material::UsdMaterialWarning>(); }
        else { world.entity_mut(child).insert(super::material::UsdMaterialWarning(warnings.join("; "))); }
        retained.push(child);
    }
    for (child, _) in old { if !retained.contains(&child) { world.despawn(child); } }
    if let Some(handle) = &prepared.remainder { world.entity_mut(entity).insert(Mesh3d(handle.clone())); }
    if let Some(warning) = &prepared.warning { world.entity_mut(entity).insert(UsdSubsetWarning(warning.clone())); }
    else { world.entity_mut(entity).remove::<UsdSubsetWarning>(); }
}

fn children(world: &World, entity: Entity) -> Vec<(Entity, String)> {
    world.get::<Children>(entity).into_iter().flat_map(|children| children.iter())
        .filter_map(|child| world.get::<UsdSubset>(child).map(|subset| (child, subset.0.clone())))
        .collect()
}

fn clear(world: &mut World, entity: Entity) {
    for (child, _) in children(world, entity) { world.despawn(child); }
    world.entity_mut(entity).remove::<UsdSubsetWarning>();
}

impl PrimRoute for SubsetRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool { super::geom::MeshRoute.matches(ctx) }

    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) { clear(world, entity); }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let read = world.get::<super::skel::CpuSubsetGeometry>(entity).map(|geometry| std::borrow::Cow::Owned(geometry.0.clone()))
            .or_else(|| ctx.read_mesh().ok().flatten().map(std::borrow::Cow::Borrowed));
        let Some(read) = read else {
            clear(world, entity);
            return;
        };
        if read.subsets.is_empty() { clear(world, entity); return; }
        let Some(source) = world.get::<Mesh3d>(entity)
            .and_then(|handle| world.get_resource::<Assets<Mesh>>()?.get(&handle.0)).cloned() else {
            clear(world, entity);
            return;
        };
        let Some(default_material) = super::flat_material::base_handle(world, entity) else { return };
        let prepared = prepare(ctx, world, &read, &source, &default_material);
        apply(world, entity, &prepared);
        update_skin(world, entity, crate::mesh::uses_flat_normals(&read));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{UsdInstanceTime, UsdInstances};

    #[test]
    fn gpu_morph_subsets_follow_independent_clocks_and_cpu_cleanup() {
        use bevy::mesh::VertexAttributeValues;
        use bevy::mesh::morph::MeshMorphWeights;
        use super::super::{gpu_morph::UsdGpuMorph, gpu_skin::GpuSkinningEnabled, flat_material::FlatMaterial};
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_subsets.usda");
        let source = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap();
        let reference_stage = source.open_stage().unwrap();
        let reference_path = openusd::sdf::path("/Test/Face").unwrap();
        let reference_ctx = RouteCtx::at(&reference_stage, &reference_path, Some(0.0));
        let reference_read = crate::read::geom::read_mesh_at(&reference_stage, &reference_path, Some(0.0)).unwrap().unwrap();
        let mut reference = crate::mesh::mesh_from_usd(&reference_read);
        super::super::gpu_morph::prepare(&reference_ctx, &reference_read, &mut reference, true).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<FlatMaterial>>();
        app.init_resource::<GpuSkinningEnabled>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let a = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let b = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let parent = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/Test/Face").unwrap();
        let (pa, pb) = (parent(app.world(), a), parent(app.world(), b));
        let (ca, cb) = (children(app.world(), pa)[0].0, children(app.world(), pb)[0].0);
        let validate = |world: &World, parent, child, weight, indices: Vec<usize>| {
            for entity in [parent, child] {
                assert!(world.get::<UsdGpuMorph>(entity).is_some());
                assert!(matches!(world.get::<MeshMorphWeights>(entity), Some(MeshMorphWeights::Value { weights }) if weights == &vec![weight]));
                assert!(world.get::<MeshMaterial3d<FlatMaterial>>(entity).is_some());
            }
            let assets = world.resource::<Assets<Mesh>>();
            let p = assets.get(&world.get::<Mesh3d>(parent).unwrap().0).unwrap();
            let c = assets.get(&world.get::<Mesh3d>(child).unwrap().0).unwrap();
            assert_eq!(p.count_vertices(), 3);
            assert_eq!(c.count_vertices(), 3);
            assert_eq!(c.indices().unwrap().iter().collect::<Vec<_>>(), [0,1,2]);
            let VertexAttributeValues::Float32x3(full_positions) = reference.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            let VertexAttributeValues::Float32x3(positions) = c.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            assert_eq!(positions, &indices.iter().map(|&index| full_positions[index]).collect::<Vec<_>>());
            let targets = reference.get_morph_targets().unwrap();
            assert_eq!(c.get_morph_targets().unwrap(), indices.iter().map(|&index| targets[index]).collect::<Vec<_>>());
        };
        validate(app.world(), pa, ca, 0.0, vec![0,1,2]);
        validate(app.world(), pb, cb, 1.0, vec![3,4,5]);
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 5.0;
        app.update();
        assert_eq!(children(app.world(), pa)[0].0, ca);
        validate(app.world(), pa, ca, 0.5, vec![0,1,2]);
        validate(app.world(), pb, cb, 1.0, vec![3,4,5]);
        app.world_mut().remove_resource::<GpuSkinningEnabled>();
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 10.0;
        app.update();
        for entity in [pa, ca] {
            assert!(app.world().get::<UsdGpuMorph>(entity).is_none());
            assert!(app.world().get::<MeshMorphWeights>(entity).is_none());
            assert!(app.world().get::<bevy::camera::visibility::NoFrustumCulling>(entity).is_none());
            assert!(app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).is_some());
        }
        assert_eq!(children(app.world(), pa)[0].0, ca);
    }

    #[test]
    fn gpu_subsets_share_palette_and_clear_on_cpu_transition() {
        use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
        use super::super::{flat_material::FlatMaterial, gpu_skin::GpuSkinningEnabled};
        for fixture in ["skel_material_subsets.usda", "skel_morph_subsets.usda"] {
        let file = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")).join(fixture);
        let source = crate::UsdSource::new(&file, std::fs::read(&file).unwrap()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<Assets<FlatMaterial>>();
        app.init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        app.init_resource::<GpuSkinningEnabled>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let root = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 30.0 })).id();
        app.update();
        let parent = app.world().get_non_send::<UsdInstances>().unwrap().entity(root, "/Test/Bar").unwrap();
        let parts = children(app.world(), parent);
        assert_eq!(parts.len(), 2);
        let skin = app.world().get::<SkinnedMesh>(parent).unwrap().clone();
        for (child, _) in &parts {
            let subset_skin = app.world().get::<SkinnedMesh>(*child).unwrap();
            assert_eq!(subset_skin.joints, skin.joints);
            assert_eq!(subset_skin.inverse_bindposes, skin.inverse_bindposes);
            assert!(app.world().get::<MeshMaterial3d<FlatMaterial>>(*child).is_some());
            assert!(app.world().get::<MeshMaterial3d<StandardMaterial>>(*child).is_none());
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(*child).unwrap().0).unwrap();
            assert!(mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some());
            assert_eq!(mesh.indices().unwrap().len(), 42);
            if fixture == "skel_morph_subsets.usda" {
                assert!(mesh.get_morph_targets().is_some());
                assert!(app.world().get::<super::super::gpu_morph::UsdGpuMorph>(*child).is_some());
            }
        }
        app.world_mut().remove_resource::<GpuSkinningEnabled>();
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 15.0;
        app.update();
        assert_eq!(children(app.world(), parent), parts);
        for (child, _) in &parts {
            assert!(app.world().get::<SkinnedMesh>(*child).is_none());
            assert!(app.world().get::<bevy::mesh::morph::MeshMorphWeights>(*child).is_none());
            assert!(app.world().get::<MeshMaterial3d<FlatMaterial>>(*child).is_none());
            assert!(app.world().get::<MeshMaterial3d<StandardMaterial>>(*child).is_some());
            assert!(app.world().get::<bevy::camera::visibility::NoFrustumCulling>(*child).is_none());
        }
        assert!(skin.joints.iter().all(|joint| app.world().get_entity(*joint).is_err()));
        app.world_mut().init_resource::<GpuSkinningEnabled>();
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 30.0;
        app.update();
        assert_eq!(children(app.world(), parent), parts);
        for (child, _) in &parts { assert!(app.world().get::<SkinnedMesh>(*child).is_some()); }
        if fixture == "skel_morph_subsets.usda" {
            let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(root).unwrap().clone();
            stage.attribute("/Test/Bar.skel:blendShapes").unwrap()
                .set(openusd::sdf::Value::TokenVec(vec![])).unwrap();
            app.update();
            for entity in std::iter::once(parent).chain(parts.iter().map(|(child, _)| *child)) {
                assert!(app.world().get::<SkinnedMesh>(entity).is_some());
                assert!(app.world().get::<bevy::mesh::morph::MeshMorphWeights>(entity).is_none());
                assert!(app.world().get::<bevy::camera::visibility::NoFrustumCulling>(entity).is_some());
            }
            app.world_mut().remove_resource::<GpuSkinningEnabled>();
            app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 0.0;
            app.update();
            for entity in std::iter::once(parent).chain(parts.iter().map(|(child, _)| *child)) {
                assert!(app.world().get::<bevy::camera::visibility::NoFrustumCulling>(entity).is_none());
            }
        }
        }
    }

    #[test]
    fn subsets_keep_cpu_morphed_positions_and_rebuilt_normals() {
        let text = include_str!("../../../../assets/blendshape_test.usda").replace(
            "def BlendShape \"smile\"",
            r#"normal3f[] normals = [(0,0,1), (0,0,1), (0,0,1), (0,0,1)] (interpolation = "faceVarying")
            def GeomSubset "Part" {
                uniform token familyName = "materialBind"
                uniform token elementType = "face"
                int[] indices = [0]
            }
            def BlendShape "smile""#,
        );
        let stage = crate::snippet::UsdSnippet::new(text).open_stage().unwrap();
        let live = crate::live::LiveStage::new(stage);
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/Test/Face").unwrap();
        let child = children(&world, parent)[0].0;
        let geometry = &world.get::<super::super::skel::CpuSubsetGeometry>(parent).unwrap().0;
        assert_eq!(geometry.points[0], [0.0,0.0,1.0]);
        assert_eq!(geometry.normals.as_ref().unwrap().values, vec![[0.0, 0.0, 1.0]; 4]);
        let expected = crate::mesh::mesh_from_usd(geometry);
        let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(child).unwrap().0).unwrap();
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_POSITION), expected.attribute(Mesh::ATTRIBUTE_POSITION));
        assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL), expected.attribute(Mesh::ATTRIBUTE_NORMAL));
        assert_eq!(mesh.indices().unwrap().iter().collect::<Vec<_>>(), expected.indices().unwrap().iter().collect::<Vec<_>>());
    }

    #[test]
    fn live_subset_edits_validate_recover_and_remove_owned_children() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    def GeomSubset "Part" {
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices = [0]
    }
}
"#).open_stage().unwrap();
        let live = crate::live::LiveStage::new(stage);
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let parent = map.entity("/M").unwrap();
        let child = children(&world, parent)[0].0;
        let unrelated = world.spawn(ChildOf(parent)).id();
        let attribute = live.stage.prim("/M/Part").unwrap().attribute("indices");
        for indices in [vec![-1], vec![1], vec![0,0]] {
            attribute.clone().set(openusd::sdf::Value::IntVec(indices)).unwrap();
            crate::live::apply_changes(&mut world, &live, &mut map);
            assert!(children(&world, parent).is_empty());
            assert!(world.get::<UsdSubsetWarning>(parent).is_some());
            let handle = &world.get::<Mesh3d>(parent).unwrap().0;
            assert_eq!(world.resource::<Assets<Mesh>>().get(handle).unwrap().indices().unwrap().len(), 3);
        }
        assert!(world.get_entity(child).is_err());
        attribute.set(openusd::sdf::Value::IntVec(vec![0])).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(children(&world, parent).len(), 1);
        assert!(world.get::<UsdSubsetWarning>(parent).is_none());
        live.stage.prim("/M/Part").unwrap().set_type_name("Xform").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(children(&world, parent).is_empty());
        assert!(world.get_entity(unrelated).is_ok());
        assert_eq!(map.entity("/M"), Some(parent));
    }

    #[test]
    fn subset_only_animation_splits_faces_and_preserves_independent_children() {
        let source = crate::UsdSource::new("subsets.usda", &br#"#usda 1.0
def Mesh "M" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0), (0,0,1)]
    int[] faceVertexCounts = [3,3]
    int[] faceVertexIndices = [0,1,2,0,3,1]
    bool doubleSided = true
    def GeomSubset "Part" {
        uniform token familyName = "materialBind"
        uniform token elementType = "face"
        int[] indices.timeSamples = { 0: [0], 10: [1], 20: [] }
        rel material:binding = </Mat>
    }
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.timeSamples = { 0: (1,0,0), 10: (0,0,1) }
        token outputs:surface
    }
}
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let a = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let b = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let parent = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/M").unwrap();
        let pa = parent(app.world(), a);
        let pb = parent(app.world(), b);
        let ca = children(app.world(), pa)[0].0;
        let cb = children(app.world(), pb)[0].0;
        let indices = |world: &World, entity| {
            let handle = &world.get::<Mesh3d>(entity).unwrap().0;
            world.resource::<Assets<Mesh>>().get(handle).unwrap().indices().unwrap().iter().collect::<Vec<_>>()
        };
        let renderable = |world: &World, entity| {
            let handle = &world.get::<Mesh3d>(entity).unwrap().0;
            world.resource::<Assets<Mesh>>().get(handle).unwrap().asset_usage.contains(bevy::asset::RenderAssetUsages::RENDER_WORLD)
        };
        assert_eq!(indices(app.world(), ca), [0,1,2]);
        assert_eq!(indices(app.world(), pa), [0,2,1]);
        assert_eq!(indices(app.world(), cb), [0,2,1]);
        assert_eq!(indices(app.world(), pb), [0,1,2]);
        for (child, color) in [(ca, Color::linear_rgb(1.0,0.0,0.0)), (cb, Color::linear_rgb(0.0,0.0,1.0))] {
            let handle = &app.world().get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0;
            let material = app.world().resource::<Assets<StandardMaterial>>().get(handle).unwrap();
            assert_eq!(material.base_color, color);
            assert!(material.double_sided);
        }
        #[derive(Component)]
        struct Runtime;
        app.world_mut().entity_mut(ca).insert(Runtime);
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 20.0;
        app.update();
        assert_eq!(parent(app.world(), a), pa);
        assert_eq!(children(app.world(), pa)[0].0, ca);
        assert!(app.world().get::<Runtime>(ca).is_some());
        assert!(indices(app.world(), ca).is_empty());
        assert!(!renderable(app.world(), ca));
        assert!(renderable(app.world(), cb));
        assert_eq!(indices(app.world(), pa).len(), 6);
        assert_eq!(indices(app.world(), cb), [0,2,1]);
        app.world_mut().get_mut::<UsdInstanceTime>(a).unwrap().current = 5.0;
        app.update();
        assert_eq!(indices(app.world(), ca), [0,1,2]);
        let handle = &app.world().get::<MeshMaterial3d<StandardMaterial>>(ca).unwrap().0;
        assert!(renderable(app.world(), ca));
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().get(handle).unwrap().base_color, Color::linear_rgb(0.5,0.0,0.5));
        app.world_mut().despawn(a);
        app.update();
        assert!(app.world().get_entity(ca).is_err());
        assert!(app.world().get_entity(cb).is_ok());
    }
}
