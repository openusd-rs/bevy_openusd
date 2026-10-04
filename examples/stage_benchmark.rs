use std::time::Instant;

fn memory() -> String {
    std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect::<Vec<_>>().join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 2 || args.get(1).is_some_and(|value| value != "proxies") {
        return Err("usage: stage_benchmark ASSET [proxies]".into());
    }
    let path = std::fs::canonicalize(&args[0])?;
    let started = Instant::now();
    let source = usd_bevy::UsdSource::from_file(&path)?;
    eprintln!("phase=source elapsed_ms={:.3} {}", started.elapsed().as_secs_f64() * 1000.0, memory());
    let stage = source.open_stage()?;
    let opened = started.elapsed();
    eprintln!("phase=open elapsed_ms={:.3} {}", opened.as_secs_f64() * 1000.0, memory());
    let proxies = args.len() == 2;
    let predicate = if proxies { openusd::usd::PrimPredicate::DEFAULT_PROXIES } else { openusd::usd::PrimPredicate::DEFAULT };
    let mut count = 0_u64;
    stage.traverse(predicate, |_| { count += 1; })?;
    let traversed = started.elapsed();
    let diagnostics = stage.composition_errors();
    println!("asset={} profile={} instance_proxies={} prims={} layers={} open_ms={:.3} traverse_ms={:.3} total_ms={:.3} {} excludes=bevy-projection,textures,gpu,ui",
        path.display(), if cfg!(debug_assertions) { "debug" } else { "release" }, proxies,
        count, stage.layer_identifiers().len(), opened.as_secs_f64() * 1000.0,
        (traversed - opened).as_secs_f64() * 1000.0, traversed.as_secs_f64() * 1000.0, memory());
    for diagnostic in diagnostics.iter().take(10) { eprintln!("composition_diagnostic={diagnostic:?}"); }
    if !diagnostics.is_empty() { return Err(format!("{} composition diagnostics", diagnostics.len()).into()); }
    Ok(())
}
