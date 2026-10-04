use std::{io::Write, path::Path, time::Instant};

fn memory() -> String {
    std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect::<Vec<_>>().join(" ")
}

fn write_fixture(path: &Path, mib: usize, size: u32) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "#usda 1.0\ndef Cube \"Model\" {{ double size = {size} }}\n#")?;
    let block = vec![b'#'; 64 * 1024];
    for _ in 0..mib * 16 { file.write_all(&block)?; }
    writeln!(file)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 { return Err("usage: reload_edit_benchmark COMMENT_MIB (1..256)".into()); }
    let mib: usize = args[0].parse()?;
    if !(1..=256).contains(&mib) { return Err("comment size must be 1..256 MiB".into()); }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("root.usda");
    write_fixture(&path, mib, 1)?;
    let mut editor = usd_bevy::editor::EditorSession::from_source(usd_bevy::UsdSource::from_file(&path)?)?;
    let document = editor.document_id();
    write_fixture(&path, mib, 2)?;
    eprintln!("phase=before-reload {}", memory());
    let start = Instant::now();
    editor.reload_sources()?;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(editor.document_id(), document);
    assert_eq!(editor.stage().prim("/Model")?.attribute("size").get::<f64>()?, Some(2.0));
    println!("reload_ms={elapsed:.3} comment_mib={mib} {} scope=synthetic-large-source-small-edit excludes=fixture-generation,initial-open,projection,gpu,ui", memory());
    Ok(())
}
