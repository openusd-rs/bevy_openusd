//! Native USD instance and prototype identity on projected entities.

use bevy::prelude::*;
use super::{PrimRoute, RouteCtx};

/// Prototype identity within this entity's owning USD stage.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdNativeInstance {
    pub prototype: String,
}

/// Prototype prim represented by an instance-proxy entity.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdInstanceProxy {
    pub prototype_prim: String,
}

pub struct NativeInstanceRoute;

impl PrimRoute for NativeInstanceRoute {
    fn matches(&self, _: &RouteCtx) -> bool { true }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(prim) = ctx.stage.prim(ctx.path.clone()) else { return };
        let Ok(prototype) = prim.prototype() else { return };
        let Ok(proxy) = prim.prim_in_prototype() else { return };
        let Ok(mut entity) = world.get_entity_mut(entity) else { return };
        if let Some(prototype) = prototype {
            entity.insert(UsdNativeInstance { prototype: prototype.as_str().to_string() });
        } else { entity.remove::<UsdNativeInstance>(); }
        if let Some(proxy) = proxy {
            entity.insert(UsdInstanceProxy { prototype_prim: proxy.path().as_str().to_string() });
        } else { entity.remove::<UsdInstanceProxy>(); }
    }
}
