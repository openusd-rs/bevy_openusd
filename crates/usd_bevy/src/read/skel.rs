//! UsdSkel CPU deformation and GPU palette evaluation.
//!
//! Animation transforms map by joint name to skeleton order; missing animated
//! joints use their local rest transforms. Mesh-effective joint order and
//! geometry bind transforms are resolved through upstream skinning helpers.

use openusd::gf;
use openusd_schemas::skel::skinning::{
    BlendShapeWeighted, InbetweenRef, apply_blend_shapes, resolve_blend_shape_offsets,
};
use openusd_schemas::skel::{
    BlendShape, SkelAnimQuery, SkelBinding, BindingAPI as SkelBindingAPI, Skeleton, SkeletonResolver,
    SkinningResolver,
};
use openusd::sdf::Path;
use openusd::usd::{Stage, TimeCode};

use super::util::{read_float_vec, read_rel_first_target};

/// Whether `prim` carries skinning influences (i.e. is a skinned mesh).
pub fn is_skinned(stage: &Stage, prim: &Path) -> bool {
    read_float_vec(stage, prim, "primvars:skel:jointWeights")
        .map(|w| !w.is_empty())
        .unwrap_or(false)
        || influences_are_time_varying(stage, prim)
}

fn influences_are_time_varying(stage: &Stage, path: &Path) -> bool {
    stage.prim(path.clone()).is_ok_and(|prim| {
        ["primvars:skel:jointIndices", "primvars:skel:jointWeights"].iter().any(|name|
            prim.attribute(*name).time_sample_times().is_ok_and(|times| !times.is_empty()))
    })
}

fn sampled_influences(stage: &Stage, path: &Path, time: Option<f64>) -> anyhow::Result<(Vec<i32>, Vec<f32>)> {
    let prim = stage.prim(path.clone())?;
    let time = time.map(TimeCode::new);
    let indices = prim.attribute("primvars:skel:jointIndices").get_at::<Vec<i32>>(time)?.unwrap_or_default();
    let weights = prim.attribute("primvars:skel:jointWeights").get_at::<Vec<f32>>(time)?.unwrap_or_default();
    Ok((indices, weights))
}

/// Whether `prim` binds any blend shapes (`skel:blendShapes`).
pub fn has_blend_shapes(stage: &Stage, prim: &Path) -> bool {
    SkelBindingAPI::get(stage, prim.clone())
        .ok()
        .flatten()
        .and_then(|b| b.blend_shapes().ok())
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Whether the mesh's bound blend-shape *weights* may vary over time (so the
/// mesh must be re-morphed when [`StageTime`](crate::route::StageTime) moves).
pub fn blend_is_time_varying(stage: &Stage, mesh_path: &Path) -> bool {
    let Some(binding) = binding_of(stage, mesh_path) else {
        return false;
    };
    let Some(skel_path) = binding.skeleton.clone() else {
        return false;
    };
    let Some(anim_path) = animation_source(stage, &skel_path, &binding) else {
        return false;
    };
    matches!(
        SkelAnimQuery::new(stage, anim_path),
        Ok(Some(anim)) if anim.blend_shape_weights_might_be_time_varying()
    )
}

/// Dense, weight-independent position and normal targets in source-point order.
pub struct MorphSample {
    pub targets: Vec<Vec<[f32; 3]>>,
    pub normal_targets: Vec<Vec<[f32; 3]>>,
    pub weights: Vec<f32>,
}

/// Expands sparse shapes and inbetweens into reusable linear morph channels.
pub fn morph_sample(stage: &Stage, path: &Path, time: Option<f64>) -> anyhow::Result<MorphSample> {
    let mesh = super::geom::read_mesh_at(stage, path, time)?.ok_or_else(|| anyhow::anyhow!("missing morph mesh"))?;
    let binding = binding_of(stage, path).ok_or_else(|| anyhow::anyhow!("missing morph binding"))?;
    let skeleton = binding.skeleton.clone().ok_or_else(|| anyhow::anyhow!("missing morph skeleton"))?;
    let animation = animation_source(stage, &skeleton, &binding).ok_or_else(|| anyhow::anyhow!("missing morph animation"))?;
    let animation = SkelAnimQuery::new(stage, animation)?.ok_or_else(|| anyhow::anyhow!("invalid morph animation"))?;
    let weights = animation.compute_blend_shape_weights(stage, TimeCode::new(time.unwrap_or(0.0)))?;
    let names = binding.binding.blend_shapes()?;
    let paths = binding.binding.blend_shape_targets()?;
    anyhow::ensure!(names.len() == paths.len(), "blend shape name/target count mismatch");
    let mut result = MorphSample { targets: Vec::new(), normal_targets: Vec::new(), weights: Vec::new() };
    for (name, path) in names.iter().zip(paths) {
        let weight = animation.blend_shape_order().iter().position(|value| value == name)
            .and_then(|index| weights.get(index)).copied().unwrap_or(0.0);
        anyhow::ensure!(weight.is_finite(), "nonfinite blend shape weight");
        let shape = BlendShape::get(stage, path)?.ok_or_else(|| anyhow::anyhow!("invalid blend shape target"))?;
        let indices = shape.point_indices()?;
        let primary = shape.offsets()?;
        let primary_normals = shape.normal_offsets()?;
        let normal_target = |offsets: &[gf::Vec3f]| -> anyhow::Result<Vec<[f32; 3]>> {
            if offsets.is_empty() { Ok(vec![[0.0; 3]; mesh.points.len()]) }
            else { dense_morph_offsets(offsets, &indices, mesh.points.len()) }
        };
        let inbetweens = shape.inbetweens()?;
        let mut breakpoints = Vec::new();
        for inbetween in &inbetweens {
            let at = inbetween.weight.ok_or_else(|| anyhow::anyhow!("inbetween has no weight"))?;
            anyhow::ensure!(at.is_finite() && inbetween.offsets.len() == primary.len(), "invalid inbetween offsets or weight");
            breakpoints.push((at, result.targets.len()));
            result.targets.push(dense_morph_offsets(&inbetween.offsets, &indices, mesh.points.len())?);
            result.normal_targets.push(normal_target(&inbetween.normal_offsets)?);
            result.weights.push(0.0);
        }
        let primary_index = result.targets.len();
        result.targets.push(dense_morph_offsets(&primary, &indices, mesh.points.len())?);
        result.normal_targets.push(normal_target(&primary_normals)?);
        result.weights.push(0.0);
        if inbetweens.is_empty() { result.weights[primary_index] = weight; continue; }
        breakpoints.push((1.0, primary_index));
        breakpoints.sort_by(|a, b| a.0.total_cmp(&b.0));
        let weight = weight.clamp(0.0, 1.0);
        let mut lower = (0.0, None);
        let mut resolved = false;
        for (at, index) in breakpoints {
            if weight <= at {
                let factor = if at > lower.0 { (weight-lower.0)/(at-lower.0) } else { 0.0 };
                result.weights[index] = factor;
                if let Some(index) = lower.1 { result.weights[index] = 1.0-factor; }
                resolved = true;
                break;
            }
            lower = (at, Some(index));
        }
        if !resolved { result.weights[primary_index] = 1.0; }
    }
    Ok(result)
}

pub(crate) fn morph_normals(read: &super::geom::ReadMesh, sample: &MorphSample)
    -> anyhow::Result<(super::geom::MeshPrimvar<[f32; 3]>, Vec<usize>)> {
    use super::geom::Interpolation;
    let normals = read.normals.as_ref().ok_or_else(|| anyhow::anyhow!("missing authored normals"))?;
    let corner_mode = matches!(normals.interpolation, Interpolation::Uniform | Interpolation::FaceVarying);
    let mut slots = Vec::new();
    if corner_mode {
        let mut corner: usize = 0;
        for (face, &count) in read.face_vertex_counts.iter().enumerate() {
            let count = usize::try_from(count)?;
            let end = corner.checked_add(count).ok_or_else(|| anyhow::anyhow!("normal corner count overflow"))?;
            let indices = read.face_vertex_indices.get(corner..end).ok_or_else(|| anyhow::anyhow!("missing normal corners"))?;
            for (offset, &point) in indices.iter().enumerate() {
                let point = usize::try_from(point)?;
                anyhow::ensure!(point < read.points.len(), "normal corner point out of range");
                slots.push((point, if normals.interpolation == Interpolation::Uniform { face } else { corner + offset }));
            }
            corner = end;
        }
        anyhow::ensure!(corner == read.face_vertex_indices.len(), "normal corner count mismatch");
    } else {
        slots.extend((0..read.points.len()).map(|point| (point, if normals.interpolation == Interpolation::Constant { 0 } else { point })));
    }
    anyhow::ensure!(sample.normal_targets.len() == sample.weights.len()
        && sample.normal_targets.iter().all(|target| target.len() == read.points.len()), "normal target layout mismatch");
    let mut values = Vec::with_capacity(slots.len());
    let mut points = Vec::with_capacity(slots.len());
    for (point, slot) in slots {
        let index = if normals.indices.is_empty() { if normals.values.len() == 1 { 0 } else { slot } } else {
            usize::try_from(*normals.indices.get(slot).ok_or_else(|| anyhow::anyhow!("missing normal index"))?)?
        };
        let mut normal = bevy::math::Vec3::from(*normals.values.get(index).ok_or_else(|| anyhow::anyhow!("missing authored normal"))?);
        for (target, weight) in sample.normal_targets.iter().zip(&sample.weights) {
            normal += bevy::math::Vec3::from(target[point]) * *weight;
        }
        anyhow::ensure!(normal.is_finite(), "nonfinite morphed normal");
        values.push(normal.to_array());
        points.push(point);
    }
    Ok((super::geom::MeshPrimvar { values, indices: Vec::new(), interpolation:
        if corner_mode { Interpolation::FaceVarying } else { Interpolation::Vertex } }, points))
}

fn dense_morph_offsets(offsets: &[gf::Vec3f], indices: &[i32], count: usize) -> anyhow::Result<Vec<[f32; 3]>> {
    anyhow::ensure!(if indices.is_empty() { offsets.len() == count } else { offsets.len() == indices.len() },
        "morph offset count does not match source points or sparse indices");
    let mut dense = vec![[0.0; 3]; count];
    for (slot, offset) in offsets.iter().enumerate() {
        let point = if indices.is_empty() { slot } else { usize::try_from(indices[slot])? };
        anyhow::ensure!(point < count && offset.x.is_finite() && offset.y.is_finite() && offset.z.is_finite(),
            "invalid morph point index or offset");
        for (value, delta) in dense[point].iter_mut().zip([offset.x, offset.y, offset.z]) { *value += delta; }
        anyhow::ensure!(dense[point].iter().all(|value| value.is_finite()), "morph offset overflow");
    }
    Ok(dense)
}

pub(crate) fn skin_normals(stage: &Stage, path: &Path, time: Option<f64>, normals: &mut super::geom::MeshPrimvar<[f32; 3]>, source_points: &[usize], point_count: usize) -> anyhow::Result<()> {
    use bevy::math::{Mat4, Vec3};
    let binding = binding_of(stage, path).ok_or_else(|| anyhow::anyhow!("missing normal skin binding"))?;
    let skeleton_path = binding.skeleton.clone().ok_or_else(|| anyhow::anyhow!("missing normal skeleton"))?;
    let skeleton = Skeleton::get(stage, skeleton_path.clone())?.ok_or_else(|| anyhow::anyhow!("invalid normal skeleton"))?;
    let joints = skeleton.joints()?;
    let skin = SkinningResolver::from_binding(&binding.binding, &joints)?;
    let resolver = SkeletonResolver::from_skeleton(&skeleton)?;
    let locals = sample_joint_locals(stage, &skeleton_path, &binding, &joints, &resolver, time)?;
    let transforms = resolver.compute_skinning_transforms_from_local(&locals, gf::Matrix4d::IDENTITY);
    let transforms: Vec<_> = skin.remap_skinning_xforms(&transforms).iter().map(|matrix|
        Mat4::from_cols_array(&(skin.geom_bind_transform() * *matrix).0.map(|value| value as f32))).collect();
    let (indices, weights) = sampled_influences(stage, path, time)?;
    let stride = skin.num_influences_per_component();
    let count = if skin.is_rigidly_deformed() { 1 } else { point_count };
    anyhow::ensure!(source_points.len() == normals.values.len() && source_points.iter().all(|point| *point < point_count), "invalid normal point map");
    anyhow::ensure!(count.checked_mul(stride) == Some(indices.len()) && indices.len() == weights.len(), "invalid normal influence count");
    anyhow::ensure!(indices.iter().all(|index| *index >= 0 && (*index as usize) < transforms.len())
        && weights.iter().all(|weight| weight.is_finite() && *weight >= 0.0), "invalid normal influences");
    let mut normal_palette = vec![None; transforms.len()];
    for (&point, normal) in source_points.iter().zip(&mut normals.values) {
        let start = if skin.is_rigidly_deformed() { 0 } else { point * stride };
        let normal_matrix = if skin.is_rigidly_deformed() {
            let matrix = (start..start+stride).fold(Mat4::ZERO, |matrix, slot|
                matrix + transforms[indices[slot] as usize] * weights[slot]);
            normal_skin_matrix(matrix)?
        } else {
            let mut result = bevy::math::Mat3::ZERO;
            for slot in start..start+stride {
                if weights[slot] != 0.0 { result += cached_normal_skin_matrix(&transforms, &mut normal_palette, indices[slot] as usize)? * weights[slot]; }
            }
            result
        };
        let result = (normal_matrix * Vec3::from(*normal)).try_normalize()
            .ok_or_else(|| anyhow::anyhow!("invalid skinned normal"))?;
        *normal = result.to_array();
    }
    Ok(())
}

fn normal_skin_matrix(matrix: bevy::math::Mat4) -> anyhow::Result<bevy::math::Mat3> {
    let linear = bevy::math::Mat3::from_mat4(matrix);
    anyhow::ensure!(linear.is_finite() && linear.determinant().is_finite()
        && linear.determinant().abs() > 1e-20, "singular normal skin matrix");
    let normal = linear.inverse().transpose();
    anyhow::ensure!(normal.is_finite(), "nonfinite normal skin matrix");
    Ok(normal)
}

fn cached_normal_skin_matrix(matrices: &[bevy::math::Mat4], cache: &mut [Option<bevy::math::Mat3>], index: usize) -> anyhow::Result<bevy::math::Mat3> {
    if let Some(matrix) = cache[index] { return Ok(matrix); }
    let matrix = normal_skin_matrix(matrices[index])?;
    cache[index] = Some(matrix);
    Ok(matrix)
}

/// Apply the mesh's bound blend shapes to `rest` at `time`, per the canonical
/// UsdSkel pipeline (morph *before* skinning). Returns `None` when the mesh has
/// no effective (non-zero-weight) blend shapes, so the caller keeps `rest`.
///
/// Weights come from the bound `SkelAnimation`, mapped to the mesh's blend
/// shapes *by name* (the animation's `blendShapes` order need not match the
/// mesh's). Inbetween shapes are resolved via [`resolve_blend_shape_offsets`].
fn blend_shape_deform(
    stage: &Stage,
    mesh_path: &Path,
    rest: &[[f32; 3]],
    time: Option<f64>,
) -> Option<Vec<[f32; 3]>> {
    let api = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    let names = api.blend_shapes().ok()?;
    let targets = api.blend_shape_targets().ok()?;
    if names.is_empty() {
        return None;
    }

    // Resolve the weight vector from the bound animation (by-name mapping).
    let binding = binding_of(stage, mesh_path)?;
    let skel_path = binding.skeleton.clone()?;
    let anim_path = animation_source(stage, &skel_path, &binding)?;
    let anim = SkelAnimQuery::new(stage, anim_path).ok()??;
    let order = anim.blend_shape_order();
    let weights = anim
        .compute_blend_shape_weights(stage, TimeCode::new(time.unwrap_or(0.0)))
        .ok()?;
    let weight_of = |name: &str| -> f32 {
        order
            .iter()
            .position(|n| n == name)
            .and_then(|i| weights.get(i).copied())
            .unwrap_or(0.0)
    };

    // Collect owned per-shape offset/index data so the `BlendShapeWeighted`
    // slices below can borrow it.
    let mut owned: Vec<(f32, Vec<gf::Vec3f>, Vec<i32>)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let w = weight_of(name);
        if w == 0.0 {
            continue;
        }
        let Some(target) = targets.get(i) else { continue };
        let Ok(Some(bs)) = BlendShape::get(stage, target.clone()) else { continue };
        let primary = bs.offsets().unwrap_or_default();
        let point_indices = bs.point_indices().unwrap_or_default();
        let inbetweens = bs.inbetweens().unwrap_or_default();
        if inbetweens.is_empty() {
            owned.push((w, primary, point_indices));
        } else {
            // Resolve inbetweens → effective offsets at `w`; apply at weight 1.
            let ib: Vec<(f32, Vec<gf::Vec3f>)> = inbetweens
                .iter()
                .filter_map(|ib| Some((ib.weight?, ib.offsets.clone())))
                .collect();
            let ib_refs: Vec<InbetweenRef> = ib.iter().map(|(w, o)| (*w, o.as_slice())).collect();
            let resolved = resolve_blend_shape_offsets(w, &ib_refs, &primary);
            owned.push((1.0, resolved, point_indices));
        }
    }
    if owned.is_empty() {
        return Some(rest.to_vec());
    }

    let shapes: Vec<BlendShapeWeighted> = owned
        .iter()
        .map(|(w, offsets, point_indices)| BlendShapeWeighted {
            weight: *w,
            offsets,
            point_indices,
        })
        .collect();
    let pts: Vec<gf::Vec3f> = rest.iter().map(|p| gf::Vec3f::from(*p)).collect();
    let morphed = apply_blend_shapes(&pts, &shapes);
    Some(morphed.into_iter().map(|v| [v.x, v.y, v.z]).collect())
}

/// The blend-shape-morphed points for `mesh_path` at `time`, for a mesh that has
/// blend shapes but no skinning. Zero weights preserve the sampled base points.
pub fn blend_shaped_points_at(
    stage: &Stage,
    mesh_path: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    let Some(mesh) = super::geom::read_mesh_at(stage, mesh_path, time)? else {
        return Ok(None);
    };
    Ok(blend_shape_deform(stage, mesh_path, &mesh.points, time))
}

/// Walk `prim` and its ancestors for the first authored first-target of `rel`.
fn inherited_rel(stage: &Stage, prim: &Path, rel: &str) -> Option<String> {
    let mut cur = Some(prim.clone());
    while let Some(p) = cur {
        if p.is_abs_root() {
            break;
        }
        if let Ok(Some(t)) = read_rel_first_target(stage, &p, rel) {
            return Some(t);
        }
        cur = p.parent();
    }
    None
}

/// Resolve the SkelAnimation bound to the mesh: `skel:animationSource` is
/// authored on the *skeleton* (or inherited to it), not necessarily in the
/// mesh's own namespace — so resolve from the skeleton first, then fall back to
/// the mesh's binding.
fn animation_source(stage: &Stage, skel_path: &Path, mesh_binding: &SkelBinding) -> Option<Path> {
    inherited_rel(stage, skel_path, "skel:animationSource")
        .and_then(|s| openusd::sdf::path(&s).ok())
        .or_else(|| mesh_binding.animation_source.clone())
}

fn sample_joint_locals(
    stage: &Stage, skeleton: &Path, binding: &SkelBinding, joints: &[String],
    resolver: &SkeletonResolver, time: Option<f64>,
) -> anyhow::Result<Vec<gf::Matrix4d>> {
    let rest = resolver.rest_pose_local();
    anyhow::ensure!(rest.len() == joints.len(), "skeleton rest-pose count does not match joint order");
    let Some(path) = animation_source(stage, skeleton, binding) else { return Ok(rest.to_vec()) };
    let Some(animation) = SkelAnimQuery::new(stage, path)? else { return Ok(rest.to_vec()) };
    let values = animation.compute_joint_local_transforms(stage, TimeCode::new(time.unwrap_or(0.0)))?;
    anyhow::ensure!(values.len() == animation.joint_order().len(), "animation transform count does not match joint order");
    let mapper = openusd_schemas::skel::AnimMapper::new(animation.joint_order(), joints);
    Ok((0..joints.len()).map(|joint| mapper.source_index(joint).map_or(rest[joint], |index| values[index])).collect())
}

/// The enclosing `SkelRoot` of `prim` (walking up, inclusive), which scopes
/// UsdSkel binding resolution.
fn enclosing_skel_root(stage: &Stage, prim: &Path) -> Option<Path> {
    let mut cur = Some(prim.clone());
    while let Some(p) = cur {
        let is_root = stage
            .prim(p.clone()).expect("validated USD path")
            .type_name()
            .ok()
            .flatten()
            .as_deref()
            == Some("SkelRoot");
        if is_root {
            return Some(p);
        }
        cur = p.parent();
    }
    None
}

/// Resolve a mesh's binding and inherited skeleton / animation targets.
fn binding_of(stage: &Stage, mesh_path: &Path) -> Option<SkelBinding> {
    enclosing_skel_root(stage, mesh_path)?;
    let binding = SkelBindingAPI::get(stage, mesh_path.clone()).ok()??;
    let target = |name| inherited_rel(stage, mesh_path, name)
        .and_then(|path| openusd::sdf::path(&path).ok());
    Some(SkelBinding {
        prim: mesh_path.as_str().to_string(),
        skeleton: target("skel:skeleton"),
        animation_source: target("skel:animationSource"),
        binding,
    })
}

/// Whether the skinned mesh's bound SkelAnimation may vary over time (so the
/// mesh must be resampled when [`StageTime`](crate::route::StageTime) moves).
pub fn skin_is_time_varying(stage: &Stage, mesh_path: &Path) -> bool {
    if influences_are_time_varying(stage, mesh_path) { return true; }
    let Some(binding) = binding_of(stage, mesh_path) else {
        return false;
    };
    let Some(skel_path) = binding.skeleton.clone() else {
        return false;
    };
    let Some(anim_path) = animation_source(stage, &skel_path, &binding) else {
        return false;
    };
    matches!(
        SkelAnimQuery::new(stage, anim_path),
        Ok(Some(anim)) if anim.joint_transforms_might_be_time_varying()
    )
}

/// Whether joint influences, joint transforms or blend weights have time samples.
pub(crate) fn deformation_is_time_varying(stage: &Stage, mesh_path: &Path) -> bool {
    if influences_are_time_varying(stage, mesh_path) { return true; }
    let Some(binding) = binding_of(stage, mesh_path) else { return false };
    let Some(skeleton) = binding.skeleton.clone() else { return false };
    let Some(animation) = animation_source(stage, &skeleton, &binding) else { return false };
    matches!(SkelAnimQuery::new(stage, animation), Ok(Some(animation))
        if animation.joint_transforms_might_be_time_varying() || animation.blend_shape_weights_might_be_time_varying())
}

/// The skinned mesh points for `mesh_path` at `time` (`None` = rest/default),
/// or `None` if the prim isn't a resolvable skinned mesh. The result is in the
/// mesh's local space — a drop-in replacement for its rest `points`.
pub fn skinned_points_at(
    stage: &Stage,
    mesh_path: &Path,
    time: Option<f64>,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    let Some(binding) = binding_of(stage, mesh_path) else {
        return Ok(None);
    };

    // Skip absent or mismatched influence arrays.
    let (indices, weights) = sampled_influences(stage, mesh_path, time)?;
    let n_indices = indices.len();
    let n_weights = weights.len();
    if n_indices == 0 || n_indices != n_weights {
        if n_indices != n_weights {
            log::warn!(
                "usd_bevy::skel: {}: jointIndices ({n_indices}) / jointWeights ({n_weights}) \
                 length mismatch — showing the mesh un-skinned",
                mesh_path.as_str()
            );
        }
        return Ok(None);
    }

    let Some(skel_path) = binding.skeleton.clone() else {
        return Ok(None);
    };
    let Some(skeleton) = Skeleton::get(stage, skel_path.clone())? else {
        return Ok(None);
    };
    let joints = skeleton.joints()?;
    let skinning = SkinningResolver::from_binding(&binding.binding, &joints)?;
    let resolver = SkeletonResolver::from_skeleton(&skeleton)?;

    // Joint-local transforms at `time`: from the bound SkelAnimation, else the
    // skeleton's rest pose (which yields the undeformed mesh).
    let locals = sample_joint_locals(stage, &skel_path, &binding, &joints, &resolver, time)?;

    let skel_xforms =
        resolver.compute_skinning_transforms_from_local(&locals, gf::Matrix4d::IDENTITY);

    let Some(mesh) = super::geom::read_mesh_at(stage, mesh_path, time)? else {
        return Ok(None);
    };
    // Canonical UsdSkel order: morph blend shapes first, then skin the result.
    let rest = blend_shape_deform(stage, mesh_path, &mesh.points, time).unwrap_or(mesh.points);
    let pts: Vec<gf::Vec3f> = rest.iter().map(|p| gf::Vec3f::from(*p)).collect();
    let components = if skinning.is_rigidly_deformed() { 1 } else { pts.len() };
    let expected = components.checked_mul(skinning.num_influences_per_component());
    let joint_count = skinning.remap_skinning_xforms(&skel_xforms).len();
    if expected != Some(indices.len()) || indices.len() != weights.len()
        || indices.iter().any(|&i| i < 0 || i as usize >= joint_count)
        || weights.iter().any(|weight| !weight.is_finite() || *weight < 0.0)
    {
        log::warn!("usd_bevy::skel: {}: invalid influences; showing un-skinned mesh", mesh_path.as_str());
        return Ok(None);
    }
    let transforms = skinning.remap_skinning_xforms(&skel_xforms);
    let deformed = if skinning.is_rigidly_deformed() {
        let transform = openusd_schemas::skel::skinning::rigid_skinning_transform(&indices, &weights,
            skinning.num_influences_per_component(), skinning.geom_bind_transform(), &transforms);
        pts.into_iter().map(|point| transform.transform_point(point)).collect()
    } else {
        openusd_schemas::skel::skinning::skin_points_lbs(&pts, &indices, &weights,
            skinning.num_influences_per_component(), skinning.geom_bind_transform(), &transforms)
    };
    Ok(Some(deformed.into_iter().map(|v| [v.x, v.y, v.z]).collect()))
}

/// Four-lane influences and mesh-local matrices for Bevy GPU skinning.
pub struct GpuSkinSample {
    pub indices: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub matrices: Vec<bevy::math::Mat4>,
    pub(crate) normal_corrections: Vec<bevy::math::Mat3>,
}

pub fn gpu_skin_sample(stage: &Stage, mesh_path: &Path, time: Option<f64>) -> anyhow::Result<GpuSkinSample> {
    let binding = binding_of(stage, mesh_path).ok_or_else(|| anyhow::anyhow!("missing skin binding"))?;
    let skeleton_path = binding.skeleton.clone().ok_or_else(|| anyhow::anyhow!("missing skeleton"))?;
    let skeleton = Skeleton::get(stage, skeleton_path.clone())?.ok_or_else(|| anyhow::anyhow!("invalid skeleton"))?;
    let joints = skeleton.joints()?;
    let skin = SkinningResolver::from_binding(&binding.binding, &joints)?;
    anyhow::ensure!(matches!(skin.skinning_method(), openusd_schemas::skel::SkinningMethod::ClassicLinear),
        "GPU dual-quaternion skinning is unsupported");
    let stride = skin.num_influences_per_component();
    anyhow::ensure!((1..=4).contains(&stride), "GPU skinning requires 1–4 influences");
    let resolver = SkeletonResolver::from_skeleton(&skeleton)?;
    let locals = sample_joint_locals(stage, &skeleton_path, &binding, &joints, &resolver, time)?;
    let transforms = resolver.compute_skinning_transforms_from_local(&locals, gf::Matrix4d::IDENTITY);
    let matrices: Vec<_> = skin.remap_skinning_xforms(&transforms).iter().map(|matrix| {
        let combined = skin.geom_bind_transform() * *matrix;
        bevy::math::Mat4::from_cols_array(&combined.0.map(|value| value as f32))
    }).collect();
    anyhow::ensure!(!matrices.is_empty() && matrices.len() <= 256, "GPU joint palette must contain 1–256 joints");
    anyhow::ensure!(matrices.iter().all(|matrix| matrix.is_finite()), "nonfinite skinning matrix");
    let mesh = super::geom::read_mesh_at(stage, mesh_path, time)?.ok_or_else(|| anyhow::anyhow!("missing mesh"))?;
    let (indices, weights) = sampled_influences(stage, mesh_path, time)?;
    let components = if skin.is_rigidly_deformed() { 1 } else { mesh.points.len() };
    anyhow::ensure!(components.checked_mul(stride) == Some(indices.len()) && indices.len() == weights.len(), "invalid influence count");
    anyhow::ensure!(indices.iter().all(|&index| index >= 0 && (index as usize) < matrices.len())
        && weights.iter().all(|weight| weight.is_finite() && *weight >= 0.0), "invalid joint indices or weights");
    let mut packed_indices = Vec::with_capacity(mesh.points.len());
    let mut packed_weights = Vec::with_capacity(mesh.points.len());
    let mut normal_corrections = Vec::with_capacity(mesh.points.len());
    let mut normal_palette = vec![None; matrices.len()];
    for (indices, weights) in indices.chunks_exact(stride).zip(weights.chunks_exact(stride)) {
        anyhow::ensure!((weights.iter().sum::<f32>() - 1.0).abs() <= 1e-5, "GPU skinning requires normalized weights");
        let mut correction = bevy::math::Mat3::IDENTITY;
        if !crate::mesh::uses_flat_normals(&mesh) {
            let blended = indices.iter().zip(weights).fold(bevy::math::Mat4::ZERO,
                |matrix, (&index, &weight)| matrix + matrices[index as usize] * weight);
            normal_skin_matrix(blended)?;
            if !skin.is_rigidly_deformed() {
                let mut native_normal = bevy::math::Mat3::ZERO;
                for (&index, &weight) in indices.iter().zip(weights) {
                    if weight != 0.0 { native_normal += cached_normal_skin_matrix(&matrices, &mut normal_palette, index as usize)? * weight; }
                }
                correction = bevy::math::Mat3::from_mat4(blended).transpose() * native_normal;
                anyhow::ensure!(correction.is_finite(), "nonfinite normal correction");
            }
        }
        normal_corrections.push(correction);
        let mut packed_i = [0; 4];
        let mut packed_w = [0.0; 4];
        for i in 0..stride { packed_i[i] = indices[i] as u16; packed_w[i] = weights[i]; }
        packed_indices.push(packed_i);
        packed_weights.push(packed_w);
    }
    if skin.is_rigidly_deformed() {
        packed_indices.resize(mesh.points.len(), packed_indices[0]);
        packed_weights.resize(mesh.points.len(), packed_weights[0]);
        normal_corrections.resize(mesh.points.len(), normal_corrections[0]);
    }
    Ok(GpuSkinSample { indices: packed_indices, weights: packed_weights, matrices, normal_corrections })
}

#[cfg(test)]
mod tests {
    #[test]
    fn normal_palette_is_lazy_and_sample_local() {
        use bevy::math::{Mat3, Mat4, Vec3};
        let matrices = [Mat4::from_scale(Vec3::new(2.0, 1.0, 0.5)), Mat4::ZERO];
        let mut cache = vec![None; matrices.len()];
        let expected = super::normal_skin_matrix(matrices[0]).unwrap();
        assert_eq!(super::cached_normal_skin_matrix(&matrices, &mut cache, 0).unwrap(), expected);
        assert_eq!(cache, vec![Some(expected), None]);
        assert_eq!(super::cached_normal_skin_matrix(&matrices, &mut cache, 0).unwrap(), expected);
        assert!(super::cached_normal_skin_matrix(&matrices, &mut cache, 1).is_err());
        assert!(cache[1].is_none());
        let mut next_sample = vec![None; 2];
        assert_eq!(super::cached_normal_skin_matrix(&[Mat4::IDENTITY; 2], &mut next_sample, 0).unwrap(), Mat3::IDENTITY);
    }

    #[test]
    fn native_normal_fixture_matches_indexed_deformation() {
        let stages = ["skel_morph_reference.usda", "skel_morph_native_normals.usda"].map(|name| {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets").join(name);
            crate::UsdSource::new(file.to_str().unwrap(), std::fs::read(&file).unwrap()).unwrap().open_stage().unwrap()
        });
        let path = openusd::sdf::path("/Test/Face").unwrap();
        for time in [0.0, 2.5, 5.0, 7.5, 10.0] {
            let normals = stages.each_ref().map(|stage| {
                let read = super::super::geom::read_mesh_at(stage, &path, Some(time)).unwrap().unwrap();
                let sample = super::morph_sample(stage, &path, Some(time)).unwrap();
                let (mut normals, points) = super::morph_normals(&read, &sample).unwrap();
                super::skin_normals(stage, &path, Some(time), &mut normals, &points, read.points.len()).unwrap();
                normals.values
            });
            assert_eq!(normals[0], normals[1]);
        }
    }

    #[test]
    #[ignore = "requires USD_NATIVE_DEFORMATION_TOOL built against native OpenUSD"]
    fn native_baked_normals_match_combined_skin_and_morph() {
        compare_native_normals(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_native_normals.usda"));
    }

    #[test]
    #[ignore = "requires USD_NATIVE_DEFORMATION_TOOL built against native OpenUSD"]
    fn native_baked_normals_match_blended_joints() {
        compare_native_normals(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_blended_normals.usda"));
    }

    fn compare_native_normals(file: &str) {
        let tool = std::env::var("USD_NATIVE_DEFORMATION_TOOL").expect("native deformation executable");
        let original = std::fs::read(file).unwrap();
        let stage = crate::UsdSource::new(file, original.clone()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        for time in [0.0, 2.5, 5.0, 7.5, 10.0] {
            let read = super::super::geom::read_mesh_at(&stage, &path, Some(time)).unwrap().unwrap();
            let morph = super::morph_sample(&stage, &path, Some(time)).unwrap();
            let (mut normals, points) = super::morph_normals(&read, &morph).unwrap();
            super::skin_normals(&stage, &path, Some(time), &mut normals, &points, read.points.len()).unwrap();
            let output = std::process::Command::new(&tool).args([file, path.as_str(), &time.to_string(), "--normals"])
                .output().expect("run native normal tool");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let text = String::from_utf8(output.stdout).unwrap();
            let mut lines = text.lines();
            assert!(lines.next().unwrap().ends_with(" normals=4"));
            let samples = lines.collect::<Vec<_>>();
            assert_eq!(samples.len(), normals.values.len());
            for (index, (line, expected)) in samples.iter().zip(normals.values).enumerate() {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                assert_eq!(fields.len(), 4);
                assert_eq!(fields[0].parse::<usize>().unwrap(), index);
                for axis in 0..3 {
                    let actual = fields[axis + 1].parse::<f64>().unwrap();
                    assert!(actual.is_finite() && (actual - f64::from(expected[axis])).abs() < 1e-5,
                        "time {time}, normal {index}, axis {axis}: native {actual}, Bevy {}", expected[axis]);
                }
            }
        }
        assert_eq!(std::fs::read(file).unwrap(), original);
    }

    #[test]
    #[ignore = "requires USD_NATIVE_DEFORMATION_TOOL built against native OpenUSD"]
    fn native_baked_positions_match_combined_skin_and_morph() {
        let tool = std::env::var("USD_NATIVE_DEFORMATION_TOOL").expect("native deformation executable");
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/skel_morph_reference.usda");
        let original = std::fs::read(file).unwrap();
        let stage = crate::UsdSource::new(file, original.clone()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        for time in [0.0, 2.5, 5.0, 7.5, 10.0] {
            let output = std::process::Command::new(&tool).args([file, path.as_str(), &time.to_string()])
                .output().expect("run native deformation tool");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let text = String::from_utf8(output.stdout).unwrap();
            let mut lines = text.lines();
            assert!(lines.next().unwrap().ends_with(" points=4"));
            let expected = super::skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            let samples = lines.collect::<Vec<_>>();
            assert_eq!(samples.len(), expected.len());
            for (index, (line, expected)) in samples.iter().zip(expected).enumerate() {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                assert_eq!(fields.len(), 4);
                assert_eq!(fields[0].parse::<usize>().unwrap(), index);
                for axis in 0..3 {
                    let actual = fields[axis + 1].parse::<f64>().unwrap();
                    assert!(actual.is_finite() && (actual - f64::from(expected[axis])).abs() < 1e-5,
                        "time {time}, point {index}, axis {axis}: native {actual}, Bevy {}", expected[axis]);
                }
            }
        }
        assert_eq!(std::fs::read(file).unwrap(), original);
    }

    use super::*;
    use super::super::util::read_int_vec;
    use openusd::usd::Stage;

    #[test]
    fn inverse_normal_transform_validates_the_blend_not_only_each_joint() {
        use bevy::math::{Mat4, Vec3};
        let a = Mat4::IDENTITY;
        let b = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
        assert!(normal_skin_matrix(a).is_ok());
        assert!(normal_skin_matrix(b).is_ok());
        assert!(normal_skin_matrix(a * 0.5 + b * 0.5).is_err());
        assert!(normal_skin_matrix(Mat4::from_scale(Vec3::splat(f32::MAX))).is_err());
    }

    #[test]
    fn constant_influences_match_expanded_vertex_influences() {
        let open = |name| Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(&std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")).join(name).to_string_lossy()).unwrap();
        let vertex = open("skel_influences.usda");
        let constant = open("skel_constant_influences.usda");
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        for time in [0.0, 2.5, 5.0, 10.0] {
            let cpu = skinned_points_at(&constant, &path, Some(time)).unwrap().unwrap();
            assert_eq!(cpu, skinned_points_at(&vertex, &path, Some(time)).unwrap().unwrap());
            let gpu = gpu_skin_sample(&constant, &path, Some(time)).unwrap();
            let expanded = gpu_skin_sample(&vertex, &path, Some(time)).unwrap();
            assert_eq!(gpu.indices, expanded.indices);
            assert_eq!(gpu.weights, expanded.weights);
            assert_eq!(gpu.matrices, expanded.matrices);
        }
        constant.attribute("/Test/Bar.primvars:skel:jointWeights").unwrap()
            .set_at(openusd::sdf::Value::FloatVec(vec![1.0]), TimeCode::new(20.0)).unwrap();
        assert!(skinned_points_at(&constant, &path, Some(20.0)).unwrap().is_none());
        assert!(gpu_skin_sample(&constant, &path, Some(20.0)).is_err());
        assert!(gpu_skin_sample(&constant, &path, Some(0.0)).is_ok());
    }

    #[test]
    fn sampled_influences_without_defaults_drive_cpu_and_gpu() {
        use openusd::sdf::Value;
        let stage = fixture();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let rotations = stage.attribute("/Test/Skel/Anim.rotations").unwrap();
        let pose = rotations.get_at::<Value>(Some(TimeCode::new(30.0))).unwrap().unwrap();
        rotations.clear().unwrap().set(pose).unwrap();
        let indices = stage.attribute("/Test/Bar.primvars:skel:jointIndices").unwrap().clear_default().unwrap()
            .set_metadata("elementSize", Value::Int(2)).unwrap();
        let weights = stage.attribute("/Test/Bar.primvars:skel:jointWeights").unwrap().clear_default().unwrap()
            .set_metadata("elementSize", Value::Int(2)).unwrap();
        let count = super::super::geom::read_mesh(&stage, &path).unwrap().unwrap().points.len();
        indices.clone().set_at(Value::IntVec([0,1].repeat(count)), TimeCode::new(0.0)).unwrap()
            .set_at(Value::IntVec([1,0].repeat(count)), TimeCode::new(20.0)).unwrap();
        weights.clone().set_at(Value::FloatVec([1.0,0.0].repeat(count)), TimeCode::new(0.0)).unwrap()
            .set_at(Value::FloatVec([0.0,1.0].repeat(count)), TimeCode::new(10.0)).unwrap();
        assert!(is_skinned(&stage, &path));
        assert!(skin_is_time_varying(&stage, &path));
        assert!(crate::live::prim_is_animated(&stage, &path));
        let base = super::super::geom::read_mesh(&stage, &path).unwrap().unwrap().points;
        let mut poses = Vec::new();
        for time in [0.0, 5.0, 10.0, 20.0] {
            let cpu = skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            let gpu = gpu_skin_sample(&stage, &path, Some(time)).unwrap();
            for point in 0..count {
                let expected: bevy::math::Vec3 = (0..4).map(|lane|
                    gpu.matrices[gpu.indices[point][lane] as usize]
                        .transform_point3(base[point].into()) * gpu.weights[point][lane]).sum();
                assert!(expected.distance(cpu[point].into()) < 1e-5);
            }
            poses.push(cpu);
        }
        assert_ne!(poses[0], poses[2]);
        for point in 0..count {
            let midpoint = (bevy::math::Vec3::from(poses[0][point]) + bevy::math::Vec3::from(poses[2][point])) * 0.5;
            assert!(midpoint.distance(poses[1][point].into()) < 1e-5);
        }
        assert_eq!(poses[0], poses[3]);
        weights.set_at(Value::FloatVec(vec![1.0]), TimeCode::new(40.0)).unwrap();
        assert!(skinned_points_at(&stage, &path, Some(40.0)).unwrap().is_none());
        assert!(gpu_skin_sample(&stage, &path, Some(40.0)).is_err());
        assert!(skinned_points_at(&stage, &path, Some(10.0)).unwrap().is_some());
    }

    fn fixture() -> Stage {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_test_simple.usda"
        );
        Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open skel_test_simple.usda")
    }

    #[test]
    fn detects_skinned_and_animated() {
        let stage = fixture();
        let mesh = openusd::sdf::path("/Test/Bar").unwrap();
        assert!(is_skinned(&stage, &mesh), "mesh carries joint weights");
        assert!(
            skin_is_time_varying(&stage, &mesh),
            "bound SkelAnimation has time-sampled rotations"
        );
    }

    #[test]
    fn skinned_bar_fixture_is_closed_and_outward_wound() {
        use bevy::math::Vec3;
        let stage = fixture();
        let path = openusd::sdf::path("/Test/Bar").unwrap();
        let mut read = super::super::geom::read_mesh(&stage, &path).unwrap().unwrap();
        assert_eq!(read.face_vertex_counts, vec![4; 14]);
        let mut edges = std::collections::BTreeMap::<(i32,i32), Vec<(i32,i32)>>::new();
        for face in read.face_vertex_indices.chunks_exact(4) {
            let points: Vec<_> = face.iter().map(|&i| Vec3::from_array(read.points[i as usize])).collect();
            let normal = (points[1]-points[0]).cross(points[2]-points[0]);
            let center = points.iter().copied().sum::<Vec3>() / 4.0;
            assert!(normal.dot(center-Vec3::new(0.0,1.5,0.0)) > 0.0);
            for i in 0..4 {
                let (a,b) = (face[i], face[(i+1)%4]);
                edges.entry((a.min(b),a.max(b))).or_default().push((a,b));
            }
        }
        for uses in edges.values() {
            assert_eq!(uses.len(), 2);
            assert_eq!(uses[0], (uses[1].1,uses[1].0));
        }
        for time in [0.0,15.0,30.0,60.0] {
            read.points = skinned_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            let mesh = crate::mesh::mesh_from_usd(&read);
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(bevy::mesh::Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            let triangles: Vec<_> = mesh.indices().unwrap().iter().collect();
            let volume = triangles.chunks_exact(3).map(|triangle| {
                let a = Vec3::from_array(positions[triangle[0]]);
                let b = Vec3::from_array(positions[triangle[1]]);
                let c = Vec3::from_array(positions[triangle[2]]);
                a.dot(b.cross(c)) / 6.0
            }).sum::<f32>();
            assert!(volume.is_finite() && volume > 0.0);
        }
    }

    #[test]
    fn invalid_influence_indices_and_stride_do_not_panic() {
        for bad_index in [true, false] {
            let stage = fixture();
            let mesh = openusd::sdf::path("/Test/Bar").unwrap();
            let mut indices = read_int_vec(&stage, &mesh, "primvars:skel:jointIndices").unwrap();
            let mut weights = read_float_vec(&stage, &mesh, "primvars:skel:jointWeights").unwrap();
            if bad_index {
                indices[0] = -1;
            } else {
                indices.pop();
                weights.pop();
            }
            stage.attribute("/Test/Bar.primvars:skel:jointIndices").unwrap()
                .set(openusd::sdf::Value::IntVec(indices)).unwrap();
            stage.attribute("/Test/Bar.primvars:skel:jointWeights").unwrap()
                .set(openusd::sdf::Value::FloatVec(weights)).unwrap();
            assert!(skinned_points_at(&stage, &mesh, Some(0.0)).unwrap().is_none());
        }
    }

    /// A mesh whose `jointIndices`/`jointWeights` disagree in length must not
    /// panic openusd's LBS — the reader skips skinning and returns `None`
    /// (the viewer then shows the mesh un-skinned). Regression for the
    /// Hummingbird crash.
    #[test]
    fn mismatched_influences_do_not_panic() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("badskin.usda").unwrap();
        stage.define_prim("/Root").unwrap().set_type_name("SkelRoot").unwrap();
        let skel = stage.define_prim("/Root/Skel").unwrap();
        skel.set_type_name("Skeleton").unwrap();
        stage
            .create_attribute("/Root/Skel.joints", "token[]")
            .unwrap()
            .set(openusd::sdf::Value::TokenVec(vec!["J".into()]))
            .unwrap();
        let mesh = stage.define_prim("/Root/Mesh").unwrap();
        mesh.set_type_name("Mesh").unwrap().add_applied_schema("SkelBindingAPI").unwrap();
        stage
            .prim(openusd::sdf::path("/Root/Mesh").unwrap()).expect("validated USD path")
            .author_relationship_targets("skel:skeleton", [openusd::sdf::path("/Root/Skel").unwrap()])
            .unwrap();
        stage
            .create_attribute("/Root/Mesh.points", "point3f[]")
            .unwrap()
            .set(openusd::sdf::Value::Vec3fVec(vec![[0.0, 0.0, 0.0].into()]))
            .unwrap();
        stage
            .create_attribute("/Root/Mesh.faceVertexCounts", "int[]")
            .unwrap()
            .set(openusd::sdf::Value::IntVec(vec![]))
            .unwrap();
        stage
            .create_attribute("/Root/Mesh.faceVertexIndices", "int[]")
            .unwrap()
            .set(openusd::sdf::Value::IntVec(vec![]))
            .unwrap();
        // Deliberately mismatched: 4 indices, 1 weight.
        stage
            .create_attribute("/Root/Mesh.primvars:skel:jointIndices", "int[]")
            .unwrap()
            .set(openusd::sdf::Value::IntVec(vec![0, 0, 0, 0]))
            .unwrap();
        stage
            .create_attribute("/Root/Mesh.primvars:skel:jointWeights", "float[]")
            .unwrap()
            .set(openusd::sdf::Value::FloatVec(vec![1.0]))
            .unwrap();

        let mesh_path = openusd::sdf::path("/Root/Mesh").unwrap();
        // Must return Ok(None) rather than panicking.
        let result = skinned_points_at(&stage, &mesh_path, Some(0.0));
        assert!(
            matches!(result, Ok(None)),
            "mismatched influences skip skinning without panicking, got {result:?}"
        );
    }

    #[test]
    fn skinned_points_deform_over_time() {
        let stage = fixture();
        let mesh = openusd::sdf::path("/Test/Bar").unwrap();

        let p0 = skinned_points_at(&stage, &mesh, Some(0.0))
            .unwrap()
            .expect("skinned points at t=0");
        let p30 = skinned_points_at(&stage, &mesh, Some(30.0))
            .unwrap()
            .expect("skinned points at t=30");
        assert_eq!(p0.len(), p30.len(), "vertex count stable");

        // At t=0 (identity rotations) the skin equals the rest mesh.
        let mesh_read = super::super::geom::read_mesh(&stage, &mesh).unwrap().unwrap();
        assert_eq!(p0.len(), mesh_read.points.len());
        let rest_matches = p0
            .iter()
            .zip(&mesh_read.points)
            .all(|(a, b)| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3);
        assert!(rest_matches, "t=0 skin ≈ rest pose");

        // The Tip rotation at t=30 moves at least one vertex.
        let moved = p0.iter().zip(&p30).any(|(a, b)| {
            (a[0] - b[0]).abs() > 1e-3 || (a[1] - b[1]).abs() > 1e-3 || (a[2] - b[2]).abs() > 1e-3
        });
        assert!(moved, "Tip rotation deforms the mesh at t=30");
    }

    fn blend_fixture() -> Stage {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/blendshape_test.usda"
        );
        Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open blendshape_test.usda")
    }

    #[test]
    fn linear_morph_channels_match_cpu_inbetweens_without_changing_targets() {
        use openusd::sdf::Value;
        let stage = blend_fixture();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        stage.create_attribute("/Test/Face/smile.inbetweens:half", "vector3f[]").unwrap()
            .set(Value::Vec3fVec(vec![[0.0, 2.0, 0.0].into()])).unwrap()
            .set_metadata("weight", Value::Float(0.5)).unwrap();
        let animation = stage.attribute("/Test/Skel/Anim.blendShapeWeights").unwrap();
        let rest = super::super::geom::read_mesh(&stage, &path).unwrap().unwrap().points;
        let mut baseline = None;
        for weight in [-0.5, 0.0, 0.25, 0.5, 0.75, 1.0, 1.5] {
            animation.clone().set(Value::FloatVec(vec![weight])).unwrap();
            let sample = morph_sample(&stage, &path, Some(0.0)).unwrap();
            assert_eq!(sample.targets.len(), 2);
            if let Some(baseline) = &baseline { assert_eq!(&sample.targets, baseline); }
            else { baseline = Some(sample.targets.clone()); }
            let cpu = blend_shaped_points_at(&stage, &path, Some(0.0)).unwrap().unwrap();
            for point in 0..rest.len() {
                let mut result = bevy::math::Vec3::from(rest[point]);
                for (target, weight) in sample.targets.iter().zip(&sample.weights) {
                    result += bevy::math::Vec3::from(target[point]) * *weight;
                }
                assert!(result.distance(cpu[point].into()) < 1e-6);
            }
        }
        stage.attribute("/Test/Face/smile.pointIndices").unwrap().set(Value::IntVec(vec![-1])).unwrap();
        assert!(morph_sample(&stage, &path, Some(0.0)).is_err());
    }

    #[test]
    fn dense_morph_channels_validate_and_accumulate_sparse_offsets() {
        let offsets = [[1.0,0.0,0.0].into(), [0.0,2.0,0.0].into()];
        assert_eq!(dense_morph_offsets(&offsets, &[0,0], 2).unwrap(), vec![[1.0,2.0,0.0], [0.0; 3]]);
        assert!(dense_morph_offsets(&offsets, &[], 3).is_err());
        assert!(dense_morph_offsets(&offsets, &[0], 2).is_err());
        assert!(dense_morph_offsets(&offsets, &[0,2], 2).is_err());
        assert!(dense_morph_offsets(&[[f32::NAN,0.0,0.0].into()], &[], 1).is_err());
    }

    #[test]
    fn blend_shapes_apply_to_sampled_base_points() {
        let stage = blend_fixture();
        let path = openusd::sdf::path("/Test/Face").unwrap();
        let rest = super::super::geom::read_mesh(&stage, &path).unwrap().unwrap().points;
        for (time, z) in [(0.0, 0.0), (10.0, 4.0)] {
            stage.prim(path.clone()).unwrap().attribute("points").set_at(
                openusd::sdf::Value::Vec3fVec(rest.iter().map(|point| [point[0], point[1], point[2] + z].into()).collect()),
                openusd::usd::TimeCode::new(time),
            ).unwrap();
        }
        for time in [0.0, 5.0, 10.0] {
            let points = blend_shaped_points_at(&stage, &path, Some(time)).unwrap().unwrap();
            for (index, point) in points.iter().enumerate() {
                let expected = rest[index][2] + time as f32 * 0.4 + if index == 0 { 1.0 } else { 0.0 };
                assert!((point[2] - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn blend_shape_only_mesh_morphs() {
        let stage = blend_fixture();
        let mesh = openusd::sdf::path("/Test/Face").unwrap();

        assert!(has_blend_shapes(&stage, &mesh), "mesh binds a blend shape");
        assert!(!is_skinned(&stage, &mesh), "no skinning influences");

        let rest = super::super::geom::read_mesh(&stage, &mesh).unwrap().unwrap();
        let morphed = blend_shaped_points_at(&stage, &mesh, Some(0.0))
            .unwrap()
            .expect("blend-shaped points");
        assert_eq!(morphed.len(), rest.points.len());

        // The 'smile' shape at weight 1.0 pushes vertex 0 by +1 in Z; the other
        // three vertices are untouched (sparse pointIndices = [0]).
        let dz = morphed[0][2] - rest.points[0][2];
        assert!((dz - 1.0).abs() < 1e-4, "vertex 0 morphed +1 in Z, got {dz}");
        for (i, (m, r)) in morphed.iter().zip(&rest.points).enumerate().skip(1) {
            assert_eq!(m, r, "untargeted vertex {i} unchanged");
        }
    }
}
