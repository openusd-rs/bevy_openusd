//! Transform route: any prim's composed local transform → Bevy [`Transform`].
//!
//! The composed *world* transform comes from Bevy's transform propagation up
//! the projected prim hierarchy (see [`crate::live::project_stage`]); this
//! route only reads the prim-local transform.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};
use crate::read::xform::{Transform3, read_transform_at};

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
        let t = transform_of(ctx);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(t);
        }
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
        let t = transform_of(ctx);
        if let Some(mut tr) = world.get_mut::<Transform>(entity) {
            *tr = t;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(t);
        }
    }
}
