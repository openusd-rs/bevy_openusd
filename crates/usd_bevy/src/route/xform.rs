//! Transform route: any prim's composed local transform → Bevy [`Transform`].
//!
//! Bevy propagates ordinary TRS transforms through the projected hierarchy.
//! A post-propagation pass applies affine residuals and USD stack resets.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};
use crate::read::xform::{Transform3, read_transform_at};

mod propagation;

/// Exact linear transform and USD inheritance reset applied after Bevy propagation.
#[derive(Component, Clone, Debug)]
pub struct UsdTransformOverride { residual: Mat4, reset: bool }

/// Invalid authored transform that left the last projected pose unchanged.
#[derive(Component, Clone, Debug)]
pub struct UsdTransformError(pub String);

/// Updates USD affine globals before camera frusta and GPU joint palettes.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub enum UsdTransformSystems { Propagate }

pub(crate) fn configure(app: &mut App) { propagation::configure(app); }

/// Maps an `xformOp` stack to [`Transform`]. Applies to every prim — an
/// unauthored transform reads as identity, matching USD's Xformable fallback.
pub struct XformRoute;

fn to_bevy_transform(t: Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

/// The prim's local transform, or identity when none is authored / the read
/// fails.
pub fn transform_of(ctx: &RouteCtx) -> Transform {
    read_transform_at(ctx.stage, ctx.path, ctx.time)
        .ok()
        .flatten()
        .map(to_bevy_transform)
        .unwrap_or_default()
}

impl PrimRoute for XformRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        propagation::project(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        // Only react to xformOp changes (`xformOp:*`, `xformOpOrder`). A patch
        // that touches nothing transform-related is a no-op here.
        let touches_xform = changed.is_empty()
            || changed
                .iter()
                .any(|p| p.starts_with("xformOp") || *p == "xformOpOrder");
        if !touches_xform {
            return;
        }
        self.project(ctx, world, entity);
    }
}
