//! UsdSkel readers: `Skeleton`, `SkelRoot`, `SkelBindingAPI`, `BlendShape`,
//! and `SkelAnimation` — decoded from the composed stage via openusd.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::skel_anim_text::{OrdF64, ReadSkelAnimText};
use super::util::{
    read_float_vec_opt, read_int_metadata, read_int_vec_opt, read_mat4f_vec, read_rel_first_target, read_rel_targets,
    read_time_samples, read_token_vec, read_vec3f_vec,
};

#[derive(Debug, Clone)]
pub struct ReadSkeleton {
    pub path: String,
    pub joints: Vec<String>,
    pub bind_transforms: Vec<[f32; 16]>,
    pub rest_transforms: Vec<[f32; 16]>,
}

#[derive(Debug, Clone)]
pub struct ReadSkelRoot {
    pub path: String,
    pub skeleton: Option<String>,
    pub animation_source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReadBlendShape {
    pub path: String,
    pub offsets: Vec<[f32; 3]>,
    pub normal_offsets: Vec<[f32; 3]>,
    pub point_indices: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct ReadSkelBinding {
    pub prim_path: String,
    pub skeleton: Option<String>,
    pub joint_indices: Vec<i32>,
    pub joint_weights: Vec<f32>,
    pub elements_per_vertex: i32,
    pub joint_subset: Vec<String>,
    pub blend_shapes: Vec<String>,
    pub blend_shape_targets: Vec<String>,
}

impl ReadSkeleton {
    pub fn joint_parent_indices(&self) -> Vec<Option<usize>> {
        let by_path: std::collections::HashMap<&str, usize> =
            self.joints.iter().enumerate().map(|(i, p)| (p.as_str(), i)).collect();
        self.joints
            .iter()
            .map(|p| {
                p.rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .and_then(|parent_path| by_path.get(parent_path).copied())
            })
            .collect()
    }

    pub fn joint_short_names(&self) -> Vec<&str> {
        self.joints
            .iter()
            .map(|p| p.rsplit_once('/').map(|(_, n)| n).unwrap_or(p.as_str()))
            .collect()
    }
}

fn type_name(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    Ok(stage.prim_at(prim.clone()).type_name()?.unwrap_or_default())
}

pub fn read_skeleton(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadSkeleton>> {
    if type_name(stage, prim)? != "Skeleton" {
        return Ok(None);
    }
    let joints = read_token_vec(stage, prim, "joints")?;
    if joints.is_empty() {
        return Ok(None);
    }
    Ok(Some(ReadSkeleton {
        path: prim.as_str().to_string(),
        joints,
        bind_transforms: read_mat4f_vec(stage, prim, "bindTransforms")?,
        rest_transforms: read_mat4f_vec(stage, prim, "restTransforms")?,
    }))
}

pub fn read_skel_root(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadSkelRoot>> {
    if type_name(stage, prim)? != "SkelRoot" {
        return Ok(None);
    }
    Ok(Some(ReadSkelRoot {
        path: prim.as_str().to_string(),
        skeleton: read_rel_first_target(stage, prim, "skel:skeleton")?,
        animation_source: read_rel_first_target(stage, prim, "skel:animationSource")?,
    }))
}

pub fn read_skel_binding(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadSkelBinding>> {
    let Some(joint_indices) = read_int_vec_opt(stage, prim, "primvars:skel:jointIndices")? else {
        return Ok(None);
    };
    let Some(joint_weights) = read_float_vec_opt(stage, prim, "primvars:skel:jointWeights")? else {
        return Ok(None);
    };
    Ok(Some(ReadSkelBinding {
        prim_path: prim.as_str().to_string(),
        skeleton: read_rel_first_target(stage, prim, "skel:skeleton")?,
        joint_indices,
        joint_weights,
        elements_per_vertex: read_int_metadata(stage, prim, "primvars:skel:jointIndices", "elementSize")?.unwrap_or(1),
        joint_subset: read_token_vec(stage, prim, "skel:joints")?,
        blend_shapes: read_token_vec(stage, prim, "skel:blendShapes")?,
        blend_shape_targets: read_rel_targets(stage, prim, "skel:blendShapeTargets")?,
    }))
}

pub fn read_blend_shape(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadBlendShape>> {
    if type_name(stage, prim)? != "BlendShape" {
        return Ok(None);
    }
    let offsets = read_vec3f_vec(stage, prim, "offsets")?;
    if offsets.is_empty() {
        return Ok(None);
    }
    Ok(Some(ReadBlendShape {
        path: prim.as_str().to_string(),
        offsets,
        normal_offsets: read_vec3f_vec(stage, prim, "normalOffsets")?,
        point_indices: read_int_vec_opt(stage, prim, "pointIndices")?.unwrap_or_default(),
    }))
}

/// Read a `UsdSkelAnimation` prim into the same shape the rest of the
/// pipeline consumes. Now that openusd's parser handles tuple-valued time
/// samples, this reads them directly — no text-scrape workaround needed.
pub fn read_skel_animation_stage(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadSkelAnimText>> {
    if type_name(stage, prim)? != "SkelAnimation" {
        return Ok(None);
    }
    let joints = read_token_vec(stage, prim, "joints")?;
    let blend_shapes = read_token_vec(stage, prim, "blendShapes")?;
    if joints.is_empty() && blend_shapes.is_empty() {
        return Ok(None);
    }
    let prim_name = prim
        .as_str()
        .rsplit_once('/')
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| prim.as_str().to_string());

    let mut anim = ReadSkelAnimText {
        prim_name,
        joints,
        blend_shapes,
        ..Default::default()
    };

    for (t, val) in read_time_samples(stage, prim, "translations")? {
        if let Some(v) = vec3f_samples(val) {
            anim.translations.insert(OrdF64(t), v);
        }
    }
    for (t, val) in read_time_samples(stage, prim, "rotations")? {
        let v: Option<Vec<[f32; 4]>> = match val {
            Value::QuatfVec(v) => Some(v.into_iter().map(Into::into).collect()),
            Value::QuatdVec(v) => Some(v.into_iter().map(|q| [q.w as f32, q.x as f32, q.y as f32, q.z as f32]).collect()),
            Value::QuathVec(v) => {
                Some(v.into_iter().map(|q| [q.w.to_f32(), q.x.to_f32(), q.y.to_f32(), q.z.to_f32()]).collect())
            }
            _ => None,
        };
        if let Some(v) = v {
            anim.rotations.insert(OrdF64(t), v);
        }
    }
    for (t, val) in read_time_samples(stage, prim, "scales")? {
        if let Some(v) = vec3f_samples(val) {
            anim.scales.insert(OrdF64(t), v);
        }
    }
    for (t, val) in read_time_samples(stage, prim, "blendShapeWeights")? {
        if let Value::FloatVec(v) = val {
            anim.blend_shape_weights.insert(OrdF64(t), v);
        }
    }
    Ok(Some(anim))
}

/// `float3[]` / `double3[]` / `half3[]` time-sample value → `Vec<[f32; 3]>`.
fn vec3f_samples(val: Value) -> Option<Vec<[f32; 3]>> {
    match val {
        Value::Vec3fVec(v) => Some(v.into_iter().map(Into::into).collect()),
        Value::Vec3dVec(v) => Some(v.into_iter().map(|a| [a.x as f32, a.y as f32, a.z as f32]).collect()),
        Value::Vec3hVec(v) => Some(v.into_iter().map(|a| [a.x.to_f32(), a.y.to_f32(), a.z.to_f32()]).collect()),
        _ => None,
    }
}
