use std::time::Instant;

fn memory() -> String {
    std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect::<Vec<_>>().join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 { return Err("usage: reload_benchmark ASSET".into()); }
    let path = std::fs::canonicalize(&args[0])?;
    let source = usd_bevy::UsdSource::from_file(&path)?;
    let mut editor = usd_bevy::editor::EditorSession::from_source(source)?;
    let document = editor.document_id();
    eprintln!("phase=before-reload {}", memory());
    for sample in 0..3 {
        let started = Instant::now();
        editor.reload_sources()?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(editor.document_id(), document);
        println!("sample={sample} reload_ms={elapsed:.3} {} scope=unchanged-disk-verification excludes=projection,gpu,ui", memory());
    }
    Ok(())
}
