//! Geometric fragment normals for GPU-deformed polygonal meshes.

use bevy::{
    pbr::MaterialExtension, prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef,
};

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct FlatNormals {}

impl MaterialExtension for FlatNormals {
    fn fragment_shader() -> ShaderRef {
        "embedded://usd_bevy/route/flat_material.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        Self::fragment_shader()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://usd_bevy/route/flat_prepass.wgsl".into()
    }
}

pub type FlatMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, FlatNormals>;

pub(crate) fn configure(app: &mut App) {
    if !super::wrapped::configure::<FlatNormals>(app) {
        return;
    }
    bevy::asset::embedded_asset!(app, "flat_material.wgsl");
    bevy::asset::embedded_asset!(app, "flat_prepass.wgsl");
    bevy::asset::embedded_asset!(app, "flat_functions.wgsl");
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::wrapped::clear::<FlatNormals>(world, entity);
}

pub(crate) fn attach(world: &mut World, entity: Entity) {
    super::wrapped::attach::<FlatNormals>(world, entity);
}

/// Whether the geometric-normal material is available to draw meshes that
/// carry no normals.
pub(crate) fn enabled(world: &World) -> bool {
    super::wrapped::enabled::<FlatNormals>(world)
}

/// Wraps `entity`'s material in the geometric-normal material when its mesh
/// carries no normals.
pub(crate) fn attach_if_normalless(world: &mut World, entity: Entity) {
    let normalless = world
        .get::<Mesh3d>(entity)
        .and_then(|mesh| world.get_resource::<Assets<Mesh>>()?.get(&mesh.0))
        .is_some_and(|mesh| mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_none());
    if normalless {
        attach(world, entity);
    }
}
