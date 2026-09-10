//! Xform reader — compose `xformOpOrder` into a single 4×4 and decompose to
//! TRS, read from the composed stage via openusd.

use glam::{DMat4, DVec3, Mat4};
use openusd::sdf::{Path, Value};
use openusd::usd::{Stage, TimeCode};

/// Decomposed local transform: translate, rotate (quaternion `xyzw`), scale.
#[derive(Debug, Clone, Copy)]
pub struct Transform3 {
    pub translate: [f32; 3],
    pub rotate: [f32; 4],
    pub scale: [f32; 3],
}

fn attr_value(
    stage: &Stage,
    prim: &Path,
    name: &str,
    time: Option<TimeCode>,
) -> anyhow::Result<Option<Value>> {
    Ok(stage.prim(prim.clone()).expect("validated USD path").attribute(name).get_at::<Value>(time)?)
}

/// Read `xformOpOrder` and compose every listed op into a single 4×4, then
/// decompose to TRS, at the stage's default time. `None` when no
/// `xformOpOrder` is authored.
pub fn read_transform(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Transform3>> {
    read_transform_at(stage, prim, None)
}

/// Like [`read_transform`], but resolves attribute values at `time` (a USD
/// time code). `None` reads the default (unanimated) value.
pub fn read_transform_at(
    stage: &Stage,
    prim: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Transform3>> {
    let Some(matrix) = read_transform_matrix_at(stage, prim, time)? else { return Ok(None) };
    let (s, r, t) = Mat4::from_cols_array(&matrix).to_scale_rotation_translation();
    Ok(Some(Transform3 {
        translate: [t.x, t.y, t.z],
        rotate: [r.x, r.y, r.z, r.w],
        scale: [s.x, s.y, s.z],
    }))
}

/// Composed local matrix in column-major order before TRS decomposition.
pub fn read_transform_matrix_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<Option<[f32; 16]>> {
    Ok(read_transform_stack_at(stage, prim, time)?.map(|(matrix, _)| matrix))
}

/// Composed local matrix and inheritance-reset flag at the requested time.
pub fn read_transform_stack_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<Option<([f32; 16], bool)>> {
    let Some((matrix, reset)) = read_transform_stack_f64_at(stage, prim, time)? else { return Ok(None) };
    let matrix = matrix.map(|value| value as f32);
    anyhow::ensure!(matrix.iter().all(|value| value.is_finite()), "composed transform exceeds f32 range");
    Ok(Some((matrix, reset)))
}

/// Composes local operations in double precision before any render conversion.
pub fn read_transform_stack_f64_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<Option<([f64; 16], bool)>> {
    let tc = time.map(TimeCode::new);
    let Some(raw) = attr_value(stage, prim, "xformOpOrder", tc)? else {
        return Ok(None);
    };
    let order: Vec<String> = match raw {
        Value::TokenVec(v) => v.into_iter().map(|t| t.as_str().to_string()).collect(),
        Value::StringVec(v) => v,
        Value::TokenListOp(op) => op
            .flatten()
            .into_iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        _ => anyhow::bail!("unsupported xformOpOrder value type"),
    };

    let mut m = DMat4::IDENTITY;
    let reset = order.iter().rposition(|op| op == "!resetXformStack!");
    let mut index = reset.map_or(0, |index| index + 1);
    while index < order.len() {
        let op = &order[index];
        if order.get(index + 1).is_some_and(|next|
            op.strip_prefix("!invert!") == Some(next.as_str()) || next.strip_prefix("!invert!") == Some(op.as_str())) {
            index += 2;
            continue;
        }
        m *= build_op_matrix(stage, prim, op, tc)?;
        index += 1;
    }

    anyhow::ensure!(m.is_finite(), "non-finite composed transform");
    Ok(Some((m.to_cols_array(), reset.is_some())))
}

fn build_op_matrix(
    stage: &Stage,
    prim: &Path,
    op_token: &str,
    time: Option<TimeCode>,
) -> anyhow::Result<DMat4> {
    const INVERT: &str = "!invert!";
    let (inverted, base) = match op_token.strip_prefix(INVERT) {
        Some(stripped) => (true, stripped),
        None => (false, op_token),
    };

    let Some(raw) = attr_value(stage, prim, base, time)? else {
        return Ok(DMat4::IDENTITY);
    };

    let kind = base.strip_prefix("xformOp:").unwrap_or(base);
    let kind = kind.split(':').next().unwrap_or(kind);
    let invalid = || anyhow::anyhow!("unsupported value type for {op_token}");

    let m = match kind {
        "translate" => {
            DMat4::from_translation(DVec3::from(value_to_vec3d(&raw).ok_or_else(invalid)?))
        }
        "scale" => DMat4::from_scale(DVec3::from(value_to_vec3d(&raw).ok_or_else(invalid)?)),
        "translateX" | "translateY" | "translateZ" | "scaleX" | "scaleY" | "scaleZ" => {
            let scalar = value_to_scalar_f64(&raw).ok_or_else(invalid)?;
            let scale = kind.starts_with("scale");
            let axis = match kind.as_bytes().last() { Some(b'X') => 0, Some(b'Y') => 1, _ => 2 };
            let mut vector = if scale { DVec3::ONE } else { DVec3::ZERO };
            vector[axis] = if scale && inverted { -scalar } else { scalar };
            if scale { DMat4::from_scale(vector) } else { DMat4::from_translation(vector) }
        }
        "orient" => {
            let q = value_to_quat_wxyz(&raw).ok_or_else(invalid)?;
            orientation_matrix(q).ok_or_else(|| anyhow::anyhow!("invalid orientation in {op_token}"))?
        }
        "rotateX" => DMat4::from_rotation_x(value_to_scalar_f64(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateY" => DMat4::from_rotation_y(value_to_scalar_f64(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateZ" => DMat4::from_rotation_z(value_to_scalar_f64(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateXYZ" | "rotateYXZ" | "rotateZXY" | "rotateXZY" | "rotateYZX" | "rotateZYX" => {
            let v = value_to_vec3d(&raw).ok_or_else(invalid)?;
            let rx_m = DMat4::from_rotation_x(v[0].to_radians());
            let ry_m = DMat4::from_rotation_y(v[1].to_radians());
            let rz_m = DMat4::from_rotation_z(v[2].to_radians());
            match kind {
                "rotateXYZ" => rz_m * ry_m * rx_m,
                "rotateYXZ" => rz_m * rx_m * ry_m,
                "rotateZXY" => ry_m * rx_m * rz_m,
                "rotateXZY" => ry_m * rz_m * rx_m,
                "rotateYZX" => rx_m * rz_m * ry_m,
                "rotateZYX" => rx_m * ry_m * rz_m,
                _ => unreachable!(),
            }
        }
        "transform" => value_to_mat4_glam(&raw).ok_or_else(invalid)?,
        _ => anyhow::bail!("unsupported transform op {op_token}"),
    };

    anyhow::ensure!(m.is_finite(), "non-finite transform in {op_token}");
    if !inverted || matches!(kind, "scaleX" | "scaleY" | "scaleZ") { return Ok(m); }
    let inverse = if kind == "scale" {
        let scale = DVec3::new(m.x_axis.x, m.y_axis.y, m.z_axis.z);
        anyhow::ensure!(scale.x != 0.0 && scale.y != 0.0 && scale.z != 0.0, "singular inverse transform in {op_token}");
        DMat4::from_scale(scale.recip())
    } else {
        anyhow::ensure!(m.determinant() != 0.0, "singular inverse transform in {op_token}");
        m.inverse()
    };
    anyhow::ensure!(inverse.is_finite(), "non-finite inverse transform in {op_token}");
    Ok(inverse)
}

fn value_to_mat4_glam(v: &Value) -> Option<DMat4> {
    match v {
        Value::Matrix4d(m) => Some(DMat4::from_cols_array(&m.0)),
        _ => None,
    }
}

fn value_to_vec3d(v: &Value) -> Option<[f64; 3]> {
    match v {
        Value::Vec3f(a) => Some([a.x as f64, a.y as f64, a.z as f64]),
        Value::Vec3d(a) => Some([a.x, a.y, a.z]),
        Value::Vec3h(a) => Some([a.x.to_f64(), a.y.to_f64(), a.z.to_f64()]),
        _ => None,
    }
}

fn value_to_scalar_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f as f64),
        Value::Double(d) => Some(*d),
        Value::Half(h) => Some(h.to_f64()),
        Value::Int(i) => Some(*i as f64),
        Value::Int64(i) => Some(*i as f64),
        _ => None,
    }
}

fn orientation_matrix(q: [f64; 4]) -> Option<DMat4> {
    if !q.iter().all(|value| value.is_finite()) { return None; }
    let axis = DVec3::new(q[1], q[2], q[3]);
    let length = axis.length();
    if !length.is_finite() { return None; }
    if length <= 1e-10 { return Some(DMat4::IDENTITY); }
    Some(DMat4::from_axis_angle(axis / length, 2.0 * q[0].clamp(-1.0, 1.0).acos()))
}

fn value_to_quat_wxyz(v: &Value) -> Option<[f64; 4]> {
    match v {
        Value::Quatf(q) => Some([q.w as f64, q.x as f64, q.y as f64, q.z as f64]),
        Value::Quatd(q) => Some([q.w, q.x, q.y, q.z]),
        Value::Quath(q) => Some([q.w.to_f64(), q.x.to_f64(), q.y.to_f64(), q.z.to_f64()]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn vector_scale_inverse_avoids_determinant_range_limits() {
        let stage = Stage::builder().in_memory("inverse-range.usda").unwrap();
        stage.define_prim("/Root").unwrap();
        let scale = stage.create_attribute("/Root.xformOp:scale", "double3").unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([1.0; 3]))).unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        for values in [[1e-200, -1e-200, 1e-200], [1e200, -1e200, 1e200]] {
            scale.clone().set(Value::Vec3d(openusd::gf::Vec3d::from(values))).unwrap();
            let inverse = build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).unwrap();
            assert_eq!(inverse, DMat4::from_scale(DVec3::from(values).recip()));
            assert!((DMat4::from_scale(DVec3::from(values)) * inverse).abs_diff_eq(DMat4::IDENTITY, 1e-14));
        }
        for values in [[0.0, 1.0, 1.0], [1e-320, 1.0, 1.0], [f64::INFINITY, 1.0, 1.0]] {
            scale.clone().set(Value::Vec3d(openusd::gf::Vec3d::from(values))).unwrap();
            assert!(build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).is_err());
        }
    }

    #[test]
    fn double_stack_keeps_small_residual_before_render_conversion() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/xform_precision.usda")).unwrap();
        let path = openusd::sdf::path("/Panels").unwrap();
        assert_eq!(read_transform_matrix_at(&stage, &path, None).unwrap(), Some(Mat4::from_translation(Vec3::X).to_cols_array()));
        stage.attribute("/Panels.xformOp:translate:offset").unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([-100000000.0 + 1e-6,0.0,0.0]))).unwrap();
        let precise = read_transform_stack_f64_at(&stage, &path, None).unwrap().unwrap().0;
        assert!(precise[12] > 1.0);
        assert!((precise[12] - 1.000001).abs() < 1e-8);
        assert_eq!(read_transform_matrix_at(&stage, &path, None).unwrap().unwrap()[12], precise[12] as f32);
    }

    #[test]
    fn double_ops_can_exceed_float_range_before_final_composition() {
        let stage = Stage::builder().in_memory("double-range.usda").unwrap();
        stage.define_prim("/Root").unwrap();
        stage.create_attribute("/Root.xformOp:scale:a", "double3").unwrap().set(Value::Vec3d(openusd::gf::Vec3d::from([1e40;3]))).unwrap();
        stage.create_attribute("/Root.xformOp:scale:b", "double3").unwrap().set(Value::Vec3d(openusd::gf::Vec3d::from([1e-40;3]))).unwrap();
        let order = stage.create_attribute("/Root.xformOpOrder", "token[]").unwrap()
            .set(Value::TokenVec(vec!["xformOp:scale:a".into(), "xformOp:scale:b".into()])).unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        assert_eq!(read_transform_matrix_at(&stage, &path, None).unwrap(), Some(Mat4::IDENTITY.to_cols_array()));
        order.set(Value::TokenVec(vec!["xformOp:scale:a".into()])).unwrap();
        assert!(read_transform_stack_f64_at(&stage, &path, None).unwrap().unwrap().0[0].is_finite());
        assert!(read_transform_stack_at(&stage, &path, None).unwrap_err().to_string().contains("f32 range"));
    }

    #[test]
    fn scalar_axis_stack_matches_vector_reference_and_native_inverse_values() {
        let open = |file| Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(&format!("{}/../../assets/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let stage = open("xform_scalar_axes.usda");
        let reference = open("xform_half_reference.usda");
        let path = openusd::sdf::path("/Panels").unwrap();
        assert_eq!(read_transform_matrix_at(&stage, &path, None).unwrap(), read_transform_matrix_at(&reference, &path, None).unwrap());
        for (axis, suffix, scale, translate) in [(0,"X",1.5,1.0), (1,"Y",0.5,0.5), (2,"Z",2.0,-2.0)] {
            let inverse_scale = build_op_matrix(&stage, &path, &format!("!invert!xformOp:scale{suffix}"), None).unwrap();
            let mut expected = Vec3::ONE;
            expected[axis] = -scale;
            assert_eq!(inverse_scale, Mat4::from_scale(expected).as_dmat4());
            let inverse_translate = build_op_matrix(&stage, &path, &format!("!invert!xformOp:translate{suffix}"), None).unwrap();
            let mut expected = Vec3::ZERO;
            expected[axis] = -translate;
            assert_eq!(inverse_translate, Mat4::from_translation(expected).as_dmat4());
        }
    }

    #[test]
    fn adjacent_inverse_pairs_cancel_before_evaluating_singular_ops() {
        let stage = Stage::builder().in_memory("cancel-inverse.usda").unwrap();
        stage.define_prim("/Root").unwrap();
        stage.create_attribute("/Root.xformOp:scale", "double3").unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([0.0,1.0,1.0]))).unwrap();
        stage.create_attribute("/Root.xformOp:scaleX", "double").unwrap().set(Value::Double(2.0)).unwrap();
        let mut order = stage.create_attribute("/Root.xformOpOrder", "token[]").unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        for name in ["scale", "scaleX"] {
            for inverted_first in [false, true] {
                let mut ops = vec![format!("xformOp:{name}"), format!("!invert!xformOp:{name}")];
                if inverted_first { ops.reverse(); }
                order = order.set(Value::TokenVec(ops.into_iter().map(Into::into).collect())).unwrap();
                assert_eq!(read_transform_matrix_at(&stage, &path, None).unwrap(), Some(Mat4::IDENTITY.to_cols_array()));
            }
        }
    }

    #[test]
    fn quaternion_conversion_matches_native_axis_angle_cases() {
        for q in [[0.0,0.0,0.0,0.0], [2.0,0.0,0.0,0.0], [0.5,0.0,0.0,1e-10]] {
            assert_eq!(orientation_matrix(q), Some(DMat4::IDENTITY));
        }
        let native = Mat4::from_cols_array(&[-0.00021362304687522204,0.9999999771825967,0.0,0.0,
            -0.9999999771825967,-0.00021362304687522204,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]);
        assert!(orientation_matrix([0.70703125,0.0,0.0,0.70703125]).unwrap().abs_diff_eq(native.as_dmat4(), 1e-7));
        let non_unit = Mat4::from_rotation_z(120_f32.to_radians());
        for length in [1e-9, 2.0, 1e100] {
            assert!(orientation_matrix([0.5,0.0,0.0,length]).unwrap().abs_diff_eq(non_unit.as_dmat4(), 1e-6));
        }
        for q in [[f64::NAN,0.0,0.0,1.0], [0.5,f64::INFINITY,0.0,0.0], [0.5,0.0,0.0,1e300]] {
            assert!(orientation_matrix(q).is_none());
        }
    }

    #[test]
    fn half_quaternion_fixture_matches_native_matrix_reference() {
        let open = |file| Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(&format!("{}/../../assets/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let stage = open("xform_quath.usda");
        let reference = open("xform_quath_reference.usda");
        let path = openusd::sdf::path("/Panels").unwrap();
        let actual = Mat4::from_cols_array(&read_transform_matrix_at(&stage, &path, None).unwrap().unwrap());
        let expected = Mat4::from_cols_array(&read_transform_matrix_at(&reference, &path, None).unwrap().unwrap());
        assert!(actual.abs_diff_eq(expected, 1e-7));
        let direct = build_op_matrix(&stage, &path, "xformOp:orient", None).unwrap();
        let inverse = build_op_matrix(&stage, &path, "!invert!xformOp:orient", None).unwrap();
        assert!((direct * inverse).abs_diff_eq(DMat4::IDENTITY, 1e-6));
    }

    #[test]
    fn half_precision_stack_matches_double_reference() {
        let open = |file| Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(&format!("{}/../../assets/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let stage = open("xform_half.usda");
        let reference = open("xform_half_reference.usda");
        let path = openusd::sdf::path("/Panels").unwrap();
        let actual = read_transform_matrix_at(&stage, &path, None).unwrap().unwrap();
        assert_eq!(Some(actual), read_transform_matrix_at(&reference, &path, None).unwrap());
        let expected = Mat4::from_translation(Vec3::new(1.0, 0.5, -2.0))
            * Mat4::from_rotation_y(30_f32.to_radians()) * Mat4::from_scale(Vec3::new(1.5, 0.5, 2.0));
        assert!(Mat4::from_cols_array(&actual).abs_diff_eq(expected, 1e-6));
        for op in ["translate", "rotateY", "scale"] {
            let direct = build_op_matrix(&stage, &path, &format!("xformOp:{op}"), None).unwrap();
            let inverse = build_op_matrix(&stage, &path, &format!("!invert!xformOp:{op}"), None).unwrap();
            assert!((direct * inverse).abs_diff_eq(DMat4::IDENTITY, 1e-6));
        }
    }

    #[test]
    fn wrong_transform_value_types_and_unknown_ops_are_diagnosed() {
        let stage = Stage::builder().in_memory("wrong-xform-types.usda").unwrap();
        stage.define_prim("/Root").unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        for op in ["translate", "scale", "orient", "rotateX", "rotateY", "rotateZ", "rotateXYZ", "transform", "unknown"] {
            let name = format!("xformOp:{op}");
            stage.create_attribute(path.append_property(&name).unwrap(), "string").unwrap().set(Value::String("bad".into())).unwrap();
            let error = build_op_matrix(&stage, &path, &name, None).unwrap_err().to_string();
            assert!(error.contains(&name));
        }
        assert_eq!(build_op_matrix(&stage, &path, "xformOp:translate:missing", None).unwrap(), DMat4::IDENTITY);
        stage.create_attribute("/Root.xformOpOrder", "string").unwrap().set(Value::String("bad".into())).unwrap();
        assert!(read_transform_stack_at(&stage, &path, None).unwrap_err().to_string().contains("xformOpOrder"));
    }

    #[test]
    fn invalid_orientation_and_singular_inverse_return_errors() {
        let stage = Stage::builder().in_memory("invalid-ops.usda").unwrap();
        stage.define_prim("/Root").unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        let mut orientation = stage.create_attribute("/Root.xformOp:orient", "quatf").unwrap();
        for w in [f32::NAN, f32::INFINITY] {
            orientation = orientation.set(Value::Quatf(openusd::gf::Quatf { w, x: 0.0, y: 0.0, z: 0.0 })).unwrap();
            assert!(build_op_matrix(&stage, &path, "xformOp:orient", None).is_err());
        }
        orientation.set(Value::Quatf(openusd::gf::Quatf::IDENTITY)).unwrap();
        assert_eq!(build_op_matrix(&stage, &path, "xformOp:orient", None).unwrap(), DMat4::IDENTITY);
        let scale = stage.create_attribute("/Root.xformOp:scale", "double3").unwrap();
        let scale = scale.set(Value::Vec3d(openusd::gf::Vec3d::from([0.0, 1.0, 1.0]))).unwrap();
        assert!(build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).is_err());
        scale.set(Value::Vec3d(openusd::gf::Vec3d::from([1e-20; 3]))).unwrap();
        let inverse = build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).unwrap();
        assert!(inverse.is_finite());
        assert!((inverse.x_axis.x / 1e20 - 1.0).abs() < 1e-6);
    }
}
