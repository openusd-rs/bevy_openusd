//! Geometric fragment normals for GPU-deformed polygonal meshes.

use bevy::{pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin}, prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef};

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct FlatNormals {}

impl MaterialExtension for FlatNormals {
    fn fragment_shader() -> ShaderRef { "embedded://usd_bevy/route/flat_material.wgsl".into() }
    fn deferred_fragment_shader() -> ShaderRef { Self::fragment_shader() }
    fn prepass_fragment_shader() -> ShaderRef { "embedded://usd_bevy/route/flat_prepass.wgsl".into() }
}

pub type FlatMaterial = ExtendedMaterial<StandardMaterial, FlatNormals>;

#[derive(Component)]
struct OwnedFlatMaterial {
    base: Handle<StandardMaterial>,
    material: Handle<FlatMaterial>,
}

#[derive(Resource, Default)]
struct FlatMaterialCache(std::collections::HashMap<AssetId<StandardMaterial>, Handle<FlatMaterial>>);

pub(crate) fn configure(app: &mut App) {
    app.init_resource::<FlatMaterialCache>();
    app.add_systems(Last, prune_cache);
    bevy::asset::embedded_asset!(app, "flat_material.wgsl");
    bevy::asset::embedded_asset!(app, "flat_prepass.wgsl");
    bevy::asset::embedded_asset!(app, "flat_functions.wgsl");
    app.add_plugins(MaterialPlugin::<FlatMaterial>::default());
}

fn prune_cache(cache: Option<ResMut<FlatMaterialCache>>, assets: Option<Res<Assets<FlatMaterial>>>) {
    let (Some(mut cache), Some(assets)) = (cache, assets) else { return };
    cache.0.retain(|_, handle| assets.contains(handle.id()) && super::cache::externally_owned(handle));
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    let Some(owned) = world.entity_mut(entity).take::<OwnedFlatMaterial>() else { return };
    if world.get::<MeshMaterial3d<FlatMaterial>>(entity).is_some_and(|material| material.0 == owned.material) {
        world.entity_mut(entity).remove::<MeshMaterial3d<FlatMaterial>>();
        if world.get::<MeshMaterial3d<StandardMaterial>>(entity).is_none() {
            world.entity_mut(entity).insert(MeshMaterial3d(owned.base));
        }
    }
}

pub(crate) fn attach(world: &mut World, entity: Entity) {
    if !world.contains_resource::<Assets<FlatMaterial>>() { return; }
    let base = world.get::<MeshMaterial3d<StandardMaterial>>(entity).map(|material| material.0.clone())
        .or_else(|| world.get::<OwnedFlatMaterial>(entity).map(|owned| owned.base.clone()));
    let Some(base) = base else { return };
    let Some(material) = world.get_resource::<Assets<StandardMaterial>>().and_then(|assets| assets.get(&base)).cloned() else { return };
    let existing = world.get::<OwnedFlatMaterial>(entity).and_then(|owned| {
        world.resource::<Assets<FlatMaterial>>().get(&owned.material)
            .filter(|existing| existing.base.cull_mode == material.cull_mode
                && existing.base.reflect_partial_eq(&material) == Some(true))
            .map(|_| owned.material.clone())
    });
    let cached = world.get_resource::<FlatMaterialCache>()
        .and_then(|cache| cache.0.get(&base.id()))
        .filter(|handle| world.resource::<Assets<FlatMaterial>>().get(*handle)
            .is_some_and(|existing| existing.base.cull_mode == material.cull_mode
                && existing.base.reflect_partial_eq(&material) == Some(true)))
        .cloned();
    let handle = existing.or(cached).unwrap_or_else(|| {
        let handle = world.resource_mut::<Assets<FlatMaterial>>()
            .add(FlatMaterial { base: material, extension: FlatNormals {} });
        if let Some(mut cache) = world.get_resource_mut::<FlatMaterialCache>() {
            if cache.0.len() >= 1024 { cache.0.clear(); }
            cache.0.insert(base.id(), handle.clone());
        }
        handle
    });
    world.entity_mut(entity).remove::<MeshMaterial3d<StandardMaterial>>()
        .insert((MeshMaterial3d(handle.clone()), OwnedFlatMaterial { base, material: handle }));
}

pub(crate) fn base_handle(world: &World, entity: Entity) -> Option<Handle<StandardMaterial>> {
    world.get::<MeshMaterial3d<StandardMaterial>>(entity).map(|material| material.0.clone())
        .or_else(|| world.get::<OwnedFlatMaterial>(entity).map(|owned| owned.base.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (World, Entity, Handle<StandardMaterial>) {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<FlatMaterial>>();
        world.init_resource::<FlatMaterialCache>();
        let base = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(base.clone())).id();
        (world, entity, base)
    }

    #[test]
    fn pruning_preserves_owned_materials_and_releases_history() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, entity, base) = setup();
        attach(&mut world, entity);
        let handle = world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0.clone();
        world.run_system_once(prune_cache).unwrap();
        assert_eq!(world.resource::<FlatMaterialCache>().0.len(), 1);
        clear(&mut world, entity);
        world.run_system_once(prune_cache).unwrap();
        assert_eq!(world.resource::<FlatMaterialCache>().0.len(), 1);
        drop(handle);
        world.run_system_once(prune_cache).unwrap();
        assert!(world.resource::<FlatMaterialCache>().0.is_empty());
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0, base);
        attach(&mut world, entity);
        let handle = world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0.clone();
        world.resource_mut::<Assets<FlatMaterial>>().remove(handle.id());
        world.run_system_once(prune_cache).unwrap();
        assert!(world.resource::<FlatMaterialCache>().0.is_empty());
    }

    #[test]
    fn conversion_reuses_updates_and_restores_base_material() {
        let (mut world, entity, base) = setup();
        attach(&mut world, entity);
        let first = world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0.clone();
        assert!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).is_none());
        attach(&mut world, entity);
        assert_eq!(world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0, first);
        world.resource_mut::<Assets<StandardMaterial>>().get_mut(&base).unwrap().perceptual_roughness = 0.17;
        attach(&mut world, entity);
        let updated = &world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0;
        assert_ne!(*updated, first);
        assert_eq!(world.resource::<Assets<FlatMaterial>>().get(updated).unwrap().base.perceptual_roughness, 0.17);
        clear(&mut world, entity);
        assert!(world.get::<MeshMaterial3d<FlatMaterial>>(entity).is_none());
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0, base);
        clear(&mut world, entity);
    }

    #[test]
    fn conversion_updates_culling_when_reflected_fields_are_unchanged() {
        use bevy::render::render_resource::Face;
        let (mut world, entity, base) = setup();
        for cull_mode in [Some(Face::Back), None, Some(Face::Front)] {
            world.resource_mut::<Assets<StandardMaterial>>().get_mut(&base).unwrap().cull_mode = cull_mode;
            attach(&mut world, entity);
            let handle = &world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0;
            assert_eq!(world.resource::<Assets<FlatMaterial>>().get(handle).unwrap().base.cull_mode, cull_mode);
        }
    }

    #[test]
    fn conversion_shares_base_material_and_rejects_mutated_or_removed_assets() {
        let (mut world, first, base) = setup();
        attach(&mut world, first);
        let initial = world.get::<MeshMaterial3d<FlatMaterial>>(first).unwrap().0.clone();
        let second = world.spawn(MeshMaterial3d(base.clone())).id();
        attach(&mut world, second);
        assert_eq!(world.get::<MeshMaterial3d<FlatMaterial>>(second).unwrap().0, initial);
        assert_eq!(world.resource::<Assets<FlatMaterial>>().len(), 1);
        world.resource_mut::<Assets<FlatMaterial>>().get_mut(&initial).unwrap().base.cull_mode = None;
        let third = world.spawn(MeshMaterial3d(base.clone())).id();
        attach(&mut world, third);
        let repaired = world.get::<MeshMaterial3d<FlatMaterial>>(third).unwrap().0.clone();
        assert_ne!(repaired, initial);
        assert_eq!(world.resource::<Assets<FlatMaterial>>().get(&repaired).unwrap().base.cull_mode,
            Some(bevy::render::render_resource::Face::Back));
        world.resource_mut::<Assets<FlatMaterial>>().remove(repaired.id());
        let fourth = world.spawn(MeshMaterial3d(base)).id();
        attach(&mut world, fourth);
        let recreated = &world.get::<MeshMaterial3d<FlatMaterial>>(fourth).unwrap().0;
        assert_ne!(*recreated, repaired);
        assert!(world.resource::<Assets<FlatMaterial>>().contains(recreated));
    }

    #[test]
    fn conversion_cache_bounds_retained_handles() {
        let (mut world, entity, _) = setup();
        for _ in 0..1025 {
            let base = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
            world.entity_mut(entity).insert(MeshMaterial3d(base));
            world.entity_mut(entity).remove::<OwnedFlatMaterial>();
            attach(&mut world, entity);
            assert!(world.resource::<FlatMaterialCache>().0.len() <= 1024);
        }
        assert_eq!(world.resource::<FlatMaterialCache>().0.len(), 1);
    }

    #[test]
    fn independent_gpu_morph_instances_share_flat_material_assets() {
        use crate::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/morph_animation.usda");
        let source = UsdSource::new(path, std::fs::read(path).unwrap()).unwrap();
        for cached in [false, true] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default(), UsdPlugin, UsdAssetPlugin,
                super::super::gpu_skin::UsdGpuSkinningPlugin));
            app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
            if !cached { app.world_mut().remove_resource::<FlatMaterialCache>(); }
            let scene = app.world_mut().resource_mut::<Assets<UsdScene>>()
                .add(UsdScene { source: source.clone(), textures: default() });
            let roots: Vec<_> = (0..8).map(|_| app.world_mut().spawn(UsdSceneRoot(scene.clone())).id()).collect();
            app.update();
            for root in roots {
                assert_eq!(app.world().get::<crate::UsdSceneState>(root), Some(&crate::UsdSceneState::Ready));
            }
            let handles: Vec<_> = app.world_mut().query::<&MeshMaterial3d<FlatMaterial>>()
                .iter(app.world()).map(|material| material.0.id()).collect();
            assert_eq!(handles.len(), 8);
            let unique: std::collections::HashSet<_> = handles.into_iter().collect();
            assert_eq!(unique.len(), if cached { 1 } else { 8 });
            assert_eq!(app.world().resource::<Assets<FlatMaterial>>().len(), unique.len());
        }
    }

    #[test]
    fn cleanup_preserves_external_replacements() {
        let (mut world, entity, _) = setup();
        attach(&mut world, entity);
        let replacement = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        world.entity_mut(entity).insert(MeshMaterial3d(replacement.clone()));
        clear(&mut world, entity);
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0, replacement);
        attach(&mut world, entity);
        let external = world.resource_mut::<Assets<FlatMaterial>>().add(FlatMaterial::default());
        world.entity_mut(entity).insert(MeshMaterial3d(external.clone()));
        clear(&mut world, entity);
        assert_eq!(world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0, external);
        assert!(world.get::<MeshMaterial3d<StandardMaterial>>(entity).is_none());
    }
}
