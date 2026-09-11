//! Composed curve widths and orientation normals.

use openusd::{sdf::{Path, Value}, usd::{Stage, TimeCode}};
use super::geom::{Interpolation, MeshPrimvar};

fn field(stage: &Stage, path: &Path, name: &str, time: Option<f64>)
    -> anyhow::Result<Option<(Value, Interpolation, Vec<i32>)>> {
    anyhow::ensure!(time.is_none_or(f64::is_finite), "curve sample time must be finite");
    let primvar = format!("primvars:{name}");
    let owner = super::geom::inherited_primvar_owner(stage, path, &primvar)?;
    let attribute = stage.prim(owner.clone())?.attribute(&primvar);
    let sample = attribute.get_at::<Value>(time.map(TimeCode::new))?;
    let (attribute, value, indexed) = if let Some(value) = sample {
        (attribute, value, true)
    } else {
        let attribute = stage.prim(path.clone())?.attribute(name);
        let Some(value) = attribute.get_at::<Value>(time.map(TimeCode::new))? else { return Ok(None) };
        (attribute, value, false)
    };
    let interpolation = {
        let token = match attribute.get_metadata::<Value>("interpolation")? {
            Some(Value::Token(value)) => value.to_string(),
            Some(Value::String(value)) => value,
            None => if indexed { "constant".into() } else { "vertex".into() },
            _ => anyhow::bail!("invalid {name} interpolation type"),
        };
        match token.as_str() {
            "constant" => Interpolation::Constant,
            "uniform" => Interpolation::Uniform,
            "vertex" => Interpolation::Vertex,
            "varying" => Interpolation::Varying,
            _ => anyhow::bail!("unsupported {name} interpolation {token}"),
        }
    };
    let indices = if indexed {
        match stage.prim(owner)?.attribute(format!("{primvar}:indices")).get_at::<Value>(time.map(TimeCode::new))? {
            Some(Value::IntVec(values)) => values,
            None => Vec::new(),
            _ => anyhow::bail!("invalid {name} indices type"),
        }
    } else { Vec::new() };
    Ok(Some((value, interpolation, indices)))
}

fn validate_indices(indices: &[i32], count: usize, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(indices.iter().all(|index| usize::try_from(*index).is_ok_and(|index| index < count)),
        "{name} index outside its value array");
    Ok(())
}

/// Reads object-space diameters, preferring composed primvars:widths over widths.
/// Missing values remain absent; cardinality depends on the curve's topology.
pub fn read_widths_at(stage: &Stage, path: &Path, time: Option<f64>) -> anyhow::Result<Option<MeshPrimvar<f32>>> {
    let Some((value, interpolation, indices)) = field(stage, path, "widths", time)? else { return Ok(None) };
    let Value::FloatVec(values) = value else { anyhow::bail!("curve widths must be float[]") };
    anyhow::ensure!(values.iter().all(|value| value.is_finite() && *value >= 0.0), "curve widths must be finite and nonnegative");
    validate_indices(&indices, values.len(), "widths")?;
    Ok(Some(MeshPrimvar { values, interpolation, indices }))
}

/// Reads curve orientation normals without normalization or topology expansion.
pub fn read_normals_at(stage: &Stage, path: &Path, time: Option<f64>) -> anyhow::Result<Option<MeshPrimvar<[f32; 3]>>> {
    let Some((value, interpolation, indices)) = field(stage, path, "normals", time)? else { return Ok(None) };
    let Value::Vec3fVec(values) = value else { anyhow::bail!("curve normals must be normal3f[]") };
    let values = values.into_iter().map(|value| [value.x, value.y, value.z]).collect::<Vec<_>>();
    anyhow::ensure!(values.iter().flatten().all(|value| value.is_finite()), "curve normals must be finite");
    validate_indices(&indices, values.len(), "normals")?;
    Ok(Some(MeshPrimvar { values, interpolation, indices }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_width_and_normal_data_is_not_silently_replaced() {
        let stage = Stage::builder().in_memory("invalid-widths.usda").unwrap();
        stage.define_prim("/Curve").unwrap().set_type_name("BasisCurves").unwrap();
        let path = openusd::sdf::path("/Curve").unwrap();
        assert!(read_widths_at(&stage, &path, None).unwrap().is_none());
        let width = stage.create_attribute("/Curve.widths", "float[]").unwrap();
        width.clone().set(Value::FloatVec(vec![0.2,0.3])).unwrap();
        assert_eq!(read_widths_at(&stage, &path, None).unwrap().unwrap().interpolation, Interpolation::Vertex);
        for value in [-1.0, f32::INFINITY, f32::NAN] {
            width.clone().set(Value::FloatVec(vec![value])).unwrap();
            assert!(read_widths_at(&stage, &path, None).is_err());
        }
        width.set(Value::FloatVec(vec![0.2])).unwrap();
        stage.create_attribute("/Curve.primvars:widths", "string").unwrap().set(Value::String("invalid".into())).unwrap();
        assert!(read_widths_at(&stage, &path, None).is_err());
        let normals = stage.create_attribute("/Curve.normals", "normal3f[]").unwrap();
        normals.clone().set(Value::Vec3fVec(vec![[0.,0.,2.].into()])).unwrap();
        assert_eq!(read_normals_at(&stage, &path, None).unwrap().unwrap().values, [[0.,0.,2.]]);
        normals.set(Value::Vec3fVec(vec![[0.,f32::INFINITY,0.].into()])).unwrap();
        assert!(read_normals_at(&stage, &path, None).is_err());
    }

    #[test]
    fn fixture_widths_distinguish_interpolation_and_oriented_ribbons() {
        let source = crate::UsdSource::snapshot("widths.usda", include_bytes!("../../../../assets/curve_widths.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        for (name, values, interpolation) in [
            ("/ConstantTube", vec![0.2], Interpolation::Constant),
            ("/VertexTube", vec![0.05,0.4], Interpolation::Vertex),
            ("/VaryingTube", vec![0.4,0.05], Interpolation::Varying),
            ("/Ribbon", vec![0.2], Interpolation::Constant),
            ("/PrimvarWidth", vec![0.3], Interpolation::Constant),
        ] {
            let path = openusd::sdf::path(name).unwrap();
            let widths = read_widths_at(&stage, &path, Some(0.0)).unwrap().unwrap();
            assert_eq!(widths.values, values);
            assert_eq!(widths.interpolation, interpolation);
            assert!(widths.indices.is_empty());
            let normals = read_normals_at(&stage, &path, None).unwrap();
            if name == "/Ribbon" {
                assert_eq!(normals.unwrap().values, [[0.,0.,1.]]);
            } else { assert!(normals.is_none()); }
        }
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn sampled_indexed_and_inherited_widths_preserve_source_values() {
        let source = crate::UsdSource::snapshot("indexed-widths.usda", br#"#usda 1.0
def Xform "Root" {
    float[] primvars:widths = [0.25]
    def BasisCurves "Inherited" {}
    def BasisCurves "Local" {
        float[] widths = [9,9]
        float[] primvars:widths ( interpolation = "vertex" )
        float[] primvars:widths.timeSamples = {0: [0.1,0.3], 10: [0.5,0.7]}
        int[] primvars:widths:indices = [1,0]
    }
}
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let inherited = read_widths_at(&stage, &openusd::sdf::path("/Root/Inherited").unwrap(), None).unwrap().unwrap();
        assert_eq!(inherited.values, [0.25]);
        assert_eq!(inherited.interpolation, Interpolation::Constant);
        let path = openusd::sdf::path("/Root/Local").unwrap();
        let midpoint = read_widths_at(&stage, &path, Some(5.)).unwrap().unwrap();
        assert!(midpoint.values.iter().zip([0.3,0.5]).all(|(a,b)| (a-b).abs() < 1e-6));
        for (time, expected) in [(0.,[0.1,0.3]), (10.,[0.5,0.7]), (0.,[0.1,0.3])] {
            let widths = read_widths_at(&stage, &path, Some(time)).unwrap().unwrap();
            assert_eq!(widths.values, expected);
            assert_eq!(widths.indices, [1,0]);
            assert_eq!(widths.interpolation, Interpolation::Vertex);
        }
        assert!(read_widths_at(&stage, &path, Some(f64::NAN)).is_err());
        stage.attribute("/Root/Local.primvars:widths:indices").unwrap().set(Value::IntVec(vec![-1])).unwrap();
        assert!(read_widths_at(&stage, &path, Some(0.)).is_err());
    }
}
