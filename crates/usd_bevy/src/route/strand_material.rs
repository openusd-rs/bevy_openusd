//! Pixel coverage for curve strands drawn as lines.
//!
//! A strand narrower than a pixel covers only part of the pixels its line
//! touches, so each pixel sample keeps it with a probability equal to its
//! projected width. Dense, fine strands (needles, hair) then weigh against
//! coarse ones (twigs) by area, and the depth test still keeps the nearest
//! strand in front, as in a ray-traced render.

use bevy::{
    mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat},
    pbr::{MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// A strand's authored width, in its mesh's local units, at each line vertex.
pub const ATTRIBUTE_STRAND_WIDTH: MeshVertexAttribute =
    MeshVertexAttribute::new("UsdStrandWidth", 0x5553_4457, VertexFormat::Float32);

const SHADER: &str = "embedded://usd_bevy/route/strand_material.wgsl";

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct StrandCoverage {}

impl MaterialExtension for StrandCoverage {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode() -> Option<AlphaMode> {
        Some(AlphaMode::Opaque)
    }
    // A depth prepass would draw strands at full coverage and hide those behind.
    fn enable_prepass() -> bool {
        false
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Prepass and shadow pipelines keep the standard vertex shader.
        if descriptor
            .vertex
            .shader_defs
            .contains(&"PREPASS_PIPELINE".into())
        {
            return Ok(());
        }
        let mut attributes = vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)];
        if layout.0.contains(Mesh::ATTRIBUTE_NORMAL) {
            attributes.push(Mesh::ATTRIBUTE_NORMAL.at_shader_location(1));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(5));
        }
        attributes.push(ATTRIBUTE_STRAND_WIDTH.at_shader_location(8));
        descriptor.vertex.buffers = vec![layout.0.get_layout(&attributes)?];
        Ok(())
    }
}

pub type StrandMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, StrandCoverage>;

pub(crate) fn configure(app: &mut App) {
    if super::wrapped::configure::<StrandCoverage>(app) {
        bevy::asset::embedded_asset!(app, "strand_material.wgsl");
    }
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::wrapped::clear::<StrandCoverage>(world, entity);
}

/// Wraps `entity`'s material in strand coverage when its mesh is lit lines
/// that carry strand widths.
pub(crate) fn attach_if_strand(world: &mut World, entity: Entity) {
    let strand = world
        .get::<Mesh3d>(entity)
        .and_then(|mesh| world.get_resource::<Assets<Mesh>>()?.get(&mesh.0))
        .is_some_and(|mesh| {
            mesh.primitive_topology() == bevy::mesh::PrimitiveTopology::LineList
                && mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some()
                && mesh.attribute(ATTRIBUTE_STRAND_WIDTH).is_some()
        });
    if strand {
        super::wrapped::attach::<StrandCoverage>(world, entity);
    } else {
        clear(world, entity);
    }
}
