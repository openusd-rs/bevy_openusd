//! Composed subdivision rules and sharpness data for mesh refinement.

use anyhow::{Result, bail, ensure};
use openusd::{sdf::{Path, Value}, usd::{Stage, TimeCode}};

#[derive(Debug, Clone, PartialEq)]
pub struct ReadSubdivision {
    pub scheme: String,
    pub boundary: String,
    pub face_varying: String,
    pub triangle_rule: String,
    pub crease_indices: Vec<i32>,
    pub crease_lengths: Vec<i32>,
    pub crease_sharpnesses: Vec<f32>,
    pub corner_indices: Vec<i32>,
    pub corner_sharpnesses: Vec<f32>,
    pub holes: Vec<i32>,
}

impl ReadSubdivision {
    /// Validates sharpness cardinality and indices against sampled mesh dimensions.
    pub fn validate(&self, points: usize, faces: usize) -> Result<()> {
        ensure!(self.crease_lengths.iter().all(|&length| length >= 2), "crease lengths must be at least two");
        let count = self.crease_lengths.iter().try_fold(0usize, |count, &length|
            count.checked_add(length as usize).ok_or_else(|| anyhow::anyhow!("crease length overflow")))?;
        ensure!(count == self.crease_indices.len(), "crease lengths do not match indices");
        ensure!(self.crease_sharpnesses.len() == self.crease_lengths.len()
            || self.crease_sharpnesses.len() == count - self.crease_lengths.len(), "crease sharpness count must be per crease or per edge");
        ensure!(self.corner_indices.len() == self.corner_sharpnesses.len(), "corner sharpness count does not match indices");
        ensure!(self.crease_indices.iter().chain(&self.corner_indices).all(|&index| index >= 0 && (index as usize) < points), "sharpness point index out of range");
        ensure!(self.holes.iter().all(|&index| index >= 0 && (index as usize) < faces), "hole face index out of range");
        ensure!(self.crease_sharpnesses.iter().chain(&self.corner_sharpnesses).all(|value| value.is_finite() && *value >= 0.0), "sharpness must be finite and nonnegative");
        Ok(())
    }
}

/// Reads subdivision attributes at a USD time code without changing the stage.
pub fn read_subdivision_at(stage: &Stage, path: &Path, time: Option<f64>) -> Result<ReadSubdivision> {
    ensure!(time.is_none_or(f64::is_finite), "nonfinite subdivision time");
    let prim = stage.prim(path.clone())?;
    ensure!(prim.type_name()?.as_deref() == Some("Mesh"), "subdivision owner must be a Mesh");
    let value = |name: &str| -> Result<Option<Value>> {
        Ok(prim.attribute(name).get_at::<Value>(time.map(TimeCode::new))?)
    };
    let token = |name: &str, allowed: &[&str]| -> Result<String> {
        let Some(Value::Token(token)) = value(name)? else { bail!("missing or invalid {name}") };
        ensure!(allowed.contains(&token.as_str()), "unsupported {name} token: {token}");
        Ok(token.as_str().to_owned())
    };
    let ints = |name: &str| -> Result<Vec<i32>> {
        match value(name)? {
            Some(Value::IntVec(values)) => Ok(values),
            None => Ok(Vec::new()),
            _ => bail!("invalid {name}: expected int array"),
        }
    };
    let floats = |name: &str| -> Result<Vec<f32>> {
        match value(name)? {
            Some(Value::FloatVec(values)) => Ok(values),
            None => Ok(Vec::new()),
            _ => bail!("invalid {name}: expected float array"),
        }
    };
    Ok(ReadSubdivision {
        scheme: token("subdivisionScheme", &["none", "catmullClark", "loop", "bilinear"] )?,
        boundary: token("interpolateBoundary", &["none", "edgeOnly", "edgeAndCorner"] )?,
        face_varying: token("faceVaryingLinearInterpolation", &["none", "cornersOnly", "cornersPlus1", "cornersPlus2", "boundaries", "all"] )?,
        triangle_rule: token("triangleSubdivisionRule", &["catmullClark", "smooth"] )?,
        crease_indices: ints("creaseIndices")?,
        crease_lengths: ints("creaseLengths")?,
        crease_sharpnesses: floats("creaseSharpnesses")?,
        corner_indices: ints("cornerIndices")?,
        corner_sharpnesses: floats("cornerSharpnesses")?,
        holes: ints("holeIndices")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(body: &str) -> Stage {
        crate::UsdSource::new("subdivision.usda", format!("#usda 1.0\ndef Mesh \"M\" {{\n{body}\n}}\n").as_bytes())
            .unwrap().open_stage().unwrap()
    }

    #[test]
    fn composed_defaults_and_sampled_sharpness() {
        let path = Path::new("/M").unwrap();
        let defaults = read_subdivision_at(&stage(""), &path, None).unwrap();
        assert_eq!(defaults.scheme, "catmullClark");
        assert_eq!(defaults.boundary, "edgeAndCorner");
        assert_eq!(defaults.face_varying, "cornersPlus1");
        assert_eq!(defaults.triangle_rule, "catmullClark");
        defaults.validate(0, 0).unwrap();
        let stage = stage(r#"
            int[] creaseIndices = [0,1,2]
            int[] creaseLengths = [3]
            float[] creaseSharpnesses.timeSamples = { 0: [0], 10: [4] }
            int[] cornerIndices = [3]
            float[] cornerSharpnesses = [10]
            int[] holeIndices = [1]
            token faceVaryingLinearInterpolation = "all"
        "#);
        let sample = read_subdivision_at(&stage, &path, Some(5.0)).unwrap();
        assert_eq!(sample.crease_sharpnesses, [2.0]);
        assert_eq!(sample.corner_sharpnesses, [10.0]);
        assert_eq!(sample.holes, [1]);
        assert_eq!(sample.face_varying, "all");
        sample.validate(4, 2).unwrap();
        let mut per_edge = sample.clone();
        per_edge.crease_sharpnesses = vec![1.0,2.0];
        per_edge.validate(4, 2).unwrap();
        assert!(sample.validate(3, 2).is_err());
        assert!(sample.validate(4, 1).is_err());
        assert!(read_subdivision_at(&stage, &path, Some(f64::NAN)).is_err());
    }

    #[test]
    fn malformed_rules_and_sharpness_are_rejected() {
        let path = Path::new("/M").unwrap();
        assert!(read_subdivision_at(&stage("token interpolateBoundary = \"invalid\""), &path, None).is_err());
        let original = read_subdivision_at(&stage(""), &path, None).unwrap();
        for length in [-1,0,1,3] {
            let mut invalid = original.clone();
            invalid.crease_lengths = vec![length];
            assert!(invalid.validate(4,2).is_err());
        }
        for sharpness in [-1.0, f32::NAN, f32::INFINITY] {
            let mut invalid = original.clone();
            invalid.corner_indices = vec![0];
            invalid.corner_sharpnesses = vec![sharpness];
            assert!(invalid.validate(4,2).is_err());
        }
        let mut invalid = original.clone();
        invalid.corner_indices = vec![0];
        assert!(invalid.validate(4,2).is_err());
        invalid = original;
        invalid.crease_indices = vec![0,1,2];
        invalid.crease_lengths = vec![3];
        invalid.crease_sharpnesses = vec![1.0;3];
        assert!(invalid.validate(4,2).is_err());
    }
}
