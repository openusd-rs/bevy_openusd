use anyhow::{Result, ensure};
use openusd::{sdf, usd::{PrimPredicate, Stage}};
use std::collections::BTreeMap;

pub(crate) fn preserving_instances(stage: &Stage) -> Result<sdf::Layer> {
    let mut paths = Vec::new();
    stage.traverse(PrimPredicate::ALL, |path| paths.push(path.clone()))?;
    let mut groups: BTreeMap<sdf::Path, Vec<sdf::Path>> = BTreeMap::new();
    for path in paths {
        if let Some(prototype) = stage.prim(&path)?.prototype()? {
            groups.entry(prototype).or_default().push(path);
        }
    }
    let mut layer = stage.flatten()?;
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

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn nested_retimed_instances_preserve_paths_values_and_root_opinions() {
        verify_nested_retimed_instances(false);
    }

    pub(crate) fn verify_nested_retimed_instances(native: bool) {
        let source = crate::UsdSource::snapshot("nested-flat.usda", br#"#usda 1.0
over "Flattened_Prototype_1" { string marker = "existing" }
class Xform "Leaf" {
    float weight = 2
    def Cube "Geometry" {
        double size.timeSamples = {0: 1, 10: 3}
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
                assert_eq!(output.time_sample_times().unwrap(), original.time_sample_times().unwrap(), "{property}");
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
