//! Shared attribute/relationship plumbing for the `read` modules.
//!
//! These helpers read composed values straight off openusd's public
//! `Prim` / `Attribute` / `Relationship` handles — the crate reads the
//! authored scene through openusd only, with no schema layer in between.
//! Each helper takes the owning prim path plus the property name and
//! returns the decoded value (`None` when unauthored or a type mismatch),
//! mirroring the old `usd_schema` reader plumbing it replaces.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

/// Raw composed `default` value of attribute `name` on `prim`.
fn attr_default(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Value>> {
    stage.prim(prim.clone()).attribute(name).get::<Value>()
}

pub fn read_f32(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Float(v)) => Some(v),
        Some(Value::Double(v)) => Some(v as f32),
        Some(Value::Int(v)) => Some(v as f32),
        _ => None,
    })
}

pub fn read_double(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Double(v)) => Some(v),
        Some(Value::Float(v)) => Some(v as f64),
        Some(Value::Int(v)) => Some(v as f64),
        _ => None,
    })
}

/// A `double`, `float`, or `timecode` scalar as `f64`. (openusd decodes
/// `timecode` into a `Double`, so the two collapse here.)
pub fn read_double_or_timecode(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Double(v)) => Some(v),
        Some(Value::Float(v)) => Some(v as f64),
        _ => None,
    })
}

pub fn read_int(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<i32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Int(v)) => Some(v),
        Some(Value::Int64(v)) => Some(v as i32),
        _ => None,
    })
}

pub fn read_bool(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<bool>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Bool(v)) => Some(v),
        _ => None,
    })
}

/// A `token` or `string` scalar.
pub fn read_token_or_string(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Token(s)) => Some(s.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}

/// An `asset`, `string`, or `token` scalar (asset path or plain text).
pub fn read_asset_path(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::AssetPath(s)) => Some(s.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        Some(Value::Token(s)) => Some(s.as_str().to_string()),
        _ => None,
    })
}

pub fn read_vec3f(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[f32; 3]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec3f(v)) => Some(v.into()),
        Some(Value::Vec3d(v)) => Some([v.x as f32, v.y as f32, v.z as f32]),
        _ => None,
    })
}

pub fn read_vec2f(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[f32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2f(v)) => Some(v.into()),
        Some(Value::Vec2d(v)) => Some([v.x as f32, v.y as f32]),
        _ => None,
    })
}

pub fn read_vec2i(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[i32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2i(v)) => Some(v.into()),
        Some(Value::IntVec(v)) if v.len() == 2 => Some([v[0], v[1]]),
        _ => None,
    })
}

pub fn read_token_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::TokenVec(v)) => v.into_iter().map(|t| t.as_str().to_string()).collect(),
        Some(Value::StringVec(v)) => v,
        _ => Vec::new(),
    })
}

pub fn read_int_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<i32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::IntVec(v)) => v,
        Some(Value::Int64Vec(v)) => v.into_iter().map(|i| i as i32).collect(),
        _ => Vec::new(),
    })
}

pub fn read_float_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<f32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::FloatVec(v)) => v,
        Some(Value::DoubleVec(v)) => v.into_iter().map(|d| d as f32).collect(),
        _ => Vec::new(),
    })
}

pub fn read_double_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::DoubleVec(v)) => v,
        Some(Value::FloatVec(v)) => v.into_iter().map(|f| f as f64).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec3f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 3]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec3fVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec2f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2fVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_vec2d_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f64; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2dVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

pub fn read_quatf_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 4]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::QuatfVec(v)) => v.into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    })
}

/// Like [`read_int_vec`] but distinguishes "unauthored" (`None`) from "empty".
pub fn read_int_vec_opt(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<i32>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::IntVec(v)) => Some(v),
        Some(Value::Int64Vec(v)) => Some(v.into_iter().map(|i| i as i32).collect()),
        _ => None,
    })
}

/// Like [`read_float_vec`] but distinguishes "unauthored" (`None`) from "empty".
pub fn read_float_vec_opt(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::FloatVec(v)) => Some(v),
        Some(Value::DoubleVec(v)) => Some(v.into_iter().map(|d| d as f32).collect()),
        _ => None,
    })
}

/// `matrix4d[]` flattened to row-major `[f32; 16]` per element.
pub fn read_mat4f_vec(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Vec<[f32; 16]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Matrix4dVec(v)) => v
            .into_iter()
            .map(|m| {
                let mut out = [0.0f32; 16];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = m.0[i] as f32;
                }
                out
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// `i32`-valued attribute *metadata* field (e.g. primvar `elementSize`).
pub fn read_int_metadata(
    stage: &Stage,
    prim: &Path,
    attr: &str,
    key: &str,
) -> anyhow::Result<Option<i32>> {
    Ok(
        match stage
            .prim(prim.clone())
            .attribute(attr)
            .get_metadata::<Value>(key)?
        {
            Some(Value::Int(n)) => Some(n),
            Some(Value::Int64(n)) => Some(n as i32),
            _ => None,
        },
    )
}

/// Composed `timeSamples` for an attribute, as `(time, value)` pairs.
pub fn read_time_samples(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Vec<(f64, Value)>> {
    Ok(stage
        .prim(prim.clone())
        .attribute(name)
        .time_samples()?
        .unwrap_or_default())
}

/// Composed relationship target paths (as strings), in authored order.
pub fn read_rel_targets(stage: &Stage, prim: &Path, rel_name: &str) -> anyhow::Result<Vec<String>> {
    let targets = stage.prim(prim.clone()).relationship(rel_name).targets()?;
    Ok(targets
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect())
}

/// The first composed relationship target (strongest), if any.
pub fn read_rel_first_target(
    stage: &Stage,
    prim: &Path,
    rel_name: &str,
) -> anyhow::Result<Option<String>> {
    Ok(read_rel_targets(stage, prim, rel_name)?.into_iter().next())
}

// ── Property-path-based access (for graph walks like UsdShade) ──────────

/// Composed `connectionPaths` of the attribute at property path `attr_path`.
pub fn connections_at(stage: &Stage, attr_path: &Path) -> anyhow::Result<Vec<Path>> {
    let Some((prim, name)) = attr_path.split_property() else {
        return Ok(Vec::new());
    };
    stage.prim(prim).attribute(name).connections()
}

/// Composed relationship target paths at property path `rel_path`.
pub fn targets_at(stage: &Stage, rel_path: &Path) -> anyhow::Result<Vec<Path>> {
    let Some((prim, name)) = rel_path.split_property() else {
        return Ok(Vec::new());
    };
    stage.prim(prim).relationship(name).targets()
}

/// Raw composed `default` value of the attribute at property path `attr_path`.
pub fn default_at(stage: &Stage, attr_path: &Path) -> anyhow::Result<Option<Value>> {
    let Some((prim, name)) = attr_path.split_property() else {
        return Ok(None);
    };
    stage.prim(prim).attribute(name).get::<Value>()
}
