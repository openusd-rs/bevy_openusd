//! Media that take on their color over the depth of water behind them.
//!
//! A refracting surface over a murky medium, such as a lagoon, lets the floor
//! show through where it is shallow and turns to the medium's color where it
//! is deep. With a depth prepass on the camera, each fragment measures how far
//! its view ray travels to the scene behind and absorbs the transmitted light
//! over that distance; without one, it assumes twice the medium's attenuation
//! distance.

use bevy::{
    pbr::MaterialExtension, prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef,
};

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct DepthMedium {}

impl MaterialExtension for DepthMedium {
    fn fragment_shader() -> ShaderRef {
        "embedded://usd_bevy/route/medium_material.wgsl".into()
    }
}

pub type MediumMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, DepthMedium>;

pub(crate) fn configure(app: &mut App) {
    if super::wrapped::configure::<DepthMedium>(app) {
        bevy::asset::embedded_asset!(app, "medium_material.wgsl");
    }
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::wrapped::clear::<DepthMedium>(world, entity);
}

/// Wraps `entity`'s material in the depth medium when it refracts into a
/// medium with a finite attenuation distance.
pub(crate) fn attach_if_medium(world: &mut World, entity: Entity) {
    let medium = world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .and_then(|material| {
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&material.0)
        })
        .is_some_and(|material| {
            material.specular_transmission > 0.0 && material.attenuation_distance.is_finite()
        });
    if medium {
        super::wrapped::attach::<DepthMedium>(world, entity);
    } else {
        clear(world, entity);
    }
}
