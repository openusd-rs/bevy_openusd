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

pub(crate) fn configure(app: &mut App) {
    bevy::asset::embedded_asset!(app, "flat_material.wgsl");
    bevy::asset::embedded_asset!(app, "flat_prepass.wgsl");
    bevy::asset::embedded_asset!(app, "flat_functions.wgsl");
    app.add_plugins(MaterialPlugin::<FlatMaterial>::default());
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
    let handle = existing.unwrap_or_else(|| world.resource_mut::<Assets<FlatMaterial>>()
        .add(FlatMaterial { base: material, extension: FlatNormals {} }));
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
        let base = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(base.clone())).id();
        (world, entity, base)
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
