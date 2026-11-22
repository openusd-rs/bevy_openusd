//! Real-GPU compute verification of `dgf::WGSL_DECODER_SOURCE` against the CPU `dgf::decode_block` oracle.
//!
//! Dispatches the WGSL DGF1 block decoder on a live `wgpu` GPU adapter and verifies **bit-for-bit**
//! equality (`f32::to_bits`, triangle indices, geometry IDs, opaque flags, primitive IDs,
//! user data, and OMM selectors) across:
//! 1. Exhaustive DGF1 edge-case fixtures (starter vector, EDGE1/EDGE2/BACKTRACK in both directions,
//!    3/4/5/6-bit repeated index thresholds, geom_id 511 vs 512 25-bit prefix palette, 0-bit prefix
//!    25-bit payload palette, all OMM descriptor counts 0..=7 + user_data + geom palette, >64
//!    first-use flags, extreme signed 24-bit anchors & exponents 1 and 232, MAX_PRIM_ID).
//! 2. Real production OpenUSD / USDZ corpus meshes (`teapot.usdz`, `Kitchen_set.usdz`,
//!    `chameleon.usdz`, `seahorse.usdz`, `hummingbird.usdz`, `HumanFemale.usdz`).
//!
//! Run with:
//!   cargo run -p usd_bevy --features dgf --release --example dgf_gpu_verify

use std::path::PathBuf;
use std::time::Instant;

use dgf::{
    ANCHOR_MAX, ANCHOR_MIN, BiasedExponent, Block, MAX_PRIM_ID, PreparedBlock, PreparedOmm,
    PreparedTopology, TriControl, ValidationMode, decode_block, encode_block,
    validate_block_with_mode,
};
use openusd::usd::Stage;
use usd_bevy::mesh::dgf::{
    DgfMesh, GpuDgfBatch, GpuDgfBlockInfo, GpuDgfDecodedTriangle, GpuDgfDecodedVertex,
    default_encode_options,
};
use usd_bevy::read::geom::read_mesh;

struct GpuDgfDecoder {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    adapter_info: wgpu::AdapterInfo,
}

struct GpuDecodeOutput {
    vertices: Vec<GpuDgfDecodedVertex>,
    triangles: Vec<GpuDgfDecodedTriangle>,
    block_info: Vec<GpuDgfBlockInfo>,
    gpu_ms: f64,
}

impl GpuDgfDecoder {
    fn new() -> Self {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .expect("no wgpu adapter found (check Vulkan ICDs / /dev/dri)");
        let adapter_info = adapter.get_info();

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("dgf-gpu-verify"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .expect("request wgpu device");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dgf_decode.wgsl"),
            source: wgpu::ShaderSource::Wgsl(dgf::WGSL_DECODER_SOURCE.into()),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("dgf_decode_blocks"),
            layout: None,
            module: &shader,
            entry_point: Some("decode_blocks"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
            adapter_info,
        }
    }

    fn decode_batch(&self, batch: &GpuDgfBatch) -> GpuDecodeOutput {
        assert!(!batch.blocks.is_empty());

        let block_bytes = bytemuck::cast_slice(&batch.blocks);
        let dispatch_bytes = bytemuck::cast_slice(&batch.dispatch);
        let vert_byte_len =
            (batch.total_vertices.max(1) * std::mem::size_of::<GpuDgfDecodedVertex>()) as u64;
        let tri_byte_len =
            (batch.total_triangles.max(1) * std::mem::size_of::<GpuDgfDecodedTriangle>()) as u64;
        let info_byte_len = (batch.blocks.len() * std::mem::size_of::<GpuDgfBlockInfo>()) as u64;

        let blocks_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dgf_blocks"),
            size: block_bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dispatch_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dgf_dispatch"),
            size: dispatch_bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&blocks_buf, 0, block_bytes);
        self.queue.write_buffer(&dispatch_buf, 0, dispatch_bytes);

        let out_verts_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out_vertices"),
            size: vert_byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_tris_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out_triangles"),
            size: tri_byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_info_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out_block_info"),
            size: info_byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let read_verts_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_vertices"),
            size: vert_byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let read_tris_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_triangles"),
            size: tri_byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let read_info_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_block_info"),
            size: info_byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dgf_decode_bg"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: blocks_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dispatch_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_verts_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_tris_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: out_info_buf.as_entire_binding(),
                },
            ],
        });

        // Flush uploads before timing the compute dispatch + readback.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let t0 = Instant::now();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dgf_decode_enc"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dgf_decode_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let workgroups = (batch.blocks.len() as u32).div_ceil(64);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_verts_buf, 0, &read_verts_buf, 0, vert_byte_len);
        encoder.copy_buffer_to_buffer(&out_tris_buf, 0, &read_tris_buf, 0, tri_byte_len);
        encoder.copy_buffer_to_buffer(&out_info_buf, 0, &read_info_buf, 0, info_byte_len);
        self.queue.submit(Some(encoder.finish()));

        let v_slice = read_verts_buf.slice(..);
        let t_slice = read_tris_buf.slice(..);
        let i_slice = read_info_buf.slice(..);
        v_slice.map_async(wgpu::MapMode::Read, |r| r.expect("map verts"));
        t_slice.map_async(wgpu::MapMode::Read, |r| r.expect("map tris"));
        i_slice.map_async(wgpu::MapMode::Read, |r| r.expect("map info"));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll device");
        let gpu_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let v_view = v_slice.get_mapped_range().expect("mapped vertex range");
        let t_view = t_slice.get_mapped_range().expect("mapped triangle range");
        let i_view = i_slice.get_mapped_range().expect("mapped info range");

        let vertices = bytemuck::cast_slice::<u8, GpuDgfDecodedVertex>(&v_view)
            [..batch.total_vertices]
            .to_vec();
        let triangles = bytemuck::cast_slice::<u8, GpuDgfDecodedTriangle>(&t_view)
            [..batch.total_triangles]
            .to_vec();
        let block_info =
            bytemuck::cast_slice::<u8, GpuDgfBlockInfo>(&i_view)[..batch.blocks.len()].to_vec();

        drop(v_view);
        drop(t_view);
        drop(i_view);
        read_verts_buf.unmap();
        read_tris_buf.unmap();
        read_info_buf.unmap();

        GpuDecodeOutput {
            vertices,
            triangles,
            block_info,
            gpu_ms,
        }
    }
}

fn verify_blocks_bit_for_bit(
    decoder: &GpuDgfDecoder,
    label: &str,
    blocks: &[Block],
    add_vertex_offset: bool,
) -> (usize, usize, f64) {
    let batch = GpuDgfBatch::from_blocks(blocks, add_vertex_offset).expect("build GPU batch");
    let gpu = decoder.decode_batch(&batch);

    for (bi, blk) in blocks.iter().enumerate() {
        let validated =
            validate_block_with_mode(blk.as_bytes(), ValidationMode::Asset).expect("valid block");
        let cpu = decode_block(&validated).expect("CPU oracle decode");
        let disp = batch.dispatch[bi];
        let info = gpu.block_info[bi];

        assert_eq!(
            info.num_vertices,
            u32::from(cpu.num_vertices),
            "{label} block {bi}: num_vertices mismatch"
        );
        assert_eq!(
            info.num_triangles,
            u32::from(cpu.num_triangles),
            "{label} block {bi}: num_triangles mismatch"
        );
        assert_eq!(
            info.user_data_opt(),
            cpu.user_data,
            "{label} block {bi}: user_data mismatch"
        );
        assert_eq!(
            info.omm_descriptor_count(),
            cpu.omm.descriptor_count,
            "{label} block {bi}: omm_descriptor_count mismatch"
        );
        assert_eq!(
            info.bits_per_index(),
            cpu.header.bits_per_index,
            "{label} block {bi}: bits_per_index mismatch"
        );
        assert_eq!(
            info.exponent(),
            cpu.exponent.raw(),
            "{label} block {bi}: exponent mismatch"
        );

        let v_off = disp.vertex_offset as usize;
        for (vi, &cpu_pos) in cpu.positions().iter().enumerate() {
            let gpu_pos = gpu.vertices[v_off + vi].pos;
            assert_eq!(
                [
                    gpu_pos[0].to_bits(),
                    gpu_pos[1].to_bits(),
                    gpu_pos[2].to_bits()
                ],
                [
                    cpu_pos[0].to_bits(),
                    cpu_pos[1].to_bits(),
                    cpu_pos[2].to_bits()
                ],
                "{label} block {bi} vert {vi}: position mismatch gpu={gpu_pos:?} cpu={cpu_pos:?}"
            );
        }

        let t_off = disp.triangle_offset as usize;
        let expected_vbase = if add_vertex_offset {
            disp.vertex_offset
        } else {
            0
        };
        for (ti, &cpu_tri) in cpu.triangles().iter().enumerate() {
            let gt = gpu.triangles[t_off + ti];
            let expected_idx = [
                u32::from(cpu_tri[0]) + expected_vbase,
                u32::from(cpu_tri[1]) + expected_vbase,
                u32::from(cpu_tri[2]) + expected_vbase,
            ];
            assert_eq!(
                gt.indices, expected_idx,
                "{label} block {bi} tri {ti}: indices mismatch"
            );
            assert_eq!(
                gt.geometry_id, cpu.geometry_ids[ti],
                "{label} block {bi} tri {ti}: geometry_id mismatch"
            );
            assert_eq!(
                gt.opaque(),
                cpu.opaque[ti],
                "{label} block {bi} tri {ti}: opaque mismatch"
            );
            assert_eq!(
                gt.prim_id,
                cpu.prim_id(ti as u8).unwrap(),
                "{label} block {bi} tri {ti}: prim_id mismatch"
            );
            let expected_omm = if cpu.omm.descriptor_count > 0 {
                Some(cpu.omm.triangle_descriptors[ti])
            } else {
                None
            };
            assert_eq!(
                gt.omm_descriptor(),
                expected_omm,
                "{label} block {bi} tri {ti}: omm_descriptor mismatch"
            );
        }
    }

    (batch.total_vertices, batch.total_triangles, gpu.gpu_ms)
}

fn build_edge_case_blocks() -> Vec<Block> {
    let mut blocks = Vec::new();

    // 1. Section 21 starter single triangle.
    blocks.push(
        encode_block(&PreparedBlock {
            quantized_positions: vec![[0, 0, 0], [1, 0, 0], [0, 1, 0]],
            exponent: BiasedExponent::new(127).unwrap(),
            topology: PreparedTopology {
                controls: vec![TriControl::Restart],
                strip_indices: vec![0, 1, 2],
                num_vertices: 3,
            },
            geometry_ids: vec![0],
            opaque: vec![true],
            prim_id_base: 0,
            user_data: None,
            omm: None,
            allow_exponent_adjustment: false,
        })
        .unwrap(),
    );

    // 2. Both BACKTRACK directions (after EDGE1 and after EDGE2) + RESTART.
    blocks.push(
        encode_block(&PreparedBlock {
            quantized_positions: (0..10).map(|i| [i, i % 3, -(i % 2)]).collect(),
            exponent: BiasedExponent::new(125).unwrap(),
            topology: PreparedTopology {
                controls: vec![
                    TriControl::Restart,
                    TriControl::Edge1,
                    TriControl::Backtrack,
                    TriControl::Edge2,
                    TriControl::Backtrack,
                    TriControl::Restart,
                ],
                strip_indices: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
                num_vertices: 10,
            },
            geometry_ids: vec![12; 6],
            opaque: vec![true; 6],
            prim_id_base: 77,
            user_data: Some(0xCAFE_BABE),
            omm: None,
            allow_exponent_adjustment: false,
        })
        .unwrap(),
    );

    // 3. Repeated-index width thresholds: 3, 4, 5, 6 bits per index.
    for &repeat_idx in &[7u8, 8, 15, 16, 31, 32, 62] {
        let num_v = repeat_idx + 2;
        let mut controls = vec![TriControl::Restart];
        let mut strip = vec![0u8, 1, 2];
        for v in 3..num_v {
            controls.push(TriControl::Edge1);
            strip.push(v);
        }
        controls.push(TriControl::Edge1);
        strip.push(repeat_idx);
        let t_count = controls.len();
        blocks.push(
            encode_block(&PreparedBlock {
                quantized_positions: (0..i32::from(num_v))
                    .map(|i| [i % 4, i / 4, i % 2])
                    .collect(),
                exponent: BiasedExponent::new(127).unwrap(),
                topology: PreparedTopology {
                    controls,
                    strip_indices: strip,
                    num_vertices: num_v,
                },
                geometry_ids: vec![0; t_count],
                opaque: vec![true; t_count],
                prim_id_base: u32::from(repeat_idx),
                user_data: None,
                omm: None,
                allow_exponent_adjustment: false,
            })
            .unwrap(),
        );
    }

    // 4. Geometry ID constant 511 vs 1-entry 25-bit prefix palette (512) vs 0-bit prefix palette.
    for &(gid, opq) in &[(511u32, false), (512u32, true), (0x00FF_FFFF, false)] {
        blocks.push(
            encode_block(&PreparedBlock {
                quantized_positions: vec![[0, 0, 0], [1, 0, 0], [0, 1, 0]],
                exponent: BiasedExponent::new(127).unwrap(),
                topology: PreparedTopology {
                    controls: vec![TriControl::Restart],
                    strip_indices: vec![0, 1, 2],
                    num_vertices: 3,
                },
                geometry_ids: vec![gid],
                opaque: vec![opq],
                prim_id_base: 1000,
                user_data: None,
                omm: None,
                allow_exponent_adjustment: false,
            })
            .unwrap(),
        );
    }

    // 0-bit prefix (25-bit payload) palette with contrasting high bits + mixed opacity.
    blocks.push(
        encode_block(&PreparedBlock {
            quantized_positions: vec![[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [2, 1, 0]],
            exponent: BiasedExponent::new(127).unwrap(),
            topology: PreparedTopology {
                controls: vec![TriControl::Restart, TriControl::Edge1, TriControl::Edge2],
                strip_indices: vec![0, 1, 2, 3, 4],
                num_vertices: 5,
            },
            geometry_ids: vec![0, 0x00FF_FFFF, 0],
            opaque: vec![false, true, true],
            prim_id_base: 500,
            user_data: Some(0x1234_5678),
            omm: None,
            allow_exponent_adjustment: false,
        })
        .unwrap(),
    );

    // 5. All OMM descriptor counts D = 0..=7 combined with user_data + geometry palette!
    for d in 0u8..=7 {
        let num_tris = 8usize;
        let mut controls = vec![TriControl::Restart];
        let mut strip = vec![0u8, 1, 2];
        for i in 1..num_tris {
            controls.push(TriControl::Edge1);
            strip.push((i + 2) as u8);
        }
        let omm = if d == 0 {
            None
        } else {
            Some(PreparedOmm {
                descriptor_count: d,
                triangle_descriptors: (0..num_tris).map(|t| (t as u8) % d).collect(),
                hot_patch_bytes: None,
            })
        };
        blocks.push(
            encode_block(&PreparedBlock {
                quantized_positions: (0..10).map(|i| [i, i % 2, 0]).collect(),
                exponent: BiasedExponent::new(127).unwrap(),
                topology: PreparedTopology {
                    controls,
                    strip_indices: strip,
                    num_vertices: 10,
                },
                geometry_ids: vec![1000, 1001, 1000, 1002, 1001, 1003, 1000, 1002],
                opaque: vec![true, false, false, true, false, true, true, true],
                prim_id_base: 42 + u32::from(d),
                user_data: Some(0xDEAD_BEEF ^ u32::from(d)),
                omm,
                allow_exponent_adjustment: false,
            })
            .unwrap(),
        );
    }

    // 6. >64 first-use flags (25 RESTART triangles = 75 strip indices = 72 first-use flags).
    {
        let num_tris = 25usize;
        let controls = vec![TriControl::Restart; num_tris];
        let mut strip = vec![0u8, 1, 2];
        let mut next_v = 3u8;
        for _ in 1..num_tris {
            for _ in 0..3 {
                if next_v < 60 {
                    strip.push(next_v);
                    next_v += 1;
                } else {
                    strip.push(0);
                }
            }
        }
        blocks.push(
            encode_block(&PreparedBlock {
                quantized_positions: (0..60).map(|i| [i % 2, (i / 2) % 2, 0]).collect(),
                exponent: BiasedExponent::new(127).unwrap(),
                topology: PreparedTopology {
                    controls,
                    strip_indices: strip,
                    num_vertices: 60,
                },
                geometry_ids: vec![0; num_tris],
                opaque: vec![true; num_tris],
                prim_id_base: 9000,
                user_data: None,
                omm: None,
                allow_exponent_adjustment: false,
            })
            .unwrap(),
        );
    }

    // 7. Extreme signed 24-bit anchors, min/max exponents (1 and 232), and MAX_PRIM_ID.
    for &(anc, exp) in &[
        (ANCHOR_MIN, 1u8),
        (-1, 1),
        (ANCHOR_MAX - 1, 1),
        (ANCHOR_MAX, 232),
        (ANCHOR_MIN + 1, 232),
    ] {
        let pts = if exp == 232 && anc == ANCHOR_MAX {
            vec![[anc, 0, 0], [anc, 0, 0], [anc, 0, 0]]
        } else {
            vec![[anc, anc, anc], [anc + 1, anc, anc], [anc, anc + 1, anc]]
        };
        blocks.push(
            encode_block(&PreparedBlock {
                quantized_positions: pts,
                exponent: BiasedExponent::new(exp).unwrap(),
                topology: PreparedTopology {
                    controls: vec![TriControl::Restart],
                    strip_indices: vec![0, 1, 2],
                    num_vertices: 3,
                },
                geometry_ids: vec![3],
                opaque: vec![true],
                prim_id_base: MAX_PRIM_ID,
                user_data: None,
                omm: None,
                allow_exponent_adjustment: false,
            })
            .unwrap(),
        );
    }

    blocks
}

fn main() {
    let decoder = GpuDgfDecoder::new();
    println!(
        "GPU Adapter: {} ({:?}, backend: {:?})",
        decoder.adapter_info.name, decoder.adapter_info.device_type, decoder.adapter_info.backend
    );
    println!("{}", "=".repeat(96));

    // Suite 1: Edge-case conformance suite (both local and global vertex indexing modes).
    let edge_blocks = build_edge_case_blocks();
    let (ev_loc, et_loc, ems_loc) =
        verify_blocks_bit_for_bit(&decoder, "edge-cases(local)", &edge_blocks, false);
    let (ev_glb, et_glb, ems_glb) =
        verify_blocks_bit_for_bit(&decoder, "edge-cases(global)", &edge_blocks, true);
    assert_eq!((ev_loc, et_loc), (ev_glb, et_glb));
    println!(
        "{:<26} {:>7} blocks {:>8} verts {:>8} tris  local={:>5.2}ms  global={:>5.2}ms  [EXACT BIT MATCH]",
        "edge-case fixtures",
        edge_blocks.len(),
        ev_loc,
        et_loc,
        ems_loc,
        ems_glb,
    );

    // Suite 2: Real OpenUSD / USDZ production assets.
    let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let corpus = [
        "material_subsets.usda",
        "external/teapot.usdz",
        "external/Kitchen_set.usdz",
        "external/chameleon.usdz",
        "external/seahorse.usdz",
        "external/hummingbird.usdz",
        "external/HumanFemale.usdz",
    ];
    let options = default_encode_options();

    let mut total_blocks = edge_blocks.len();
    let mut total_verts = ev_loc;
    let mut total_tris = et_loc;
    let mut total_gpu_ms = ems_glb;

    for rel in corpus {
        let full = assets_dir.join(rel);
        if !full.exists() {
            println!("{rel:<26} (missing, skipped)");
            continue;
        }
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .open(full.to_str().unwrap())
            .expect("open USD stage");

        let mut blocks = Vec::new();
        let mut stack = Vec::new();
        for name in stage.root_prims().unwrap_or_default() {
            if let Ok(p) = stage.prim(format!("/{name}")) {
                stack.push(p);
            }
        }
        while let Some(prim) = stack.pop() {
            if let Ok(Some(read)) = read_mesh(&stage, prim.path()) {
                if let Ok(dm) = DgfMesh::from_read_mesh(&read, &options) {
                    blocks.extend(dm.encoded.blocks);
                }
            }
            if let Ok(children) = prim.children() {
                for child in children {
                    stack.push(child);
                }
            }
        }
        if blocks.is_empty() {
            continue;
        }

        let (v_cnt, t_cnt, gpu_ms) = verify_blocks_bit_for_bit(&decoder, rel, &blocks, true);
        let tri_per_sec_m = if gpu_ms > 0.0 {
            (t_cnt as f64) / (gpu_ms * 1000.0)
        } else {
            0.0
        };
        println!(
            "{:<26} {:>7} blocks {:>8} verts {:>8} tris  gpu+readback={:>6.2}ms ({:>6.1} Mtri/s)  [EXACT BIT MATCH]",
            rel,
            blocks.len(),
            v_cnt,
            t_cnt,
            gpu_ms,
            tri_per_sec_m,
        );
        total_blocks += blocks.len();
        total_verts += v_cnt;
        total_tris += t_cnt;
        total_gpu_ms += gpu_ms;
    }

    println!("{}", "=".repeat(96));
    println!(
        "ALL PASS: {total_blocks} DGF1 blocks, {total_verts} vertices, {total_tris} triangles verified BIT-FOR-BIT on GPU in {total_gpu_ms:.2} ms!"
    );
}
