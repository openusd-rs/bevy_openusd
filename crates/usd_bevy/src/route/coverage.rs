//! Render / procedural / UI coverage routes (PLAN Phase 7): `RenderSettings`,
//! `GenerativeProcedural` and `Backdrop` prims → typed marker components.
//!
//! These schemas have no direct Bevy runtime equivalent (render config lives in
//! Bevy's own render graph; procedurals need an evaluator; backdrops are DCC
//! editor annotations). Like [`super::physics`] and [`super::audio`], the
//! routes project **data markers** so an app can discover them and act — read
//! render config, invoke a procedural evaluator, show a graph-editor note —
//! without re-walking the stage.

use bevy::prelude::*;
use openusd_schemas::render::SettingsSchema;
use openusd_schemas::proc::GenerativeProceduralSchema;
use openusd_schemas::ui::BackdropSchema;

use openusd::sdf::Value;
use openusd_schemas::proc::GenerativeProcedural;
use openusd_schemas::render::Settings as RenderSettings;
use openusd_schemas::ui::Backdrop;

use super::{PrimRoute, RouteCtx};

/// A `RenderSettings` prim: top-level render configuration.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdRenderSettings {
    /// `includedPurposes` (e.g. `["default", "render"]`).
    pub included_purposes: Vec<String>,
    /// `renderingColorSpace` token, if authored.
    pub color_space: Option<String>,
}

/// A `GenerativeProcedural` prim: geometry produced by an external evaluator.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdProcedural {
    /// `proceduralSystem`: which evaluator owns this prim.
    pub system: Option<String>,
}

/// A `Backdrop` prim: a DCC graph-editor annotation region.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdBackdrop {
    /// `ui:description` free-text note.
    pub description: Option<String>,
}

fn token_string(v: Option<Value>) -> Option<String> {
    match v? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn token_vec(v: Option<Value>) -> Vec<String> {
    match v {
        Some(Value::TokenVec(a)) => a.iter().map(|t| t.as_str().to_string()).collect(),
        Some(Value::StringVec(a)) => a,
        _ => Vec::new(),
    }
}

/// Projects `RenderSettings` prims as [`UsdRenderSettings`] markers.
pub struct RenderSettingsRoute;

impl PrimRoute for RenderSettingsRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<UsdRenderSettings>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("RenderSettings")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(rs)) = RenderSettings::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdRenderSettings {
            included_purposes: token_vec(
                rs.included_purposes_attr().get::<Value>().ok().flatten(),
            ),
            color_space: token_string(
                rs.rendering_color_space_attr().get::<Value>().ok().flatten(),
            ),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

/// Projects `GenerativeProcedural` prims as [`UsdProcedural`] markers.
pub struct ProceduralRoute;

impl PrimRoute for ProceduralRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<UsdProcedural>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("GenerativeProcedural")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(p)) = GenerativeProcedural::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdProcedural {
            system: token_string(p.procedural_system_attr().get::<Value>().ok().flatten()),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

/// Projects `Backdrop` prims as [`UsdBackdrop`] markers.
pub struct BackdropRoute;

impl PrimRoute for BackdropRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<UsdBackdrop>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Backdrop")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(b)) = Backdrop::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdBackdrop {
            description: token_string(b.description_attr().get::<Value>().ok().flatten()),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_routes_remove_stale_markers_on_type_changes() {
        use crate::live::{LiveStage, PrimEntities, project_stage, apply_changes};
        use crate::route::audio::{UsdSpatialAudio, UsdVolume};
        #[derive(Component)]
        struct RuntimeOnly;
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("markers.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("SpatialAudio").unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        world.entity_mut(entity).insert(RuntimeOnly);
        let child = world.spawn((RuntimeOnly, ChildOf(entity))).id();
        assert!(world.get::<UsdSpatialAudio>(entity).is_some());
        for kind in ["Volume", "RenderSettings", "GenerativeProcedural", "Backdrop", "Xform", "SpatialAudio", "Xform"] {
            live.stage.prim("/Prim").unwrap().set_type_name(kind).unwrap();
            apply_changes(&mut world, &live, &mut map);
            assert_eq!(map.entity("/Prim"), Some(entity));
            assert_eq!(world.get::<UsdSpatialAudio>(entity).is_some(), kind == "SpatialAudio");
            assert_eq!(world.get::<UsdVolume>(entity).is_some(), kind == "Volume");
            assert_eq!(world.get::<UsdRenderSettings>(entity).is_some(), kind == "RenderSettings");
            assert_eq!(world.get::<UsdProcedural>(entity).is_some(), kind == "GenerativeProcedural");
            assert_eq!(world.get::<UsdBackdrop>(entity).is_some(), kind == "Backdrop");
            assert!(world.get::<RuntimeOnly>(entity).is_some());
            assert!(world.get::<RuntimeOnly>(child).is_some());
        }
    }
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;

    #[test]
    fn render_proc_ui_project_markers() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cov.usda").unwrap();
        RenderSettings::define(&stage, "/Render/Settings").unwrap();
        stage
            .define_prim("/Render")
            .unwrap()
            .set_type_name("Scope")
            .unwrap();
        GenerativeProcedural::define(&stage, "/Proc").unwrap();
        Backdrop::define(&stage, "/Note").unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        assert!(
            world
                .get::<UsdRenderSettings>(map.entity("/Render/Settings").unwrap())
                .is_some(),
            "render settings marker"
        );
        assert!(
            world
                .get::<UsdProcedural>(map.entity("/Proc").unwrap())
                .is_some(),
            "procedural marker"
        );
        assert!(
            world
                .get::<UsdBackdrop>(map.entity("/Note").unwrap())
                .is_some(),
            "backdrop marker"
        );
    }
}
