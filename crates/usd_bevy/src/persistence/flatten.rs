use anyhow::{Result, ensure};
use openusd::{sdf, usd::{PrimPredicate, Stage}};
use std::collections::BTreeMap;

pub(crate) fn preserving_instances(stage: &Stage) -> Result<sdf::Layer> {
    let mut paths = Vec::new();
    stage.traverse(PrimPredicate::ALL, |path| paths.push(path.clone()))?;
    let mut groups: BTreeMap<sdf::Path, Vec<sdf::Path>> = BTreeMap::new();
    for path in &paths {
        if let Some(prototype) = stage.prim(path)?.prototype()? {
            groups.entry(prototype).or_default().push(path.clone());
        }
    }
    let mut layer = stage.flatten()?;
    bake_clips(stage, &paths, &mut layer)?;
    let mut serial = 0;
    while !groups.is_empty() {
        let next = groups.iter().find(|(prototype, instances)| !groups.iter().any(|(other, children)|
            other != *prototype && children.iter().any(|child| instances.iter().any(|instance|
                child != instance && child.has_prefix(instance))))).map(|(prototype, _)| prototype.clone());
        ensure!(next.is_some(), "cyclic native instance prototype dependencies");
        let instances = groups.remove(&next.unwrap()).unwrap();
        let destination = loop {
            serial += 1;
            let path = sdf::path(format!("/Flattened_Prototype_{serial}"))?;
            if layer.prim(&path)?.is_none() { break path; }
        };
        layer.edit(|edit| {
            sdf::copy_spec_within(edit.data_mut(), &instances[0], &destination)?;
            let properties = edit.prim(&destination)?.unwrap().property_children().unwrap_or_default();
            for property in properties { edit.remove_spec(&destination.append_property(property.as_str())?)?; }
            for field in edit.data().list_fields(&destination).unwrap_or_default() {
                if field != sdf::ChildrenKey::PrimChildren.as_str() {
                    edit.data_mut().erase_field(&destination, &field);
                }
            }
            edit.prim_mut(&destination)?.unwrap().set_specifier(sdf::Specifier::Over);
            for instance in &instances {
                let children = edit.prim(instance)?.unwrap().prim_children().unwrap_or_default();
                for child in children { edit.remove_spec(&instance.append_path(child.as_str())?)?; }
                edit.prim_mut(instance)?.unwrap().set_instanceable(true);
                edit.data_mut().set_field(instance, sdf::FieldKey::References.as_str(),
                    sdf::Value::ReferenceListOp(sdf::ReferenceListOp::explicit(vec![sdf::Reference {
                        prim_path: destination.clone(), ..Default::default()
                    }])));
            }
            Ok(())
        })?;
    }
    Ok(layer)
}

fn bake_clips(stage: &Stage, paths: &[sdf::Path], layer: &mut sdf::Layer) -> Result<()> {
    let mut total = 0_usize;
    for path in paths {
        for attribute in stage.prim(path)?.attributes()? {
            if attribute.resolve_info()?.source() != openusd::usd::ResolveInfoSource::ValueClips { continue; }
            let type_name = attribute.type_name()?.unwrap_or(sdf::ValueTypeName::TOKEN);
            ensure!(!matches!(type_name.as_str(), "timecode" | "timecode[]"),
                "flattened save does not support timecode clip values at {}", attribute.path());
            let times = attribute.time_sample_times()?;
            total = total.checked_add(times.len().saturating_mul(2)).ok_or_else(|| anyhow::anyhow!("clip bake sample budget exceeded"))?;
            ensure!(total <= 1_000_000, "clip bake sample budget exceeded");
            let mut samples = Vec::new();
            for time in times.into_iter().flat_map(|time| [time.next_down(), time]).filter(|time| time.is_finite()) {
                let time_code = Some(openusd::usd::TimeCode::new(time));
                let value = if attribute.resolve_info_at(time_code)?.value_is_blocked() { sdf::Value::ValueBlock }
                    else { attribute.get_at::<sdf::Value>(time_code)?.unwrap_or(sdf::Value::ValueBlock) };
                let mut unresolved = false;
                let value = value.map_asset_paths(&mut |asset| {
                    if let Some(resolved) = asset.resolved_path() {
                        sdf::AssetPath::with_resolved_path(resolved, resolved)
                    } else {
                        unresolved |= !asset.is_empty();
                        asset
                    }
                });
                ensure!(!unresolved, "unresolved clip asset at {} at time {time}", attribute.path());
                samples.push((time, value));
            }
            let property = attribute.path().clone();
            let variability = attribute.variability()?.unwrap_or_default();
            let custom = attribute.is_custom()?;
            layer.edit(|edit| {
                if edit.attribute(&property)?.is_none() {
                    sdf::AttributeSpec::new(edit.data_mut(), &property, type_name, variability, custom)?;
                }
                edit.data_mut().set_field(&property, sdf::FieldKey::TimeSamples.as_str(),
                    sdf::Value::TimeSamples(sdf::normalize_time_samples(samples)));
                Ok(())
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn baked_clip_assets_survive_package_relocation() { verify_baked_clip_assets(false); }

    pub(crate) fn verify_baked_clip_assets(native: bool) {
        let input = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::snapshot(input.path().join("root.usda"), br#"#usda 1.0
def "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@a.usda@, @b.usda@]
            double2[] active = [(0, 0), (10, 1)]
            string primPath = "/Model"
        }
    }
) {
    asset image
}
"#.as_slice()).unwrap();
        let mut source = source;
        let mut incomplete = None;
        for name in ["a", "b"] {
            let clip = crate::UsdSource::snapshot(input.path().join(format!("{name}.usda")), format!(
                "#usda 1.0\ndef \"Model\" {{\n    asset image.timeSamples = {{0: @{name}.bin@}}\n}}\n"
            ).into_bytes()).unwrap();
            let image = crate::UsdSource::snapshot(input.path().join(format!("{name}.bin")), name.as_bytes().to_vec()).unwrap();
            if name == "b" { incomplete = Some(source.with_dependency(&clip).unwrap()); }
            source = source.with_dependency(&clip).unwrap().with_dependency(&image).unwrap();
        }
        let editor = crate::editor::EditorSession::new(source.open_stage().unwrap());
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let output = tempfile::tempdir().unwrap();
        let relocated = tempfile::tempdir().unwrap();
        let original = output.path().join("assets.usdz");
        let rejected = output.path().join("unchanged.usdz");
        std::fs::write(&rejected, b"existing output").unwrap();
        let missing = crate::editor::EditorSession::new(incomplete.unwrap().open_stage().unwrap());
        let error = missing.save(rejected.to_str().unwrap(), crate::editor::SaveMode::Flattened).unwrap_err();
        assert!(error.to_string().contains("unresolved"), "{error:#}");
        assert_eq!(std::fs::read(rejected).unwrap(), b"existing output");
        editor.save(original.to_str().unwrap(), crate::editor::SaveMode::Flattened).unwrap();
        let moved = relocated.path().join("assets.usdz");
        std::fs::rename(original, &moved).unwrap();
        let reopened = if native {
            let executable = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
            let result = std::process::Command::new(executable).arg("--flatten").arg(&moved).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stderr.is_empty(), "{}", String::from_utf8_lossy(&result.stderr));
            crate::UsdSource::new(relocated.path().join("native.usda"), result.stdout).unwrap()
        } else { crate::UsdSource::new(&moved, std::fs::read(&moved).unwrap()).unwrap() };
        let stage = reopened.open_stage().unwrap();
        assert!(stage.prim("/Model").unwrap().get_metadata::<sdf::Value>("clips").unwrap().is_none());
        for (time, expected) in [(0.0, "a"), (5.0, "a"), (10.0, "b"), (20.0, "b")] {
            let asset = stage.attribute("/Model.image").unwrap().get_at::<sdf::AssetPath>(Some(openusd::usd::TimeCode::new(time))).unwrap().unwrap();
            let resolved = asset.resolved_path().unwrap();
            assert!(openusd::ar::is_package_relative_path(resolved), "{resolved}");
            assert_eq!(reopened.read_asset(resolved).unwrap(), expected.as_bytes());
        }
        assert_eq!(std::fs::read_dir(input.path()).unwrap().count(), 0);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn clip_baking_preserves_interpolation_and_value_blocks() {
        verify_baked_clips(false);
    }

    pub(crate) fn verify_baked_clips(native: bool) {
        let input = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::snapshot(input.path().join("root.usda"), br#"#usda 1.0
def Sphere "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@a.usda@, @b.usda@]
            double2[] active = [(0, 0), (10, 1)]
            string primPath = "/Model"
        }
    }
) {
    double radius
    float strength
}
"#.as_slice()).unwrap();
        let a = crate::UsdSource::snapshot(input.path().join("a.usda"), br#"#usda 1.0
def Sphere "Model" {
    double radius.timeSamples = {0: 1, 20: 3}
    float strength.timeSamples = {50: 50}
}
"#.as_slice()).unwrap();
        let b = crate::UsdSource::snapshot(input.path().join("b.usda"), br#"#usda 1.0
def Sphere "Model" {
    double radius.timeSamples = {0: 5, 20: 7}
}
"#.as_slice()).unwrap();
        let source = source.with_dependency(&a).unwrap().with_dependency(&b).unwrap();
        let editor = crate::editor::EditorSession::new(source.open_stage().unwrap());
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let output = tempfile::tempdir().unwrap();
        let relocated = tempfile::tempdir().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let original = output.path().join(format!("flat.{extension}"));
            editor.save(original.to_str().unwrap(), crate::editor::SaveMode::Flattened).unwrap();
            let moved = relocated.path().join(format!("flat.{extension}"));
            std::fs::rename(original, &moved).unwrap();
            let stage = if native {
                let executable = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
                let result = std::process::Command::new(executable).arg("--flatten").arg(&moved).output().unwrap();
                assert!(result.status.success(), "{extension}: {}", String::from_utf8_lossy(&result.stderr));
                assert!(result.stderr.is_empty(), "{extension}: {}", String::from_utf8_lossy(&result.stderr));
                crate::UsdSource::snapshot(relocated.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap()
            } else {
                crate::UsdSource::new(&moved, std::fs::read(&moved).unwrap()).unwrap().open_stage().unwrap()
            };
            assert!(stage.prim("/Model").unwrap().get_metadata::<sdf::Value>("clips").unwrap().is_none());
            for time in [-1.0, 0.0, 5.0, 9.0, 10.0, 15.0, 20.0, 21.0] {
                let time = Some(openusd::usd::TimeCode::new(time));
                let expected = editor.stage().attribute("/Model.radius").unwrap().get_at::<f64>(time).unwrap().unwrap();
                let actual = stage.attribute("/Model.radius").unwrap().get_at::<f64>(time).unwrap().unwrap();
                assert!((actual - expected).abs() < 1e-10, "{extension}: {actual} != {expected}");
                assert_eq!(stage.attribute("/Model.strength").unwrap().get_at::<sdf::Value>(time).unwrap(),
                    editor.stage().attribute("/Model.strength").unwrap().get_at::<sdf::Value>(time).unwrap(), "{extension}");
            }
        }
        assert_eq!(std::fs::read_dir(input.path()).unwrap().count(), 0);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn nested_retimed_instances_preserve_paths_values_and_root_opinions() {
        verify_nested_retimed_instances(false);
    }

    pub(crate) fn verify_nested_retimed_instances(native: bool) {
        let source = crate::UsdSource::snapshot("nested-flat.usda", br#"#usda 1.0
over "Flattened_Prototype_1" { string marker = "existing" }
class Xform "Leaf" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@nested-values.usda@]
            double2[] active = [(0, 0)]
            string primPath = "/Leaf"
        }
    }
) {
    float weight = 2
    def Cube "Geometry" {
        double size
        rel driver = </Leaf.weight>
    }
}
class Xform "Model" {
    def Xform "Nested" (instanceable = true; prepend references = </Leaf>) {
        float weight = 4
    }
}
def Xform "A" (instanceable = true; prepend references = </Model>) {
    double3 xformOp:translate = (-4, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "B" (instanceable = true; prepend references = </Model>) {
    double3 xformOp:translate = (0, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "C" (instanceable = true; prepend references = </Model> (offset = 10; scale = 2)) {
    double3 xformOp:translate = (4, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#.as_slice()).unwrap();
        let values = crate::UsdSource::snapshot("nested-values.usda", br#"#usda 1.0
def Xform "Leaf" {
    def Cube "Geometry" {
        double size.timeSamples = {0: 1, 10: 3}
    }
}
"#.as_slice()).unwrap();
        let source = source.with_dependency(&values).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let flat = preserving_instances(&stage).unwrap();
        let bytes = if native {
            let output = tempfile::tempdir().unwrap();
            let relocated = tempfile::tempdir().unwrap();
            let original = output.path().join("nested.usdz");
            crate::persistence::export_layer(&stage, &flat, original.to_str().unwrap()).unwrap();
            let moved = relocated.path().join("nested.usdz");
            std::fs::rename(original, &moved).unwrap();
            let executable = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
            let result = std::process::Command::new(executable).arg("--flatten").arg(moved).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stderr.is_empty(), "{}", String::from_utf8_lossy(&result.stderr));
            result.stdout
        } else { flat.export_to_string().unwrap().into_bytes() };
        let reopened = crate::UsdSource::snapshot("nested-output.usda", bytes).unwrap().open_stage().unwrap();
        let visible = |stage: &Stage| {
            let mut paths = Vec::new();
            stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| paths.push(path.clone())).unwrap();
            paths
        };
        assert_eq!(visible(&reopened), visible(&stage));
        assert_eq!(reopened.attribute("/Flattened_Prototype_1.marker").unwrap().get::<String>().unwrap().as_deref(), Some("existing"));
        for root in ["/A", "/B", "/C"] {
            assert!(reopened.prim(root).unwrap().is_instance().unwrap());
            let nested = format!("{root}/Nested");
            assert!(reopened.prim(&nested).unwrap().is_instance().unwrap(), "{nested}");
            for property in [format!("{root}.xformOp:translate"), format!("{nested}.weight"), format!("{nested}/Geometry.size")] {
                let original = stage.attribute(property.as_str()).unwrap();
                let output = reopened.attribute(property.as_str()).unwrap();
                assert_eq!(output.get::<sdf::Value>().unwrap(), original.get::<sdf::Value>().unwrap(), "{property}");
                for time in original.time_sample_times().unwrap() {
                    assert!(output.time_sample_times().unwrap().contains(&time), "{property}: missing sample {time}");
                }
                for time in [0.0, 10.0, 20.0, 30.0] {
                    let time = Some(openusd::usd::TimeCode::new(time));
                    assert_eq!(output.get_at::<sdf::Value>(time).unwrap(), original.get_at::<sdf::Value>(time).unwrap(), "{property}");
                }
            }
            let geometry = format!("{nested}/Geometry");
            assert_eq!(reopened.prim(&geometry).unwrap().relationship("driver").targets().unwrap(), stage.prim(&geometry).unwrap().relationship("driver").targets().unwrap());
        }
        assert_eq!(reopened.prim("/A").unwrap().prototype().unwrap(), reopened.prim("/B").unwrap().prototype().unwrap());
        assert_ne!(reopened.prim("/A").unwrap().prototype().unwrap(), reopened.prim("/C").unwrap().prototype().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }
}
