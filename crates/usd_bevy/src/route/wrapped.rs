//! Standard materials wrapped per entity in a shader extension.

use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin},
    prelude::*,
    render::render_resource::AsBindGroup,
};

pub type Wrapped<E> = ExtendedMaterial<StandardMaterial, E>;

#[derive(Component)]
struct Owned<E: MaterialExtension> {
    base: Handle<StandardMaterial>,
    material: Handle<Wrapped<E>>,
}

#[derive(Resource)]
struct Cache<E: MaterialExtension>(
    std::collections::HashMap<AssetId<StandardMaterial>, Handle<Wrapped<E>>>,
);

impl<E: MaterialExtension> Default for Cache<E> {
    fn default() -> Self {
        Self(default())
    }
}

pub(crate) fn configure<E: MaterialExtension>(app: &mut App) -> bool
where
    <Wrapped<E> as AsBindGroup>::Data: PartialEq + Eq + std::hash::Hash + Clone,
{
    if app.is_plugin_added::<MaterialPlugin<Wrapped<E>>>() {
        return false;
    }
    app.init_resource::<Cache<E>>();
    app.add_systems(Last, prune_cache::<E>);
    app.add_plugins(MaterialPlugin::<Wrapped<E>>::default());
    true
}

fn prune_cache<E: MaterialExtension>(
    cache: Option<ResMut<Cache<E>>>,
    assets: Option<Res<Assets<Wrapped<E>>>>,
) {
    let (Some(mut cache), Some(assets)) = (cache, assets) else {
        return;
    };
    cache
        .0
        .retain(|_, handle| assets.contains(handle.id()) && super::cache::externally_owned(handle));
}

pub(crate) fn clear<E: MaterialExtension>(world: &mut World, entity: Entity) {
    let Some(owned) = world.entity_mut(entity).take::<Owned<E>>() else {
        return;
    };
    if world
        .get::<MeshMaterial3d<Wrapped<E>>>(entity)
        .is_some_and(|material| material.0 == owned.material)
    {
        world
            .entity_mut(entity)
            .remove::<MeshMaterial3d<Wrapped<E>>>();
        if world
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_none()
        {
            world.entity_mut(entity).insert(MeshMaterial3d(owned.base));
        }
    }
}

pub(crate) fn attach<E: MaterialExtension + Default>(world: &mut World, entity: Entity) {
    if !enabled::<E>(world) {
        return;
    }
    let base = world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .map(|material| material.0.clone())
        .or_else(|| {
            world
                .get::<Owned<E>>(entity)
                .map(|owned| owned.base.clone())
        });
    let Some(base) = base else { return };
    let Some(material) = world
        .get_resource::<Assets<StandardMaterial>>()
        .and_then(|assets| assets.get(&base))
        .cloned()
    else {
        return;
    };
    let matches = |existing: &Wrapped<E>| {
        existing.base.cull_mode == material.cull_mode
            && existing.base.reflect_partial_eq(&material) == Some(true)
    };
    let existing = world.get::<Owned<E>>(entity).and_then(|owned| {
        world
            .resource::<Assets<Wrapped<E>>>()
            .get(&owned.material)
            .filter(|existing| matches(existing))
            .map(|_| owned.material.clone())
    });
    let cached = world
        .get_resource::<Cache<E>>()
        .and_then(|cache| cache.0.get(&base.id()))
        .filter(|handle| {
            world
                .resource::<Assets<Wrapped<E>>>()
                .get(*handle)
                .is_some_and(|existing| matches(existing))
        })
        .cloned();
    let handle = existing.or(cached).unwrap_or_else(|| {
        let handle = world.resource_mut::<Assets<Wrapped<E>>>().add(Wrapped {
            base: material,
            extension: E::default(),
        });
        if let Some(mut cache) = world.get_resource_mut::<Cache<E>>() {
            if cache.0.len() >= 1024 {
                cache.0.clear();
            }
            cache.0.insert(base.id(), handle.clone());
        }
        handle
    });
    world
        .entity_mut(entity)
        .remove::<MeshMaterial3d<StandardMaterial>>()
        .insert((
            MeshMaterial3d(handle.clone()),
            Owned::<E> {
                base,
                material: handle,
            },
        ));
}

/// Whether the wrapped material can draw.
pub(crate) fn enabled<E: MaterialExtension>(world: &World) -> bool {
    world.contains_resource::<Assets<Wrapped<E>>>()
}

/// The standard material under `entity`'s wrapper, or the one it draws with.
pub(crate) fn base_handle(world: &World, entity: Entity) -> Option<Handle<StandardMaterial>> {
    let owned = |entity| {
        world
            .get::<Owned<super::flat_material::FlatNormals>>(entity)
            .map(|owned| owned.base.clone())
            .or_else(|| {
                world
                    .get::<Owned<super::strand_material::StrandCoverage>>(entity)
                    .map(|owned| owned.base.clone())
            })
    };
    world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .map(|material| material.0.clone())
        .or_else(|| owned(entity))
}

#[cfg(test)]
mod tests {
    use super::super::flat_material::{FlatMaterial, FlatNormals};
    use super::*;

    fn setup() -> (World, Entity, Handle<StandardMaterial>) {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<FlatMaterial>>();
        world.init_resource::<Cache<FlatNormals>>();
        let base = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(base.clone())).id();
        (world, entity, base)
    }

    fn attach(world: &mut World, entity: Entity) {
        super::attach::<FlatNormals>(world, entity);
    }

    fn clear(world: &mut World, entity: Entity) {
        super::clear::<FlatNormals>(world, entity);
    }

    #[test]
    fn pruning_preserves_owned_materials_and_releases_history() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, entity, base) = setup();
        attach(&mut world, entity);
        let handle = world
            .get::<MeshMaterial3d<FlatMaterial>>(entity)
            .unwrap()
            .0
            .clone();
        world.run_system_once(prune_cache::<FlatNormals>).unwrap();
        assert_eq!(world.resource::<Cache<FlatNormals>>().0.len(), 1);
        clear(&mut world, entity);
        world.run_system_once(prune_cache::<FlatNormals>).unwrap();
        assert_eq!(world.resource::<Cache<FlatNormals>>().0.len(), 1);
        drop(handle);
        world.run_system_once(prune_cache::<FlatNormals>).unwrap();
        assert!(world.resource::<Cache<FlatNormals>>().0.is_empty());
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .unwrap()
                .0,
            base
        );
        attach(&mut world, entity);
        let handle = world
            .get::<MeshMaterial3d<FlatMaterial>>(entity)
            .unwrap()
            .0
            .clone();
        world
            .resource_mut::<Assets<FlatMaterial>>()
            .remove(handle.id());
        world.run_system_once(prune_cache::<FlatNormals>).unwrap();
        assert!(world.resource::<Cache<FlatNormals>>().0.is_empty());
    }

    #[test]
    fn conversion_reuses_updates_and_restores_base_material() {
        let (mut world, entity, base) = setup();
        attach(&mut world, entity);
        let first = world
            .get::<MeshMaterial3d<FlatMaterial>>(entity)
            .unwrap()
            .0
            .clone();
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_none()
        );
        attach(&mut world, entity);
        assert_eq!(
            world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0,
            first
        );
        world
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&base)
            .unwrap()
            .perceptual_roughness = 0.17;
        attach(&mut world, entity);
        let updated = &world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0;
        assert_ne!(*updated, first);
        assert_eq!(
            world
                .resource::<Assets<FlatMaterial>>()
                .get(updated)
                .unwrap()
                .base
                .perceptual_roughness,
            0.17
        );
        clear(&mut world, entity);
        assert!(world.get::<MeshMaterial3d<FlatMaterial>>(entity).is_none());
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .unwrap()
                .0,
            base
        );
        clear(&mut world, entity);
    }

    #[test]
    fn conversion_updates_culling_when_reflected_fields_are_unchanged() {
        use bevy::render::render_resource::Face;
        let (mut world, entity, base) = setup();
        for cull_mode in [Some(Face::Back), None, Some(Face::Front)] {
            world
                .resource_mut::<Assets<StandardMaterial>>()
                .get_mut(&base)
                .unwrap()
                .cull_mode = cull_mode;
            attach(&mut world, entity);
            let handle = &world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0;
            assert_eq!(
                world
                    .resource::<Assets<FlatMaterial>>()
                    .get(handle)
                    .unwrap()
                    .base
                    .cull_mode,
                cull_mode
            );
        }
    }

    #[test]
    fn conversion_shares_base_material_and_rejects_mutated_or_removed_assets() {
        let (mut world, first, base) = setup();
        attach(&mut world, first);
        let initial = world
            .get::<MeshMaterial3d<FlatMaterial>>(first)
            .unwrap()
            .0
            .clone();
        let second = world.spawn(MeshMaterial3d(base.clone())).id();
        attach(&mut world, second);
        assert_eq!(
            world.get::<MeshMaterial3d<FlatMaterial>>(second).unwrap().0,
            initial
        );
        assert_eq!(world.resource::<Assets<FlatMaterial>>().len(), 1);
        world
            .resource_mut::<Assets<FlatMaterial>>()
            .get_mut(&initial)
            .unwrap()
            .base
            .cull_mode = None;
        let third = world.spawn(MeshMaterial3d(base.clone())).id();
        attach(&mut world, third);
        let repaired = world
            .get::<MeshMaterial3d<FlatMaterial>>(third)
            .unwrap()
            .0
            .clone();
        assert_ne!(repaired, initial);
        assert_eq!(
            world
                .resource::<Assets<FlatMaterial>>()
                .get(&repaired)
                .unwrap()
                .base
                .cull_mode,
            Some(bevy::render::render_resource::Face::Back)
        );
        world
            .resource_mut::<Assets<FlatMaterial>>()
            .remove(repaired.id());
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
            let base = world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial::default());
            world.entity_mut(entity).insert(MeshMaterial3d(base));
            world.entity_mut(entity).remove::<Owned<FlatNormals>>();
            attach(&mut world, entity);
            assert!(world.resource::<Cache<FlatNormals>>().0.len() <= 1024);
        }
        assert_eq!(world.resource::<Cache<FlatNormals>>().0.len(), 1);
    }

    #[test]
    fn independent_gpu_morph_instances_share_flat_material_assets() {
        use crate::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource};
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/morph_animation.usda"
        );
        let source = UsdSource::new(path, std::fs::read(path).unwrap()).unwrap();
        for cached in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                UsdPlugin,
                UsdAssetPlugin,
                super::super::gpu_skin::UsdGpuSkinningPlugin,
            ));
            app.init_asset::<Mesh>()
                .init_asset::<StandardMaterial>()
                .init_asset::<Image>()
                .insert_resource(crate::UsdProjectionBudget(std::time::Duration::MAX));
            if !cached {
                app.world_mut().remove_resource::<Cache<FlatNormals>>();
            }
            let scene = app
                .world_mut()
                .resource_mut::<Assets<UsdScene>>()
                .add(UsdScene {
                    source: source.clone(),
                    textures: default(),
                });
            let roots: Vec<_> = (0..8)
                .map(|_| app.world_mut().spawn(UsdSceneRoot(scene.clone())).id())
                .collect();
            app.update();
            for root in roots {
                assert_eq!(
                    app.world().get::<crate::UsdSceneState>(root),
                    Some(&crate::UsdSceneState::Ready)
                );
            }
            let handles: Vec<_> = app
                .world_mut()
                .query::<&MeshMaterial3d<FlatMaterial>>()
                .iter(app.world())
                .map(|material| material.0.id())
                .collect();
            assert_eq!(handles.len(), 8);
            let unique: std::collections::HashSet<_> = handles.into_iter().collect();
            assert_eq!(unique.len(), if cached { 1 } else { 8 });
            assert_eq!(
                app.world().resource::<Assets<FlatMaterial>>().len(),
                unique.len()
            );
        }
    }

    #[test]
    fn cleanup_preserves_external_replacements() {
        let (mut world, entity, _) = setup();
        attach(&mut world, entity);
        let replacement = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        world
            .entity_mut(entity)
            .insert(MeshMaterial3d(replacement.clone()));
        clear(&mut world, entity);
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .unwrap()
                .0,
            replacement
        );
        attach(&mut world, entity);
        let external = world
            .resource_mut::<Assets<FlatMaterial>>()
            .add(FlatMaterial::default());
        world
            .entity_mut(entity)
            .insert(MeshMaterial3d(external.clone()));
        clear(&mut world, entity);
        assert_eq!(
            world.get::<MeshMaterial3d<FlatMaterial>>(entity).unwrap().0,
            external
        );
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_none()
        );
    }
}
