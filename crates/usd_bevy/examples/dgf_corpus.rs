//! Real-world OpenUSD / USDZ corpus verification for `dgf` + `openusd-dgf` inside `bevy_openusd`.
//!
//! Compares two real integration paths across production USDZ assets:
//! 1. **Shared-point USD topology** (triangulating on shared USD `points` before face-varying expansion,
//!    which lets DGF1 strip continuations & vertex reuse compress at ~4–8 B/tri).
//! 2. **Expanded Bevy `Mesh`** (`DgfMesh::from_bevy_mesh` after `mesh_from_usd` attribute expansion).
//!
//! Run with:
//!   cargo run -p usd_bevy --features dgf --release --example dgf_corpus

use std::path::PathBuf;
use std::time::Instant;

use dgf::{ValidationMode, validate_block_with_mode};
use openusd::usd::Stage;
use usd_bevy::mesh::dgf::{DgfMesh, default_encode_options};
use usd_bevy::mesh::mesh_from_usd;
use usd_bevy::read::geom::read_mesh;

fn main() {
    let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let corpus = [
        "material_subsets.usda",
        "inherited_uv.usda",
        "external/teapot.usdz",
        "external/Kitchen_set.usdz",
        "external/chameleon.usdz",
        "external/seahorse.usdz",
        "external/hummingbird.usdz",
        "external/HumanFemale.usdz",
    ];

    let options = default_encode_options();

    println!(
        "{:<26} {:<8} {:>5} {:>8} {:>7} {:>8} {:>8} {:>6} {:>6} {:>9} {:>7} {:>6}",
        "asset",
        "mode",
        "mesh",
        "tris",
        "blocks",
        "raw_KB",
        "dgf_KB",
        "ratio",
        "B/tri",
        "max_err",
        "enc_ms",
        "dec_ms"
    );
    println!("{}", "-".repeat(120));

    let mut total_blocks_shared = 0usize;
    let mut total_tris_shared = 0usize;
    let mut total_skips_shared = 0usize;

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

        let mut reads = Vec::new();
        let mut stack = Vec::new();
        for name in stage.root_prims().unwrap_or_default() {
            if let Ok(p) = stage.prim(format!("/{name}")) {
                stack.push(p);
            }
        }
        while let Some(prim) = stack.pop() {
            if let Ok(Some(read)) = read_mesh(&stage, prim.path()) {
                reads.push((prim.path().to_string(), read));
            }
            if let Ok(children) = prim.children() {
                for child in children {
                    stack.push(child);
                }
            }
        }

        // Mode 1: Shared-point USD topology (`DgfMesh::from_read_mesh`)
        {
            let (mut meshes, mut tris, mut blocks, mut raw_b, mut dgf_b) = (0, 0, 0, 0, 0);
            let (mut max_err, mut enc_ms, mut dec_ms) = (0.0f64, 0.0f64, 0.0f64);
            for (path, read) in &reads {
                let t0 = Instant::now();
                match DgfMesh::from_read_mesh(read, &options) {
                    Ok(dm) => {
                        if dm.encoded.blocks.is_empty() {
                            continue;
                        }
                        enc_ms += t0.elapsed().as_secs_f64() * 1000.0;
                        for blk in &dm.encoded.blocks {
                            validate_block_with_mode(blk.as_bytes(), ValidationMode::Asset)
                                .unwrap_or_else(|e| panic!("invalid block on {rel} {path}: {e}"));
                        }
                        let t1 = Instant::now();
                        let pts = dm.decode_shared_points(&read.points).unwrap();
                        dec_ms += t1.elapsed().as_secs_f64() * 1000.0;
                        assert_eq!(pts.len(), read.points.len());

                        meshes += 1;
                        tris += dm.encoded.statistics.retained_triangles;
                        blocks += dm.encoded.blocks.len();
                        raw_b += dm.uncompressed_geom_bytes;
                        dgf_b += dm.dgf_bytes();
                        max_err = max_err.max(dm.encoded.quantization.max_coord_error);
                    }
                    Err(e) => {
                        total_skips_shared += 1;
                        eprintln!("  shared skip {rel} {path}: {e}");
                    }
                }
            }
            let ratio = if dgf_b > 0 {
                raw_b as f64 / dgf_b as f64
            } else {
                0.0
            };
            let bpt = if tris > 0 {
                dgf_b as f64 / tris as f64
            } else {
                0.0
            };
            println!(
                "{:<26} {:<8} {:>5} {:>8} {:>7} {:>8.1} {:>8.1} {:>5.2}x {:>6.2} {:>9.5} {:>7.1} {:>6.1}",
                rel,
                "shared",
                meshes,
                tris,
                blocks,
                raw_b as f64 / 1024.0,
                dgf_b as f64 / 1024.0,
                ratio,
                bpt,
                max_err,
                enc_ms,
                dec_ms
            );
            total_blocks_shared += blocks;
            total_tris_shared += tris;
        }

        // Mode 2: Expanded Bevy Mesh (DgfMesh::from_bevy_mesh)
        {
            let (mut meshes, mut tris, mut blocks, mut raw_b, mut dgf_b) = (0, 0, 0, 0, 0);
            let (mut max_err, mut enc_ms, mut dec_ms) = (0.0f64, 0.0f64, 0.0f64);
            for (path, read) in &reads {
                let bevy_mesh = mesh_from_usd(read);
                let t0 = Instant::now();
                if let Ok(dm) = DgfMesh::from_bevy_mesh(&bevy_mesh, &options) {
                    enc_ms += t0.elapsed().as_secs_f64() * 1000.0;
                    for blk in &dm.encoded.blocks {
                        validate_block_with_mode(blk.as_bytes(), ValidationMode::Asset)
                            .unwrap_or_else(|e| panic!("invalid block on {rel} {path}: {e}"));
                    }
                    let t1 = Instant::now();
                    let decoded = dm.to_bevy_mesh().unwrap();
                    dec_ms += t1.elapsed().as_secs_f64() * 1000.0;
                    assert_eq!(
                        decoded.indices().unwrap().len(),
                        dm.encoded.statistics.retained_triangles * 3
                    );

                    meshes += 1;
                    tris += dm.encoded.statistics.retained_triangles;
                    blocks += dm.encoded.blocks.len();
                    raw_b += dm.uncompressed_geom_bytes;
                    dgf_b += dm.dgf_bytes();
                    max_err = max_err.max(dm.encoded.quantization.max_coord_error);
                }
            }
            let ratio = if dgf_b > 0 {
                raw_b as f64 / dgf_b as f64
            } else {
                0.0
            };
            let bpt = if tris > 0 {
                dgf_b as f64 / tris as f64
            } else {
                0.0
            };
            println!(
                "{:<26} {:<8} {:>5} {:>8} {:>7} {:>8.1} {:>8.1} {:>5.2}x {:>6.2} {:>9.5} {:>7.1} {:>6.1}",
                "",
                "bevy",
                meshes,
                tris,
                blocks,
                raw_b as f64 / 1024.0,
                dgf_b as f64 / 1024.0,
                ratio,
                bpt,
                max_err,
                enc_ms,
                dec_ms
            );
        }
    }

    println!("{}", "-".repeat(120));
    println!(
        "SHARED-USD TOTAL: {total_tris_shared} triangles -> {total_blocks_shared} DGF1 blocks (skips={total_skips_shared}, 0 validation errors)."
    );
}
