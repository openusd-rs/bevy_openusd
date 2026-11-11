use std::{env, fs};

fn number_after(line: &str, prefix: &str) -> Option<u32> {
    let value = line.split_once(prefix)?.1;
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    value[..digits].parse().ok()
}

fn verify(text: &str) -> Result<(usize, usize), String> {
    if !text.lines().any(|line| line == "morph_probe_omitted=0")
        || text.contains("morph_probe_text_truncated=true")
        || text.contains("source_text_truncated=true")
    {
        return Err("probe history is missing, omitted or truncated".into());
    }
    let mut collecting = false;
    let mut latest = false;
    let mut pending = None;
    let mut history_inputs = 0;
    let mut final_inputs = 0;
    for (index, line) in text.lines().enumerate() {
        if line.starts_with("morph_probe_event ") || line.starts_with("morph_probe_latest ") {
            collecting = line.ends_with("phase=after-collection");
            latest = line.starts_with("morph_probe_latest ");
            pending = None;
        } else if collecting && line.starts_with("render_entity=") {
            pending = (line.contains("extracted_weights=true")
                && line.contains("source_asset_ready=true")
                && line.contains("source_target_range=Some("))
            .then(|| number_after(line, "descriptor_index=Some(MorphDescriptorIndex("));
        } else if collecting
            && line.starts_with("instance_builder=")
            && let Some(expected) = pending.take()
        {
            if line.ends_with("input=None") {
                continue;
            }
            let actual = number_after(line, "morph_descriptor_index: ");
            if expected.is_none() || expected == Some(u32::MAX) || expected != actual {
                return Err(format!(
                    "line {}: collected morph descriptor {actual:?}, expected {expected:?}",
                    index + 1
                ));
            }
            if latest {
                final_inputs += 1;
            } else {
                history_inputs += 1;
            }
        }
    }
    if history_inputs == 0 || final_inputs == 0 {
        return Err("requires visible GPU morph inputs in history and final collection".into());
    }
    Ok((history_inputs, final_inputs))
}

fn main() -> Result<(), String> {
    let paths: Vec<_> = env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: morph_publication_check CAPTURE.txt ...".into());
    }
    for path in paths {
        let bytes = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if bytes > 4 * 1024 * 1024 {
            return Err(format!("capture report exceeds 4 MiB: {path}"));
        }
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let (history, final_inputs) = verify(&text).map_err(|e| format!("{path}: {e}"))?;
        println!(
            "MORPH_PUBLICATION_OK history_inputs={history} final_inputs={final_inputs} {path}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(descriptor: Option<u32>, input: u32) -> String {
        let descriptor = descriptor.map_or("None".into(), |index| {
            format!("Some(MorphDescriptorIndex({index}))")
        });
        format!(
            "render_entity=MainEntity(1v0) extracted_weights=true instance_asset=Some(mesh) source_asset_ready=true source_target_range=Some(0..3) descriptor_index={descriptor}\ninstance_builder=gpu input_index=Some(1) input=Some(MeshInputUniform {{ morph_descriptor_index: {input}, metadata_index: 0 }})\n"
        )
    }

    fn report(first: &str, final_draw: &str) -> String {
        format!(
            "morph_probe_omitted=0\nmorph_probe_event frame=66 phase=after-collection\n{first}morph_probe_latest phase=after-collection\n{final_draw}"
        )
    }

    #[test]
    fn rejects_first_frame_sentinel_even_if_latest_is_repaired() {
        for descriptor in [None, Some(1)] {
            let invalid = draw(descriptor, u32::MAX);
            assert!(verify(&report(&invalid, &draw(Some(1), 1))).is_err());
        }
    }

    #[test]
    fn requires_matching_descriptors_for_every_collected_input() {
        let valid = draw(Some(7), 7);
        assert_eq!(verify(&report(&valid, &valid)), Ok((1, 1)));
        assert!(verify(&report(&valid, &draw(Some(8), 7))).is_err());
        assert!(verify(&report(&draw(Some(u32::MAX), u32::MAX), &valid)).is_err());
        let many = format!("{valid}{}", draw(Some(8), 8));
        assert_eq!(verify(&report(&many, &many)), Ok((2, 2)));
    }

    #[test]
    fn uncollected_and_preparation_states_are_not_draws() {
        let waiting = "render_entity=MainEntity(1v0) extracted_weights=true source_asset_ready=true source_target_range=Some(0..3) descriptor_index=None\ninstance_builder=gpu input_index=None input=None\n";
        let valid = draw(Some(1), 1);
        let history = format!(
            "{waiting}morph_probe_event frame=67 phase=after-descriptors\n{}morph_probe_event frame=67 phase=after-collection\n{valid}",
            draw(None, u32::MAX)
        );
        assert_eq!(verify(&report(&history, &valid)), Ok((1, 1)));
        assert!(verify(&report(waiting, &valid)).is_err());
        assert!(verify(&report(&valid, waiting)).is_err());
    }

    #[test]
    fn refuses_incomplete_evidence() {
        let valid = draw(Some(1), 1);
        let text = report(&valid, &valid);
        assert!(verify("").is_err());
        assert!(verify(&text.replace("morph_probe_omitted=0", "morph_probe_omitted=1")).is_err());
        for marker in [
            "morph_probe_text_truncated=true",
            "source_text_truncated=true",
        ] {
            assert!(verify(&format!("{text}{marker}\n")).is_err());
        }
    }
}
