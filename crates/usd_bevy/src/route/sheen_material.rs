//! Disney's sheen, which a standard material has no lobe for.
//!
//! Sand, dust and cloth glow toward grazing light: sunlight raking across a
//! beach behind it lights the sand far brighter than its color alone would.
//! A material with sheen draws through this extension, which adds that
//! glow from each directional light to the standard lighting.

use bevy::{
    pbr::MaterialExtension, prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef,
};

/// The sheen's weight in `x` and its tint toward the base hue in `y`.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default, PartialEq)]
pub struct SheenLobe {
    #[uniform(100)]
    pub sheen: Vec4,
}

impl MaterialExtension for SheenLobe {
    fn fragment_shader() -> ShaderRef {
        "embedded://usd_bevy/route/sheen_material.wgsl".into()
    }
}

pub type SheenMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, SheenLobe>;

pub(crate) fn configure(app: &mut App) {
    if super::wrapped::configure::<SheenLobe>(app) {
        bevy::asset::embedded_asset!(app, "sheen_material.wgsl");
        bevy::asset::embedded_asset!(app, "sheen_functions.wgsl");
    }
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::wrapped::clear::<SheenLobe>(world, entity);
}

/// Wraps `entity`'s material in the sheen lobe when it has sheen and no
/// other extension has wrapped it.
pub(crate) fn attach_if_sheen(world: &mut World, entity: Entity) {
    let sheen = world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .and_then(|material| super::cache::sheen_of(world, material.0.id()));
    if let Some(sheen) = sheen {
        super::wrapped::attach_with(
            world,
            entity,
            SheenLobe {
                sheen: Vec4::new(sheen.weight, sheen.tint, 0.0, 0.0),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheen_keeps_otherwise_equal_materials_apart_and_draws_its_lobe() {
        let stage = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0
def Mesh "Sand" (prepend apiSchemas = ["MaterialBindingAPI"]) {
    point3f[] points = [(0,0,0), (1,0,0), (0,0,1)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    rel material:binding = </Sand>
}
def Mesh "Soil" (prepend apiSchemas = ["MaterialBindingAPI"]) {
    point3f[] points = [(0,0,0), (1,0,0), (0,0,1)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    rel material:binding = </Soil>
}
def Material "Sand" {
    token outputs:ri:surface.connect = </Sand/Disney.outputs:bxdf_out>
    def Shader "Disney" {
        uniform token info:id = "PxrDisneyBsdf"
        color3f inputs:baseColor = (0.6, 0.5, 0.4)
        float inputs:sheen = 0.5
        float inputs:sheenTint = 1
        token outputs:bxdf_out
    }
}
def Material "Soil" {
    token outputs:ri:surface.connect = </Soil/Disney.outputs:bxdf_out>
    def Shader "Disney" {
        uniform token info:id = "PxrDisneyBsdf"
        color3f inputs:baseColor = (0.6, 0.5, 0.4)
        token outputs:bxdf_out
    }
}
"#,
        )
        .open_stage()
        .unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<SheenMaterial>>();
        world.init_resource::<super::super::cache::MaterialCache>();
        let live = crate::live::LiveStage::new(stage);
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let sand = map.entity("/Sand").unwrap();
        let soil = map.entity("/Soil").unwrap();
        let lobe = &world.get::<MeshMaterial3d<SheenMaterial>>(sand).unwrap().0;
        let lobe = world.resource::<Assets<SheenMaterial>>().get(lobe).unwrap();
        assert_eq!(lobe.extension.sheen, Vec4::new(0.5, 1.0, 0.0, 0.0));
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(sand)
                .is_none()
        );
        let plain = &world
            .get::<MeshMaterial3d<StandardMaterial>>(soil)
            .unwrap()
            .0;
        assert!(super::super::cache::sheen_of(&world, plain.id()).is_none());
        assert_eq!(
            super::super::wrapped::base_handle(&world, sand)
                .map(|base| super::super::cache::sheen_of(&world, base.id())),
            Some(Some(super::super::cache::Sheen {
                weight: 0.5,
                tint: 1.0
            }))
        );
    }
}
