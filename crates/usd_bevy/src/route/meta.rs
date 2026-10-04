//! Prim metadata route: `kind` and `displayName` → marker components, so a
//! tree view can label and classify prims without touching the stage.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};

/// The prim's model `kind` metadata (`component`, `group`, `assembly`, …).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdKind {
    pub kind: String,
}

/// The prim's `displayName` metadata, the label a UI shows instead of the
/// prim name.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdDisplayName(pub String);

/// Projects `kind` and `displayName` on every prim that authors them.
pub struct MetaRoute;

impl PrimRoute for MetaRoute {
    fn matches(&self, _: &RouteCtx) -> bool {
        true
    }

    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world
            .entity_mut(entity)
            .remove::<(UsdKind, UsdDisplayName)>();
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(prim) = ctx.stage.prim(ctx.path.clone()) else {
            return;
        };
        let kind = prim
            .kind()
            .ok()
            .flatten()
            .map(|t| t.as_str().to_string())
            .filter(|k| !k.is_empty());
        let display_name = prim
            .get_metadata::<String>("displayName")
            .ok()
            .flatten()
            .filter(|n| !n.is_empty());
        let Ok(mut e) = world.get_entity_mut(entity) else {
            return;
        };
        e.remove::<(UsdKind, UsdDisplayName)>();
        if let Some(kind) = kind {
            e.insert(UsdKind { kind });
        }
        if let Some(name) = display_name {
            e.insert(UsdDisplayName(name));
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
    fn kind_and_display_name_project() {
        let stage = Stage::builder().in_memory("meta.usda").unwrap();
        stage
            .define_prim("/Tractor")
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .set_kind("component")
            .unwrap()
            .set_metadata("displayName", openusd::sdf::Value::String("Fendt".into()))
            .unwrap();
        stage
            .define_prim("/Plain")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let tractor = map.entity("/Tractor").unwrap();
        assert_eq!(
            world.get::<UsdKind>(tractor).map(|k| k.kind.as_str()),
            Some("component")
        );
        assert_eq!(
            world.get::<UsdDisplayName>(tractor).map(|n| n.0.as_str()),
            Some("Fendt")
        );
        let plain = map.entity("/Plain").unwrap();
        assert!(world.get::<UsdKind>(plain).is_none());
        assert!(world.get::<UsdDisplayName>(plain).is_none());
    }
}
