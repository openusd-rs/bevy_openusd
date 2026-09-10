use std::time::{Duration, Instant};
use usd_bevy::UsdSource;

fn assemble(root: &UsdSource, model: &UsdSource, paths: &[String], batch: bool) -> (UsdSource, Duration) {
    let start = Instant::now();
    let source = if batch {
        root.with_references(paths.iter().map(|path| (path.as_str(), model, "/Model"))).unwrap()
    } else {
        paths.iter().fold(root.clone(), |source, path| source.with_reference(path.as_str(), model, "/Model").unwrap())
    };
    (source, start.elapsed())
}

fn verify(source: &UsdSource, paths: &[String]) {
    let stage = source.open_stage().unwrap();
    for path in paths {
        let prim = stage.prim(path.as_str()).unwrap();
        assert_eq!(prim.type_name().unwrap().as_deref(), Some("Cube"));
        assert_eq!(prim.attribute("size").get::<f64>().unwrap(), Some(2.0));
    }
    assert_eq!(source.dependencies().count(), 1);
}

fn measure(count: usize, samples: usize) {
    let root = UsdSource::snapshot("reference-benchmark/root.usda", &b"#usda 1.0\n"[..]).unwrap();
    let model = UsdSource::snapshot("reference-benchmark/model.usda",
        &b"#usda 1.0\ndef Cube \"Model\" { double size = 2 }\n"[..]).unwrap();
    let paths: Vec<_> = (0..count).map(|index| format!("/Mount{index}")).collect();
    for batch in [false, true] { verify(&assemble(&root, &model, &paths, batch).0, &paths); }
    for sample in 0..samples {
        let mut outputs = Vec::new();
        for batch in if sample % 2 == 0 { [false, true] } else { [true, false] } {
            let (source, elapsed) = assemble(&root, &model, &paths, batch);
            verify(&source, &paths);
            let bytes = source.open_stage().unwrap().root_layer().export_to_string().unwrap();
            println!("{count},{sample},{},{:.3},{}", if batch { "batch" } else { "sequential" }, elapsed.as_secs_f64() * 1000.0, bytes.len());
            outputs.push(bytes);
        }
        assert_eq!(outputs[0], outputs[1]);
    }
    assert_eq!(root.dependencies().count(), 0);
    assert!(!root.open_stage().unwrap().prim("/Mount0").unwrap().is_valid().unwrap());
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.len() <= 2, "usage: reference_benchmark [MOUNTS] [SAMPLES]");
    let count = args.first().map(|value| value.parse::<usize>().expect("integer mount count")).unwrap_or(32);
    let samples = args.get(1).map(|value| value.parse::<usize>().expect("integer sample count")).unwrap_or(3);
    assert!((1..=1024).contains(&count), "mount count must be 1..=1024");
    assert!((1..=100).contains(&samples), "sample count must be 1..=100");
    println!("profile={} source=captured model=shared-cube warmups=1-per-mode order=alternating excludes=input-generation,verification,bevy-projection,gpu,disk-io", if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("mounts,sample,mode,assembly_ms,root_bytes");
    measure(count, samples);
}

#[test]
fn reference_benchmark_verifies_both_assembly_paths() {
    measure(3, 2);
}
