//! Curves route (SCHEMA_INTEGRATION Phase C): `UsdGeomBasisCurves` → a Bevy
//! `LineList` mesh. Read through the geom `BasisCurves` / `Curves` schema.
//!
//! Bevy has no native curve primitive, so each curve is drawn as line segments.
//! Linear curves connect their vertices directly; **cubic** curves are
//! tessellated (PLAN Phase 6e) — each segment is evaluated through its basis
//! matrix (bezier / b-spline / catmull-rom) at configurable samples, so the
//! rendered polyline follows the smooth curve rather than its control hull.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy::math::DVec3;
use openusd_schemas::geom::BasisCurvesSchema;

use openusd_schemas::geom::{BasisCurves, Curves, PointBased};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};
use crate::read::geom::{Interpolation, MeshPrimvar};

/// Maps `UsdGeomBasisCurves` to a line-list mesh.
pub struct CurvesRoute;

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct UsdCurveError(pub String);

/// Cubic samples per segment, applied when curves are projected.
#[derive(Resource, Clone, Copy, Debug)]
pub struct UsdCurveSettings { cubic_steps: usize }

impl UsdCurveSettings {
    pub fn new(cubic_steps: usize) -> Result<Self, String> {
        if !(1..=64).contains(&cubic_steps) { return Err("cubic curve steps must be 1..=64".into()); }
        Ok(Self { cubic_steps })
    }

    pub fn cubic_steps(self) -> usize { self.cubic_steps }
}

impl Default for UsdCurveSettings {
    fn default() -> Self { Self { cubic_steps: CUBIC_STEPS } }
}

pub(crate) fn current_steps(world: &World) -> usize {
    world.get_resource::<UsdCurveSettings>().copied().unwrap_or_default().cubic_steps
}

pub(crate) fn refresh_geometry(world: &mut World, stage: &openusd::usd::Stage, map: &crate::live::PrimEntities) {
    let registry = world.resource::<super::SchemaRegistry>().clone();
    for (path, entity) in map.iter() {
        let Ok(path) = openusd::sdf::path(path) else { continue };
        if stage.prim(&path).ok().and_then(|prim| prim.type_name().ok().flatten()).as_deref() == Some("BasisCurves") {
            registry.patch_prim(stage, &path, world, entity, &[]);
        }
    }
}

pub const MAX_CURVE_VERTICES: usize = 1_000_000;
pub const MAX_CURVE_INDICES: usize = 2_000_000;

fn accumulate_curve_output(total: &mut (usize, usize), vertices: usize, indices: usize) -> Result<(), String> {
    let vertices = total.0.checked_add(vertices).ok_or("curve vertex count overflow")?;
    let indices = total.1.checked_add(indices).ok_or("curve index count overflow")?;
    if vertices > MAX_CURVE_VERTICES || indices > MAX_CURVE_INDICES {
        return Err(format!("curve output {vertices} vertices/{indices} indices exceeds limits {MAX_CURVE_VERTICES}/{MAX_CURVE_INDICES}"));
    }
    *total = (vertices, indices);
    Ok(())
}

fn validate_curves(ctx: &RouteCtx, steps: usize) -> Result<(), String> {
    let curves = BasisCurves::get(ctx.stage, ctx.path.clone()).map_err(|error| error.to_string())?
        .ok_or("missing BasisCurves schema")?;
    let points = read_points(&curves, ctx.time).ok_or("missing or invalid curve points")?;
    if points.iter().flatten().any(|value| !value.is_finite()) { return Err("non-finite curve point".into()); }
    let counts = curves.curve_vertex_counts_attr().get_at::<Value>(ctx.time.map(openusd::usd::TimeCode::new))
        .map_err(|error| error.to_string())?;
    let counts = if let Some(counts) = counts {
        let Value::IntVec(counts) = counts else { return Err("invalid curveVertexCounts type".into()); };
        let total = counts.iter().try_fold(0usize, |total, count| {
            usize::try_from(*count).ok().and_then(|count| total.checked_add(count))
        }).ok_or("negative or overflowing curveVertexCounts")?;
        if total != points.len() { return Err(format!("curveVertexCounts sum {total} differs from {} points", points.len())); }
        counts.into_iter().map(|count| count as usize).collect::<Vec<_>>()
    } else { vec![points.len()] };
    let kind = read_token(curves.type_attr(), "cubic", ctx.time);
    if !matches!(kind.as_str(), "linear" | "cubic") { return Err(format!("unsupported curve type {kind}")); }
    let wrap = read_token(curves.wrap_attr(), "nonperiodic", ctx.time);
    if !matches!(wrap.as_str(), "nonperiodic" | "periodic" | "pinned") { return Err(format!("unsupported curve wrap {wrap}")); }
    let basis = read_token(curves.basis_attr(), "bezier", ctx.time);
    if kind == "cubic" && !matches!(basis.as_str(), "bezier" | "bspline" | "catmullRom") { return Err(format!("unsupported cubic basis {basis}")); }
    let stride = if basis == "bezier" { 3 } else { 1 };
    let mut varying_count = 0usize;
    let mut output_size = (0usize, 0usize);
    for (curve,count) in counts.iter().copied().enumerate() {
        if count == 0 { continue; }
        let valid = if kind == "linear" { count >= 2 }
            else if wrap == "periodic" { count >= 3 && count % stride == 0 }
            else if wrap == "pinned" && basis != "bezier" { count >= 2 }
            else { count >= 4 && (count - 4) % stride == 0 };
        if !valid { return Err(format!("unsupported {kind}/{basis}/{wrap} layout: curve {curve} has {count} control points")); }
        let varying = if kind == "linear" { count }
            else if wrap == "periodic" { count / stride }
            else if wrap == "pinned" && basis != "bezier" { count }
            else { (count - 4) / stride + 2 };
        varying_count = varying_count.checked_add(varying).ok_or("varying sample count overflow")?;
        let segments = varying - usize::from(wrap != "periodic");
        let (vertices, segments) = if kind == "linear" { (count, segments) } else {
            let segments = segments.checked_mul(steps).ok_or("curve segment count overflow")?;
            (segments.checked_add(1).ok_or("curve vertex count overflow")?, segments)
        };
        accumulate_curve_output(&mut output_size, vertices, segments.checked_mul(2).ok_or("curve index count overflow")?)?;
    }
    if let Some(color) = crate::read::geom::read_primvar_vec3f(ctx.stage, ctx.path, "primvars:displayColor", ctx.time).map_err(|error| error.to_string())? {
        validate_curve_primvar(&color, counts.len(), points.len(), varying_count, "displayColor")?;
        if color.values.iter().flatten().any(|value| !value.is_finite()) { return Err("non-finite displayColor".into()); }
    }
    if let Some(opacity) = crate::read::geom::read_primvar_float(ctx.stage, ctx.path, "primvars:displayOpacity", ctx.time).map_err(|error| error.to_string())? {
        validate_curve_primvar(&opacity, counts.len(), points.len(), varying_count, "displayOpacity")?;
        if opacity.values.iter().any(|value| !value.is_finite()) { return Err("non-finite displayOpacity".into()); }
    }
    Ok(())
}

fn validate_curve_primvar<T>(value: &MeshPrimvar<T>, curves: usize, points: usize, varying: usize, name: &str) -> Result<(), String> {
    let expected = match value.interpolation {
        Interpolation::Constant => 1,
        Interpolation::Uniform => curves,
        Interpolation::Vertex => points,
        Interpolation::Varying => varying,
        Interpolation::FaceVarying => return Err(format!("unsupported faceVarying {name}")),
    };
    if value.values.len() == 1 && value.indices.is_empty() { return Ok(()); }
    let actual = if value.indices.is_empty() { value.values.len() } else { value.indices.len() };
    if actual != expected { return Err(format!("{name} has {actual} samples; expected {expected}")); }
    if value.indices.iter().any(|index| usize::try_from(*index).map_or(true, |index| index >= value.values.len())) {
        return Err(format!("{name} index outside its value array"));
    }
    Ok(())
}

fn read_points(curves: &BasisCurves, time: Option<f64>) -> Option<Vec<[f32; 3]>> {
    match curves.points_attr().get_at::<Value>(time.map(openusd::usd::TimeCode::new)) {
        Ok(Some(Value::Vec3fVec(v))) => Some(v.iter().map(|p| [p.x, p.y, p.z]).collect()),
        Ok(Some(Value::Vec3dVec(v))) => {
            Some(v.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect())
        }
        _ => None,
    }
}

fn read_counts(curves: &BasisCurves, time: Option<f64>) -> Vec<i32> {
    match curves.curve_vertex_counts_attr().get_at::<Value>(time.map(openusd::usd::TimeCode::new)) {
        Ok(Some(Value::IntVec(v))) => v,
        _ => Vec::new(),
    }
}

fn read_token(attr: openusd::usd::Attribute, default: &str, time: Option<f64>) -> String {
    match attr.get_at::<Value>(time.map(openusd::usd::TimeCode::new)) {
        Ok(Some(Value::Token(t))) => t.as_str().to_string(),
        Ok(Some(Value::String(s))) => s,
        _ => default.to_string(),
    }
}

/// Default samples per cubic segment.
pub const CUBIC_STEPS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Basis {
    Bezier,
    Bspline,
    CatmullRom,
}

impl Basis {
    fn parse(s: &str) -> Self {
        match s {
            "bezier" => Basis::Bezier,
            "catmullRom" => Basis::CatmullRom,
            _ => Basis::Bspline,
        }
    }

    /// Control-point stride between consecutive cubic segments.
    fn vstep(self) -> usize {
        match self {
            Basis::Bezier => 3,
            Basis::Bspline | Basis::CatmullRom => 1,
        }
    }

    /// The four blending weights for parameter `t` in `[0, 1]`.
    fn weights(self, t: f64) -> [f64; 4] {
        let (t2, t3) = (t * t, t * t * t);
        match self {
            Basis::Bezier => {
                let u = 1.0 - t;
                [u * u * u, 3.0 * t * u * u, 3.0 * t2 * u, t3]
            }
            Basis::Bspline => [
                (-t3 + 3.0 * t2 - 3.0 * t + 1.0) / 6.0,
                (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0,
                (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0,
                t3 / 6.0,
            ],
            Basis::CatmullRom => [
                0.5 * (-t3 + 2.0 * t2 - t),
                0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
                0.5 * (-3.0 * t3 + 4.0 * t2 + t),
                0.5 * (t3 - t2),
            ],
        }
    }
}

fn eval(cvs: [[f32; 3]; 4], w: [f32; 4]) -> [f32; 3] {
    let mut p = [0.0f32; 3];
    for k in 0..4 {
        for c in 0..3 {
            p[c] += w[k] * cvs[k][c];
        }
    }
    p
}

/// Append a tessellated cubic curve (points `cv`, `periodic` wrap) to the
/// output position + line-index buffers.
fn tessellate_cubic(cv: &[[f32; 3]], basis: Basis, periodic: bool, out: &mut Vec<[f32; 3]>, idx: &mut Vec<u32>, steps: usize) {
    let n = cv.len();
    if n < 4 && !(periodic && n >= 3) {
        // Not enough CVs for a cubic segment; fall back to a polyline.
        emit_polyline(cv, periodic, out, idx);
        return;
    }
    let vstep = basis.vstep();
    // Nonperiodic: last full window starts at n-4. Periodic: windows wrap.
    let seg_count = if periodic {
        n / vstep
    } else {
        (n - 4) / vstep + 1
    };
    for s in 0..seg_count {
        let base = s * vstep;
        let cvs = [
            cv[base % n],
            cv[(base + 1) % n],
            cv[(base + 2) % n],
            cv[(base + 3) % n],
        ];
        // First sample of a segment coincides with the previous segment's last;
        // start at step 1 for continued segments to avoid duplicate joints.
        let start = if s == 0 { 0 } else { 1 };
        for step in start..=steps {
            let t = step as f32 / steps as f32;
            let cur = out.len() as u32;
            out.push(eval(cvs, basis.weights(t as f64).map(|weight| weight as f32)));
            if cur > 0 && !(s == 0 && step == 0) {
                idx.push(cur - 1);
                idx.push(cur);
            }
        }
    }
}

fn tessellate_pinned(cv: &[[f32; 3]], basis: Basis, out: &mut Vec<[f32; 3]>, idx: &mut Vec<u32>, steps: usize) {
    if cv.len() < 2 || basis == Basis::Bezier {
        tessellate_cubic(cv, basis, false, out, idx, steps);
        return;
    }
    let phantom = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| (2.0 * a[i] as f64 - b[i] as f64) as f32);
    let mut expanded = Vec::with_capacity(cv.len() + 2);
    expanded.push(phantom(cv[0], cv[1]));
    expanded.extend_from_slice(cv);
    expanded.push(phantom(cv[cv.len() - 1], cv[cv.len() - 2]));
    tessellate_cubic(&expanded, basis, false, out, idx, steps);
}

/// Append consecutive straight segments and optional last-to-first closure.
fn emit_polyline(cv: &[[f32; 3]], periodic: bool, out: &mut Vec<[f32; 3]>, idx: &mut Vec<u32>) {
    let base = out.len() as u32;
    out.extend_from_slice(cv);
    for i in 0..cv.len().saturating_sub(1) {
        idx.push(base + i as u32);
        idx.push(base + i as u32 + 1);
    }
    if periodic && cv.len() > 1 {
        idx.extend([base + cv.len() as u32 - 1, base]);
    }
}

/// Positions + line indices for every curve. Linear curves connect vertices
/// directly; cubic curves are tessellated through their basis.
fn line_geometry(ctx: &RouteCtx, steps: usize) -> Option<(Vec<[f32; 3]>, Vec<u32>, Option<Vec<[f32; 4]>>)> {
    let curves = BasisCurves::get(ctx.stage, ctx.path.clone()).ok()??;
    let points = read_points(&curves, ctx.time)?;
    if points.is_empty() {
        return None;
    }
    let is_cubic = read_token(curves.type_attr(), "cubic", ctx.time) == "cubic";
    let basis = Basis::parse(&read_token(curves.basis_attr(), "bezier", ctx.time));
    let wrap = read_token(curves.wrap_attr(), "nonperiodic", ctx.time);
    let periodic = wrap == "periodic";

    let mut counts = read_counts(&curves, ctx.time);
    // Absent counts ⇒ one curve spanning all points.
    if counts.is_empty() {
        counts = vec![points.len() as i32];
    }

    let mut out: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let color = crate::read::geom::read_primvar_vec3f(ctx.stage, ctx.path, "primvars:displayColor", ctx.time).ok().flatten();
    let color = color.map(|value| MeshPrimvar { values: value.values.into_iter().map(|value| Vec3::from(value).as_dvec3()).collect(), interpolation: value.interpolation, indices: value.indices });
    let opacity = crate::read::geom::read_primvar_float(ctx.stage, ctx.path, "primvars:displayOpacity", ctx.time).ok().flatten();
    let opacity = opacity.map(|value| MeshPrimvar { values: value.values.into_iter().map(f64::from).collect(), interpolation: value.interpolation, indices: value.indices });
    let mut colors = (color.is_some() || opacity.is_some()).then(Vec::new);
    let mut cursor = 0usize;
    let mut varying_offset = 0usize;
    for (curve, c) in counts.into_iter().enumerate() {
        let n = c.max(0) as usize;
        let end = (cursor + n).min(points.len());
        let cv = &points[cursor..end];
        let first = out.len();
        if is_cubic && wrap == "pinned" {
            tessellate_pinned(cv, basis, &mut out, &mut indices, steps);
        } else if is_cubic {
            tessellate_cubic(cv, basis, periodic, &mut out, &mut indices, steps);
        } else {
            emit_polyline(cv, periodic, &mut out, &mut indices);
        }
        let pinned = wrap == "pinned" && basis != Basis::Bezier;
        let cubic = is_cubic && (cv.len() >= 4 || (periodic && cv.len() >= 3) || (pinned && cv.len() >= 2));
        let samples = out.len() - first;
        let segments = if cubic { samples.saturating_sub(1) / steps } else { 0 };
        let layout = CurveSampling { steps, curve, point_offset: cursor, varying_offset, count: cv.len(), segments, basis, cubic, periodic, pinned };
        if let Some(colors) = &mut colors {
            for sample in 0..samples {
                let rgb = color.as_ref().map(|value| layout.sample(value, sample, DVec3::ONE)).unwrap_or(DVec3::ONE);
                let alpha = opacity.as_ref().map(|value| layout.sample(value, sample, 1.)).unwrap_or(1.);
                colors.push([rgb.x as f32, rgb.y as f32, rgb.z as f32, alpha as f32]);
            }
        }
        varying_offset += if cubic { segments + usize::from(!periodic) } else { cv.len() };
        cursor = end;
    }
    Some((out, indices, colors))
}

struct CurveSampling {
    steps: usize,
    curve: usize,
    point_offset: usize,
    varying_offset: usize,
    count: usize,
    segments: usize,
    basis: Basis,
    cubic: bool,
    periodic: bool,
    pinned: bool,
}

impl CurveSampling {
    fn sample<T>(&self, value: &MeshPrimvar<T>, sample: usize, fallback: T) -> T
    where T: Copy + std::ops::Add<Output=T> + std::ops::Sub<Output=T> + std::ops::Mul<f64, Output=T> {
        let lookup = |slot: usize| {
            let index = if value.indices.is_empty() { Some(slot) }
                else { value.indices.get(slot).and_then(|index| usize::try_from(*index).ok()) };
            index.and_then(|index| value.values.get(index)).copied().unwrap_or(fallback)
        };
        if value.values.len() == 1 && value.indices.is_empty() { return value.values[0]; }
        match value.interpolation {
            Interpolation::Constant => return lookup(0),
            Interpolation::Uniform => return lookup(self.curve),
            Interpolation::FaceVarying => return fallback,
            _ => {}
        }
        if !self.cubic {
            let offset = if value.interpolation == Interpolation::Vertex { self.point_offset } else { self.varying_offset };
            return lookup(offset + sample);
        }
        let segment = (sample / self.steps).min(self.segments - 1);
        let t = (sample - segment * self.steps) as f64 / self.steps as f64;
        if value.interpolation == Interpolation::Varying {
            let next = if self.periodic { (segment + 1) % self.segments } else { segment + 1 };
            return lookup(self.varying_offset + segment) * (1. - t) + lookup(self.varying_offset + next) * t;
        }
        let control = |index: usize| {
            if self.periodic { return lookup(self.point_offset + index % self.count); }
            if self.pinned {
                if index == 0 { return lookup(self.point_offset) * 2. - lookup(self.point_offset + 1); }
                if index == self.count + 1 { return lookup(self.point_offset + self.count - 1) * 2. - lookup(self.point_offset + self.count - 2); }
                return lookup(self.point_offset + index - 1);
            }
            lookup(self.point_offset + index)
        };
        let weights = self.basis.weights(t);
        let base = segment * self.basis.vstep();
        control(base) * weights[0] + control(base + 1) * weights[1] + control(base + 2) * weights[2] + control(base + 3) * weights[3]
    }
}

impl PrimRoute for CurvesRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Curves);
        world.entity_mut(entity).remove::<UsdCurveError>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("BasisCurves")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            return;
        }
        let steps = current_steps(world);
        if let Err(error) = validate_curves(ctx, steps) {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Curves);
            world.entity_mut(entity).insert(UsdCurveError(error));
            return;
        }
        world.entity_mut(entity).remove::<UsdCurveError>();
        let Some((points, indices, colors)) = line_geometry(ctx, steps) else {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Curves);
            return;
        };
        if points.iter().flatten().any(|value| !value.is_finite()) {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Curves);
            world.entity_mut(entity).insert(UsdCurveError("non-finite tessellated curve point".into()));
            return;
        }
        if colors.as_ref().is_some_and(|colors| colors.iter().flatten().any(|value| !value.is_finite())) {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Curves);
            world.entity_mut(entity).insert(UsdCurveError("non-finite tessellated curve color/opacity".into()));
            return;
        }
        let translucent = colors.as_ref().is_some_and(|colors| colors.iter().any(|color| color[3].is_finite() && color[3] < 1.));
        let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, points);
        if let Some(colors) = colors { mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors); }
        mesh.insert_indices(bevy::mesh::Indices::U32(indices));
        let mesh_handle = super::cache::intern_mesh(world, mesh);
        let mut material = super::material::default_material(ctx);
        material.unlit = true;
        material.alpha_mode = if translucent { AlphaMode::Blend } else { AlphaMode::Opaque };
        let material = super::cache::intern_material(world, material);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material), super::geom::GeometryOwner::Curves));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;

    #[test]
    fn curve_color_intermediates_stay_wide_and_overflow_is_reported() {
        let source = crate::UsdSource::new("gradients.usda", include_bytes!("../../../../assets/curve_gradients.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Vertex").unwrap();
        stage.attribute("/Vertex.basis").unwrap().set(Value::Token("catmullRom".into())).unwrap();
        stage.create_attribute("/Vertex.wrap", "token").unwrap().set(Value::Token("pinned".into())).unwrap();
        stage.attribute("/Vertex.primvars:displayColor:indices").unwrap().clear().unwrap();
        stage.attribute("/Vertex.primvars:displayOpacity").unwrap().clear().unwrap();
        let color = stage.attribute("/Vertex.primvars:displayColor").unwrap();
        color.clone().set(Value::Vec3fVec(vec![[f32::MAX;3].into();8])).unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let entity = world.spawn_empty().id();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<UsdCurveError>(entity).is_none());
        let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
        assert!(colors.iter().all(|color| color[..3] == [f32::MAX;3]));
        stage.attribute("/Vertex.wrap").unwrap().set(Value::Token("nonperiodic".into())).unwrap();
        color.clone().set(Value::Vec3fVec(vec![[0.;3].into(), [f32::MAX;3].into(), [f32::MAX;3].into(), [0.;3].into(),
            [0.;3].into(), [f32::MAX;3].into(), [f32::MAX;3].into(), [0.;3].into()])).unwrap();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_none());
        assert_eq!(world.get::<UsdCurveError>(entity).unwrap().0, "non-finite tessellated curve color/opacity");
        color.set(Value::Vec3fVec(vec![[1.;3].into();8])).unwrap();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_some());
        assert!(world.get::<UsdCurveError>(entity).is_none());
    }

    #[test]
    fn curve_output_limits_are_inclusive_and_do_not_wrap() {
        let mut total = (0,0);
        accumulate_curve_output(&mut total, MAX_CURVE_VERTICES, MAX_CURVE_INDICES).unwrap();
        assert!(accumulate_curve_output(&mut total, 1, 0).is_err());
        assert!(accumulate_curve_output(&mut total, 0, 1).is_err());
        assert_eq!(total, (MAX_CURVE_VERTICES, MAX_CURVE_INDICES));
        assert!(accumulate_curve_output(&mut total, usize::MAX, 0).is_err());
        assert!(accumulate_curve_output(&mut total, 0, usize::MAX).is_err());
    }

    #[test]
    fn oversized_curve_projection_allocates_no_mesh_and_recovers() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("oversized.usda").unwrap();
        stage.define_prim("/Curve").unwrap().set_type_name("BasisCurves").unwrap();
        stage.create_attribute("/Curve.basis", "token").unwrap().set(Value::Token("bspline".into())).unwrap();
        let points = stage.create_attribute("/Curve.points", "point3f[]").unwrap();
        let counts = stage.create_attribute("/Curve.curveVertexCounts", "int[]").unwrap();
        let count = MAX_CURVE_VERTICES / CUBIC_STEPS + 3;
        points.clone().set(Value::Vec3fVec(vec![[0.,0.,0.].into();count])).unwrap();
        counts.clone().set(Value::IntVec(vec![count as i32])).unwrap();
        let path = openusd::sdf::path("/Curve").unwrap();
        assert!(validate_curves(&RouteCtx::new(&stage, &path), 1).is_ok());
        assert!(validate_curves(&RouteCtx::new(&stage, &path), 64).is_err());
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let entity = world.spawn_empty().id();
        let child = world.spawn(ChildOf(entity)).id();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
        assert!(world.get::<UsdCurveError>(entity).unwrap().0.contains("exceeds limits"));
        points.set(Value::Vec3fVec(vec![[0.,0.,0.].into();4])).unwrap();
        counts.set(Value::IntVec(vec![4])).unwrap();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_some());
        assert!(world.get::<UsdCurveError>(entity).is_none());
        assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), entity);
    }

    #[test]
    fn curve_primvar_cardinality_and_indices_are_validated() {
        for (interpolation, count) in [(Interpolation::Constant,1), (Interpolation::Uniform,2), (Interpolation::Vertex,8), (Interpolation::Varying,4)] {
            let mut value = MeshPrimvar { values: vec![0.5;count], interpolation, indices: Vec::new() };
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_ok());
            value.values = vec![0.5,0.75];
            value.indices = vec![1;count];
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_ok());
            value.indices[0] = -1;
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_err());
            value.indices[0] = 2;
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_err());
            value.indices = vec![0;count+1];
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_err());
            value.values = vec![0.5];
            value.indices.clear();
            assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_ok());
        }
        let value = MeshPrimvar { values: vec![1.], interpolation: Interpolation::FaceVarying, indices: Vec::new() };
        assert!(validate_curve_primvar(&value, 2, 8, 4, "test").is_err());
    }

    #[test]
    fn invalid_curve_display_arrays_suppress_and_recover() {
        let source = crate::UsdSource::new("gradients.usda", include_bytes!("../../../../assets/curve_gradients.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins(crate::live::LiveStagePlugin);
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.world_mut().insert_non_send(LiveStage::new(stage.clone()));
        app.world_mut().run_schedule(Update);
        let entity = app.world().resource::<PrimEntities>().entity("/Vertex").unwrap();
        let child = app.world_mut().spawn(ChildOf(entity)).id();
        let indices = stage.attribute("/Vertex.primvars:displayColor:indices").unwrap();
        for invalid in [vec![0,1], vec![-1;8], vec![4;8]] {
            indices.clone().set(Value::IntVec(invalid)).unwrap();
            app.world_mut().run_schedule(Update);
            assert!(app.world().get::<Mesh3d>(entity).is_none());
            assert!(app.world().get::<UsdCurveError>(entity).unwrap().0.contains("displayColor"));
            indices.clone().set(Value::IntVec(vec![0,1,1,2,2,3,3,0])).unwrap();
            app.world_mut().run_schedule(Update);
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(app.world().get::<UsdCurveError>(entity).is_none());
        }
        stage.attribute("/Vertex.primvars:displayOpacity").unwrap().set(Value::FloatVec(vec![f32::NAN;4])).unwrap();
        app.world_mut().run_schedule(Update);
        assert!(app.world().get::<Mesh3d>(entity).is_none());
        assert_eq!(app.world().get::<UsdCurveError>(entity).unwrap().0, "non-finite displayOpacity");
        stage.attribute("/Vertex.primvars:displayOpacity").unwrap().set(Value::FloatVec(vec![1.;4])).unwrap();
        app.world_mut().run_schedule(Update);
        assert!(app.world().get::<Mesh3d>(entity).is_some());
        assert!(app.world().get::<UsdCurveError>(entity).is_none());
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Vertex"), Some(entity));
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
    }

    #[test]
    fn curve_layout_validation_rejects_unused_controls_and_unknown_tokens() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("layout.usda").unwrap();
        stage.define_prim("/Curve").unwrap().set_type_name("BasisCurves").unwrap();
        let path = openusd::sdf::path("/Curve").unwrap();
        for (kind,basis,wrap,count,valid) in [
            ("cubic","bezier","nonperiodic",4,true), ("cubic","bezier","nonperiodic",5,false),
            ("cubic","bezier","nonperiodic",7,true), ("cubic","bezier","periodic",3,true),
            ("cubic","bezier","periodic",4,false), ("cubic","bezier","periodic",6,true),
            ("cubic","bspline","pinned",2,true), ("cubic","catmullRom","pinned",2,true),
            ("cubic","bspline","nonperiodic",3,false), ("cubic","catmullRom","periodic",3,true),
            ("linear","ignored","nonperiodic",2,true), ("linear","ignored","periodic",3,true),
            ("cubic","power","nonperiodic",4,false), ("unknown","bezier","nonperiodic",4,false),
            ("cubic","bezier","unknown",4,false),
        ] {
            for (name,value) in [("type",kind), ("basis",basis), ("wrap",wrap)] {
                stage.create_attribute(format!("/Curve.{name}"), "token").unwrap().set(Value::Token(value.into())).unwrap();
            }
            stage.create_attribute("/Curve.points", "point3f[]").unwrap().set(Value::Vec3fVec(vec![[0.,0.,0.].into();count])).unwrap();
            stage.create_attribute("/Curve.curveVertexCounts", "int[]").unwrap().set(Value::IntVec(vec![count as i32])).unwrap();
            let result = validate_curves(&RouteCtx::new(&stage, &path), CUBIC_STEPS);
            assert_eq!(result.is_ok(), valid, "{kind}/{basis}/{wrap}/{count}: {result:?}");
        }
    }

    #[test]
    fn curve_errors_clear_geometry_and_recover_without_losing_children() {
        let source = crate::UsdSource::new("curves.usda", include_bytes!("../../../../assets/curve_colors.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins(crate::live::LiveStagePlugin);
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.world_mut().insert_non_send(LiveStage::new(stage.clone()));
        app.world_mut().run_schedule(Update);
        let entity = app.world().resource::<PrimEntities>().entity("/Root/Inherited").unwrap();
        let child = app.world_mut().spawn(ChildOf(entity)).id();
        let counts = stage.attribute("/Root/Inherited.curveVertexCounts").unwrap();
        for invalid in [vec![5], vec![-1,5], vec![2], vec![]] {
            counts.clone().set(Value::IntVec(invalid)).unwrap();
            app.world_mut().run_schedule(Update);
            assert!(app.world().get::<Mesh3d>(entity).is_none());
            assert!(app.world().get::<UsdCurveError>(entity).is_some());
            counts.clone().set(Value::IntVec(vec![2,2])).unwrap();
            app.world_mut().run_schedule(Update);
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(app.world().get::<UsdCurveError>(entity).is_none());
        }
        stage.attribute("/Root/Inherited.points").unwrap().set(Value::Vec3fVec(vec![[f32::INFINITY,0.,0.].into();4])).unwrap();
        app.world_mut().run_schedule(Update);
        assert!(app.world().get::<Mesh3d>(entity).is_none());
        assert_eq!(app.world().get::<UsdCurveError>(entity).unwrap().0, "non-finite curve point");
        stage.prim(openusd::sdf::path("/Root/Inherited").unwrap()).unwrap().set_type_name("Xform").unwrap();
        app.world_mut().run_schedule(Update);
        assert!(app.world().get::<UsdCurveError>(entity).is_none());
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Root/Inherited"), Some(entity));
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
    }

    #[test]
    fn curve_blending_tracks_generated_opacity_and_recovers() {
        let source = crate::UsdSource::new("opacity.usda", br#"#usda 1.0
def BasisCurves "Curve" {
    uniform token type = "cubic"
    uniform token basis = "catmullRom"
    int[] curveVertexCounts = [4]
    point3f[] points = [(-2,0,0), (-1,0,0), (1,0,0), (2,0,0)]
    float[] primvars:displayOpacity (interpolation = "vertex")
    float[] primvars:displayOpacity.timeSamples = {0: [2,1,1,2], 10: [1,1,1,1]}
}
"#.as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins(crate::live::LiveStagePlugin);
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.world_mut().insert_non_send(LiveStage::new(source.open_stage().unwrap()));
        for time in [0.,10.,0.] {
            app.world_mut().resource_mut::<super::super::StageTime>().current = time;
            app.world_mut().run_schedule(Update);
            let entity = app.world().resource::<PrimEntities>().entity("/Curve").unwrap();
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
            assert_eq!(colors[CUBIC_STEPS/2][3], if time == 0. { 0.875 } else { 1. });
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.alpha_mode, if time == 0. { AlphaMode::Blend } else { AlphaMode::Opaque });
        }
    }

    #[test]
    fn indexed_curve_gradients_keep_vertex_and_varying_offsets_distinct() {
        let source = crate::UsdSource::new("gradients.usda", include_bytes!("../../../../assets/curve_gradients.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        for (name, midpoints) in [("Vertex", [[0.125,0.75,0.125], [0.875,0.75,0.125]]), ("Varying", [[0.5,0.,0.5];2])] {
            let path = openusd::sdf::path(&format!("/{name}")).unwrap();
            let (positions, _, colors) = line_geometry(&RouteCtx::new(&stage, &path), CUBIC_STEPS).unwrap();
            let colors = colors.unwrap();
            assert_eq!(colors.len(), positions.len());
            assert_eq!(colors.len(), 2 * (CUBIC_STEPS + 1));
            for curve in 0..2 {
                let base = curve * (CUBIC_STEPS + 1);
                let actual = colors[base + CUBIC_STEPS / 2];
                assert!(Vec3::new(actual[0],actual[1],actual[2]).abs_diff_eq(Vec3::from(midpoints[curve]), 1e-6));
                assert!((actual[3] - 0.6).abs() < 1e-6);
                assert_eq!(colors[base], if curve == 0 { [1.,0.,0.,0.2] } else { [0.,0.,1.,1.] });
                assert_eq!(colors[base + CUBIC_STEPS], if curve == 0 { [0.,0.,1.,1.] } else { [1.,0.,0.,0.2] });
            }
        }
    }

    #[test]
    fn vertex_curve_sampling_matches_geometry_for_all_cubic_wraps() {
        let controls = [[0.,0.,0.], [1.,2.,0.], [2.,1.,0.], [3.,0.,0.], [4.,1.,0.], [5.,0.,0.]];
        for basis in [Basis::Bezier, Basis::Bspline, Basis::CatmullRom] {
            for (periodic,pinned,count) in [(false,false,4), (true,false,6), (true,false,3), (false,true,2), (false,true,4)] {
                if pinned && basis == Basis::Bezier { continue; }
                let cv = &controls[..count];
                let mut positions = Vec::new();
                let mut indices = Vec::new();
                if pinned { tessellate_pinned(cv, basis, &mut positions, &mut indices, CUBIC_STEPS); }
                else { tessellate_cubic(cv, basis, periodic, &mut positions, &mut indices, CUBIC_STEPS); }
                let value = MeshPrimvar { values: cv.iter().copied().map(|value| Vec3::from(value).as_dvec3()).collect(), interpolation: Interpolation::Vertex, indices: Vec::new() };
                let layout = CurveSampling { steps: CUBIC_STEPS, curve: 0, point_offset: 0, varying_offset: 0, count, segments: (positions.len()-1)/CUBIC_STEPS, basis, cubic: true, periodic, pinned };
                for (sample, expected) in positions.into_iter().enumerate() {
                    assert!(layout.sample(&value, sample, DVec3::ONE).abs_diff_eq(Vec3::from(expected).as_dvec3(), 1e-5));
                }
            }
        }
    }

    #[test]
    fn varying_periodic_samples_wrap_within_their_batch() {
        let value = MeshPrimvar { values: vec![99., 0., 1., 2.], interpolation: Interpolation::Varying, indices: vec![0,3,2,1] };
        let layout = CurveSampling { steps: CUBIC_STEPS, curve: 1, point_offset: 7, varying_offset: 1, count: 3, segments: 3, basis: Basis::Bspline, cubic: true, periodic: true, pinned: false };
        assert_eq!(layout.sample(&value, 0, -1.), 2.);
        assert_eq!(layout.sample(&value, CUBIC_STEPS, -1.), 1.);
        assert_eq!(layout.sample(&value, 2*CUBIC_STEPS, -1.), 0.);
        assert_eq!(layout.sample(&value, 3*CUBIC_STEPS, -1.), 2.);
        assert_eq!(layout.sample(&value, 2*CUBIC_STEPS+CUBIC_STEPS/2, -1.), 1.);
    }

    #[test]
    fn curve_display_colors_sample_and_reconcile_parent_edits() {
        let source = crate::UsdSource::new("curve-colors.usda", include_bytes!("../../../../assets/curve_colors.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins(crate::live::LiveStagePlugin);
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.world_mut().insert_non_send(LiveStage::new(stage.clone()));
        app.world_mut().run_schedule(Update);
        let inherited = app.world().resource::<PrimEntities>().entity("/Root/Inherited").unwrap();
        let uniform = app.world().resource::<PrimEntities>().entity("/Root/Uniform").unwrap();
        let child = app.world_mut().spawn(ChildOf(inherited)).id();
        let check = |app: &App, entity, expected: Vec<[f32;4]>| {
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
            assert_eq!(colors, &expected);
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.alpha_mode, if expected.iter().any(|color| color[3] < 1.) { AlphaMode::Blend } else { AlphaMode::Opaque });
        };
        for time in [0.,10.,0.,10.] {
            app.world_mut().resource_mut::<super::super::StageTime>().current = time;
            app.world_mut().run_schedule(Update);
            check(&app, inherited, vec![if time == 0. { [1.,0.,0.,0.25] } else { [0.,1.,0.,1.] };4]);
            let pair = if time == 0. { [[1.,1.,0.,0.5], [0.,0.,1.,1.]] } else { [[0.,0.,1.,0.5], [1.,1.,0.,1.]] };
            check(&app, uniform, pair.into_iter().flat_map(|color| std::iter::repeat_n(color, CUBIC_STEPS + 1)).collect());
        }
        stage.attribute("/Root.primvars:displayColor:indices").unwrap().set_at(Value::IntVec(vec![0]), openusd::usd::TimeCode::new(10.)).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, inherited, vec![[1.,0.,0.,1.];4]);
        let local = stage.create_attribute("/Root/Inherited.primvars:displayColor", "color3f[]").unwrap();
        local.clone().set(Value::Vec3fVec(vec![[0.,0.,1.].into()])).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, inherited, vec![[0.,0.,1.,1.];4]);
        local.clear().unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, inherited, vec![[1.,0.,0.,1.];4]);
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Root/Inherited"), Some(inherited));
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), inherited);
    }

    #[test]
    fn three_point_periodic_bezier_matches_explicit_closure() {
        let source = crate::UsdSource::new("periodic-bezier.usda", include_bytes!("../../../../assets/periodic_bezier.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let periodic = openusd::sdf::path("/Periodic").unwrap();
        let explicit = openusd::sdf::path("/Explicit").unwrap();
        let actual = line_geometry(&RouteCtx::new(&stage, &periodic), CUBIC_STEPS).unwrap();
        assert_eq!(actual, line_geometry(&RouteCtx::new(&stage, &explicit), CUBIC_STEPS).unwrap());
        assert_eq!(actual.0.len(), CUBIC_STEPS + 1);
        assert_eq!(actual.1.len(), CUBIC_STEPS * 2);
        assert_eq!(actual.0.first(), actual.0.last());
        assert!(Vec3::from(actual.0[CUBIC_STEPS / 2]).distance(Vec3::new(0.,0.5,0.)) < 1e-6);
    }

    #[test]
    fn pinned_cubics_match_explicit_phantom_points() {
        let source = crate::UsdSource::new("pinned.usda", include_bytes!("../../../../assets/pinned_curves.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        for basis in ["Bspline", "CatmullRom"] {
            let pinned = openusd::sdf::path(&format!("/Pinned{basis}")).unwrap();
            let explicit = openusd::sdf::path(&format!("/Explicit{basis}")).unwrap();
            let actual = line_geometry(&RouteCtx::new(&stage, &pinned), CUBIC_STEPS).unwrap();
            assert_eq!(actual, line_geometry(&RouteCtx::new(&stage, &explicit), CUBIC_STEPS).unwrap());
            assert_eq!(actual.0.len(), 3 * CUBIC_STEPS + 1);
            assert!(Vec3::from(actual.0[0]).distance(Vec3::new(-1.,0.,0.)) < 1e-6);
            assert!(Vec3::from(*actual.0.last().unwrap()).distance(Vec3::new(1.,0.,0.)) < 1e-6);
            stage.prim(pinned.clone()).unwrap().attribute("wrap").set(Value::Token("nonperiodic".into())).unwrap();
            assert_eq!(line_geometry(&RouteCtx::new(&stage, &pinned), CUBIC_STEPS).unwrap().0.len(), CUBIC_STEPS + 1);
        }
    }

    #[test]
    fn pinned_two_point_cubics_interpolate_and_keep_batches_separate() {
        let cv = [[-2.,1.,0.], [2.,3.,0.]];
        for basis in [Basis::Bspline, Basis::CatmullRom] {
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for _ in 0..2 {
                let base = positions.len();
                let index_base = indices.len();
                tessellate_pinned(&cv, basis, &mut positions, &mut indices, CUBIC_STEPS);
                assert_eq!(positions.len() - base, CUBIC_STEPS + 1);
                for step in 0..=CUBIC_STEPS {
                    let expected = Vec3::from(cv[0]).lerp(Vec3::from(cv[1]), step as f32 / CUBIC_STEPS as f32);
                    assert!(Vec3::from(positions[base + step]).distance(expected) < 1e-6);
                }
                assert!(indices[index_base..].iter().all(|index| (*index as usize) >= base && (*index as usize) < positions.len()));
            }
        }
    }

    #[test]
    fn periodic_linear_batches_close_without_cross_curve_edges() {
        let source = crate::UsdSource::new("periodic.usda", include_bytes!("../../../../assets/periodic_curves.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let closed = openusd::sdf::path("/Closed").unwrap();
        let open = openusd::sdf::path("/Open").unwrap();
        let (positions, indices, _) = line_geometry(&RouteCtx::new(&stage, &closed), CUBIC_STEPS).unwrap();
        assert_eq!(positions.len(), 8);
        assert_eq!(indices, [0,1,1,2,2,3,3,0,4,5,5,6,6,7,7,4]);
        assert_eq!(line_geometry(&RouteCtx::new(&stage, &open), CUBIC_STEPS).unwrap().1, [0,1,1,2,2,3]);
        stage.prim(closed.clone()).unwrap().attribute("wrap").set(Value::Token("nonperiodic".into())).unwrap();
        assert_eq!(line_geometry(&RouteCtx::new(&stage, &closed), CUBIC_STEPS).unwrap().1, [0,1,1,2,2,3,4,5,5,6,6,7]);
    }

    #[test]
    fn periodic_cubic_bases_close_each_batch_independently() {
        let cv = [[0.,0.,0.], [1.,0.,0.], [2.,1.,0.], [2.,2.,0.], [1.,2.,0.], [0.,1.,0.]];
        for basis in [Basis::Bezier, Basis::Bspline, Basis::CatmullRom] {
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for _ in 0..2 {
                let base = positions.len();
                let index_base = indices.len();
                tessellate_cubic(&cv, basis, true, &mut positions, &mut indices, CUBIC_STEPS);
                assert_eq!(positions.len() - base, cv.len() / basis.vstep() * CUBIC_STEPS + 1);
                assert!(Vec3::from(positions[base]).distance(Vec3::from(*positions.last().unwrap())) < 1e-6);
                assert!(indices[index_base..].iter().all(|index| (*index as usize) >= base && (*index as usize) < positions.len()));
            }
        }
    }

    #[test]
    fn curves_project_line_mesh() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("crv.usda").unwrap();
        stage
            .define_prim("/Curve")
            .unwrap()
            .set_type_name("BasisCurves")
            .unwrap();
        stage.create_attribute("/Curve.type", "token").unwrap().set(Value::Token("linear".into())).unwrap();
        stage
            .create_attribute("/Curve.points", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 0.0, 0.0].into(),
                [2.0, 0.0, 0.0].into(),
            ]))
            .unwrap();
        stage
            .create_attribute("/Curve.curveVertexCounts", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![3]))
            .unwrap();

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Curve").unwrap();
        let handle = world.get::<Mesh3d>(e).expect("curve mesh").0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        assert_eq!(mesh.primitive_topology(), PrimitiveTopology::LineList);
        // Three linear points produce two segments.
        assert_eq!(mesh.indices().map(|i| i.len()), Some(4));
    }

    #[test]
    fn curve_quality_changes_refresh_live_and_independent_stages() {
        let source = crate::UsdSource::snapshot("quality-animation.usda", &br#"#usda 1.0
def BasisCurves "Curve" {
    uniform token type = "cubic"
    uniform token basis = "bezier"
    int[] curveVertexCounts = [4]
    point3f[] points.timeSamples = {
        0: [(0,0,0),(1,1,0),(2,1,0),(3,0,0)],
        10: [(0,2,0),(1,3,0),(2,3,0),(3,2,0)]
    }
}
"#[..]).unwrap();
        let check = |world: &World, entity, steps, height| {
            assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "runtime name");
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            assert_eq!(mesh.count_vertices(), steps + 1);
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            assert_eq!(points[0][1], height);
        };
        let mut live_app = App::new();
        live_app.add_plugins((MinimalPlugins, crate::UsdPlugin, crate::live::LiveStagePlugin));
        live_app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        live_app.world_mut().insert_non_send(LiveStage::new(source.open_stage().unwrap()));
        live_app.world_mut().resource_mut::<crate::route::StageTime>().current = 10.0;
        live_app.update();
        let live_entity = live_app.world().resource::<PrimEntities>().entity("/Curve").unwrap();
        live_app.world_mut().entity_mut(live_entity).insert(Name::new("runtime name"));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0.0, 10.0].map(|current| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), crate::instance::UsdInstanceTime { current })).id());
        app.update();
        let entities = roots.map(|root| app.world().non_send::<crate::instance::UsdInstances>().entity(root, "/Curve").unwrap());
        for entity in entities { app.world_mut().entity_mut(entity).insert(Name::new("runtime name")); }
        for setting in [Some(2), Some(64), None] {
            for world in [live_app.world_mut(), app.world_mut()] {
                if let Some(steps) = setting { world.insert_resource(UsdCurveSettings::new(steps).unwrap()); }
                else { world.remove_resource::<UsdCurveSettings>(); }
            }
            live_app.update();
            app.update();
            let steps = setting.unwrap_or(CUBIC_STEPS);
            assert_eq!(live_app.world().resource::<PrimEntities>().entity("/Curve"), Some(live_entity));
            check(live_app.world(), live_entity, steps, 2.0);
            for (index, root) in roots.into_iter().enumerate() {
                assert_eq!(app.world().non_send::<crate::instance::UsdInstances>().entity(root, "/Curve"), Some(entities[index]));
                assert_eq!(app.world().get::<crate::UsdSceneState>(root), Some(&crate::UsdSceneState::Ready));
                check(app.world(), entities[index], steps, index as f32 * 2.0);
            }
        }
    }

    #[test]
    fn curve_quality_settings_control_projection() {
        assert!(UsdCurveSettings::new(0).is_err());
        assert!(UsdCurveSettings::new(65).is_err());
        assert_eq!(UsdCurveSettings::default().cubic_steps(), CUBIC_STEPS);
        let source = crate::UsdSource::new("gradients.usda", include_bytes!("../../../../assets/curve_gradients.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Vertex").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let entity = world.spawn_empty().id();
        for steps in [1, 2, 16, 64] {
            world.insert_resource(UsdCurveSettings::new(steps).unwrap());
            CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
            assert!(world.get::<UsdCurveError>(entity).is_none());
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            assert_eq!(mesh.count_vertices(), 2 * (steps + 1));
            assert_eq!(mesh.indices().unwrap().len(), 4 * steps);
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
            assert_eq!(colors[0], [1.,0.,0.,0.2]);
            assert_eq!(colors[steps], [0.,0.,1.,1.]);
            if steps % 2 == 0 {
                assert!(Vec4::from(colors[steps / 2]).abs_diff_eq(Vec4::new(0.125,0.75,0.125,0.6), 1e-6));
            }
        }
    }

    #[test]
    fn cubic_curve_tessellates() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("crv.usda").unwrap();
        stage
            .define_prim("/Curve")
            .unwrap()
            .set_type_name("BasisCurves")
            .unwrap();
        stage
            .create_attribute("/Curve.type", "token")
            .unwrap()
            .set(Value::Token("cubic".into()))
            .unwrap();
        stage
            .create_attribute("/Curve.basis", "token")
            .unwrap()
            .set(Value::Token("bezier".into()))
            .unwrap();
        // 4 CVs = one bezier segment.
        stage
            .create_attribute("/Curve.points", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 2.0, 0.0].into(),
                [2.0, 2.0, 0.0].into(),
                [3.0, 0.0, 0.0].into(),
            ]))
            .unwrap();
        stage
            .create_attribute("/Curve.curveVertexCounts", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![4]))
            .unwrap();

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Curve").unwrap();
        let handle = world.get::<Mesh3d>(e).expect("curve mesh").0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        // One cubic segment → CUBIC_STEPS+1 samples, CUBIC_STEPS line segments.
        assert_eq!(mesh.count_vertices(), CUBIC_STEPS + 1);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(CUBIC_STEPS * 2));
        // The tessellated midpoint bows off the control-hull chord: at t=0.5 a
        // symmetric bezier peaks above the straight line between endpoints.
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            let mid = pos[CUBIC_STEPS / 2];
            assert!(mid[1] > 0.5, "curve bows upward, got y={}", mid[1]);
        } else {
            panic!("no positions");
        }
    }

    /// Invalid curve counts suppress geometry and expose a projection error.
    #[test]
    fn malformed_counts_do_not_panic() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("crv.usda").unwrap();
        stage.define_prim("/Curve").unwrap().set_type_name("BasisCurves").unwrap();
        stage
            .create_attribute("/Curve.points", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 0.0, 0.0].into(),
                [2.0, 0.0, 0.0].into(),
            ]))
            .unwrap();
        // 10 exceeds the 3 authored points; -1 is nonsense — both must be safe.
        stage
            .create_attribute("/Curve.curveVertexCounts", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![10, -1]))
            .unwrap();

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Curve").unwrap();
        assert!(world.get::<Mesh3d>(entity).is_none());
        assert!(world.get::<UsdCurveError>(entity).is_some());
    }

    /// A periodic cubic curve exercises the wrap-around index path (`% n`).
    #[test]
    fn periodic_cubic_wraps_without_panic() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("crv.usda").unwrap();
        stage.define_prim("/Curve").unwrap().set_type_name("BasisCurves").unwrap();
        stage.create_attribute("/Curve.basis", "token").unwrap().set(Value::Token("bspline".into())).unwrap();
        stage
            .create_attribute("/Curve.type", "token")
            .unwrap()
            .set(Value::Token("cubic".into()))
            .unwrap();
        stage
            .create_attribute("/Curve.wrap", "token")
            .unwrap()
            .set(Value::Token("periodic".into()))
            .unwrap();
        stage
            .create_attribute("/Curve.points", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 1.0, 0.0].into(),
                [2.0, 0.0, 0.0].into(),
                [1.0, -1.0, 0.0].into(),
            ]))
            .unwrap();
        stage
            .create_attribute("/Curve.curveVertexCounts", "int[]")
            .unwrap()
            .set(Value::IntVec(vec![4]))
            .unwrap();

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let handle = world.get::<Mesh3d>(map.entity("/Curve").unwrap()).unwrap().0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        // Four Bspline control points produce four periodic segments, each closing
        // back through the ring; a non-empty index buffer proves the wrap ran.
        assert!(mesh.indices().map(|i| i.len()).unwrap_or(0) > 0);
    }
}
