//! Compares direct and memoized upstream animation-attribute reads.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use openusd::usd::{Attribute, AttributeQuery, PrimPredicate, Stage, TimeCode};
use openusd::vt::{Value, ValueKind};

const WARMUP: usize = 4;

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
        _ => anyhow::bail!("unsupported animation value: {:?}", ValueKind::from(value)),
    })
}

fn digest(values: &[Option<Value>]) -> Result<blake3::Hash> {
    let mut digest = blake3::Hasher::new();
    digest.update(&(values.len() as u64).to_le_bytes());
    for value in values {
        let kind = value
            .as_ref()
            .map(|v| format!("{:?}", ValueKind::from(v)))
            .unwrap_or_else(|| "Absent".into());
        digest.update(&(kind.len() as u64).to_le_bytes());
        digest.update(kind.as_bytes());
        let bytes = value
            .as_ref()
            .map(numeric_bytes)
            .transpose()?
            .unwrap_or(&[]);
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(digest.finalize())
}

fn same_values(left: &[Option<Value>], right: &[Option<Value>]) -> Result<()> {
    ensure!(left.len() == right.len(), "attribute count changed");
    for (left, right) in left.iter().zip(right) {
        match (left, right) {
            (None, None) => (),
            (Some(left), Some(right)) => {
                ensure!(
                    ValueKind::from(left) == ValueKind::from(right),
                    "value kind changed"
                );
                ensure!(
                    numeric_bytes(left)? == numeric_bytes(right)?,
                    "value bits changed"
                );
            }
            _ => anyhow::bail!("value presence changed"),
        }
    }
    Ok(())
}

fn attributes(stage: &Stage) -> Result<Vec<Attribute>> {
    let mut paths = Vec::new();
    stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
        paths.push(path.clone())
    })?;
    let mut attributes = Vec::new();
    for path in paths {
        let prim = stage.prim(path)?;
        if prim
            .type_name()?
            .is_some_and(|name| name.as_str() == "SkelAnimation")
        {
            for name in ["translations", "rotations", "scales", "blendShapeWeights"] {
                attributes.push(prim.attribute(name));
            }
        }
    }
    ensure!(!attributes.is_empty(), "no SkelAnimation attributes");
    attributes.sort_by(|a, b| a.path().as_str().cmp(b.path().as_str()));
    Ok(attributes)
}

fn inventory(attributes: &[Attribute]) -> Result<()> {
    let mut total_samples = 0usize;
    let mut total_bytes = 0usize;
    for attribute in attributes {
        let samples = attribute.time_samples()?.unwrap_or_default();
        let mut bytes = 0usize;
        for (_, value) in &samples {
            bytes = bytes
                .checked_add(numeric_bytes(value)?.len())
                .context("payload overflow")?;
        }
        let default_bytes = attribute
            .get::<Value>()?
            .as_ref()
            .map(numeric_bytes)
            .transpose()?
            .map_or(0, <[u8]>::len);
        println!(
            "sample_query_inventory path={} samples={} numeric_payload_bytes={bytes} default_numeric_bytes={default_bytes}",
            attribute.path(),
            samples.len()
        );
        total_samples = total_samples
            .checked_add(samples.len())
            .context("sample count overflow")?;
        total_bytes = total_bytes
            .checked_add(bytes)
            .context("total payload overflow")?;
    }
    println!(
        "sample_query_inventory_total attributes={} samples={total_samples} numeric_payload_bytes={total_bytes} excludes=map-slots,keys,capacities,allocator,static-values,query-metadata,stage",
        attributes.len()
    );
    Ok(())
}

fn clock(start: f64, end: f64, iteration: usize, samples: usize) -> f64 {
    let fraction = if iteration < WARMUP {
        [0.0, 0.5, 1.0, 0.5][iteration]
    } else {
        (iteration - WARMUP + 1) as f64 / samples as f64
    };
    start + (end - start) * fraction
}

fn first(iteration: usize, reverse: bool) -> usize {
    usize::from(matches!(iteration % 4, 1 | 2) ^ reverse)
}

fn measure(
    read: impl FnOnce() -> openusd::Result<Vec<Option<Value>>>,
) -> Result<(Duration, Vec<Option<Value>>)> {
    let started = Instant::now();
    let values = read()?;
    Ok((started.elapsed(), values))
}

fn report(values: &[Duration], mode: &str, first: usize) {
    let mut values = values.to_vec();
    values.sort_unstable();
    println!(
        "sample_query_summary mode={mode} first={first} samples={} median_ns={} p95_ns={} max_ns={}",
        values.len(),
        values[values.len().div_ceil(2) - 1].as_nanos(),
        values[(values.len() * 95).div_ceil(100) - 1].as_nanos(),
        values.last().unwrap().as_nanos()
    );
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: sample_query_probe ASSET SAMPLES_OR_inventory REVERSE_0_OR_1"
    );
    let inventory_only = args[1] == "inventory";
    let samples = if inventory_only {
        0
    } else {
        args[1].parse::<usize>()?
    };
    ensure!(
        inventory_only || (4..=1000).contains(&samples) && samples.is_multiple_of(4),
        "samples must be 4..1000 and divisible by four"
    );
    let reverse = match args[2].as_str() {
        "0" => false,
        "1" => true,
        _ => anyhow::bail!("reverse must be 0 or 1"),
    };
    let opening = Instant::now();
    let stage = usd_bevy::UsdSource::from_file(&args[0])?.open_stage()?;
    let attributes = attributes(&stage)?;
    if inventory_only {
        inventory(&attributes)?;
        println!(
            "SAMPLE_QUERY_INVENTORY_COMPLETE attributes={} scope=one-map-at-a-time-no-queries",
            attributes.len()
        );
        return Ok(());
    }
    ensure!(
        attributes.len() <= 512,
        "probe attribute limit exceeded: {} > 512",
        attributes.len()
    );
    let (start, end) = (stage.start_time_code(), stage.end_time_code());
    ensure!(
        start.is_finite() && end.is_finite() && start < end,
        "invalid stage range"
    );
    println!(
        "sample_query_config samples={samples} warmup={WARMUP} reverse={reverse} attributes={} start={start:.17} end={end:.17} opening_ns={} scope=animation-attribute-batch excludes=hashing,comparison,inventory,printing,geometry,rendering",
        attributes.len(),
        opening.elapsed().as_nanos()
    );
    for identifier in stage.layer_identifiers() {
        println!("sample_query_layer identifier={identifier:?}");
    }
    inventory(&attributes)?;
    let queries: Vec<_> = attributes.iter().map(AttributeQuery::new).collect();
    let mut timings: [[Vec<Duration>; 2]; 2] = Default::default();
    for iteration in 0..samples + WARMUP {
        let clock = clock(start, end, iteration, samples);
        let at = Some(TimeCode::new(clock));
        let first = first(iteration, reverse);
        let mut results = [None, None];
        for mode in [first, 1 - first] {
            results[mode] = Some(if mode == 0 {
                measure(|| attributes.iter().map(|a| a.get_at::<Value>(at)).collect())?
            } else {
                measure(|| queries.iter().map(|q| q.get_at::<Value>(at)).collect())?
            });
        }
        let [(direct_ns, direct), (query_ns, query)] = results.map(Option::unwrap);
        same_values(&direct, &query)?;
        let fingerprint = digest(&direct)?;
        ensure!(fingerprint == digest(&query)?, "digest mismatch");
        let phase = if iteration < WARMUP {
            "warmup"
        } else {
            "sample"
        };
        let index = if iteration < WARMUP {
            iteration
        } else {
            iteration - WARMUP
        };
        println!(
            "sample_query_{phase} index={index} first={first} clock={clock:.17} direct_ns={} query_ns={} digest={fingerprint}",
            direct_ns.as_nanos(),
            query_ns.as_nanos()
        );
        if iteration >= WARMUP {
            timings[first][0].push(direct_ns);
            timings[first][1].push(query_ns);
        }
    }
    for (first, [direct, query]) in timings.iter().enumerate() {
        report(direct, "direct", first);
        report(query, "query", first);
    }
    println!(
        "SAMPLE_QUERY_COMPLETE samples={samples} warmup={WARMUP} attributes={}",
        attributes.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_fixture() -> Result<usd_bevy::UsdSource> {
        Ok(usd_bevy::UsdSource::snapshot(
            "query-lifetime.usda",
            br#"#usda 1.0
def Xform "Source" {
    def SkelAnimation "Anim" {
        float3[] translations.timeSamples = { 0: [(0, 0, 0)], 10: [(10, 0, 0)] }
    }
}
def Xform "Inst" (instanceable = true; prepend references = </Source>) {}
"#
            .as_slice(),
        )?)
    }

    fn sample_bytes(attribute: &Attribute) -> Result<usize> {
        attribute
            .time_samples()?
            .unwrap_or_default()
            .iter()
            .try_fold(0usize, |sum, (_, value)| {
                sum.checked_add(numeric_bytes(value)?.len())
                    .context("sample payload overflow")
            })
    }

    fn grow_samples(attribute: &Attribute) -> Result<()> {
        for time in [0.0, 10.0] {
            attribute.clone().set_at(
                Value::Vec3fVec(vec![openusd::gf::vec3f(time as f32, 1.0, 2.0); 4096]),
                TimeCode::new(time),
            )?;
        }
        Ok(())
    }

    fn compare_clocks(attribute: &Attribute, query: &AttributeQuery) -> Result<()> {
        for time in [0.0, 5.0, 10.0, 5.0, -1.0, 11.0] {
            let at = TimeCode::new(time);
            same_values(
                &[attribute.get_at::<Value>(at)?],
                &[query.get_at::<Value>(at)?],
            )?;
        }
        Ok(())
    }

    #[test]
    fn query_refresh_can_exceed_initial_sample_budget() -> Result<()> {
        let stage = source_fixture()?.open_stage()?;
        let attribute = stage.attribute("/Source/Anim.translations")?;
        let query = attribute.query();
        let prim = openusd::sdf::path("/Source/Anim")?;
        compare_clocks(&attribute, &query)?;
        let revision = stage.index_revision(&prim).context("indexed source")?;
        let initial_bytes = sample_bytes(&attribute)?;
        let budget = 1024;
        assert_eq!(initial_bytes, 24);
        assert!(initial_bytes < budget);

        grow_samples(&attribute)?;
        stage.flush()?;
        assert_eq!(stage.index_revision(&prim), Some(revision));
        let edited_bytes = sample_bytes(&attribute)?;
        assert_eq!(edited_bytes, 98_304);
        assert!(edited_bytes > budget);
        compare_clocks(&attribute, &query)?;
        assert_eq!(
            numeric_bytes(&query.get_at::<Value>(TimeCode::new(5.0))?.unwrap())?.len(),
            49_152
        );

        attribute.clone().block()?;
        stage.flush()?;
        compare_clocks(&attribute, &query)?;
        assert!(query.get_at::<Value>(TimeCode::new(5.0))?.is_none());
        Ok(())
    }

    #[test]
    fn proxy_path_revision_does_not_guard_query_readmission() -> Result<()> {
        let stage = source_fixture()?.open_stage()?;
        let prim = openusd::sdf::path("/Inst/Anim")?;
        assert!(stage.prim(prim.clone())?.is_instance_proxy()?);
        let attribute = stage.attribute("/Inst/Anim.translations")?;
        let query = attribute.query();
        compare_clocks(&attribute, &query)?;
        assert_eq!(stage.index_revision(&prim), None);
        assert_eq!(sample_bytes(&attribute)?, 24);

        grow_samples(&stage.attribute("/Source/Anim.translations")?)?;
        stage.flush()?;
        compare_clocks(&attribute, &query)?;
        assert_eq!(stage.index_revision(&prim), None);
        assert_eq!(sample_bytes(&attribute)?, 98_304);
        assert_eq!(
            numeric_bytes(&query.get_at::<Value>(TimeCode::new(5.0))?.unwrap())?.len(),
            49_152
        );
        Ok(())
    }

    #[test]
    fn equal_root_labels_do_not_share_query_state_or_lifetime() -> Result<()> {
        let source = source_fixture()?;
        let stage = source.open_stage()?;
        let other = source.open_stage()?;
        assert!(stage != other);
        assert_eq!(
            stage.root_layer().identifier(),
            other.root_layer().identifier()
        );
        let attribute = stage.attribute("/Source/Anim.translations")?;
        let other_attribute = other.attribute("/Source/Anim.translations")?;
        assert_eq!(attribute.path(), other_attribute.path());
        let query = attribute.query();
        let other_query = other_attribute.query();
        compare_clocks(&attribute, &query)?;
        compare_clocks(&other_attribute, &other_query)?;
        grow_samples(&attribute)?;
        compare_clocks(&attribute, &query)?;
        compare_clocks(&other_attribute, &other_query)?;
        assert_eq!(sample_bytes(&attribute)?, 98_304);
        assert_eq!(sample_bytes(&other_attribute)?, 24);
        assert!(
            same_values(
                &[query.get_at::<Value>(TimeCode::new(5.0))?],
                &[other_query.get_at::<Value>(TimeCode::new(5.0))?],
            )
            .is_err()
        );

        let sentinel = std::rc::Rc::new(());
        let weak = std::rc::Rc::downgrade(&sentinel);
        stage.add_sink(move |_: &Stage, _: &openusd::usd::CommittedChange<'_>| {
            let _keep_alive = &sentinel;
        })?;
        drop(attribute);
        drop(stage);
        assert!(weak.upgrade().is_some());
        assert!(query.get_at::<Value>(TimeCode::new(5.0))?.is_some());
        drop(query);
        assert!(weak.upgrade().is_none());
        compare_clocks(&other_attribute, &other_query)?;
        Ok(())
    }

    #[test]
    fn exact_comparison_detects_sign_kind_and_presence() {
        let zero = [Some(Value::FloatVec(vec![0.0]))];
        for changed in [
            Some(Value::FloatVec(vec![-0.0])),
            Some(Value::DoubleVec(vec![0.0])),
            None,
        ] {
            assert!(same_values(&zero, std::slice::from_ref(&changed)).is_err());
            assert_ne!(digest(&zero).unwrap(), digest(&[changed]).unwrap());
        }
        assert!(numeric_bytes(&Value::String("unsupported".into())).is_err());
    }

    #[test]
    fn clocks_and_orders_cover_unique_samples_in_both_orders() {
        for reverse in [false, true] {
            let order: Vec<_> = (0..4).map(|i| first(i, reverse)).collect();
            assert_eq!(
                order,
                if reverse {
                    vec![1, 0, 0, 1]
                } else {
                    vec![0, 1, 1, 0]
                }
            );
            for i in 0..1000 {
                assert_eq!(clock(0.0, 1000.0, i + WARMUP, 1000), (i + 1) as f64);
            }
        }
        assert_eq!(
            (0..WARMUP)
                .map(|i| clock(0.0, 10.0, i, 1000))
                .collect::<Vec<_>>(),
            vec![0.0, 5.0, 10.0, 5.0]
        );
    }

    #[test]
    fn query_matches_direct_across_interpolation_and_edits() -> Result<()> {
        let stage = usd_bevy::UsdSource::snapshot(
            "query-probe.usda",
            br#"#usda 1.0
def SkelAnimation "Anim" {
    float3[] translations = [(7, 0, 0)]
    float3[] translations.timeSamples = { 0: [(0, 0, 0)], 10: [(10, 0, 0)] }
}
"#
            .as_slice(),
        )?
        .open_stage()?;
        let attribute = stage.attribute("/Anim.translations")?;
        let query = AttributeQuery::new(&attribute);
        for edited in [false, true] {
            if edited {
                attribute.clone().set_at(
                    Value::Vec3fVec(vec![openusd::gf::vec3f(20.0, 0.0, 0.0)]),
                    TimeCode::new(10.0),
                )?;
            }
            for interpolation in [
                openusd::usd::InterpolationType::Linear,
                openusd::usd::InterpolationType::Held,
            ] {
                stage.set_interpolation(interpolation);
                for time in [
                    None,
                    Some(0.0),
                    Some(5.0),
                    Some(10.0),
                    Some(5.0),
                    Some(-1.0),
                ] {
                    let at = time.map(TimeCode::new);
                    same_values(
                        &[attribute.get_at::<Value>(at)?],
                        &[query.get_at::<Value>(at)?],
                    )?;
                }
            }
        }
        Ok(())
    }
}
