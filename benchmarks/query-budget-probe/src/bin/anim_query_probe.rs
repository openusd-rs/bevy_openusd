//! Compares direct and budgeted animation-local matrix evaluation.

use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use openusd::gf;
use openusd::usd::{QueryCacheBudget, TimeCode};
use openusd_schemas::skel::{AnimQuery, Animation};
use serde_json::json;

const SAMPLES: usize = 1000;
const WARMUP: usize = 4;
type Batch = Vec<Option<Vec<gf::Mat4d>>>;

fn clock(start: f64, end: f64, iteration: usize) -> f64 {
    let fraction = if iteration < WARMUP {
        [0.0, 0.5, 1.0, 0.5][iteration]
    } else {
        (iteration - WARMUP + 1) as f64 / SAMPLES as f64
    };
    start + (end - start) * fraction
}

fn first(iteration: usize, reverse: bool) -> usize {
    usize::from(matches!(iteration % 4, 1 | 2) ^ reverse)
}

fn limits(policy: &str) -> Result<(usize, usize)> {
    Ok(match policy {
        "admit" => (8 * 1024 * 1024, 8),
        "small-sources" => (64 * 1024, 8),
        "zero-bytes" => (0, 8),
        "one-byte" => (1, 8),
        "zero-entries" => (8 * 1024 * 1024, 0),
        _ => anyhow::bail!("unknown policy"),
    })
}

fn compare(left: &Batch, right: &Batch) -> Result<()> {
    ensure!(left.len() == right.len(), "animation count changed");
    for (left, right) in left.iter().zip(right) {
        match (left, right) {
            (None, None) => (),
            (Some(left), Some(right)) => {
                ensure!(
                    bytemuck::cast_slice::<_, u8>(left) == bytemuck::cast_slice::<_, u8>(right),
                    "matrix bits changed"
                );
            }
            _ => anyhow::bail!("matrix presence changed"),
        }
    }
    Ok(())
}

fn digest(batch: &Batch) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(&(batch.len() as u64).to_le_bytes());
    for matrices in batch {
        hash.update(&[u8::from(matrices.is_some())]);
        if let Some(matrices) = matrices {
            hash.update(&(matrices.len() as u64).to_le_bytes());
            hash.update(bytemuck::cast_slice(matrices));
        }
    }
    hash.finalize().to_string()
}

fn evaluate(queries: &[AnimQuery], time: f64) -> openusd::Result<Batch> {
    queries
        .iter()
        .map(|query| query.joint_local_transforms(TimeCode::new(time)))
        .collect()
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 4,
        "usage: anim_query_probe ASSET CHANNELS REVERSE_0_OR_1 POLICY"
    );
    let reverse = match args[2].as_str() {
        "0" => false,
        "1" => true,
        _ => anyhow::bail!("invalid reverse"),
    };
    let policy = args[3].as_str();
    let (max_bytes, max_entries) = limits(policy)?;
    let channel_text = std::fs::read_to_string(&args[1])?;
    let channels: Vec<_> = channel_text.lines().collect();
    ensure!(!channels.is_empty() && channels.len() <= 8, "channel limit");
    ensure!(
        channels.iter().copied().collect::<BTreeSet<_>>().len() == channels.len(),
        "duplicate channels"
    );
    let opening = Instant::now();
    let stage = usd_bevy::UsdSource::from_file(&args[0])?.open_stage()?;
    let paths = channels
        .iter()
        .map(|path| {
            let attribute = stage.attribute(*path)?;
            ensure!(
                ["translations", "rotations", "scales"].contains(&attribute.name()),
                "non-component channel"
            );
            Ok(attribute.path().prim_path().as_str().to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        paths.len() * 3 == channels.len(),
        "incomplete component selection"
    );
    let mut direct = Vec::new();
    let mut cached = Vec::new();
    let budget = QueryCacheBudget::new(max_bytes, max_entries);
    for path in &paths {
        let animation = Animation::get(&stage, path.as_str())?.context("animation missing")?;
        direct.push(AnimQuery::new(animation.clone())?);
        cached.push(AnimQuery::with_budget(animation, &budget)?);
    }
    let orders: Vec<_> = direct.iter().map(AnimQuery::joint_order).collect();
    ensure!(
        orders.iter().all(|order| !order.is_empty()),
        "empty joint order"
    );
    let expected_entries = match policy {
        "admit" => channels.len(),
        "small-sources" => paths.len(),
        _ => 0,
    };
    let (start, end) = (stage.start_time_code(), stage.end_time_code());
    ensure!(
        start.is_finite() && end.is_finite() && start < end,
        "invalid time range"
    );
    println!(
        "{}",
        json!({"type":"config", "scope":"animation-local-matrices-not-editor-fps", "samples":SAMPLES,
        "warmup":WARMUP, "reverse":reverse, "policy":policy, "max_bytes":max_bytes, "max_entries":max_entries,
        "channels":channels, "animations":paths, "joint_orders":orders, "start":start, "end":end,
        "opening_ns":opening.elapsed().as_nanos(), "layers":stage.layer_identifiers()})
    );
    for iteration in 0..SAMPLES + WARMUP {
        let time = clock(start, end, iteration);
        let first = first(iteration, reverse);
        let mut results = [None, None];
        for mode in [first, 1 - first] {
            let started = Instant::now();
            let answer = evaluate(if mode == 0 { &direct } else { &cached }, time)?;
            results[mode] = Some((started.elapsed().as_nanos(), answer));
        }
        let [(direct_ns, direct_values), (query_ns, query_values)] = results.map(Option::unwrap);
        compare(&direct_values, &query_values)?;
        ensure!(
            direct_values.iter().zip(&orders).all(|(value, order)| value
                .as_ref()
                .is_some_and(|value| value.len() == order.len())),
            "missing or mismatched transforms"
        );
        ensure!(
            budget.retained_bytes() <= max_bytes && budget.retained_entries() <= max_entries,
            "source budget exceeded"
        );
        ensure!(
            budget.retained_entries() == expected_entries,
            "admission mismatch"
        );
        if expected_entries == 0 {
            ensure!(budget.retained_bytes() == 0, "refused source retained");
        }
        println!(
            "{}",
            json!({"type":if iteration < WARMUP {"warmup"} else {"sample"},
            "index":if iteration < WARMUP {iteration} else {iteration - WARMUP}, "clock":time, "first":first,
            "direct_ns":direct_ns, "query_ns":query_ns, "digest":digest(&direct_values),
            "joint_counts":direct_values.iter().map(|value| value.as_ref().map(Vec::len)).collect::<Vec<_>>(),
            "retained_bytes":budget.retained_bytes(), "retained_entries":budget.retained_entries()})
        );
    }
    drop(cached);
    ensure!(
        budget.retained_bytes() == 0 && budget.retained_entries() == 0,
        "source charges leaked"
    );
    println!(
        "{}",
        json!({"type":"complete", "samples":SAMPLES, "warmup":WARMUP, "animations":paths.len(),
        "retained_bytes":budget.retained_bytes(), "retained_entries":budget.retained_entries()})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_bits_presence() {
        let original = vec![Some(vec![gf::Mat4d::IDENTITY])];
        let mut changed = gf::Mat4d::IDENTITY;
        changed.0[1] = -0.0;
        for other in [vec![None], vec![Some(vec![])], vec![Some(vec![changed])]] {
            assert!(compare(&original, &other).is_err());
            assert_ne!(digest(&original), digest(&other));
        }
    }

    #[test]
    fn clocks_orders_limits() {
        assert_eq!(
            (0..4).map(|i| first(i, false)).collect::<Vec<_>>(),
            [0, 1, 1, 0]
        );
        assert_eq!(
            (0..4).map(|i| first(i, true)).collect::<Vec<_>>(),
            [1, 0, 0, 1]
        );
        assert_eq!(clock(0.0, 1000.0, 4), 1.0);
        assert_eq!(clock(0.0, 1000.0, 1003), 1000.0);
        assert_eq!(limits("admit").unwrap(), (8 * 1024 * 1024, 8));
        assert_eq!(limits("small-sources").unwrap(), (64 * 1024, 8));
        assert_eq!(limits("zero-bytes").unwrap(), (0, 8));
        assert_eq!(limits("one-byte").unwrap(), (1, 8));
        assert_eq!(limits("zero-entries").unwrap(), (8 * 1024 * 1024, 0));
        assert!(limits("unknown").is_err());
    }
}
