//! Compares direct and budgeted reads of an explicit animation-channel list.

use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use openusd::usd::{AttributeQuery, QueryCacheBudget, TimeCode};
use openusd::vt::{Value, ValueKind};
use serde_json::json;

const SAMPLES: usize = 1000;
const WARMUP: usize = 4;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 8;

fn numeric_bytes(value: &Value) -> Result<&[u8]> {
    Ok(match value {
        Value::None | Value::ValueBlock => &[],
        Value::FloatVec(v) => bytemuck::cast_slice(v),
        Value::DoubleVec(v) => bytemuck::cast_slice(v),
        Value::Vec3hVec(v) => bytemuck::cast_slice(v),
        Value::Vec3fVec(v) => bytemuck::cast_slice(v),
        Value::Vec3dVec(v) => bytemuck::cast_slice(v),
        Value::QuathVec(v) => bytemuck::cast_slice(v),
        Value::QuatfVec(v) => bytemuck::cast_slice(v),
        Value::QuatdVec(v) => bytemuck::cast_slice(v),
        _ => anyhow::bail!("unaccounted numeric value: {:?}", ValueKind::from(value)),
    })
}

fn compare(left: &[Option<Value>], right: &[Option<Value>]) -> Result<()> {
    ensure!(left.len() == right.len(), "answer count mismatch");
    for (left, right) in left.iter().zip(right) {
        match (left, right) {
            (None, None) => (),
            (Some(left), Some(right)) => {
                ensure!(
                    ValueKind::from(left) == ValueKind::from(right),
                    "kind mismatch"
                );
                ensure!(
                    numeric_bytes(left)? == numeric_bytes(right)?,
                    "numeric bit mismatch"
                );
            }
            _ => anyhow::bail!("presence mismatch"),
        }
    }
    Ok(())
}

fn digest(values: &[Option<Value>]) -> Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(&(values.len() as u64).to_le_bytes());
    for value in values {
        let kind = value
            .as_ref()
            .map_or_else(|| "Absent".into(), |v| format!("{:?}", ValueKind::from(v)));
        hash.update(&(kind.len() as u64).to_le_bytes());
        hash.update(kind.as_bytes());
        let bytes = value
            .as_ref()
            .map(numeric_bytes)
            .transpose()?
            .unwrap_or(&[]);
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(hash.finalize().to_string())
}

fn paths(text: &str) -> Result<Vec<&str>> {
    let paths: Vec<_> = text.lines().collect();
    ensure!(
        !paths.is_empty() && paths.len() <= MAX_ENTRIES,
        "channel limit"
    );
    ensure!(
        paths.iter().copied().collect::<BTreeSet<_>>().len() == paths.len(),
        "duplicate channel"
    );
    ensure!(
        paths
            .iter()
            .all(|path| !path.is_empty() && path.trim() == *path),
        "invalid channel text"
    );
    Ok(paths)
}

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

fn limits(policy: &str) -> Result<(usize, usize, bool)> {
    Ok(match policy {
        "admit" => (MAX_BYTES, MAX_ENTRIES, true),
        "zero-bytes" => (0, MAX_ENTRIES, false),
        "one-byte" => (1, MAX_ENTRIES, false),
        "zero-entries" => (MAX_BYTES, 0, false),
        "small-sources" => (64 * 1024, MAX_ENTRIES, false),
        _ => anyhow::bail!("unknown admission policy"),
    })
}

fn within_budget(bytes: usize, entries: usize, limits: (usize, usize)) -> Result<()> {
    ensure!(
        bytes <= limits.0 && entries <= limits.1,
        "cache budget exceeded"
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        (3..=4).contains(&args.len()),
        "usage: query_budget_probe ASSET CHANNELS REVERSE_0_OR_1 [POLICY]"
    );
    let policy = args.get(3).map_or("admit", String::as_str);
    let (max_bytes, max_entries, admitted) = limits(policy)?;
    let reverse = match args[2].as_str() {
        "0" => false,
        "1" => true,
        _ => anyhow::bail!("reverse must be 0 or 1"),
    };
    let text = std::fs::read_to_string(&args[1])?;
    let paths = paths(&text)?;
    let opening = Instant::now();
    let stage = usd_bevy::UsdSource::from_file(&args[0])?.open_stage()?;
    let attributes = paths
        .iter()
        .map(|path| stage.attribute(*path))
        .collect::<Result<Vec<_>, _>>()?;
    for attribute in &attributes {
        ensure!(
            ["translations", "rotations", "scales"].contains(&attribute.name()),
            "non-component channel"
        );
        ensure!(
            stage
                .prim(attribute.path().prim_path())?
                .type_name()?
                .is_some_and(|name| name.as_str() == "SkelAnimation"),
            "non-animation owner"
        );
        ensure!(attribute.has_authored_value()?, "unauthored component");
    }
    let (start, end) = (stage.start_time_code(), stage.end_time_code());
    ensure!(
        start.is_finite() && end.is_finite() && start < end,
        "invalid time range"
    );
    println!(
        "{}",
        json!({"type":"config", "samples":SAMPLES, "warmup":WARMUP, "reverse":reverse,
        "paths":paths, "start":start, "end":end, "policy":policy, "max_bytes":max_bytes, "max_entries":max_entries,
        "opening_ns":opening.elapsed().as_nanos(), "layers":stage.layer_identifiers(),
        "scope":"animation-attribute-batch-not-editor-or-native-fps"})
    );
    for attribute in &attributes {
        let samples = attribute.time_samples()?.unwrap_or_default();
        let bytes = samples.iter().try_fold(0usize, |sum, (_, value)| {
            sum.checked_add(numeric_bytes(value)?.len())
                .context("inventory overflow")
        })?;
        let default = attribute.get::<Value>()?;
        let default_bytes = default
            .as_ref()
            .map(numeric_bytes)
            .transpose()?
            .map_or(0, <[u8]>::len);
        println!(
            "{}",
            json!({"type":"inventory", "path":attribute.path().as_str(),
            "samples":samples.len(), "numeric_bytes":bytes, "default_bytes":default_bytes})
        );
    }
    let budget = QueryCacheBudget::new(max_bytes, max_entries);
    let queries: Vec<_> = attributes
        .iter()
        .map(|attr| AttributeQuery::with_budget(attr, &budget))
        .collect();
    let expected_entries = if admitted {
        attributes.len()
    } else if policy == "small-sources" {
        attributes
            .iter()
            .filter(|attr| attr.name() == "scales")
            .count()
    } else {
        0
    };
    for iteration in 0..SAMPLES + WARMUP {
        let time = clock(start, end, iteration);
        let at = Some(TimeCode::new(time));
        let first = first(iteration, reverse);
        let mut results = [None, None];
        for mode in [first, 1 - first] {
            let started = Instant::now();
            let answer: Vec<Option<Value>> = if mode == 0 {
                attributes
                    .iter()
                    .map(|attr| attr.get_at::<Value>(at))
                    .collect::<openusd::Result<_>>()?
            } else {
                queries
                    .iter()
                    .map(|query| query.get_at::<Value>(at))
                    .collect::<openusd::Result<_>>()?
            };
            results[mode] = Some((started.elapsed().as_nanos(), answer));
        }
        let [(direct_ns, direct), (query_ns, query)] = results.map(Option::unwrap);
        compare(&direct, &query)?;
        let fingerprint = digest(&direct)?;
        ensure!(fingerprint == digest(&query)?, "fingerprint mismatch");
        within_budget(
            budget.retained_bytes(),
            budget.retained_entries(),
            (max_bytes, max_entries),
        )?;
        ensure!(
            budget.retained_entries() == expected_entries
                && (expected_entries != 0 || budget.retained_bytes() == 0),
            "source admission differs from policy"
        );
        println!(
            "{}",
            json!({"type":if iteration < WARMUP {"warmup"} else {"sample"},
            "index":if iteration < WARMUP {iteration} else {iteration - WARMUP},
            "first":first, "clock":time, "direct_ns":direct_ns, "query_ns":query_ns,
            "digest":fingerprint, "retained_bytes":budget.retained_bytes(), "retained_entries":budget.retained_entries()})
        );
    }
    drop(queries);
    ensure!(
        budget.retained_bytes() == 0 && budget.retained_entries() == 0,
        "unreleased source charge"
    );
    println!(
        "{}",
        json!({"type":"complete", "samples":SAMPLES, "warmup":WARMUP,
        "attributes":attributes.len(), "retained_bytes":budget.retained_bytes(), "retained_entries":budget.retained_entries()})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_comparison_rejects_changes() {
        let original = [Some(Value::FloatVec(vec![0.0]))];
        for changed in [
            Some(Value::FloatVec(vec![-0.0])),
            Some(Value::DoubleVec(vec![0.0])),
            None,
        ] {
            assert!(compare(&original, std::slice::from_ref(&changed)).is_err());
            assert_ne!(digest(&original).unwrap(), digest(&[changed]).unwrap());
        }
        assert!(numeric_bytes(&Value::String("unsupported".into())).is_err());
    }

    #[test]
    fn clocks_and_orders() {
        for reverse in [false, true] {
            assert_eq!(
                (0..4).map(|i| first(i, reverse)).collect::<Vec<_>>(),
                if reverse {
                    vec![1, 0, 0, 1]
                } else {
                    vec![0, 1, 1, 0]
                }
            );
        }
        for i in 0..SAMPLES {
            assert_eq!(clock(0.0, 1000.0, i + WARMUP), (i + 1) as f64);
        }
        assert_eq!(
            (0..WARMUP).map(|i| clock(0.0, 10.0, i)).collect::<Vec<_>>(),
            vec![0.0, 5.0, 10.0, 5.0]
        );
    }

    #[test]
    fn limits_and_path_lists() {
        assert!(within_budget(MAX_BYTES, MAX_ENTRIES, (MAX_BYTES, MAX_ENTRIES)).is_ok());
        assert!(within_budget(MAX_BYTES + 1, MAX_ENTRIES, (MAX_BYTES, MAX_ENTRIES)).is_err());
        assert!(within_budget(MAX_BYTES, MAX_ENTRIES + 1, (MAX_BYTES, MAX_ENTRIES)).is_err());
        for invalid in ["", "\n", "/A.x\n/A.x", " /A.x", "/A.x "] {
            assert!(paths(invalid).is_err());
        }
        let oversized = (0..=MAX_ENTRIES)
            .map(|i| format!("/A.x{i}\n"))
            .collect::<String>();
        assert!(paths(&oversized).is_err());
        assert_eq!(paths("/A.x\n/A.y\n").unwrap(), vec!["/A.x", "/A.y"]);
    }

    #[test]
    fn refusal_policies() {
        assert_eq!(limits("admit").unwrap(), (MAX_BYTES, MAX_ENTRIES, true));
        assert_eq!(
            limits("small-sources").unwrap(),
            (64 * 1024, MAX_ENTRIES, false)
        );
        for (policy, bytes, entries) in [
            ("zero-bytes", 0, 8),
            ("one-byte", 1, 8),
            ("zero-entries", MAX_BYTES, 0),
        ] {
            assert_eq!(limits(policy).unwrap(), (bytes, entries, false));
            assert!(within_budget(0, 0, (bytes, entries)).is_ok());
            assert!(within_budget(2, 1, (bytes, entries)).is_err());
        }
        assert!(limits("unknown").is_err());
    }
}
