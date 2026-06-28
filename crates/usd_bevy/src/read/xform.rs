//! Xform reader — compose `xformOpOrder` into a single 4×4 and decompose to
//! TRS, read from the composed stage via openusd.

use glam::{Mat4, Quat, Vec3};
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

/// Decomposed local transform: translate, rotate (quaternion `xyzw`), scale.
#[derive(Debug, Clone, Copy)]
pub struct Transform3 {
    pub translate: [f32; 3],
    pub rotate: [f32; 4],
    pub scale: [f32; 3],
}

fn attr_value(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Value>> {
    stage.prim(prim.clone()).attribute(name).get::<Value>()
}

/// Read `xformOpOrder` and compose every listed op into a single 4×4, then
/// decompose to TRS. `None` when no `xformOpOrder` is authored.
pub fn read_transform(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Transform3>> {
    let Some(raw) = attr_value(stage, prim, "xformOpOrder")? else {
        return Ok(None);
    };
    let order: Vec<String> = match raw {
        Value::TokenVec(v) => v.into_iter().map(|t| t.as_str().to_string()).collect(),
        Value::StringVec(v) => v,
        Value::TokenListOp(op) => op.flatten().into_iter().map(|t| t.as_str().to_string()).collect(),
        _ => return Ok(None),
    };

    let mut m = Mat4::IDENTITY;
    for op in &order {
        m *= build_op_matrix(stage, prim, op)?;
    }

    let (s, r, t) = m.to_scale_rotation_translation();
    Ok(Some(Transform3 {
        translate: [t.x, t.y, t.z],
        rotate: [r.x, r.y, r.z, r.w],
        scale: [s.x, s.y, s.z],
    }))
}

fn build_op_matrix(stage: &Stage, prim: &Path, op_token: &str) -> anyhow::Result<Mat4> {
    const INVERT: &str = "!invert!";
    let (inverted, base) = match op_token.strip_prefix(INVERT) {
        Some(stripped) => (true, stripped),
        None => (false, op_token),
    };

    let Some(raw) = attr_value(stage, prim, base)? else {
        return Ok(Mat4::IDENTITY);
    };

    let kind = base.strip_prefix("xformOp:").unwrap_or(base);
    let kind = kind.split(':').next().unwrap_or(kind);

    let m = match kind {
        "translate" => Mat4::from_translation(Vec3::from(value_to_vec3f(&raw).unwrap_or([0.0, 0.0, 0.0]))),
        "scale" => Mat4::from_scale(Vec3::from(value_to_vec3f(&raw).unwrap_or([1.0, 1.0, 1.0]))),
        "orient" => {
            let q = value_to_quat_wxyz(&raw).unwrap_or([1.0, 0.0, 0.0, 0.0]);
            Mat4::from_quat(Quat::from_xyzw(q[1], q[2], q[3], q[0]))
        }
        "rotateX" => Mat4::from_rotation_x(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians()),
        "rotateY" => Mat4::from_rotation_y(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians()),
        "rotateZ" => Mat4::from_rotation_z(value_to_scalar_f32(&raw).unwrap_or(0.0).to_radians()),
        "rotateXYZ" | "rotateYXZ" | "rotateZXY" | "rotateXZY" | "rotateYZX" | "rotateZYX" => {
            let v = value_to_vec3f(&raw).unwrap_or([0.0, 0.0, 0.0]);
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
        "transform" => value_to_mat4_glam(&raw).unwrap_or(Mat4::IDENTITY),
        _ => Mat4::IDENTITY,
    };

    Ok(if inverted { m.inverse() } else { m })
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
        _ => None,
    }
}

fn value_to_scalar_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Double(d) => Some(*d as f32),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        _ => None,
    }
}

fn value_to_quat_wxyz(v: &Value) -> Option<[f32; 4]> {
    match v {
        Value::Quatf(q) => Some([q.w, q.x, q.y, q.z]),
        Value::Quatd(q) => Some([q.w as f32, q.x as f32, q.y as f32, q.z as f32]),
        _ => None,
    }
}
