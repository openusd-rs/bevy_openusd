//! Curve strands drawn as ribbons at their true width.
//!
//! Each centerline sample carries a pair of vertices that the vertex shader
//! spreads across the view, or across the authored normal, by the strand's
//! width. A strand narrower than a pixel is drawn one pixel wide, and each
//! pixel sample keeps it with a probability equal to its projected width.
//! Dense, fine strands (needles, hair) then weigh against coarse ones
//! (twigs) by area, and the depth test still keeps the nearest strand in
//! front, as in a ray-traced render.

use bevy::{
    mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat},
    pbr::{MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// A strand's authored width, in its mesh's local units.
pub const ATTRIBUTE_STRAND_WIDTH: MeshVertexAttribute =
    MeshVertexAttribute::new("UsdStrandWidth", 0x5553_4457, VertexFormat::Float32);

/// The centerline tangent, with the side of the ribbon (-1 or 1) in `w`.
pub const ATTRIBUTE_STRAND_TANGENT: MeshVertexAttribute =
    MeshVertexAttribute::new("UsdStrandTangent", 0x5553_4458, VertexFormat::Snorm8x4);

/// An authored normal, which both shades and orients the ribbon.
pub const ATTRIBUTE_STRAND_NORMAL: MeshVertexAttribute =
    MeshVertexAttribute::new("UsdStrandNormal", 0x5553_4459, VertexFormat::Snorm8x4);

const SHADER: &str = "embedded://usd_bevy/route/strand_material.wgsl";
const PREPASS_SHADER: &str = "embedded://usd_bevy/route/strand_prepass.wgsl";

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct StrandCoverage {}

impl MaterialExtension for StrandCoverage {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER.into()
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
        let mut attributes = vec![
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_STRAND_WIDTH.at_shader_location(8),
            ATTRIBUTE_STRAND_TANGENT.at_shader_location(9),
        ];
        if layout.0.contains(ATTRIBUTE_STRAND_NORMAL) {
            attributes.push(ATTRIBUTE_STRAND_NORMAL.at_shader_location(10));
            descriptor.vertex.shader_defs.push("STRAND_NORMALS".into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push("STRAND_NORMALS".into());
            }
        }
        let prepass = descriptor
            .vertex
            .shader_defs
            .contains(&"PREPASS_PIPELINE".into());
        if !prepass && layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(5));
        }
        if !prepass && layout.0.contains(Mesh::ATTRIBUTE_UV_0) {
            attributes.push(Mesh::ATTRIBUTE_UV_0.at_shader_location(2));
        }
        descriptor.vertex.buffers = vec![layout.0.get_layout(&attributes)?];
        // Ribbons oriented by authored normals show both faces.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub type StrandMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, StrandCoverage>;

pub(crate) fn configure(app: &mut App) {
    if super::wrapped::configure::<StrandCoverage>(app) {
        bevy::asset::embedded_asset!(app, "strand_material.wgsl");
        bevy::asset::embedded_asset!(app, "strand_prepass.wgsl");
        bevy::asset::embedded_asset!(app, "strand_functions.wgsl");
    }
}

pub(crate) fn clear(world: &mut World, entity: Entity) {
    super::wrapped::clear::<StrandCoverage>(world, entity);
}

/// Whether `mesh` is strand ribbons, which only the strand material can widen.
pub(crate) fn is_strand(mesh: &Mesh) -> bool {
    mesh.attribute(ATTRIBUTE_STRAND_TANGENT).is_some()
}

/// Wraps `entity`'s material in the strand material when its mesh is strand
/// ribbons.
pub(crate) fn attach_if_strand(world: &mut World, entity: Entity) {
    let strand = world
        .get::<Mesh3d>(entity)
        .and_then(|mesh| world.get_resource::<Assets<Mesh>>()?.get(&mesh.0))
        .is_some_and(is_strand);
    if strand {
        super::wrapped::attach::<StrandCoverage>(world, entity);
    } else {
        clear(world, entity);
    }
}
