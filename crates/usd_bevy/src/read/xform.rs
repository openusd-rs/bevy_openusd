//! Xform reader — compose `xformOpOrder` into a single 4×4 and decompose to
//! TRS, read from the composed stage via openusd.

use glam::{DMat4, DVec3, Mat4, Vec3};
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

    let mut m = Mat4::IDENTITY;
    let reset = order.iter().rposition(|op| op == "!resetXformStack!");
    for op in &order[reset.map_or(0, |index| index + 1)..] {
        m *= build_op_matrix(stage, prim, op, tc)?;
    }

    Ok(Some((m.to_cols_array(), reset.is_some())))
}

fn build_op_matrix(
    stage: &Stage,
    prim: &Path,
    op_token: &str,
    time: Option<TimeCode>,
) -> anyhow::Result<Mat4> {
    const INVERT: &str = "!invert!";
    let (inverted, base) = match op_token.strip_prefix(INVERT) {
        Some(stripped) => (true, stripped),
        None => (false, op_token),
    };

    let Some(raw) = attr_value(stage, prim, base, time)? else {
        return Ok(Mat4::IDENTITY);
    };

    let kind = base.strip_prefix("xformOp:").unwrap_or(base);
    let kind = kind.split(':').next().unwrap_or(kind);
    let invalid = || anyhow::anyhow!("unsupported value type for {op_token}");

    let m = match kind {
        "translate" => {
            Mat4::from_translation(Vec3::from(value_to_vec3f(&raw).ok_or_else(invalid)?))
        }
        "scale" => Mat4::from_scale(Vec3::from(value_to_vec3f(&raw).ok_or_else(invalid)?)),
        "orient" => {
            let q = value_to_quat_wxyz(&raw).ok_or_else(invalid)?;
            orientation_matrix(q).ok_or_else(|| anyhow::anyhow!("invalid orientation in {op_token}"))?
        }
        "rotateX" => Mat4::from_rotation_x(value_to_scalar_f32(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateY" => Mat4::from_rotation_y(value_to_scalar_f32(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateZ" => Mat4::from_rotation_z(value_to_scalar_f32(&raw).ok_or_else(invalid)?.to_radians()),
        "rotateXYZ" | "rotateYXZ" | "rotateZXY" | "rotateXZY" | "rotateYZX" | "rotateZYX" => {
            let v = value_to_vec3f(&raw).ok_or_else(invalid)?;
            let rx_m = Mat4::from_rotation_x(v[0].to_radians());
            let ry_m = Mat4::from_rotation_y(v[1].to_radians());
            let rz_m = Mat4::from_rotation_z(v[2].to_radians());
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
    if !inverted { return Ok(m); }
    let matrix = m.as_dmat4();
    anyhow::ensure!(matrix.determinant() != 0.0, "singular inverse transform in {op_token}");
    let inverse = matrix.inverse().as_mat4();
    anyhow::ensure!(inverse.is_finite(), "non-finite inverse transform in {op_token}");
    Ok(inverse)
}

fn value_to_mat4_glam(v: &Value) -> Option<Mat4> {
    match v {
        Value::Matrix4d(m) => {
            let cols: [f32; 16] = std::array::from_fn(|i| m.0[i] as f32);
            Some(Mat4::from_cols_array(&cols))
        }
        _ => None,
    }
}

fn value_to_vec3f(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Vec3f(a) => Some([a.x, a.y, a.z]),
        Value::Vec3d(a) => Some([a.x as f32, a.y as f32, a.z as f32]),
        Value::Vec3h(a) => Some([a.x.to_f32(), a.y.to_f32(), a.z.to_f32()]),
        _ => None,
    }
}

fn value_to_scalar_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Double(d) => Some(*d as f32),
        Value::Half(h) => Some(h.to_f32()),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        _ => None,
    }
}

fn orientation_matrix(q: [f64; 4]) -> Option<Mat4> {
    if !q.iter().all(|value| value.is_finite()) { return None; }
    let axis = DVec3::new(q[1], q[2], q[3]);
    let length = axis.length();
    if !length.is_finite() { return None; }
    if length <= 1e-10 { return Some(Mat4::IDENTITY); }
    Some(DMat4::from_axis_angle(axis / length, 2.0 * q[0].clamp(-1.0, 1.0).acos()).as_mat4())
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

    #[test]
    fn quaternion_conversion_matches_native_axis_angle_cases() {
        for q in [[0.0,0.0,0.0,0.0], [2.0,0.0,0.0,0.0], [0.5,0.0,0.0,1e-10]] {
            assert_eq!(orientation_matrix(q), Some(Mat4::IDENTITY));
        }
        let native = Mat4::from_cols_array(&[-0.00021362304687522204,0.9999999771825967,0.0,0.0,
            -0.9999999771825967,-0.00021362304687522204,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]);
        assert!(orientation_matrix([0.70703125,0.0,0.0,0.70703125]).unwrap().abs_diff_eq(native, 1e-7));
        let non_unit = Mat4::from_rotation_z(120_f32.to_radians());
        for length in [1e-9, 2.0, 1e100] {
            assert!(orientation_matrix([0.5,0.0,0.0,length]).unwrap().abs_diff_eq(non_unit, 1e-6));
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
        assert!((direct * inverse).abs_diff_eq(Mat4::IDENTITY, 1e-6));
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
            assert!((direct * inverse).abs_diff_eq(Mat4::IDENTITY, 1e-6));
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
        assert_eq!(build_op_matrix(&stage, &path, "xformOp:translate:missing", None).unwrap(), Mat4::IDENTITY);
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
        assert_eq!(build_op_matrix(&stage, &path, "xformOp:orient", None).unwrap(), Mat4::IDENTITY);
        let scale = stage.create_attribute("/Root.xformOp:scale", "double3").unwrap();
        let scale = scale.set(Value::Vec3d(openusd::gf::Vec3d::from([0.0, 1.0, 1.0]))).unwrap();
        assert!(build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).is_err());
        scale.set(Value::Vec3d(openusd::gf::Vec3d::from([1e-20; 3]))).unwrap();
        let inverse = build_op_matrix(&stage, &path, "!invert!xformOp:scale", None).unwrap();
        assert!(inverse.is_finite());
        assert!((inverse.x_axis.x / 1e20 - 1.0).abs() < 1e-6);
    }
}
