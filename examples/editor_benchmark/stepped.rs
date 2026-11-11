use std::{
    ffi::OsStr,
    io::{self, BufRead, Read, Write},
};

pub(super) fn configured(
    value: Option<&OsStr>,
    samples: usize,
    timeline: bool,
) -> io::Result<bool> {
    match value {
        None => Ok(false),
        Some(value) if value == "1" && samples == 1 && timeline => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "USD_BENCH_STEPPED=1 requires one sample and seek-timeline",
        )),
    }
}

pub(super) fn await_step(
    reader: &mut dyn BufRead,
    writer: &mut impl Write,
    iteration: usize,
) -> io::Result<()> {
    writeln!(writer, "seek_step_ready iteration={iteration}")?;
    writer.flush()?;
    let mut command = String::new();
    if reader.take(64).read_line(&mut command)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "missing seek step",
        ));
    }
    if !command.ends_with('\n') || command.trim().parse::<usize>().ok() != Some(iteration) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "seek step index mismatch",
        ));
    }
    Ok(())
}

#[test]
fn step_configuration_rejects_ambiguous_runs() {
    assert!(!configured(None, 3, false).unwrap());
    assert!(configured(Some(OsStr::new("1")), 1, true).unwrap());
    for (value, samples, timeline) in [
        ("0", 1, true),
        ("", 1, true),
        ("1", 2, true),
        ("1", 1, false),
    ] {
        assert!(configured(Some(OsStr::new(value)), samples, timeline).is_err());
    }
}

#[test]
fn step_protocol_flushes_and_requires_every_index() {
    struct Output {
        bytes: Vec<u8>,
        flushed: bool,
    }
    impl Write for Output {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }
    let mut output = Output {
        bytes: Vec::new(),
        flushed: false,
    };
    let mut input = io::Cursor::new(b"0\n1\r\n");
    await_step(&mut input, &mut output, 0).unwrap();
    assert!(output.flushed);
    await_step(&mut input, &mut output, 1).unwrap();
    assert_eq!(
        output.bytes,
        b"seek_step_ready iteration=0\nseek_step_ready iteration=1\n"
    );
    assert_eq!(
        await_step(&mut input, &mut output, 2).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    for invalid in ["1\n", "0", "0 1\n", "-1\n", "NaN\n", "\n", &"0".repeat(65)] {
        assert!(await_step(&mut io::Cursor::new(invalid), &mut Vec::new(), 0).is_err());
    }
}

#[test]
fn stepped_editor_preserves_timeline_payloads_and_rejects_missing_steps() {
    use super::{SEEK_WARMUP, SeekMode, measure, measure_with_steps, paired};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/skel_morph_blended_animated.usda");
    let mode = SeekMode::Timeline;
    let commands: String = (0..mode.samples() + SEEK_WARMUP)
        .map(|index| format!("{index}\n"))
        .collect();
    let mut input = io::Cursor::new(commands.as_bytes());
    let stepped = measure_with_steps(&path, true, mode, Some(&mut input)).unwrap();
    assert_eq!(input.position() as usize, commands.len());
    let ordinary = measure(&path, true, mode).unwrap();
    assert_eq!(
        paired::payload_counts(&stepped),
        paired::payload_counts(&ordinary)
    );
    assert_eq!(stepped.peak_asset_counts, ordinary.peak_asset_counts);
    assert!(stepped.seek_median > std::time::Duration::ZERO);
    assert!(stepped.seek_median <= stepped.seek_p95 && stepped.seek_p95 <= stepped.seek_max);
    assert!(measure_with_steps(&path, true, mode, Some(&mut io::Cursor::new(b""))).is_err());
    assert!(
        measure_with_steps(&path, true, SeekMode::Idle, Some(&mut io::Cursor::new(b""))).is_err()
    );
}
