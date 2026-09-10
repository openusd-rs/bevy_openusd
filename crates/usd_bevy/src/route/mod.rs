//! Schema routing registry (RETHINK §12 / PLAN P1).
//!
//! The single contract that maps a composed USD prim to the components on its
//! projected Bevy entity. Every projection path — initial [`project_stage`],
//! incremental `changed_info` patching, and full `resynced` reconcile (all in
//! [`crate::live`]) — routes through here instead of hardcoding
//! transform/visibility/mesh handling.
//!
//! A [`PrimRoute`] is one prim-schema → component-set mapping. Built-in routes
//! cover the typed schemas ([`xform`], [`geom`] — Xform/Mesh/Visibility); the
//! reflect route ([`reflect`]) makes *any* registered `Reflect` component
//! authorable from USD via `bevy:`-namespaced attributes, with sparse
//! per-field patch semantics (USD's answer to BSN's `TemplatePatch`).
//!
//! USD composition does the opinion merging (LIVERPS in openusd); a route only
//! ever reads the *already-composed* value and writes it onto the entity. The
//! stage is the source of truth; entities are a projection.

pub mod audio;
pub mod cache;
pub mod camera;
pub mod coverage;
pub mod curves;
pub mod dome;
pub mod dome_environment;
pub mod environment_map;
pub mod geom;
pub mod instancer;
pub mod gpu_skin;
pub mod gpu_morph;
mod deformation;
pub mod flat_material;
pub mod native;
pub mod light;
pub mod points;
pub mod material;
mod texture_pack;
mod color_texture;
pub mod payload;
pub mod physics;
pub mod reflect;
pub mod shapes;
pub mod skel;
pub mod subset;
pub mod subdivision;
pub mod xform;

use std::sync::Arc;

use bevy::ecs::resource::Resource;
use bevy::ecs::world::World;
use bevy::prelude::Entity;
use openusd::sdf::Path;
use openusd::usd::Stage;

/// The current time to resolve animated attributes at. A plain `Resource`;
/// `current` is a USD time code. Set it (scrub / play) and the reprojection
/// loop resamples animated prims at that time. Absent ⇒ default (static) time.
#[derive(bevy::ecs::resource::Resource, Debug, Clone, Copy, Default)]
pub struct StageTime {
    /// The current USD time code.
    pub current: f64,
}

/// Which USD `purpose` classes are displayed (PLAN Phase A). `default` (and any
/// unrecognized token) is always shown; the three optional classes are toggles.
///
/// The interactive default matches USD's convention: show `proxy` (the
/// lightweight stand-in) and hide `render` (the final-quality twin) so that an
/// asset authoring both doesn't draw duplicated geometry, and hide `guide`
/// (viewport annotations). Flip these to switch a viewport to render-quality or
/// to reveal guides. Changing the resource reprojects visibility.
#[derive(bevy::ecs::resource::Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPurposes {
    /// Show `purpose = "render"` prims (final-quality twins). Off interactively.
    pub render: bool,
    /// Show `purpose = "proxy"` prims (lightweight stand-ins). On by default.
    pub proxy: bool,
    /// Show `purpose = "guide"` prims (annotations). Off by default.
    pub guide: bool,
}

impl Default for DisplayPurposes {
    fn default() -> Self {
        Self {
            render: false,
            proxy: true,
            guide: false,
        }
    }
}

impl DisplayPurposes {
    /// Whether a prim with the given effective `purpose` should be displayed.
    pub fn shows(&self, purpose: &str) -> bool {
        match purpose {
            "render" => self.render,
            "proxy" => self.proxy,
            "guide" => self.guide,
            // `default` and any unrecognized token are always shown.
            _ => true,
        }
    }
}

/// What a [`PrimRoute`] needs to read the stage for one prim. Built once per
/// prim per projection/patch and shared across every route, so the composed
/// `typeName` is read a single time.
pub struct RouteCtx<'a> {
    /// The live stage (source of truth).
    pub stage: &'a Stage,
    /// The composed absolute prim path.
    pub path: &'a Path,
    /// The prim's composed `typeName` (`"Mesh"`, `"Xform"`, …), if any.
    pub type_name: Option<String>,
    /// The time code to resolve animated attributes at (`None` = default time).
    pub time: Option<f64>,
}

impl<'a> RouteCtx<'a> {
    /// Build a context for `path` at the default (static) time.
    pub fn new(stage: &'a Stage, path: &'a Path) -> Self {
        Self::at(stage, path, None)
    }

    /// Build a context for `path`, resolving animated attributes at `time`.
    pub fn at(stage: &'a Stage, path: &'a Path, time: Option<f64>) -> Self {
        let type_name = stage
            .prim(path.clone()).expect("validated USD path")
            .type_name()
            .ok()
            .flatten()
            .map(|t| t.as_str().to_string());
        Self {
            stage,
            path,
            type_name,
            time,
        }
    }

    /// The prim path as a string.
    pub fn prim_str(&self) -> &str {
        self.path.as_str()
    }
}

/// One prim-schema → component mapping. Object-safe so routes can be boxed and
/// stored heterogeneously in the [`SchemaRegistry`].
///
/// Application has two tiers:
/// * [`project`](PrimRoute::project) — full application, run on the initial
///   projection and whenever the prim is `resynced` (composition restructured).
/// * [`patch`](PrimRoute::patch) — sparse in-place update, run on
///   `changed_info` with the set of property names that changed. Defaults to
///   [`project`](PrimRoute::project) for routes that can't refine.
pub trait PrimRoute: Send + Sync + 'static {
    fn name(&self) -> &'static str { std::any::type_name::<Self>() }

    /// Does this route apply to the prim? Cheap check off [`RouteCtx`]
    /// (`typeName`, applied API schema, or attribute-namespace presence).
    fn matches(&self, ctx: &RouteCtx) -> bool;

    /// Full application onto `entity` (fresh or being reconciled). Should be
    /// idempotent: inserting-or-overwriting the components it owns.
    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity);

    /// Removes owned projection data when this route no longer matches.
    fn remove(&self, _ctx: &RouteCtx, _world: &mut World, _entity: Entity) {}

    /// Sparse application given the property names that changed on this prim.
    /// Routes should ignore changes to properties they don't own. The default
    /// re-runs [`project`](PrimRoute::project).
    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, _changed: &[&str]) {
        self.project(ctx, world, entity);
    }
}

/// The ordered set of [`PrimRoute`]s. A plain `Resource` (routes are
/// `Send + Sync` behind `Arc`; the `!Send` stage never lives here). Route
/// order is registration order and is significant — later routes may read
/// components earlier routes inserted (e.g. a material route after the mesh
/// route).
///
/// `Clone` is cheap (an `Arc` bump per route), which lets the exclusive
/// projection systems pull a snapshot out of the world and then take `&mut
/// World` freely.
#[derive(Resource, Clone, Default)]
pub struct SchemaRegistry {
    routes: Vec<Arc<dyn PrimRoute>>,
}

#[derive(Debug, Clone, Default)]
pub struct RouteTiming {
    pub attempts: u64,
    pub matches: u64,
    pub matching: std::time::Duration,
    pub application: std::time::Duration,
}

/// Opt-in cumulative CPU timings for route matching and application.
#[derive(Resource, Debug, Default)]
pub struct ProjectionTimings(pub std::collections::BTreeMap<&'static str, RouteTiming>);

impl SchemaRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry with all built-in typed + reflect routes registered, in
    /// the canonical order (transform, visibility, mesh, reflect).
    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(xform::XformRoute);
        r.register(geom::VisibilityRoute);
        r.register(geom::MeshRoute);
        // Primitive shapes (Cube/Sphere/…) → Bevy primitive meshes.
        r.register(shapes::ShapesRoute);
        // Points → PointList mesh, BasisCurves → LineList mesh.
        r.register(points::PointsRoute);
        r.register(curves::CurvesRoute);
        // Material after mesh/shapes: it replaces the placeholder material.
        r.register(material::MaterialRoute);
        // Skin after mesh: it replaces the rest mesh with deformed geometry.
        r.register(skel::SkinRoute);
        r.register(subdivision::SubdivisionRoute);
        r.register(subset::SubsetRoute);
        r.register(light::LightRoute);
        r.register(dome::DomeLightRoute);
        r.register(camera::CameraRoute);
        r.register(instancer::PointInstancerRoute);
        // Physics schemas → marker components for an app's physics backend.
        r.register(physics::PhysicsRoute);
        // Media/volume schemas → data markers for an app's audio/volume backend.
        r.register(audio::SpatialAudioRoute);
        r.register(audio::VolumeRoute);
        // Render/procedural/UI schemas → data markers (config, evaluator, notes).
        r.register(coverage::RenderSettingsRoute);
        r.register(coverage::ProceduralRoute);
        r.register(coverage::BackdropRoute);
        // Unloaded payloads → placeholder marker.
        r.register(payload::PayloadRoute);
        r.register(native::NativeInstanceRoute);
        r.register(reflect::ReflectRoute);
        r
    }

    /// Append a route. This is the analog of "make a component available in
    /// `bsn!`": apps register routes for their own schemas/components.
    pub fn register<R: PrimRoute>(&mut self, route: R) {
        self.routes.push(Arc::new(route));
    }

    /// Number of registered routes.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether the registry has no routes.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Run every matching route's [`project`](PrimRoute::project) on `entity`,
    /// resolving animated attributes at the world's [`StageTime`] (if any).
    pub fn project_prim(&self, stage: &Stage, path: &Path, world: &mut World, entity: Entity) {
        let ctx = RouteCtx::at(stage, path, time_of(world));
        for route in &self.routes {
            run_route(route.as_ref(), &ctx, world, entity, None);
        }
    }

    /// Run every matching route's [`patch`](PrimRoute::patch) on `entity`,
    /// passing the property names that changed on this prim.
    pub fn patch_prim(
        &self,
        stage: &Stage,
        path: &Path,
        world: &mut World,
        entity: Entity,
        changed: &[&str],
    ) {
        let ctx = RouteCtx::at(stage, path, time_of(world));
        for route in &self.routes {
            run_route(route.as_ref(), &ctx, world, entity, Some(changed));
        }
    }
}

fn run_route(route: &dyn PrimRoute, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: Option<&[&str]>) {
    let timed = world.contains_resource::<ProjectionTimings>();
    if !timed {
        if route.matches(ctx) {
            if let Some(changed) = changed { route.patch(ctx, world, entity, changed); }
            else { route.project(ctx, world, entity); }
        } else { route.remove(ctx, world, entity); }
        return;
    }
    let start = std::time::Instant::now();
    let matched = route.matches(ctx);
    let matching = start.elapsed();
    let start = std::time::Instant::now();
    if matched {
        if let Some(changed) = changed { route.patch(ctx, world, entity, changed); }
        else { route.project(ctx, world, entity); }
    } else { route.remove(ctx, world, entity); }
    let application = start.elapsed();
    if let Some(mut timings) = world.get_resource_mut::<ProjectionTimings>() {
        let entry = timings.0.entry(route.name()).or_default();
        entry.attempts += 1;
        entry.matches += u64::from(matched);
        entry.matching += matching;
        entry.application += application;
    }
}

/// The current [`StageTime`] in `world`, if the resource is present.
fn time_of(world: &World) -> Option<f64> {
    world.get_resource::<StageTime>().map(|t| t.current)
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    struct Matches;
    impl PrimRoute for Matches {
        fn matches(&self, _: &RouteCtx) -> bool { true }
        fn project(&self, _: &RouteCtx, _: &mut World, _: Entity) {}
    }
    struct Skips;
    impl PrimRoute for Skips {
        fn matches(&self, _: &RouteCtx) -> bool { false }
        fn project(&self, _: &RouteCtx, _: &mut World, _: Entity) { panic!("nonmatching route applied") }
    }

    #[test]
    fn timing_is_opt_in_and_counts_project_and_patch_routes() {
        let stage = crate::UsdSource::new("timing.usda", &b"#usda 1.0\ndef Xform \"Root\" {}\n"[..]).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Root").unwrap();
        let mut registry = SchemaRegistry::new();
        registry.register(Matches);
        registry.register(Skips);
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        registry.project_prim(&stage, &path, &mut world, entity);
        assert!(!world.contains_resource::<ProjectionTimings>());
        world.init_resource::<ProjectionTimings>();
        registry.project_prim(&stage, &path, &mut world, entity);
        registry.patch_prim(&stage, &path, &mut world, entity, &["visibility"]);
        let timings = world.resource::<ProjectionTimings>();
        let matched = &timings.0[Matches.name()];
        assert_eq!((matched.attempts, matched.matches), (2, 2));
        let skipped = &timings.0[Skips.name()];
        assert_eq!((skipped.attempts, skipped.matches), (2, 0));
    }
}
