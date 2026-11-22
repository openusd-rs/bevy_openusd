//! Dense Geometry Format (DGF1) integration for OpenUSD [`ReadMesh`] and Bevy [`Mesh`] assets.
//!
//! Provides:
//! - [`DgfMesh::from_read_mesh`]: encodes shared-point USD topology into 128-byte DGF1 blocks
//!   before face-varying attribute expansion (~6 B/tri, ~3x compression vs raw indexed geometry).
//! - [`DgfMesh::from_bevy_mesh`] / [`DgfMesh::to_bevy_mesh`]: encodes/decodes any static
//!   triangle-list Bevy [`Mesh`] while remapping all `VertexAttributeValues` via `dgf::remap_vertex_attributes`.
//! - [`UsdDgfConfig`] / [`UsdDgfStats`]: opt-in Bevy resources that route static meshes built
//!   during stage projection through shared-point DGF1 compression and collect live metrics.

use std::collections::HashSet;
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use dgf::{
    DecodeLimits, DegeneratePolicy, EncodeOptions, EncodedMesh, MeshView, PackingPolicy,
    QuantizationMode, decode_mesh, encode_mesh, reconstruct_with_remap, remap_vertex_attributes,
};

use crate::read::geom::{Orientation, ReadMesh};

/// Re-export of the core `dgf` crate and `openusd-dgf` adapter for downstream consumers.
pub use ::dgf as dgf_codec;
pub use ::openusd_dgf as openusd_adapter;

/// Opt-in Bevy resource enabling DGF1 geometry compression during USD stage projection.
#[derive(Resource, Debug, Clone)]
pub struct UsdDgfConfig {
    /// DGF1 encoding options (quantization mode, packing policy, degenerate handling, etc.).
    pub options: EncodeOptions,
}

impl Default for UsdDgfConfig {
    fn default() -> Self {
        Self {
            options: default_encode_options(),
        }
    }
}

/// Default DGF1 encoding options tuned for USD static scene geometry.
///
/// Uses `TargetBitWidth(16)` so both millimeter-scale props and meter/centimeter-scale
/// architecture fit within DGF1's 16-bit per-block offset span without manual per-mesh tuning.
pub fn default_encode_options() -> EncodeOptions {
    EncodeOptions {
        quantization: QuantizationMode::TargetBitWidth(16),
        packing: PackingPolicy::Sah,
        degenerate_policy: DegeneratePolicy::DropAndReport,
        ..EncodeOptions::default()
    }
}

/// Cumulative metrics collected when [`UsdDgfConfig`] is active in a Bevy [`World`].
#[derive(Resource, Debug, Default, Clone)]
pub struct UsdDgfStats {
    /// Number of meshes successfully encoded through DGF1.
    pub meshes_encoded: usize,
    /// Number of meshes skipped (animated/skinned/morph/non-triangle or unrepresentable).
    pub meshes_skipped: usize,
    /// Total input triangles submitted to DGF1.
    pub input_triangles: usize,
    /// Total retained triangles emitted in DGF1 blocks.
    pub retained_triangles: usize,
    /// Total 128-byte DGF1 blocks produced.
    pub output_blocks: usize,
    /// Total uncompressed position + index byte size before DGF1.
    pub uncompressed_geom_bytes: usize,
    /// Total DGF1 block payload bytes (`output_blocks * 128`).
    pub dgf_block_bytes: usize,
    /// Worst measured coordinate quantization error across all encoded meshes.
    pub max_coord_error: f64,
    /// Cumulative encoding time in milliseconds.
    pub encode_ms: f64,
    /// Cumulative decoding time in milliseconds.
    pub decode_ms: f64,
}

/// Errors returned when converting between OpenUSD/Bevy meshes and [`DgfMesh`].
#[derive(Debug)]
pub enum DgfMeshError {
    /// Mesh primitive topology is not [`PrimitiveTopology::TriangleList`].
    UnsupportedTopology(PrimitiveTopology),
    /// Mesh has no `Float32x3` position attribute.
    MissingPositions,
    /// Mesh uses morph targets or skeletal skinning weights (outside static DGF1 scope).
    DynamicMeshUnsupported,
    /// Vertex or index count was not divisible by 3 or exceeded `u32::MAX`.
    InvalidTriangleBuffer,
    /// DGF1 encoding failed.
    Encode(dgf::EncodeError),
    /// DGF1 decoding failed.
    Decode(dgf::StreamDecodeError),
    /// Sideband attribute or mesh remapping failed.
    Remap(dgf::RemapError),
}

impl std::fmt::Display for DgfMeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedTopology(t) => write!(f, "unsupported topology for DGF1: {t:?}"),
            Self::MissingPositions => f.write_str("mesh has no Float32x3 ATTRIBUTE_POSITION"),
            Self::DynamicMeshUnsupported => {
                f.write_str("morph targets and skinned meshes are excluded from static DGF1")
            }
            Self::InvalidTriangleBuffer => f.write_str("triangle index buffer length is invalid"),
            Self::Encode(e) => write!(f, "DGF1 encode error: {e}"),
            Self::Decode(e) => write!(f, "DGF1 decode error: {e}"),
            Self::Remap(e) => write!(f, "DGF1 remap error: {e}"),
        }
    }
}

impl std::error::Error for DgfMeshError {}

/// A mesh stored as compact 128-byte DGF1 blocks plus optional remapped Bevy vertex attributes.
#[derive(Debug, Clone)]
pub struct DgfMesh {
    /// Encoded DGF1 blocks, remaps, quantization report, and packing statistics.
    pub encoded: EncodedMesh,
    /// Non-position Bevy vertex attributes already remapped into concatenated DGF block-vertex order.
    pub attributes: Vec<(MeshVertexAttribute, VertexAttributeValues)>,
    /// Original Bevy [`RenderAssetUsages`].
    pub asset_usage: RenderAssetUsages,
    /// Original raytracing flag.
    pub enable_raytracing: bool,
    /// Uncompressed position + index bytes of the source mesh.
    pub uncompressed_geom_bytes: usize,
}

impl DgfMesh {
    /// Encodes a [`ReadMesh`] directly on its shared USD `points` before face-varying expansion,
    /// achieving full DGF1 strip reuse (~6 B/tri, ~3x compression vs raw indexed geometry).
    pub fn from_read_mesh(read: &ReadMesh, options: &EncodeOptions) -> Result<Self, DgfMeshError> {
        let triangles = shared_point_triangles(read);
        let uncompressed_geom_bytes = read.points.len() * std::mem::size_of::<[f32; 3]>()
            + triangles.len() * std::mem::size_of::<[u32; 3]>();

        let encoded = encode_mesh(
            MeshView {
                positions: &read.points,
                triangles: &triangles,
                triangle_metadata: None,
                triangle_omm_ids: None,
            },
            options,
        )
        .map_err(DgfMeshError::Encode)?;

        Ok(Self {
            encoded,
            attributes: Vec::new(),
            asset_usage: RenderAssetUsages::default(),
            enable_raytracing: true,
            uncompressed_geom_bytes,
        })
    }

    /// Encodes a static triangle-list Bevy [`Mesh`] into DGF1 blocks and remaps all sideband attributes.
    pub fn from_bevy_mesh(mesh: &Mesh, options: &EncodeOptions) -> Result<Self, DgfMeshError> {
        if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
            return Err(DgfMeshError::UnsupportedTopology(mesh.primitive_topology()));
        }
        if mesh.get_morph_targets().is_some()
            || mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some()
            || mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some()
        {
            return Err(DgfMeshError::DynamicMeshUnsupported);
        }

        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return Err(DgfMeshError::MissingPositions);
        };

        let (triangles, index_bytes): (Vec<[u32; 3]>, usize) = match mesh.indices() {
            Some(Indices::U32(idx)) => {
                if !idx.len().is_multiple_of(3) {
                    return Err(DgfMeshError::InvalidTriangleBuffer);
                }
                let tris = idx.as_chunks::<3>().0.to_vec();
                (tris, idx.len() * std::mem::size_of::<u32>())
            }
            Some(Indices::U16(idx)) => {
                if !idx.len().is_multiple_of(3) {
                    return Err(DgfMeshError::InvalidTriangleBuffer);
                }
                let tris = idx
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|&[a, b, c]| [u32::from(a), u32::from(b), u32::from(c)])
                    .collect();
                (tris, idx.len() * std::mem::size_of::<u16>())
            }
            None => {
                if !positions.len().is_multiple_of(3) || positions.len() > (u32::MAX as usize) {
                    return Err(DgfMeshError::InvalidTriangleBuffer);
                }
                let tris = (0..positions.len() as u32)
                    .step_by(3)
                    .map(|i| [i, i + 1, i + 2])
                    .collect();
                (tris, 0)
            }
        };

        let uncompressed_geom_bytes =
            positions.len() * std::mem::size_of::<[f32; 3]>() + index_bytes;

        let encoded = encode_mesh(
            MeshView {
                positions,
                triangles: &triangles,
                triangle_metadata: None,
                triangle_omm_ids: None,
            },
            options,
        )
        .map_err(DgfMeshError::Encode)?;

        let mut attributes = Vec::new();
        for (attr, values) in mesh.attributes() {
            if attr.id == Mesh::ATTRIBUTE_POSITION.id {
                continue;
            }
            let remapped = remap_attribute_values(values, &encoded.vertex_remap)
                .map_err(DgfMeshError::Remap)?;
            attributes.push((*attr, remapped));
        }

        Ok(Self {
            encoded,
            attributes,
            asset_usage: mesh.asset_usage,
            enable_raytracing: mesh.enable_raytracing,
            uncompressed_geom_bytes,
        })
    }

    /// Decodes the DGF1 blocks and reattaches all remapped sideband attributes into a Bevy [`Mesh`].
    pub fn to_bevy_mesh(&self) -> Result<Mesh, DgfMeshError> {
        let raw = self.encoded.to_bytes();
        let decoded = decode_mesh(&raw, &DecodeLimits::default()).map_err(DgfMeshError::Decode)?;

        let mut out = Mesh::new(PrimitiveTopology::TriangleList, self.asset_usage);
        out.enable_raytracing = self.enable_raytracing;
        out.insert_attribute(Mesh::ATTRIBUTE_POSITION, decoded.positions);
        for (attr, values) in &self.attributes {
            out.insert_attribute(*attr, values.clone());
        }
        let flat_indices: Vec<u32> = decoded
            .triangles
            .into_iter()
            .flat_map(|t| t.into_iter())
            .collect();
        out.insert_indices(Indices::U32(flat_indices));
        Ok(out)
    }

    /// Decodes DGF1 blocks back into globally indexed shared USD point positions (verifying
    /// bit-for-bit shared-vertex consistency across block boundaries).
    pub fn decode_shared_points(
        &self,
        source_points: &[[f32; 3]],
    ) -> Result<Vec<[f32; 3]>, DgfMeshError> {
        let raw = self.encoded.to_bytes();
        let decoded = decode_mesh(&raw, &DecodeLimits::default()).map_err(DgfMeshError::Decode)?;
        let reconstructed = reconstruct_with_remap(
            &decoded,
            &self.encoded.vertex_remap,
            &self.encoded.triangle_remap,
            source_points.len(),
        )
        .map_err(DgfMeshError::Remap)?;
        Ok(reconstructed
            .positions
            .into_iter()
            .zip(source_points.iter())
            .map(|(opt, &fallback)| opt.unwrap_or(fallback))
            .collect())
    }

    /// Returns the raw DGF1 block payload size in bytes (`128 * blocks.len()`).
    pub fn dgf_bytes(&self) -> usize {
        self.encoded.blocks.len() * dgf::BLOCK_BYTES
    }

    /// Builds tightly packed GPU upload buffers for `dgf::WGSL_DECODER_SOURCE`.
    pub fn to_gpu_batch(&self, add_vertex_offset: bool) -> Result<GpuDgfBatch, dgf::DecodeError> {
        GpuDgfBatch::from_blocks(&self.encoded.blocks, add_vertex_offset)
    }
}

/// GPU-side 128-byte DGF1 block matching `struct DgfBlock` in [`dgf::WGSL_DECODER_SOURCE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuDgfBlock {
    /// 32 little-endian 32-bit words of the raw 128-byte DGF1 block.
    pub words: [u32; 32],
}

impl GpuDgfBlock {
    /// Converts a validated or encoded 128-byte [`dgf::Block`] into its GPU word layout.
    pub fn from_block(block: &dgf::Block) -> Self {
        Self::from_bytes(block.as_bytes())
    }

    /// Converts raw 128-byte DGF1 block bytes into little-endian `u32` words.
    pub fn from_bytes(bytes: &[u8; dgf::BLOCK_BYTES]) -> Self {
        let mut words = [0u32; 32];
        for (dst, chunk) in words.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
            *dst = u32::from_le_bytes(*chunk);
        }
        Self { words }
    }
}

/// Per-block GPU dispatch metadata matching `struct DgfBlockDispatch` in [`dgf::WGSL_DECODER_SOURCE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuDgfBlockDispatch {
    /// Starting vertex index written into `out_vertices` for this block.
    pub vertex_offset: u32,
    /// Starting triangle index written into `out_triangles` for this block.
    pub triangle_offset: u32,
    /// Nonzero to add `vertex_offset` to each decoded triangle index (global mesh indexing).
    pub add_vertex_offset: u32,
    /// Reserved padding (0).
    pub _pad: u32,
}

/// Decoded GPU vertex matching `struct DgfDecodedVertex` in [`dgf::WGSL_DECODER_SOURCE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuDgfDecodedVertex {
    /// Decoded `f32` vertex position `[x, y, z]`.
    pub pos: [f32; 3],
    /// Reserved padding (0).
    pub _pad: u32,
}

/// Decoded GPU triangle matching `struct DgfDecodedTriangle` in [`dgf::WGSL_DECODER_SOURCE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuDgfDecodedTriangle {
    /// Decoded triangle vertex indices (block-local or mesh-global depending on `add_vertex_offset`).
    pub indices: [u32; 3],
    /// Decoded 24-bit geometry ID.
    pub geometry_id: u32,
    /// Decoded 29-bit primitive ID (`prim_id_base + local_triangle_index`).
    pub prim_id: u32,
    /// Packed flags: bit 0 = opaque, bits 1..=3 = OMM descriptor selector, bit 4 = has OMM.
    pub flags: u32,
}

impl GpuDgfDecodedTriangle {
    /// Returns the decoded triangle opaque flag.
    pub const fn opaque(self) -> bool {
        (self.flags & 1) != 0
    }

    /// Returns the decoded per-triangle OMM descriptor index if the block carries OMM data.
    pub const fn omm_descriptor(self) -> Option<u8> {
        if (self.flags & 16) != 0 {
            Some(((self.flags >> 1) & 7) as u8)
        } else {
            None
        }
    }
}

/// Decoded per-block header summary matching `struct DgfBlockInfo` in [`dgf::WGSL_DECODER_SOURCE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuDgfBlockInfo {
    /// Active vertex count (`3..=64`).
    pub num_vertices: u32,
    /// Active triangle count (`1..=64`).
    pub num_triangles: u32,
    /// Optional 4-byte user data (0 when absent).
    pub user_data: u32,
    /// Packed flags: bit 0 = have_user_data, bits 1..=3 = omm_descriptor_count,
    /// bits 4..=7 = bits_per_index, bits 8..=15 = exponent.
    pub flags: u32,
}

impl GpuDgfBlockInfo {
    /// Returns `Some(user_data)` if `have_user_data` was set in the block header.
    pub const fn user_data_opt(self) -> Option<u32> {
        if (self.flags & 1) != 0 {
            Some(self.user_data)
        } else {
            None
        }
    }

    /// Returns the OMM descriptor count (`0..=7`).
    pub const fn omm_descriptor_count(self) -> u8 {
        ((self.flags >> 1) & 7) as u8
    }

    /// Returns `bits_per_index` (`3..=6`).
    pub const fn bits_per_index(self) -> u8 {
        ((self.flags >> 4) & 15) as u8
    }

    /// Returns the raw biased exponent (`1..=232`).
    pub const fn exponent(self) -> u8 {
        ((self.flags >> 8) & 255) as u8
    }
}

/// Ready-to-upload batch of DGF1 blocks and prefix-summed dispatch offsets for GPU decoding.
#[derive(Debug, Clone)]
pub struct GpuDgfBatch {
    /// Raw 128-byte blocks packed as `GpuDgfBlock`.
    pub blocks: Vec<GpuDgfBlock>,
    /// Per-block output vertex/triangle offsets.
    pub dispatch: Vec<GpuDgfBlockDispatch>,
    /// Total output vertex slots required in `out_vertices`.
    pub total_vertices: usize,
    /// Total output triangle slots required in `out_triangles`.
    pub total_triangles: usize,
}

impl GpuDgfBatch {
    /// Builds a tightly packed GPU decode batch from a slice of [`dgf::Block`]s.
    pub fn from_blocks(
        blocks: &[dgf::Block],
        add_vertex_offset: bool,
    ) -> Result<Self, dgf::DecodeError> {
        let mut gpu_blocks = Vec::with_capacity(blocks.len());
        let mut dispatch = Vec::with_capacity(blocks.len());
        let mut v_off = 0u32;
        let mut t_off = 0u32;
        let add_flag = u32::from(add_vertex_offset);

        for blk in blocks {
            let hdr = dgf::Header::parse(blk.as_bytes())?;
            gpu_blocks.push(GpuDgfBlock::from_block(blk));
            dispatch.push(GpuDgfBlockDispatch {
                vertex_offset: v_off,
                triangle_offset: t_off,
                add_vertex_offset: add_flag,
                _pad: 0,
            });
            v_off = v_off.checked_add(u32::from(hdr.num_vertices)).ok_or(
                dgf::DecodeError::ArithmeticOverflow {
                    context: "GPU batch vertex_offset",
                },
            )?;
            t_off = t_off.checked_add(u32::from(hdr.num_triangles)).ok_or(
                dgf::DecodeError::ArithmeticOverflow {
                    context: "GPU batch triangle_offset",
                },
            )?;
        }

        Ok(Self {
            blocks: gpu_blocks,
            dispatch,
            total_vertices: v_off as usize,
            total_triangles: t_off as usize,
        })
    }
}

/// Fan-triangulates `ReadMesh` on its shared USD `points` (skipping hole faces and honoring
/// `Orientation::LeftHanded`) so shared vertex IDs stay intact for DGF1 strip continuations.
pub fn shared_point_triangles(read: &ReadMesh) -> Vec<[u32; 3]> {
    let holes: HashSet<i32> = read.hole_indices.iter().copied().collect();
    let num_points = read.points.len();
    let mut tris = Vec::new();
    let mut cursor = 0usize;

    for (face_idx, &count) in read.face_vertex_counts.iter().enumerate() {
        if count <= 0 {
            continue;
        }
        let c = count as usize;
        if cursor + c > read.face_vertex_indices.len() {
            break;
        }
        if c >= 3 && !holes.contains(&(face_idx as i32)) {
            let v0 = read.face_vertex_indices[cursor];
            if v0 >= 0 && (v0 as usize) < num_points {
                for k in 1..(c - 1) {
                    let v1 = read.face_vertex_indices[cursor + k];
                    let v2 = read.face_vertex_indices[cursor + k + 1];
                    if v1 >= 0
                        && (v1 as usize) < num_points
                        && v2 >= 0
                        && (v2 as usize) < num_points
                    {
                        let tri = match read.orientation {
                            Orientation::RightHanded => [v0 as u32, v1 as u32, v2 as u32],
                            Orientation::LeftHanded => [v0 as u32, v2 as u32, v1 as u32],
                        };
                        tris.push(tri);
                    }
                }
            }
        }
        cursor += c;
    }
    tris
}

/// If [`UsdDgfConfig`] is active in `world`, round-trips `read` through shared-point DGF1
/// compression and returns a `ReadMesh` carrying the exact DGF1-decoded quantized positions,
/// preserving all USD normals, UVs, colors, opacity, subsets, and tangent generation.
pub(crate) fn maybe_roundtrip_read(world: &mut World, read: &ReadMesh) -> Option<ReadMesh> {
    let options = world.get_resource::<UsdDgfConfig>()?.options;

    let t0 = Instant::now();
    let dgf_mesh = match DgfMesh::from_read_mesh(read, &options) {
        Ok(m) => m,
        Err(err) => {
            bevy::log::debug!(target: "usd_bevy::mesh::dgf", "skipping DGF1 for mesh: {err}");
            if let Some(mut stats) = world.get_resource_mut::<UsdDgfStats>() {
                stats.meshes_skipped += 1;
            }
            return None;
        }
    };
    let enc_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let t1 = Instant::now();
    let decoded_points = match dgf_mesh.decode_shared_points(&read.points) {
        Ok(pts) => pts,
        Err(err) => {
            bevy::log::warn!(target: "usd_bevy::mesh::dgf", "DGF1 decode failed: {err}");
            if let Some(mut stats) = world.get_resource_mut::<UsdDgfStats>() {
                stats.meshes_skipped += 1;
            }
            return None;
        }
    };
    let dec_ms = t1.elapsed().as_secs_f64() * 1000.0;

    if let Some(mut stats) = world.get_resource_mut::<UsdDgfStats>() {
        stats.meshes_encoded += 1;
        stats.input_triangles += dgf_mesh.encoded.statistics.input_triangles;
        stats.retained_triangles += dgf_mesh.encoded.statistics.retained_triangles;
        stats.output_blocks += dgf_mesh.encoded.blocks.len();
        stats.uncompressed_geom_bytes += dgf_mesh.uncompressed_geom_bytes;
        stats.dgf_block_bytes += dgf_mesh.dgf_bytes();
        if dgf_mesh.encoded.quantization.max_coord_error > stats.max_coord_error {
            stats.max_coord_error = dgf_mesh.encoded.quantization.max_coord_error;
        }
        stats.encode_ms += enc_ms;
        stats.decode_ms += dec_ms;
    }

    let mut quantized = read.clone();
    quantized.points = decoded_points;
    Some(quantized)
}

fn remap_attribute_values(
    values: &VertexAttributeValues,
    vertex_remap: &[u32],
) -> Result<VertexAttributeValues, dgf::RemapError> {
    macro_rules! remap_variants {
        ($($variant:ident),+ $(,)?) => {
            match values {
                $(
                    VertexAttributeValues::$variant(v) => Ok(VertexAttributeValues::$variant(
                        remap_vertex_attributes(v, vertex_remap)?,
                    )),
                )+
            }
        };
    }
    remap_variants!(
        Uint8,
        Uint8x2,
        Uint8x4,
        Sint8,
        Sint8x2,
        Sint8x4,
        Unorm8,
        Unorm8x2,
        Unorm8x4,
        Snorm8,
        Snorm8x2,
        Snorm8x4,
        Uint16,
        Uint16x2,
        Uint16x4,
        Sint16,
        Sint16x2,
        Sint16x4,
        Unorm16,
        Unorm16x2,
        Unorm16x4,
        Snorm16,
        Snorm16x2,
        Snorm16x4,
        Float16,
        Float16x2,
        Float16x4,
        Float32,
        Float32x2,
        Float32x3,
        Float32x4,
        Uint32,
        Uint32x2,
        Uint32x3,
        Uint32x4,
        Sint32,
        Sint32x2,
        Sint32x3,
        Sint32x4,
        Float64,
        Float64x2,
        Float64x3,
        Float64x4,
        Unorm10_10_10_2,
        Unorm8x4Bgra
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgf::{ValidationMode, validate_block_with_mode};

    #[test]
    fn stage_projection_with_dgf_enabled_preserves_geometry_and_attributes() {
        let text = r#"#usda 1.0
def Mesh "Quad" {
    uniform token subdivisionScheme = "none"
    point3f[] points = [(0,0,0), (1,0,0), (1,1,0), (0,1,0)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0, 1, 2, 3]
    texCoord2f[] primvars:st = [(0,0), (1,0), (1,1), (0,1)] (interpolation = "faceVarying")
}
"#;
        let stage = crate::snippet::UsdSnippet::new(text).open_stage().unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<UsdDgfConfig>();
        world.init_resource::<UsdDgfStats>();

        let live = crate::live::LiveStage::new(stage);
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);

        let stats = world.resource::<UsdDgfStats>();
        assert_eq!(stats.meshes_encoded, 1);
        assert_eq!(stats.retained_triangles, 2);
        assert_eq!(stats.output_blocks, 1);
        assert!(stats.max_coord_error <= 1e-4);

        let entity = map.entity("/Quad").unwrap();
        let handle = &world.get::<Mesh3d>(entity).unwrap().0;
        let mesh = world.resource::<Assets<Mesh>>().get(handle).unwrap();
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
        assert_eq!(mesh.indices().unwrap().len(), 6);
    }

    #[test]
    fn dgf_mesh_blocks_pass_asset_validation() {
        let stage = openusd::usd::Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/material_subsets.usda"
            ))
            .unwrap();
        let cache = openusd_dgf::UsdDgfCache::new();
        let opts = openusd_dgf::UsdBakeOptions {
            extract: openusd_dgf::UsdExtractOptions {
                require_subdivision_none: false,
                ..Default::default()
            },
            encode: default_encode_options(),
        };
        let mut visited = 0usize;
        for name in stage.root_prims().unwrap() {
            let root = stage.prim(format!("/{name}")).unwrap();
            let mut stack = vec![root];
            while let Some(prim) = stack.pop() {
                if let Some(read) = crate::read::geom::read_mesh(&stage, prim.path()).unwrap() {
                    let shared =
                        DgfMesh::from_read_mesh(&read, &opts.encode).expect("encode shared DGF");
                    for blk in &shared.encoded.blocks {
                        validate_block_with_mode(blk.as_bytes(), ValidationMode::Asset)
                            .expect("valid shared DGF1 asset block");
                    }
                    let decoded_pts = shared
                        .decode_shared_points(&read.points)
                        .expect("decode shared points");
                    assert_eq!(decoded_pts.len(), read.points.len());

                    let bevy_mesh = crate::mesh::mesh_from_usd(&read);
                    let dgf_mesh =
                        DgfMesh::from_bevy_mesh(&bevy_mesh, &opts.encode).expect("encode DGF");
                    for blk in &dgf_mesh.encoded.blocks {
                        validate_block_with_mode(blk.as_bytes(), ValidationMode::Asset)
                            .expect("valid DGF1 asset block");
                    }
                    let decoded = dgf_mesh.to_bevy_mesh().expect("decode DGF");
                    assert_eq!(
                        decoded.indices().unwrap().len(),
                        dgf_mesh.encoded.statistics.retained_triangles * 3
                    );
                    visited += 1;
                }
                for child in prim.children().unwrap() {
                    stack.push(child);
                }
            }
        }
        assert!(visited > 0);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn wgsl_decoder_validates_with_naga() {
        let module = naga::front::wgsl::parse_str(dgf::WGSL_DECODER_SOURCE)
            .expect("dgf::WGSL_DECODER_SOURCE must parse as valid WGSL");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        let info = validator.validate(&module).expect(
            "dgf::WGSL_DECODER_SOURCE must pass strict naga validation with zero extensions",
        );
        let _ = info.get_entry_point(0);

        let ep = module
            .entry_points
            .iter()
            .find(|e| e.name == "decode_blocks")
            .expect("decode_blocks compute entry point must exist");
        assert_eq!(ep.stage, naga::ShaderStage::Compute);
        assert_eq!(ep.workgroup_size, [64, 1, 1]);

        // Verify exact byte sizes match the WGSL storage buffer layouts.
        assert_eq!(std::mem::size_of::<GpuDgfBlock>(), 128);
        assert_eq!(std::mem::size_of::<GpuDgfBlockDispatch>(), 16);
        assert_eq!(std::mem::size_of::<GpuDgfDecodedVertex>(), 16);
        assert_eq!(std::mem::size_of::<GpuDgfDecodedTriangle>(), 24);
        assert_eq!(std::mem::size_of::<GpuDgfBlockInfo>(), 16);
    }
}
