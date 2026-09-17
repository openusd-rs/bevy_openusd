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
pub mod meta;
mod texture_pack;
mod color_texture;
mod generated_image;

pub(crate) fn configure_texture_caches(app: &mut bevy::prelude::App) {
    color_texture::configure(app);
    generated_image::configure(app);
    texture_pack::configure(app);
}
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
/// `typeName` and mesh geometry are read once within that projection.
pub struct RouteCtx<'a> {
    /// The live stage (source of truth).
    pub stage: &'a Stage,
    /// The composed absolute prim path.
    pub path: &'a Path,
    /// The prim's composed `typeName` (`"Mesh"`, `"Xform"`, …), if any.
    pub type_name: Option<String>,
    /// The time code to resolve animated attributes at (`None` = default time).
    pub time: Option<f64>,
    decoded_mesh: std::cell::OnceCell<anyhow::Result<Option<crate::read::geom::ReadMesh>>>,
    read_timing: std::cell::Cell<Option<MeshReadTiming>>,
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
            decoded_mesh: Default::default(),
            read_timing: Default::default(),
        }
    }

    /// The prim path as a string.
    pub fn prim_str(&self) -> &str {
        self.path.as_str()
    }

    pub(crate) fn read_mesh(&self) -> anyhow::Result<Option<&crate::read::geom::ReadMesh>> {
        if let Some(mut timing) = self.read_timing.get() {
            timing.requests += 1;
            self.read_timing.set(Some(timing));
        }
        self.decoded_mesh.get_or_init(|| {
            let started = self.read_timing.get().map(|_| std::time::Instant::now());
            let result = crate::read::geom::read_mesh_at(self.stage, self.path, self.time);
            if let Some(started) = started {
                let mut timing = self.read_timing.get().unwrap();
                timing.elapsed += started.elapsed();
                timing.decodes += 1;
                match &result {
                    Ok(Some(read)) => timing.array_bytes += cache::read_mesh_bytes(read) as u64,
                    Ok(None) => timing.missing += 1,
                    Err(_) => timing.errors += 1,
                }
                self.read_timing.set(Some(timing));
            }
            result
        })
            .as_ref().map(Option::as_ref).map_err(|error| anyhow::anyhow!("{error:#}"))
    }

    fn report_read_timing(&self, world: &mut World) {
        if let Some(timing) = self.read_timing.get()
            && let Some(mut total) = world.get_resource_mut::<MeshReadTiming>() {
            total.requests += timing.requests;
            total.decodes += timing.decodes;
            total.missing += timing.missing;
            total.errors += timing.errors;
            total.array_bytes += timing.array_bytes;
            total.elapsed += timing.elapsed;
        }
    }
}

/// Opt-in mesh reads through registry contexts; elapsed time overlaps route timings.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct MeshReadTiming {
    pub requests: u64,
    pub decodes: u64,
    pub missing: u64,
    pub errors: u64,
    /// Returned geometry array payload, excluding subset data and allocator overhead.
    pub array_bytes: u64,
    pub elapsed: std::time::Duration,
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

/// Route costs by hierarchy visibility at entry; excludes camera/frustum visibility.
#[derive(Resource, Default)]
pub struct ProjectionVisibilityTimings(pub std::collections::BTreeMap<(&'static str, Option<bool>), RouteTiming>);

fn hierarchy_hidden(world: &World, mut entity: Entity) -> Option<bool> {
    use bevy::prelude::{ChildOf, Visibility};
    for _ in 0..128 {
        let current = world.get_entity(entity).ok()?;
        match current.get::<Visibility>() {
            Some(Visibility::Hidden) => return Some(true),
            Some(Visibility::Visible) => return Some(false),
            None => return Some(false),
            Some(Visibility::Inherited) => (),
        }
        let Some(parent) = current.get::<ChildOf>() else { return Some(false); };
        entity = parent.parent();
    }
    None
}

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
        // Prim metadata (kind, displayName) → labels for tree views.
        r.register(meta::MetaRoute);
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
        if world.contains_resource::<MeshReadTiming>() { ctx.read_timing.set(Some(MeshReadTiming::default())); }
        for route in &self.routes {
            run_route(route.as_ref(), &ctx, world, entity, None);
        }
        ctx.report_read_timing(world);
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
        if world.contains_resource::<MeshReadTiming>() { ctx.read_timing.set(Some(MeshReadTiming::default())); }
        for route in &self.routes {
            run_route(route.as_ref(), &ctx, world, entity, Some(changed));
        }
        ctx.report_read_timing(world);
    }
}

fn run_route(route: &dyn PrimRoute, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: Option<&[&str]>) {
    let timed = world.contains_resource::<ProjectionTimings>() || world.contains_resource::<ProjectionVisibilityTimings>();
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
    let hidden = (matched && world.contains_resource::<ProjectionVisibilityTimings>())
        .then(|| hierarchy_hidden(world, entity));
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
    if let Some(hidden) = hidden && let Some(mut timings) = world.get_resource_mut::<ProjectionVisibilityTimings>() {
        let entry = timings.0.entry((route.name(), hidden)).or_default();
        entry.attempts += 1;
        entry.matches += 1;
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

    #[test]
    fn hierarchy_profiling_handles_inheritance_overrides_and_depth_limits() {
        use bevy::prelude::{ChildOf, Visibility};
        let mut world = World::new();
        let parent = world.spawn(Visibility::Hidden).id();
        let child = world.spawn((Visibility::Inherited, ChildOf(parent))).id();
        assert_eq!(hierarchy_hidden(&world, child), Some(true));
        world.entity_mut(child).insert(Visibility::Visible);
        assert_eq!(hierarchy_hidden(&world, child), Some(false));
        world.entity_mut(child).insert(Visibility::Inherited);
        world.entity_mut(parent).insert(Visibility::Inherited);
        assert_eq!(hierarchy_hidden(&world, child), Some(false));
        world.entity_mut(parent).insert(Visibility::Hidden);
        let gap = world.spawn(ChildOf(parent)).id();
        let leaf = world.spawn((Visibility::Inherited, ChildOf(gap))).id();
        assert_eq!(hierarchy_hidden(&world, leaf), Some(false));
        let mut deep = child;
        for _ in 0..128 { deep = world.spawn((Visibility::Inherited, ChildOf(deep))).id(); }
        assert_eq!(hierarchy_hidden(&world, deep), None);
        world.despawn(deep);
        assert_eq!(hierarchy_hidden(&world, deep), None);
    }

    struct ReadsMesh;
    impl PrimRoute for ReadsMesh {
        fn matches(&self, ctx: &RouteCtx) -> bool { ctx.read_mesh().unwrap().is_some() }
        fn project(&self, ctx: &RouteCtx, _: &mut World, _: Entity) {
            assert!(ctx.read_mesh().unwrap().is_some());
        }
    }

    #[test]
    fn mesh_read_timing_counts_shared_reads_once_per_context() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/material_subsets.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Panels").unwrap();
        let read = crate::read::geom::read_mesh(&stage, &path).unwrap().unwrap();
        let mut registry = SchemaRegistry::new();
        registry.register(ReadsMesh);
        registry.register(ReadsMesh);
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        registry.project_prim(&stage, &path, &mut world, entity);
        assert!(!world.contains_resource::<MeshReadTiming>());
        world.init_resource::<MeshReadTiming>();
        registry.project_prim(&stage, &path, &mut world, entity);
        registry.patch_prim(&stage, &path, &mut world, entity, &["points"]);
        registry.project_prim(&stage, &openusd::sdf::path("/").unwrap(), &mut world, entity);
        let reads = world.resource::<MeshReadTiming>();
        assert_eq!((reads.requests, reads.decodes, reads.missing, reads.errors), (10, 3, 1, 0));
        assert_eq!(reads.array_bytes, 2 * cache::read_mesh_bytes(&read) as u64);
    }

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
        world.init_resource::<ProjectionVisibilityTimings>();
        registry.project_prim(&stage, &path, &mut world, entity);
        world.entity_mut(entity).insert(bevy::prelude::Visibility::Hidden);
        registry.patch_prim(&stage, &path, &mut world, entity, &["visibility"]);
        let visibility = &world.resource::<ProjectionVisibilityTimings>().0;
        assert_eq!(visibility[&(Matches.name(), Some(false))].matches, 1);
        assert_eq!(visibility[&(Matches.name(), Some(true))].matches, 1);
        assert_eq!(visibility.len(), 2);
        let timings = world.resource::<ProjectionTimings>();
        let matched = &timings.0[Matches.name()];
        assert_eq!((matched.attempts, matched.matches), (2, 2));
        let skipped = &timings.0[Skips.name()];
        assert_eq!((skipped.attempts, skipped.matches), (2, 0));
    }
}
