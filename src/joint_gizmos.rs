// Vendored from bevy_glacial (rev 3db71a9) `joint_gizmos.rs`. mara's
// bevy_mara pins glacial at d7bc6d3, which predates this module, so we
// keep a local copy rather than fork the rev. Self-contained (bevy +
// core only).
//! Joint-shaped gizmo primitives for editor / debug overlays.
//!
//! Backend-neutral helpers — no knowledge of any specific physics
//! engine or schema. Pair a joint's authored data (axis, limits) with
//! a world-space anchor and these draw the right shape: arc for a
//! revolute limit, line segment for a prismatic limit, cone for a
//! spherical limit, sphere envelope for a distance limit.
//!
//! Drawn via Bevy's immediate-mode [`Gizmos`] API — pure debug
//! visualisation, no entities or meshes spawned. Intended to be
//! called from a per-frame system that already iterates joints
//! (e.g. `bevy_openusd`'s physics overlay or `bevy_urdf`'s joint
//! debug view).
//!
//! ## Conventions
//! - All angles in **radians** (caller converts USD's degrees etc.)
//! - All distances in **metres** (caller applies metersPerUnit)
//! - All axes are **unit vectors in world space** — caller composes
//!   joint local frames with body world transforms
//! - `lower > upper` is interpreted as a **locked DOF** and renders
//!   nothing (matches USD / PhysX convention)

use bevy::gizmos::config::GizmoConfigGroup;
use bevy::prelude::*;
use core::f32::consts::TAU;

/// Draw a revolute joint's limit range as an arc around `axis` at
/// `anchor`. `lower_rad` and `upper_rad` are signed angles relative
/// to a reference perpendicular vector chosen perpendicular to
/// `axis` (so the arc orientation is deterministic but the absolute
/// reference is arbitrary — matches USD's revolute spec which has
/// no canonical zero).
///
/// Renders nothing when `lower_rad >= upper_rad` (locked DOF) or
/// when `radius <= 0`.
///
/// `resolution` sets the arc tessellation; pass `None` for the
/// gizmo subsystem default.
pub fn draw_revolute_limit_arc<C: GizmoConfigGroup>(
    gizmos: &mut Gizmos<'_, '_, C>,
    anchor: Vec3,
    axis: Vec3,
    lower_rad: f32,
    upper_rad: f32,
    radius: f32,
    color: impl Into<Color>,
    resolution: Option<u32>,
) {
    if lower_rad >= upper_rad || radius <= 0.0 {
        return;
    }
    let z = axis.normalize_or_zero();
    if z.length_squared() < 1e-6 {
        return;
    }
    // Build an isometry whose local +Z = axis (arc plane normal),
    // local +X = reference perpendicular rotated by lower_rad.
    let perp_seed = if z.abs().dot(Vec3::Y) < 0.9 { Vec3::Y } else { Vec3::X };
    let perp = (perp_seed - z * perp_seed.dot(z)).normalize();
    let x = Quat::from_axis_angle(z, lower_rad) * perp;
    let y = z.cross(x).normalize();
    let rotation = Quat::from_mat3(&Mat3::from_cols(x, y, z));
    let iso = Isometry3d::new(anchor, rotation);
    let mut builder = gizmos.arc_3d(upper_rad - lower_rad, radius, iso, color.into());
    if let Some(r) = resolution {
        builder = builder.resolution(r);
    }
    let _ = builder;
}

/// Draw a prismatic joint's limit range as a thick line segment
/// along `axis` from `anchor + axis * low_m` to `anchor + axis * high_m`.
/// Adds short perpendicular tick marks at the endpoints so the
/// limits read clearly on long sliders.
///
/// Renders nothing when `low_m >= high_m` (locked DOF).
pub fn draw_prismatic_limit_segment<C: GizmoConfigGroup>(
    gizmos: &mut Gizmos<'_, '_, C>,
    anchor: Vec3,
    axis: Vec3,
    low_m: f32,
    high_m: f32,
    color: impl Into<Color>,
) {
    if low_m >= high_m {
        return;
    }
    let dir = axis.normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return;
    }
    let col: Color = color.into();
    let p_low = anchor + dir * low_m;
    let p_high = anchor + dir * high_m;
    gizmos.line(p_low, p_high, col);

    // End ticks (perpendicular cross 1/8th the segment length).
    let perp_seed = if dir.abs().dot(Vec3::Y) < 0.9 { Vec3::Y } else { Vec3::X };
    let perp = (perp_seed - dir * perp_seed.dot(dir)).normalize();
    let perp2 = dir.cross(perp).normalize();
    let tick = (high_m - low_m).abs() * 0.08;
    for p in [p_low, p_high] {
        gizmos.line(p - perp * tick, p + perp * tick, col);
        gizmos.line(p - perp2 * tick, p + perp2 * tick, col);
    }
}

/// Draw a wireframe cone with apex at `apex`, opening along `axis`
/// (unit vector), with `half_angle_rad` aperture and `height` along
/// the axis. `segments` controls how many spokes / base-circle
/// segments are drawn (clamped to ≥ 4).
///
/// Renders nothing when `half_angle_rad <= 0`, `height <= 0`, or
/// the axis is degenerate.
pub fn draw_cone_wireframe<C: GizmoConfigGroup>(
    gizmos: &mut Gizmos<'_, '_, C>,
    apex: Vec3,
    axis: Vec3,
    half_angle_rad: f32,
    height: f32,
    segments: usize,
    color: impl Into<Color>,
) {
    if half_angle_rad <= 0.0 || height <= 0.0 {
        return;
    }
    let dir = axis.normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return;
    }
    let col: Color = color.into();
    let n = segments.max(4);

    let base_center = apex + dir * height;
    let base_radius = height * half_angle_rad.tan();

    // Local frame for the base circle (perpendicular to axis).
    let perp_seed = if dir.abs().dot(Vec3::Y) < 0.9 { Vec3::Y } else { Vec3::X };
    let x = (perp_seed - dir * perp_seed.dot(dir)).normalize();
    let y = dir.cross(x).normalize();

    // Pre-compute base-circle vertices.
    let mut verts: Vec<Vec3> = Vec::with_capacity(n);
    for i in 0..n {
        let theta = i as f32 / n as f32 * TAU;
        verts.push(base_center + (x * theta.cos() + y * theta.sin()) * base_radius);
    }
    // Base circle.
    for i in 0..n {
        gizmos.line(verts[i], verts[(i + 1) % n], col);
    }
    // Spoke lines from apex.
    for v in &verts {
        gizmos.line(apex, *v, col);
    }
}

/// Draw a distance-joint envelope: two concentric wireframe spheres
/// at `centre` with radii `min_m` and `max_m`. Useful for visualising
/// `PhysicsDistanceJoint` constraints.
///
/// Either radius ≤ 0 is skipped (caller can pass `0.0` for an
/// unlimited bound to draw only the other sphere).
pub fn draw_distance_envelope<C: GizmoConfigGroup>(
    gizmos: &mut Gizmos<'_, '_, C>,
    centre: Vec3,
    min_m: f32,
    max_m: f32,
    color_min: impl Into<Color>,
    color_max: impl Into<Color>,
) {
    if min_m > 0.0 {
        gizmos.sphere(Isometry3d::from_translation(centre), min_m, color_min.into());
    }
    if max_m > 0.0 {
        gizmos.sphere(Isometry3d::from_translation(centre), max_m, color_max.into());
    }
}
