//! UsdGeom readers: Mesh, BasisCurves, NurbsCurves, NurbsPatch, TetMesh,
//! HermiteCurves, Points, PointInstancer, plus shape (Cube/Sphere/Cylinder),
//! purpose/visibility/kind, and custom-data introspection — all decoded from
//! the composed stage through openusd's public Prim / Attribute API.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

// ── Enums ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Constant,
    Uniform,
    Varying,
    Vertex,
    FaceVarying,
}

impl Interpolation {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "constant" => Self::Constant,
            "uniform" => Self::Uniform,
            "varying" => Self::Varying,
            "vertex" => Self::Vertex,
            "faceVarying" => Self::FaceVarying,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    RightHanded,
    LeftHanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubdivScheme {
    #[default]
    None,
    CatmullClark,
    Loop,
    Bilinear,
}

impl SubdivScheme {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "catmullClark" => Some(Self::CatmullClark),
            "loop" => Some(Self::Loop),
            "bilinear" => Some(Self::Bilinear),
            _ => None,
        }
    }
    pub fn is_subdivision(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshPrimvar<T> {
    pub values: Vec<T>,
    pub interpolation: Interpolation,
    pub indices: Vec<i32>,
}

// ── Mesh ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadMesh {
    pub points: Vec<[f32; 3]>,
    /// Pre-deformation positions used to select polygon diagonals.
    pub triangulation_points: Option<Vec<[f32; 3]>>,
    pub face_vertex_counts: Vec<i32>,
    pub face_vertex_indices: Vec<i32>,
    /// Source face indices excluded from rendering.
    pub hole_indices: Vec<i32>,
    pub normals: Option<MeshPrimvar<[f32; 3]>>,
    pub uvs: Option<MeshPrimvar<[f32; 2]>>,
    pub orientation: Orientation,
    pub display_color: Option<MeshPrimvar<[f32; 3]>>,
    pub display_opacity: Option<MeshPrimvar<f32>>,
    pub subsets: Vec<ReadSubset>,
    pub double_sided: bool,
    pub extent: Option<[[f32; 3]; 2]>,
    pub subdivision_scheme: SubdivScheme,
}

#[derive(Debug, Clone)]
pub struct ReadSubset {
    pub name: String,
    pub indices: Vec<i32>,
    pub material_binding: Option<Path>,
}

pub fn read_mesh(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadMesh>> {
    read_mesh_at(stage, prim, None)
}

/// Read mesh geometry and primvars at a USD time code, or their default values.
pub fn read_mesh_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<Option<ReadMesh>> {
    let Some(points) = vec3f_values(attr_at(stage, prim, "points", time)?) else {
        return Ok(None);
    };
    let Some(face_vertex_counts) = read_int_array_at(stage, prim, "faceVertexCounts", time)? else {
        return Ok(None);
    };
    let Some(face_vertex_indices) = read_int_array_at(stage, prim, "faceVertexIndices", time)? else {
        return Ok(None);
    };

    let [normal_owner, st_owner, st0_owner, color_owner, opacity_owner] = inherited_primvar_owners(
        stage, prim, ["primvars:normals", "primvars:st", "primvars:st0", "primvars:displayColor", "primvars:displayOpacity"])?;
    let normals = match read_primvar_vec3f(stage, &normal_owner, "primvars:normals", time)? {
        Some(mut normals) => {
            normals.interpolation = read_primvar_interpolation(stage, &normal_owner, "primvars:normals")?
                .unwrap_or(Interpolation::Constant);
            Some(normals)
        }
        None => read_primvar_vec3f(stage, prim, "normals", time)?,
    };
    let uvs = read_owned_primvar(stage, prim, &st_owner, "primvars:st", time, Interpolation::FaceVarying, vec2f_values)?
        .or(read_owned_primvar(stage, prim, &st0_owner, "primvars:st0", time, Interpolation::FaceVarying, vec2f_values)?);
    let orientation = match read_token(stage, prim, "orientation")?.as_deref() {
        Some("leftHanded") => Orientation::LeftHanded,
        _ => Orientation::RightHanded,
    };
    let display_color = read_owned_primvar(stage, prim, &color_owner, "primvars:displayColor", time, Interpolation::Vertex, vec3f_values)?;
    let display_opacity = read_owned_primvar(stage, prim, &opacity_owner, "primvars:displayOpacity", time, Interpolation::Vertex, float_values)?;
    let subsets = read_material_subsets(stage, prim, time)?;
    let double_sided = read_bool(stage, prim, "doubleSided")?.unwrap_or(false);
    let extent = vec3f_values(attr_at(stage, prim, "extent", time)?)
        .filter(|values| values.len() >= 2).map(|values| [values[0], values[1]]);
    let subdivision_scheme = read_token(stage, prim, "subdivisionScheme")?
        .as_deref()
        .and_then(SubdivScheme::parse)
        .unwrap_or_default();

    Ok(Some(ReadMesh {
        triangulation_points: None,
        points,
        face_vertex_counts,
        face_vertex_indices,
        hole_indices: read_int_array_at(stage, prim, "holeIndices", time)?.unwrap_or_default(),
        normals,
        uvs,
        orientation,
        display_color,
        display_opacity,
        subsets,
        double_sided,
        extent,
        subdivision_scheme,
    }))
}

fn read_material_subsets(stage: &Stage, mesh_prim: &Path, time: Option<f64>) -> anyhow::Result<Vec<ReadSubset>> {
    let mut out = Vec::new();
    for child_name in stage.prim(mesh_prim.clone()).expect("validated USD path").child_names()? {
        let Ok(child_path) = mesh_prim.append_path(child_name.as_str()) else {
            continue;
        };
        if stage.prim(child_path.clone()).expect("validated USD path").type_name()?.as_deref() != Some("GeomSubset") {
            continue;
        }
        if read_token(stage, &child_path, "familyName")?.as_deref() != Some("materialBind") {
            continue;
        }
        if matches!(read_token(stage, &child_path, "elementType")?.as_deref(), Some(e) if e != "face")
        {
            continue;
        }
        out.push(ReadSubset {
            name: child_name.to_string(),
            indices: read_int_array_at(stage, &child_path, "indices", time)?.unwrap_or_default(),
            material_binding: super::shade::read_material_binding(stage, &child_path)?,
        });
    }
    Ok(out)
}

// ── Shapes ──────────────────────────────────────────────────────────────

pub fn read_cube_size(stage: &Stage, prim: &Path) -> anyhow::Result<Option<f64>> {
    read_double(stage, prim, "size")
}

pub fn read_sphere_radius(stage: &Stage, prim: &Path) -> anyhow::Result<Option<f64>> {
    read_double(stage, prim, "radius")
}

#[derive(Debug, Clone, Copy)]
pub struct ReadCylinder {
    pub radius: f64,
    pub height: f64,
    pub axis: Axis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "X" => Self::X,
            "Y" => Self::Y,
            "Z" => Self::Z,
            _ => return None,
        })
    }
}

pub fn read_cylinder(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCylinder>> {
    let Some(radius) = read_double(stage, prim, "radius")? else {
        return Ok(None);
    };
    let Some(height) = read_double(stage, prim, "height")? else {
        return Ok(None);
    };
    let axis = read_token(stage, prim, "axis")?
        .as_deref()
        .and_then(Axis::parse)
        .unwrap_or(Axis::Z);
    Ok(Some(ReadCylinder {
        radius,
        height,
        axis,
    }))
}

pub fn read_capsule(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCylinder>> {
    read_cylinder(stage, prim)
}

pub fn read_double_attr(stage: &Stage, prim: &Path, name: &str) -> Option<f64> {
    read_double(stage, prim, name).ok().flatten()
}

// ── PointInstancer ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ReadPointInstancer {
    pub prototypes: Vec<Path>,
    pub positions: Vec<[f32; 3]>,
    pub orientations: Vec<[f32; 4]>,
    pub scales: Vec<[f32; 3]>,
    pub proto_indices: Vec<i32>,
}

pub fn read_point_instancer(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<ReadPointInstancer>> {
    read_point_instancer_at(stage, prim, None)
}

pub fn read_point_instancer_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<Option<ReadPointInstancer>> {
    let owner = stage.prim(prim.clone())?;
    let value = |name| match time {
        Some(time) => owner.attribute(name).get_at::<Value>(openusd::usd::TimeCode::new(time)),
        None => owner.attribute(name).get::<Value>(),
    };
    let positions = match value("positions")? {
        None => return Ok(None),
        value => vec3f_values(value).ok_or_else(|| anyhow::anyhow!("invalid positions array"))?,
    };
    let proto_indices = match value("protoIndices")? {
        Some(Value::IntVec(indices)) => indices,
        None => return Ok(None),
        _ => anyhow::bail!("invalid prototype indices array"),
    };
    let prototypes = owner.relationship("prototypes").targets()?;
    let orientations = match value("orientations")? {
        None => Vec::new(),
        value => quat_values(value).ok_or_else(|| anyhow::anyhow!("invalid orientations array"))?,
    };
    let scales = match value("scales")? {
        None => Vec::new(),
        value => vec3f_values(value).ok_or_else(|| anyhow::anyhow!("invalid scales array"))?,
    };
    Ok(Some(ReadPointInstancer {
        prototypes,
        positions,
        orientations,
        scales,
        proto_indices,
    }))
}

// ── BasisCurves ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveType {
    Linear,
    Cubic,
}

impl CurveType {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Self::Linear),
            "cubic" => Some(Self::Cubic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveBasis {
    Bezier,
    Bspline,
    CatmullRom,
    Hermite,
}

impl CurveBasis {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "bezier" => Some(Self::Bezier),
            "bspline" => Some(Self::Bspline),
            "catmullRom" => Some(Self::CatmullRom),
            "hermite" => Some(Self::Hermite),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveWrap {
    Nonperiodic,
    Periodic,
    Pinned,
}

impl CurveWrap {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "nonperiodic" => Some(Self::Nonperiodic),
            "periodic" => Some(Self::Periodic),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadCurves {
    pub points: Vec<[f32; 3]>,
    pub vertex_counts: Vec<i32>,
    pub curve_type: CurveType,
    pub basis: CurveBasis,
    pub wrap: CurveWrap,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_curves(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let curve_type = read_token(stage, prim, "type")?
        .as_deref()
        .and_then(CurveType::parse)
        .unwrap_or(CurveType::Linear);
    let basis = read_token(stage, prim, "basis")?
        .as_deref()
        .and_then(CurveBasis::parse)
        .unwrap_or(CurveBasis::Bezier);
    let wrap = read_token(stage, prim, "wrap")?
        .as_deref()
        .and_then(CurveWrap::parse)
        .unwrap_or(CurveWrap::Nonperiodic);
    Ok(Some(ReadCurves {
        points,
        vertex_counts,
        curve_type,
        basis,
        wrap,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── NurbsCurves ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadNurbsCurves {
    pub points: Vec<[f32; 3]>,
    pub curve_vertex_counts: Vec<i32>,
    pub order: Vec<i32>,
    pub knots: Vec<f64>,
    pub ranges: Vec<[f64; 2]>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_nurbs_curves(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadNurbsCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(curve_vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let order = read_int_array(stage, prim, "order")?
        .unwrap_or_else(|| curve_vertex_counts.iter().map(|_| 4).collect());
    let knots = read_double_array(stage, prim, "knots")?.unwrap_or_default();
    let ranges = read_vec2d_array(stage, prim, "ranges")?.unwrap_or_else(|| {
        let mut out = Vec::with_capacity(curve_vertex_counts.len());
        let mut k_cursor = 0usize;
        for (i, count) in curve_vertex_counts.iter().enumerate() {
            let n = (*count).max(0) as usize;
            let p = order.get(i).copied().unwrap_or(4) as usize;
            let nk = n + p;
            if k_cursor + nk <= knots.len() && p > 0 && n > 0 {
                out.push([knots[k_cursor + p - 1], knots[k_cursor + n]]);
            } else {
                out.push([0.0, 1.0]);
            }
            k_cursor += nk;
        }
        out
    });
    Ok(Some(ReadNurbsCurves {
        points,
        curve_vertex_counts,
        order,
        knots,
        ranges,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── NurbsPatch ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadNurbsPatch {
    pub points: Vec<[f32; 3]>,
    pub u_vertex_count: i32,
    pub v_vertex_count: i32,
    pub u_order: i32,
    pub v_order: i32,
    pub u_knots: Vec<f64>,
    pub v_knots: Vec<f64>,
    pub u_range: [f64; 2],
    pub v_range: [f64; 2],
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_nurbs_patch(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadNurbsPatch>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let u_vertex_count = read_int_scalar(stage, prim, "uVertexCount")?.unwrap_or(0);
    let v_vertex_count = read_int_scalar(stage, prim, "vVertexCount")?.unwrap_or(0);
    let u_order = read_int_scalar(stage, prim, "uOrder")?.unwrap_or(4);
    let v_order = read_int_scalar(stage, prim, "vOrder")?.unwrap_or(4);
    let u_knots = read_double_array(stage, prim, "uKnots")?.unwrap_or_default();
    let v_knots = read_double_array(stage, prim, "vKnots")?.unwrap_or_default();
    let u_range = read_vec2d_scalar(stage, prim, "uRange")?
        .unwrap_or_else(|| inner_span(&u_knots, u_vertex_count, u_order));
    let v_range = read_vec2d_scalar(stage, prim, "vRange")?
        .unwrap_or_else(|| inner_span(&v_knots, v_vertex_count, v_order));
    Ok(Some(ReadNurbsPatch {
        points,
        u_vertex_count,
        v_vertex_count,
        u_order,
        v_order,
        u_knots,
        v_knots,
        u_range,
        v_range,
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

fn inner_span(knots: &[f64], vertex_count: i32, order: i32) -> [f64; 2] {
    let n = vertex_count as usize;
    let p = order as usize;
    if !knots.is_empty() && n > 0 && p > 0 && knots.len() >= n + p {
        [knots[p - 1], knots[n]]
    } else {
        [0.0, 1.0]
    }
}

// ── TetMesh ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadTetMesh {
    pub points: Vec<[f32; 3]>,
    pub tet_vertex_indices: Vec<i32>,
    pub surface_face_vertex_indices: Option<Vec<i32>>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_tetmesh(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadTetMesh>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(tet_vertex_indices) = read_int_array(stage, prim, "tetVertexIndices")? else {
        return Ok(None);
    };
    Ok(Some(ReadTetMesh {
        points,
        tet_vertex_indices,
        surface_face_vertex_indices: read_int_array(stage, prim, "surfaceFaceVertexIndices")?,
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── HermiteCurves ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadHermiteCurves {
    pub points: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub curve_vertex_counts: Vec<i32>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
}

pub fn read_hermite_curves(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Option<ReadHermiteCurves>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(curve_vertex_counts) = read_int_array(stage, prim, "curveVertexCounts")? else {
        return Ok(None);
    };
    let tangents = read_vec3f_array(stage, prim, "tangents")?.unwrap_or_else(|| {
        let n = points.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let a = points[i];
            let b = if i + 1 < n { points[i + 1] } else { points[i] };
            out.push([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
        }
        out
    });
    Ok(Some(ReadHermiteCurves {
        points,
        tangents,
        curve_vertex_counts,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
    }))
}

// ── Points ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadPoints {
    pub points: Vec<[f32; 3]>,
    pub widths: Vec<f32>,
    pub display_color: Option<Vec<[f32; 3]>>,
    pub ids: Vec<i64>,
}

pub fn read_points(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadPoints>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    Ok(Some(ReadPoints {
        points,
        widths: read_float_array(stage, prim, "widths")?.unwrap_or_default(),
        display_color: read_vec3f_array(stage, prim, "primvars:displayColor")?,
        ids: read_int64_array(stage, prim, "ids")?.unwrap_or_default(),
    }))
}

// ── Imageable / model metadata ──────────────────────────────────────────

pub fn read_purpose(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    Ok(read_token(stage, prim, "purpose")?.unwrap_or_else(|| "default".to_string()))
}

/// Resolve the effective (inherited) `purpose` for `prim`: the closest ancestor
/// with an authored `purpose`, falling back to `"default"`. Mirrors
/// `UsdGeomImageable::ComputePurpose` — `purpose` is a *pruning, inherited*
/// token, so a `Scope` authored as `proxy` makes its whole subtree proxy.
pub fn read_effective_purpose(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    let mut cur = prim.clone();
    loop {
        if cur.is_abs_root() || cur.is_empty() { return Ok("default".to_string()); }
        let attr = stage.prim(cur.clone())?.attribute("purpose");
        if attr.resolve_info()?.has_authored_value() {
            if let Some(t) = read_token(stage, &cur, "purpose")? {
                return Ok(t);
            }
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return Ok("default".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityState {
    Inherited,
    Invisible,
}

pub fn read_visibility(stage: &Stage, prim: &Path) -> anyhow::Result<VisibilityState> {
    read_visibility_at(stage, prim, None)
}

/// Read the prim's local visibility at a USD time code.
pub fn read_visibility_at(stage: &Stage, prim: &Path, time: Option<f64>) -> anyhow::Result<VisibilityState> {
    if prim.is_abs_root() || prim.is_empty() { return Ok(VisibilityState::Inherited); }
    Ok(match attr_at(stage, prim, "visibility", time)? {
        Some(Value::Token(value)) if value.as_str() == "invisible" => VisibilityState::Invisible,
        Some(Value::String(value)) if value == "invisible" => VisibilityState::Invisible,
        _ => VisibilityState::Inherited,
    })
}

pub fn read_kind(stage: &Stage, prim: &Path) -> anyhow::Result<Option<String>> {
    Ok(stage
        .prim(prim.clone()).expect("validated USD path")
        .kind()?
        .map(|t| t.as_str().to_string()))
}

// ── Custom data ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CustomAttrValue {
    Bool(bool),
    Uchar(u8),
    Int(i32),
    UInt(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(String),
    Token(String),
    AssetPath(String),
    Vec2f([f32; 2]),
    Vec2d([f64; 2]),
    Vec2i([i32; 2]),
    Vec3f([f32; 3]),
    Vec3d([f64; 3]),
    Vec3i([i32; 3]),
    Vec4f([f32; 4]),
    Vec4d([f64; 4]),
    Vec4i([i32; 4]),
    Quatf([f32; 4]),
    Quatd([f64; 4]),
    Matrix4d([f64; 16]),
    BoolArray(Vec<bool>),
    UcharArray(Vec<u8>),
    IntArray(Vec<i32>),
    UIntArray(Vec<u32>),
    Int64Array(Vec<i64>),
    UInt64Array(Vec<u64>),
    FloatArray(Vec<f32>),
    DoubleArray(Vec<f64>),
    StringArray(Vec<String>),
    TokenArray(Vec<String>),
    PathArray(Vec<String>),
    Vec2fArray(Vec<[f32; 2]>),
    Vec3fArray(Vec<[f32; 3]>),
    Vec4fArray(Vec<[f32; 4]>),
    Vec2dArray(Vec<[f64; 2]>),
    Vec3dArray(Vec<[f64; 3]>),
    Vec4dArray(Vec<[f64; 4]>),
    QuatfArray(Vec<[f32; 4]>),
    Matrix4dArray(Vec<[f64; 16]>),
    Dict(CustomDict),
    TimeSamples(Vec<(f64, Box<CustomAttrValue>)>),
    Other(String),
}

impl CustomAttrValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        Some(match self {
            Self::Uchar(u) => *u as i64,
            Self::Int(i) => *i as i64,
            Self::UInt(u) => *u as i64,
            Self::Int64(i) => *i,
            Self::UInt64(u) => *u as i64,
            _ => return None,
        })
    }
    pub fn as_float(&self) -> Option<f64> {
        Some(match self {
            Self::Float(f) => *f as f64,
            Self::Double(d) => *d,
            Self::Int(i) => *i as f64,
            Self::Int64(i) => *i as f64,
            Self::UInt(u) => *u as f64,
            Self::UInt64(u) => *u as f64,
            Self::Uchar(u) => *u as f64,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Token(s) | Self::AssetPath(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<[f32; 2]> {
        match self {
            Self::Vec2f(a) => Some(*a),
            Self::Vec2d(a) => Some([a[0] as f32, a[1] as f32]),
            Self::Vec2i(a) => Some([a[0] as f32, a[1] as f32]),
            _ => None,
        }
    }
    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match self {
            Self::Vec3f(a) => Some(*a),
            Self::Vec3d(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32]),
            Self::Vec3i(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32]),
            _ => None,
        }
    }
    pub fn as_vec4(&self) -> Option<[f32; 4]> {
        match self {
            Self::Vec4f(a) | Self::Quatf(a) => Some(*a),
            Self::Vec4d(a) | Self::Quatd(a) => {
                Some([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32])
            }
            Self::Vec4i(a) => Some([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32]),
            _ => None,
        }
    }
    pub fn as_matrix4(&self) -> Option<[f32; 16]> {
        match self {
            Self::Matrix4d(m) => {
                let mut out = [0.0f32; 16];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = m[i] as f32;
                }
                Some(out)
            }
            _ => None,
        }
    }
    pub fn as_dict(&self) -> Option<&CustomDict> {
        match self {
            Self::Dict(d) => Some(d),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomDict {
    pub entries: Vec<(String, CustomAttrValue)>,
}

impl CustomDict {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn get(&self, name: &str) -> Option<&CustomAttrValue> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn iter(&self) -> impl Iterator<Item = &(String, CustomAttrValue)> {
        self.entries.iter()
    }
    pub fn get_nested(&self, dotted: &str) -> Option<&CustomAttrValue> {
        let mut cur: &CustomAttrValue = self.get(dotted.split('.').next()?)?;
        for part in dotted.split('.').skip(1) {
            match cur {
                CustomAttrValue::Dict(d) => cur = d.get(part)?,
                _ => return None,
            }
        }
        Some(cur)
    }
}

/// Read every authored `custom` attribute on `prim`.
pub fn read_custom_attrs(
    stage: &Stage,
    prim: &Path,
) -> anyhow::Result<Vec<(String, CustomAttrValue)>> {
    let mut out = Vec::new();
    for name in stage.prim(prim.clone()).expect("validated USD path").property_names()? {
        let attr = stage.prim(prim.clone()).expect("validated USD path").attribute(&name);
        let is_custom = matches!(
            attr.get_metadata::<bool>("custom").ok().flatten(),
            Some(true)
        );
        if !is_custom {
            continue;
        }
        let Some(raw) = attr.get::<Value>()? else {
            continue;
        };
        out.push((name.to_string(), value_to_custom(raw)));
    }
    Ok(out)
}

fn value_to_custom(v: Value) -> CustomAttrValue {
    use CustomAttrValue as C;
    match v {
        Value::Bool(b) => C::Bool(b),
        Value::Uchar(u) => C::Uchar(u),
        Value::Int(i) => C::Int(i),
        Value::Uint(u) => C::UInt(u),
        Value::Int64(i) => C::Int64(i),
        Value::Uint64(u) => C::UInt64(u),
        Value::Half(h) => C::Float(f32::from(h)),
        Value::Float(f) => C::Float(f),
        Value::Double(d) => C::Double(d),
        Value::String(s) => C::String(s),
        Value::Token(s) => C::Token(s.as_str().to_string()),
        Value::AssetPath(s) => C::AssetPath(s.as_str().to_string()),
        Value::Vec2h(a) => C::Vec2f([f32::from(a.x), f32::from(a.y)]),
        Value::Vec2f(a) => C::Vec2f([a.x, a.y]),
        Value::Vec2d(a) => C::Vec2d([a.x, a.y]),
        Value::Vec2i(a) => C::Vec2i([a.x, a.y]),
        Value::Vec3h(a) => C::Vec3f([f32::from(a.x), f32::from(a.y), f32::from(a.z)]),
        Value::Vec3f(a) => C::Vec3f([a.x, a.y, a.z]),
        Value::Vec3d(a) => C::Vec3d([a.x, a.y, a.z]),
        Value::Vec3i(a) => C::Vec3i([a.x, a.y, a.z]),
        Value::Vec4h(a) => C::Vec4f([
            f32::from(a.x),
            f32::from(a.y),
            f32::from(a.z),
            f32::from(a.w),
        ]),
        Value::Vec4f(a) => C::Vec4f([a.x, a.y, a.z, a.w]),
        Value::Vec4d(a) => C::Vec4d([a.x, a.y, a.z, a.w]),
        Value::Vec4i(a) => C::Vec4i([a.x, a.y, a.z, a.w]),
        Value::Quath(q) => C::Quatf([
            f32::from(q.w),
            f32::from(q.x),
            f32::from(q.y),
            f32::from(q.z),
        ]),
        Value::Quatf(q) => C::Quatf([q.w, q.x, q.y, q.z]),
        Value::Quatd(q) => C::Quatd([q.w, q.x, q.y, q.z]),
        Value::Matrix4d(m) => C::Matrix4d(m.0),
        Value::BoolVec(v) => C::BoolArray(v),
        Value::UcharVec(v) => C::UcharArray(v),
        Value::IntVec(v) => C::IntArray(v),
        Value::UintVec(v) => C::UIntArray(v),
        Value::Int64Vec(v) => C::Int64Array(v),
        Value::Uint64Vec(v) => C::UInt64Array(v),
        Value::HalfVec(v) => C::FloatArray(v.into_iter().map(f32::from).collect()),
        Value::FloatVec(v) => C::FloatArray(v),
        Value::DoubleVec(v) => C::DoubleArray(v),
        Value::StringVec(v) => C::StringArray(v),
        Value::TokenVec(v) => {
            C::TokenArray(v.into_iter().map(|t| t.as_str().to_string()).collect())
        }
        Value::PathVec(v) => C::PathArray(v.into_iter().map(|p| p.as_str().to_string()).collect()),
        Value::Vec2fVec(v) => C::Vec2fArray(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Value::Vec2dVec(v) => C::Vec2dArray(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Value::Vec3fVec(v) => C::Vec3fArray(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Value::Vec3dVec(v) => C::Vec3dArray(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Value::Vec4fVec(v) => C::Vec4fArray(v.into_iter().map(|a| [a.x, a.y, a.z, a.w]).collect()),
        Value::Vec4dVec(v) => C::Vec4dArray(v.into_iter().map(|a| [a.x, a.y, a.z, a.w]).collect()),
        Value::QuatfVec(v) => C::QuatfArray(v.into_iter().map(|q| [q.w, q.x, q.y, q.z]).collect()),
        Value::Matrix4dVec(v) => C::Matrix4dArray(v.into_iter().map(|m| m.0).collect()),
        Value::Dictionary(dict) => C::Dict(dict_from_value_map(dict)),
        Value::TimeSamples(samples) => C::TimeSamples(
            samples
                .into_iter()
                .map(|(t, v)| (t, Box::new(value_to_custom(v))))
                .collect(),
        ),
        other => C::Other(format!("{other:?}")),
    }
}

fn dict_from_value_map(map: std::collections::HashMap<String, Value>) -> CustomDict {
    let mut entries: Vec<(String, CustomAttrValue)> = map
        .into_iter()
        .map(|(k, v)| (k, value_to_custom(v)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    CustomDict { entries }
}

fn dict_from_value(raw: Option<Value>) -> Option<CustomDict> {
    match raw {
        Some(Value::Dictionary(d)) => {
            let dict = dict_from_value_map(d);
            (!dict.is_empty()).then_some(dict)
        }
        _ => None,
    }
}

pub fn read_custom_data(stage: &Stage, prim: &Path) -> anyhow::Result<Option<CustomDict>> {
    Ok(dict_from_value(stage.prim(prim.clone()).expect("validated USD path").custom_data()?))
}

/// Composed `assetInfo` dictionary on a prim.
pub fn read_asset_info(stage: &Stage, prim: &Path) -> anyhow::Result<Option<CustomDict>> {
    Ok(dict_from_value(stage.prim(prim.clone())?.get_metadata::<Value>("assetInfo")?))
}

pub fn read_custom_layer_data(stage: &Stage) -> anyhow::Result<Option<CustomDict>> {
    Ok(dict_from_value(stage.custom_layer_data()?))
}

// ── Low-level attribute plumbing ─────────────────────────────────────────

/// Composed `default` value, falling back to the first time sample when the
/// default is an empty array placeholder (common in FX caches).
fn attr_default(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Value>> {
    let attr = stage.prim(prim.clone()).expect("validated USD path").attribute(name);
    if let Some(v) = attr.get::<Value>()?
        && !is_empty_array_value(&v)
    {
        return Ok(Some(v));
    }
    Ok(attr
        .time_samples()?
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|(_, v)| v))
}

fn is_empty_array_value(v: &Value) -> bool {
    match v {
        Value::IntVec(a) => a.is_empty(),
        Value::FloatVec(a) => a.is_empty(),
        Value::DoubleVec(a) => a.is_empty(),
        Value::TokenVec(a) => a.is_empty(),
        Value::StringVec(a) => a.is_empty(),
        Value::Vec2fVec(a) => a.is_empty(),
        Value::Vec2dVec(a) => a.is_empty(),
        Value::Vec3fVec(a) => a.is_empty(),
        Value::Vec3dVec(a) => a.is_empty(),
        Value::Vec4fVec(a) => a.is_empty(),
        _ => false,
    }
}

fn read_token(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<String>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Token(s)) => Some(s.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}

fn read_double(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<f64>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Double(v)) => Some(v),
        Some(Value::Float(v)) => Some(v as f64),
        _ => None,
    })
}

fn read_bool(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<bool>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    })
}

fn read_int_array(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Vec<i32>>> {
    read_int_array_at(stage, prim, name, None)
}

fn attr_at(stage: &Stage, prim: &Path, name: &str, time: Option<f64>) -> anyhow::Result<Option<Value>> {
    Ok(stage.prim(prim.clone())?.attribute(name).get_at::<Value>(time.map(openusd::usd::TimeCode::new))?)
}

fn read_int_array_at(stage: &Stage, prim: &Path, name: &str, time: Option<f64>) -> anyhow::Result<Option<Vec<i32>>> {
    Ok(match attr_at(stage, prim, name, time)? {
        Some(Value::IntVec(v)) => Some(v),
        _ => None,
    })
}

fn read_double_array(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Vec<f64>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::DoubleVec(v)) => Some(v),
        Some(Value::FloatVec(v)) => Some(v.into_iter().map(|f| f as f64).collect()),
        _ => None,
    })
}

fn read_int_scalar(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<i32>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Int(v)) => Some(v),
        _ => None,
    })
}

fn read_float_array(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(float_values(attr_default(stage, prim, name)?))
}

fn float_values(value: Option<Value>) -> Option<Vec<f32>> {
    match value {
        Some(Value::FloatVec(v)) => Some(v),
        Some(Value::DoubleVec(v)) => Some(v.into_iter().map(|d| d as f32).collect()),
        Some(Value::Float(v)) => Some(vec![v]),
        _ => None,
    }
}

fn read_int64_array(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<Vec<i64>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Int64Vec(v)) => Some(v),
        Some(Value::IntVec(v)) => Some(v.into_iter().map(|i| i as i64).collect()),
        _ => None,
    })
}

fn quat_values(value: Option<Value>) -> Option<Vec<[f32; 4]>> {
    match value {
        Some(Value::QuathVec(v)) => Some(v.into_iter().map(|q| [q.w.to_f32(), q.x.to_f32(), q.y.to_f32(), q.z.to_f32()]).collect()),
        Some(Value::QuatfVec(v)) => Some(v.into_iter().map(|q| [q.w, q.x, q.y, q.z]).collect()),
        Some(Value::QuatdVec(v)) => Some(
            v.into_iter()
                .map(|q| [q.w as f32, q.x as f32, q.y as f32, q.z as f32])
                .collect(),
        ),
        _ => None,
    }
}

fn read_vec2d_scalar(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Option<[f64; 2]>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2d(v)) => Some([v.x, v.y]),
        Some(Value::Vec2f(v)) => Some([v.x as f64, v.y as f64]),
        _ => None,
    })
}

fn read_vec2d_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f64; 2]>>> {
    Ok(match attr_default(stage, prim, name)? {
        Some(Value::Vec2dVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Some(Value::Vec2fVec(v)) => Some(v.into_iter().map(|a| [a.x as f64, a.y as f64]).collect()),
        _ => None,
    })
}

fn read_vec3f_array(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Vec<[f32; 3]>>> {
    Ok(vec3f_values(attr_default(stage, prim, name)?))
}

fn vec3f_values(value: Option<Value>) -> Option<Vec<[f32; 3]>> {
    match value {
        Some(Value::Vec3fVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y, a.z]).collect()),
        Some(Value::Vec3dVec(v)) => Some(
            v.into_iter()
                .map(|a| [a.x as f32, a.y as f32, a.z as f32])
                .collect(),
        ),
        _ => None,
    }
}

fn vec2f_values(value: Option<Value>) -> Option<Vec<[f32; 2]>> {
    match value {
        Some(Value::Vec2fVec(v)) => Some(v.into_iter().map(|a| [a.x, a.y]).collect()),
        Some(Value::Vec2dVec(v)) => Some(v.into_iter().map(|a| [a.x as f32, a.y as f32]).collect()),
        _ => None,
    }
}

pub(crate) fn inherited_primvar_owner(stage: &Stage, prim: &Path, name: &str) -> anyhow::Result<Path> {
    let mut candidate = Some(prim.clone());
    while let Some(path) = candidate {
        if path.is_abs_root() { break; }
        let info = stage.prim(path.clone())?.attribute(name).resolve_info()?;
        if info.has_authored_value() && !info.value_is_blocked()
            && (path == *prim || read_primvar_interpolation(stage, &path, name)?
                .unwrap_or(Interpolation::Constant) == Interpolation::Constant)
        {
            return Ok(path);
        }
        candidate = path.parent();
    }
    Ok(prim.clone())
}

fn inherited_primvar_owners<const N: usize>(stage: &Stage, prim: &Path, names: [&str; N]) -> anyhow::Result<[Path; N]> {
    let mut owners: [Option<Path>; N] = std::array::from_fn(|_| None);
    let mut candidate = Some(prim.clone());
    while let Some(path) = candidate {
        if path.is_abs_root() || owners.iter().all(Option::is_some) { break; }
        let current = stage.prim(&path)?;
        for (name, owner) in names.iter().zip(&mut owners) {
            if owner.is_some() { continue; }
            let info = current.attribute(*name).resolve_info()?;
            if info.has_authored_value() && !info.value_is_blocked()
                && (path == *prim || read_primvar_interpolation(stage, &path, name)?
                    .unwrap_or(Interpolation::Constant) == Interpolation::Constant) {
                *owner = Some(path.clone());
            }
        }
        candidate = path.parent();
    }
    Ok(owners.map(|owner| owner.unwrap_or_else(|| prim.clone())))
}

fn read_owned_primvar<T>(
    stage: &Stage, prim: &Path, owner: &Path, name: &str, time: Option<f64>,
    fallback: Interpolation, decode: impl FnOnce(Option<Value>) -> Option<Vec<T>>,
) -> anyhow::Result<Option<MeshPrimvar<T>>> {
    let Some(values) = decode(attr_at(stage, owner, name, time)?) else { return Ok(None); };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: if owner != prim { Interpolation::Constant } else {
            read_primvar_interpolation(stage, owner, name)?.unwrap_or(fallback)
        },
        indices: read_int_array_at(stage, owner, &format!("{name}:indices"), time)?.unwrap_or_default(),
    }))
}

pub(crate) fn read_primvar_interpolation(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<Interpolation>> {
    let raw = stage
        .prim(prim.clone()).expect("validated USD path")
        .attribute(name)
        .get_metadata::<Value>("interpolation")?;
    if let Some(s) = raw.and_then(|v| match v {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }) && let Some(i) = Interpolation::parse(&s)
    {
        return Ok(Some(i));
    }
    let fallback_name = format!("{name}:interpolation");
    Ok(read_token(stage, prim, &fallback_name)?
        .as_deref()
        .and_then(Interpolation::parse))
}

pub(crate) fn read_primvar_vec3f(
    stage: &Stage,
    prim: &Path,
    name: &str,
    time: Option<f64>,
) -> anyhow::Result<Option<MeshPrimvar<[f32; 3]>>> {
    let owner = if name == "primvars:displayColor" { inherited_primvar_owner(stage, prim, name)? } else { prim.clone() };
    let inherited = owner != *prim;
    let prim = &owner;
    let Some(values) = vec3f_values(attr_at(stage, prim, name, time)?) else {
        return Ok(None);
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: if inherited { Interpolation::Constant } else {
            read_primvar_interpolation(stage, prim, name)?.unwrap_or(Interpolation::Vertex)
        },
        indices: read_int_array_at(stage, prim, &format!("{name}:indices"), time)?.unwrap_or_default(),
    }))
}

pub(crate) fn read_primvar_float(
    stage: &Stage,
    prim: &Path,
    name: &str,
    time: Option<f64>,
) -> anyhow::Result<Option<MeshPrimvar<f32>>> {
    let owner = if name == "primvars:displayOpacity" { inherited_primvar_owner(stage, prim, name)? } else { prim.clone() };
    let inherited = owner != *prim;
    let prim = &owner;
    let Some(values) = float_values(attr_at(stage, prim, name, time)?) else {
        return Ok(None);
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: if inherited { Interpolation::Constant } else {
            read_primvar_interpolation(stage, prim, name)?.unwrap_or(Interpolation::Vertex)
        },
        indices: read_int_array_at(stage, prim, &format!("{name}:indices"), time)?.unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_owners_match_independent_walks_before_and_after_edits() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Xform "Root" {
    texCoord2f[] primvars:st = [(0,0)] (interpolation = "constant")
    color3f[] primvars:displayColor = [(1,0,0)] (interpolation = "constant")
    def Xform "Parent" {
        color3f[] primvars:displayColor = [(0,1,0)] (interpolation = "vertex")
        def Mesh "Mesh" {
            texCoord2f[] primvars:st = None
            float[] primvars:displayOpacity = [0.5]
        }
    }
}
"#).open_stage().unwrap();
        let names = ["primvars:normals", "primvars:st", "primvars:st0", "primvars:displayColor", "primvars:displayOpacity"];
        for edited in [false, true] {
            if edited {
                stage.attribute("/Root/Parent.primvars:displayColor").unwrap()
                    .set_metadata("interpolation", Value::Token("constant".into())).unwrap();
            }
            for path in ["/Root", "/Root/Parent", "/Root/Parent/Mesh"] {
                let path = openusd::sdf::path(path).unwrap();
                let owners = inherited_primvar_owners(&stage, &path, names).unwrap();
                for (name, owner) in names.iter().zip(&owners) {
                    assert_eq!(*owner, inherited_primvar_owner(&stage, &path, name).unwrap());
                }
            }
        }
    }
}
