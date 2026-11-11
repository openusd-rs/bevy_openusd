use std::collections::VecDeque;

#[derive(Default, Debug, PartialEq, Eq)]
struct Counts {
    points: usize,
    adjacent: usize,
    additional_hits: usize,
    misses: usize,
    peak_entries: usize,
}

fn count_reuse(
    indices: &[[u16; 4]],
    weights: &[[f32; 4]],
    corrections: &[[u32; 9]],
    capacity: usize,
) -> anyhow::Result<Counts> {
    anyhow::ensure!(indices.len() == weights.len() && weights.len() == corrections.len());
    anyhow::ensure!(capacity <= 256);
    let mut cache = VecDeque::with_capacity(capacity);
    let mut previous = None;
    let mut counts = Counts::default();
    for ((indices, weights), correction) in indices.iter().zip(weights).zip(corrections) {
        let key = (*indices, weights.map(f32::to_bits));
        counts.points += 1;
        if let Some((old_key, old_correction)) = previous
            && old_key == key
        {
            anyhow::ensure!(old_correction == *correction, "adjacent correction differs");
            counts.adjacent += 1;
        } else if let Some(position) = cache.iter().position(|(cached, _)| *cached == key) {
            let entry = cache.remove(position).unwrap();
            anyhow::ensure!(entry.1 == *correction, "cached correction differs");
            cache.push_back(entry);
            counts.additional_hits += 1;
        } else {
            counts.misses += 1;
            if capacity > 0 {
                if cache.len() == capacity {
                    cache.pop_front();
                }
                cache.push_back((key, *correction));
                counts.peak_entries = counts.peak_entries.max(cache.len());
            }
        }
        previous = Some((key, *correction));
    }
    anyhow::ensure!(counts.adjacent + counts.additional_hits + counts.misses == counts.points);
    Ok(counts)
}

#[test]
fn influence_reuse_model_separates_adjacent_hits_and_bounds_capacity() {
    let indices = [[0; 4]; 5];
    let weights = [
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.5; 4],
        [0.5; 4],
        [1.0, 0.0, 0.0, 0.0],
    ];
    let corrections = [[0; 9]; 5];
    let zero = count_reuse(&indices, &weights, &corrections, 0).unwrap();
    assert_eq!(
        zero,
        Counts {
            points: 5,
            adjacent: 2,
            additional_hits: 0,
            misses: 3,
            peak_entries: 0
        }
    );
    assert_eq!(
        count_reuse(&indices, &weights, &corrections, 1)
            .unwrap()
            .additional_hits,
        0
    );
    let two = count_reuse(&indices, &weights, &corrections, 2).unwrap();
    assert_eq!(two.additional_hits, 1);
    assert_eq!(two.misses, 2);
    assert_eq!(two.peak_entries, 2);
}

#[test]
fn influence_reuse_model_preserves_weight_and_matrix_bits() {
    let indices = [[0; 4]; 3];
    let weights = [[0.0; 4], [-0.0; 4], [0.0; 4]];
    let mut corrections = [[0; 9]; 3];
    let counts = count_reuse(&indices, &weights, &corrections, 2).unwrap();
    assert_eq!(counts.adjacent, 0);
    assert_eq!(counts.additional_hits, 1);
    corrections[2][0] = 1;
    assert!(count_reuse(&indices, &weights, &corrections, 2).is_err());
    assert!(count_reuse(&indices[..2], &weights, &corrections, 2).is_err());
    assert!(count_reuse(&indices, &weights, &corrections, 257).is_err());
}

#[test]
#[ignore = "release inventory requiring USD_SKIN_ASSET"]
fn real_asset_gpu_influence_reuse_inventory() -> anyhow::Result<()> {
    anyhow::ensure!(!cfg!(debug_assertions), "inventory requires release mode");
    let file = std::fs::canonicalize(std::env::var("USD_SKIN_ASSET")?)?;
    let samples = std::env::var("USD_SKIN_SAMPLES")
        .unwrap_or_else(|_| "5".into())
        .parse::<usize>()?;
    anyhow::ensure!((2..=64).contains(&samples));
    let stage = crate::UsdSource::from_file(&file)?.open_stage()?;
    let start = stage.start_time_code();
    let end = stage.end_time_code();
    anyhow::ensure!(
        start.is_finite() && end.is_finite() && end >= start && (end - start).is_finite()
    );
    let mut paths = Vec::new();
    stage.traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |path| {
        paths.push(path.clone())
    })?;
    paths.retain(|path| {
        super::is_skinned(&stage, path) && super::deformation_is_time_varying(&stage, path)
    });
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    println!(
        "skin_reuse_config\tasset={}\tsamples={samples}\tpaths={}\tstart={start}\tend={end}\tscope=source-reader-inventory-not-app-or-FPS\treuse=modeled-not-speedup",
        file.display(),
        paths.len()
    );
    let mut accepted = 0;
    let mut rejected = 0;
    for sample in 0..samples {
        let clock = start + (end - start) * sample as f64 / (samples - 1) as f64;
        let _scope = super::SkelSampleScope::begin(&stage);
        for path in &paths {
            let Some(mesh) = crate::read::geom::read_mesh_at(&stage, path, Some(clock))? else {
                continue;
            };
            let flat = crate::mesh::uses_flat_normals(&mesh);
            let started = std::time::Instant::now();
            let result = super::gpu_skin_sample_with_mesh(&stage, path, Some(clock), &mesh);
            let elapsed = started.elapsed().as_nanos();
            let input = match result {
                Ok(input) => input,
                Err(error) => {
                    rejected += 1;
                    println!("skin_reuse_rejected\tclock={clock:.12}\tpath={path}\terror={error}");
                    continue;
                }
            };
            accepted += 1;
            let skin = super::bound_skin(&stage, path)?
                .ok_or_else(|| anyhow::anyhow!("accepted sample has no skin binding"))?;
            let rigid = skin.skinning.is_rigidly_deformed();
            let components = if rigid {
                usize::from(!input.indices.is_empty())
            } else {
                input.indices.len()
            };
            let corrections: Vec<_> = input.normal_corrections[..components]
                .iter()
                .map(|matrix| matrix.to_cols_array().map(f32::to_bits))
                .collect();
            let non_identity = input
                .normal_corrections
                .iter()
                .filter(|matrix| **matrix != bevy::math::Mat3::IDENTITY)
                .count();
            for capacity in [0, 16, 64, 256] {
                let counts = count_reuse(
                    &input.indices[..components],
                    &input.weights[..components],
                    &corrections,
                    capacity,
                )?;
                println!(
                    "skin_reuse\tclock={clock:.12}\tpath={path}\tflat={flat}\trigid={rigid}\tcapacity={capacity}\tpoints={}\tadjacent={}\tadditional_hits={}\tmisses={}\tpeak_entries={}\tnon_identity={non_identity}\tsample_ns={elapsed}",
                    counts.points,
                    counts.adjacent,
                    counts.additional_hits,
                    counts.misses,
                    counts.peak_entries
                );
            }
        }
    }
    println!("SKIN_REUSE_INVENTORY_COMPLETE accepted={accepted} rejected={rejected}");
    Ok(())
}
