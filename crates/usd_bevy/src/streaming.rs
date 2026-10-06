//! Camera-driven streaming of USD payloads and LOD variants.
//!
//! A [`UsdSceneRoot`](crate::UsdSceneRoot) with [`UsdStreaming`] opens its
//! stage with every payload unloaded. Prims that carry a payload or an LOD
//! variant set become *units*. Each unit's authored `extentsHint`, placed by
//! the projected transforms, is measured against the streaming camera, as its
//! size on screen or its distance in meters: units that pass load or switch
//! to a finer level, units that fall back unload or switch to a coarser one.
//! Every change composes and projects the unit's subtree over several frames
//! within [`UsdStreaming::budget`], and the payloads it brings in become units
//! of their own.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::*;
use openusd::sdf::Path;
use openusd::usd::{LoadPolicy, Stage};

use crate::instance::{InstanceRuntime, UsdInstances};
use crate::live::ProjectionJob;
use crate::route::StageTime;

/// Streams a scene root's payloads and LOD variants around the camera.
#[derive(Component, Debug, Clone)]
pub struct UsdStreaming {
    /// When a unit loads and unloads.
    pub metric: StreamingMetric,
    /// Main-thread time per frame for loading, unloading, switching and
    /// projecting.
    pub budget: Duration,
    /// Whether LOD variant sets switch by the same metric.
    pub lod: bool,
    /// With [`StreamingMetric::ScreenSize`], the size in pixels at which a
    /// unit shows its finest LOD level, with coarser levels spread down to
    /// the load size; twice the viewport height when `None`, so the finest
    /// level waits until a unit engulfs the view.
    pub lod_full: Option<f32>,
    /// With [`StreamingMetric::Distance`], how many times closer each finer
    /// LOD level sits.
    pub lod_step: f32,
    /// How often units are measured again.
    pub interval: Duration,
}

impl Default for UsdStreaming {
    fn default() -> Self {
        Self {
            metric: StreamingMetric::default(),
            budget: Duration::from_millis(8),
            lod: true,
            lod_full: None,
            lod_step: 4.0,
            interval: Duration::from_millis(250),
        }
    }
}

/// When a unit is wanted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamingMetric {
    /// Load when a unit's bounds span at least `load` pixels on screen,
    /// unload below `unload`.
    ScreenSize { load: f32, unload: f32 },
    /// Load within `load` meters of the camera, unload beyond `unload`.
    Distance { load: f32, unload: f32 },
}

impl Default for StreamingMetric {
    fn default() -> Self {
        Self::ScreenSize {
            load: 32.0,
            unload: 16.0,
        }
    }
}

impl StreamingMetric {
    /// How wanted a unit is; higher is more.
    fn score(self, pixels: f32, meters: f32) -> f32 {
        match self {
            Self::ScreenSize { .. } => pixels,
            Self::Distance { .. } => -meters,
        }
    }

    /// The score to come in and the score to stay in, for `level` of a set
    /// with `levels` levels; level zero is the load threshold itself.
    fn thresholds(self, level: usize, levels: usize, full: f32, step: f32) -> (f32, f32) {
        if level == 0 {
            return match self {
                Self::ScreenSize { load, unload } => (load, unload),
                Self::Distance { load, unload } => (-load, -unload),
            };
        }
        match self {
            Self::ScreenSize { load, unload } => {
                let span = level as f32 / levels.saturating_sub(1).max(1) as f32;
                let factor = (full / load.max(f32::EPSILON)).max(1.0).powf(span);
                (load * factor, unload * factor)
            }
            Self::Distance { load, unload } => {
                let factor = step.max(1.0).powi(level as i32);
                (-load / factor, -unload / factor)
            }
        }
    }
}

/// A streaming root's units and outstanding work, on the root.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsdStreamingStatus {
    /// Prims streaming tracks.
    pub units: usize,
    /// Units whose payload is loaded.
    pub loaded: usize,
    /// Changes and projections not done yet; zero once streaming has caught
    /// up with the camera.
    pub pending: usize,
}

/// Marks the camera streaming roots load around; without one, the first
/// active 3D camera is used.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct UsdStreamingFocus;

/// What streaming holds for a unit prim, on its entity.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdStreamingUnit {
    /// Whether the payload is loaded; always true for an LOD-only unit.
    pub loaded: bool,
    /// The LOD variant set and its current selection.
    pub level: Option<(String, String)>,
    /// The unit's bounds in world space, when known.
    pub bounds: Option<(Vec3, Vec3)>,
}

struct Lod {
    set: String,
    /// Variants from coarsest to finest.
    levels: Vec<String>,
    current: usize,
}

struct Unit {
    payload: bool,
    loaded: bool,
    lod: Option<Lod>,
    /// The authored `extentsHint`: `None` when absent, `Some(None)` when it
    /// holds only empty boxes.
    hint: Option<Option<[Vec3; 2]>>,
}

enum Action {
    Load(Path),
    Unload(Path),
    Switch(Path, usize),
}

/// A root's units and the work streaming has in flight.
pub(crate) struct StreamingState {
    units: HashMap<Path, Unit>,
    discovered: Vec<Path>,
    jobs: VecDeque<Pending>,
    actions: Vec<(f32, Action)>,
    meters_per_unit: f32,
    measured: Option<Instant>,
    reported: Option<Instant>,
}

impl StreamingState {
    pub(crate) fn new(stage: &Stage) -> Self {
        let meters_per_unit = crate::live::stage_meters_per_unit(stage);
        Self {
            units: HashMap::new(),
            discovered: Vec::new(),
            jobs: VecDeque::new(),
            actions: Vec::new(),
            meters_per_unit,
            measured: None,
            reported: None,
        }
    }

    /// Takes prims a projection just spawned, to find units among them.
    pub(crate) fn note_projected(&mut self, paths: Vec<Path>) {
        self.discovered.extend(paths);
    }
}

/// Whether `name` names a variant set that selects levels of detail.
fn is_lod_set(name: &str) -> bool {
    name.to_ascii_lowercase().contains("lod")
}

/// Orders LOD variant names from coarsest to finest.
fn detail_rank(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if let Some(digits) = lower.strip_prefix("lod").filter(|rest| !rest.is_empty())
        && let Ok(level) = digits.parse::<i32>()
    {
        return 100 - level;
    }
    for (words, rank) in [
        (&["proxy", "bbox", "box", "card", "billboard"][..], 0),
        (&["low"][..], 20),
        (&["med", "mid"][..], 40),
        (&["high"][..], 60),
        (&["full", "render", "hero"][..], 80),
    ] {
        if words.iter().any(|word| lower.contains(word)) {
            return rank;
        }
    }
    50
}

fn classify(stage: &Stage, path: &Path, lod: bool) -> Option<Unit> {
    let prim = stage.prim(path.clone()).ok()?;
    let payload = !prim.is_loaded().unwrap_or(true);
    let lod = lod.then(|| lod_of(stage, path)).flatten();
    if !payload && lod.is_none() {
        return None;
    }
    let hint = crate::read::geom::read_extents_hint(stage, path)
        .ok()
        .flatten()
        .map(|hint| hint.map(|[min, max]| [Vec3::from_array(min), Vec3::from_array(max)]));
    Some(Unit {
        payload,
        loaded: !payload,
        lod,
        hint,
    })
}

fn lod_of(stage: &Stage, path: &Path) -> Option<Lod> {
    crate::read::variants::variant_set_names(stage, path)
        .into_iter()
        .filter(|set| is_lod_set(set))
        .find_map(|set| {
            let mut levels = crate::read::variants::variant_options(stage, path, &set);
            if levels.len() < 2 {
                return None;
            }
            levels.sort_by_key(|level| detail_rank(level));
            let selected = crate::read::variants::variant_selection(stage, path, &set);
            let current = selected
                .and_then(|selected| levels.iter().position(|level| *level == selected))
                .unwrap_or(0);
            Some(Lod {
                set,
                levels,
                current,
            })
        })
}

/// `entity`'s world matrix from the `Transform`s of its ancestors, so a unit
/// spawned this frame is placed before transform propagation runs.
fn world_matrix(world: &World, entity: Entity, cache: &mut HashMap<Entity, Mat4>) -> Mat4 {
    if let Some(matrix) = cache.get(&entity) {
        return *matrix;
    }
    let local = world
        .get::<Transform>(entity)
        .map_or(Mat4::IDENTITY, Transform::to_matrix);
    let matrix = match world.get::<ChildOf>(entity) {
        Some(parent) => world_matrix(world, parent.parent(), cache) * local,
        None => local,
    };
    cache.insert(entity, matrix);
    matrix
}

fn world_bounds(matrix: Mat4, [min, max]: [Vec3; 2]) -> (Vec3, Vec3) {
    let corners = (0..8).map(|corner| {
        matrix.transform_point3(Vec3::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
        ))
    });
    corners.fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(low, high), corner| (low.min(corner), high.max(corner)),
    )
}

/// The streaming camera: position, half the vertical field of view as a
/// tangent, and the viewport height in pixels.
struct Focus {
    position: Vec3,
    tan_half_fov: f32,
    height: f32,
}

fn focus(world: &mut World) -> Option<Focus> {
    let mut cameras = world.query::<(
        &Camera,
        &GlobalTransform,
        &Projection,
        Has<UsdStreamingFocus>,
    )>();
    let (camera, transform, projection, _) = cameras
        .iter(world)
        .filter(|(camera, ..)| camera.is_active)
        .max_by_key(|(.., marked)| *marked)?;
    let tan_half_fov = match projection {
        Projection::Perspective(perspective) => (perspective.fov * 0.5).tan(),
        _ => 1.0,
    };
    Some(Focus {
        position: transform.translation(),
        tan_half_fov,
        height: camera.logical_viewport_size().map_or(1080.0, |size| size.y),
    })
}

/// The LOD level `score` asks for, starting from `current`: a finer level
/// needs its load threshold, keeping a level needs its unload threshold.
fn wanted_level(
    settings: &UsdStreaming,
    full: f32,
    score: f32,
    current: usize,
    levels: usize,
) -> usize {
    let mut level = 0;
    for candidate in 1..levels {
        let (come, stay) = settings
            .metric
            .thresholds(candidate, levels, full, settings.lod_step);
        let needed = if candidate <= current { stay } else { come };
        if score < needed {
            break;
        }
        level = candidate;
    }
    level
}

/// Measures units, plans loads, unloads and switches, and spends each
/// streaming root's budget on them and on projecting what they bring in.
pub(crate) fn tick(world: &mut World, instances: &mut UsdInstances) {
    let roots: Vec<_> = instances
        .roots
        .iter()
        .filter(|(_, runtime)| runtime.streaming.is_some() && runtime.job.is_none())
        .map(|(&root, _)| root)
        .collect();
    if roots.is_empty() {
        return;
    }
    let focus = focus(world);
    for root in roots {
        let Some(settings) = world.get::<UsdStreaming>(root).cloned() else {
            continue;
        };
        let runtime = instances.roots.get_mut(&root).unwrap();
        let started = Instant::now();
        discover(runtime, &settings);
        if let Some(focus) = &focus {
            let state = runtime.streaming.as_mut().unwrap();
            if state
                .measured
                .is_none_or(|at| at.elapsed() >= settings.interval)
            {
                state.measured = Some(Instant::now());
                plan(world, runtime, &settings, focus);
            }
        }
        // At least one batch per tick, so a tiny budget still makes progress.
        loop {
            let actions = &mut runtime.streaming.as_mut().unwrap().actions;
            if actions.is_empty() {
                break;
            }
            let batch = actions.split_off(actions.len().saturating_sub(BATCH));
            apply(world, runtime, batch);
            if started.elapsed() >= settings.budget {
                break;
            }
        }
        project(
            world,
            runtime,
            settings.budget.saturating_sub(started.elapsed()),
        );
        let state = runtime.streaming.as_ref().unwrap();
        let status = UsdStreamingStatus {
            units: state.units.len(),
            loaded: state
                .units
                .values()
                .filter(|unit| unit.payload && unit.loaded)
                .count(),
            pending: state.actions.len() + state.jobs.len() + state.discovered.len(),
        };
        if world.get::<UsdStreamingStatus>(root) != Some(&status)
            && let Ok(mut entity) = world.get_entity_mut(root)
        {
            entity.insert(status);
        }
        let state = runtime.streaming.as_mut().unwrap();
        if std::env::var_os("USD_PROFILE_STREAMING").is_some()
            && state
                .reported
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
        {
            state.reported = Some(Instant::now());
            let finer = state
                .units
                .values()
                .filter(|unit| unit.lod.as_ref().is_some_and(|lod| lod.current > 0))
                .count();
            eprintln!(
                "streaming_profile units={} loaded={} finer={finer} actions={} jobs={} prims={}",
                status.units,
                status.loaded,
                state.actions.len(),
                state.jobs.len(),
                runtime.map.len()
            );
            if std::env::var_os("USD_PROFILE_STREAMING").is_some_and(|value| value == "2") {
                for job in &state.jobs {
                    eprintln!(
                        "streaming_job {:?} {:?}",
                        job.job.subtree_root(),
                        job.job.progress()
                    );
                }
            }
        }
    }
}

/// Classifies the prims projected since the last tick.
fn discover(runtime: &mut InstanceRuntime, settings: &UsdStreaming) {
    let state = runtime.streaming.as_mut().unwrap();
    for path in std::mem::take(&mut state.discovered) {
        if let Some(unit) = classify(&runtime.live.stage, &path, settings.lod) {
            state.units.insert(path, unit);
        }
    }
}

/// Measures every unit against `focus` and queues what should change,
/// most wanted last so it is popped first.
fn plan(world: &mut World, runtime: &mut InstanceRuntime, settings: &UsdStreaming, focus: &Focus) {
    let state = runtime.streaming.as_mut().unwrap();
    let metric = settings.metric;
    let full = settings.lod_full.unwrap_or(focus.height * 2.0);
    let mut cache = HashMap::new();
    let mut actions = Vec::new();
    let mut markers = Vec::new();
    for (path, unit) in &state.units {
        let Some(entity) = runtime.map.entity(path.as_str()) else {
            continue;
        };
        let bounds = unit
            .hint
            .flatten()
            .map(|hint| world_bounds(world_matrix(world, entity, &mut cache), hint));
        let score = match (bounds, unit.hint) {
            (Some((low, high)), _) => {
                let center = (low + high) * 0.5;
                let radius = (high - low).length() * 0.5;
                let gap = center.distance(focus.position) - radius;
                let pixels = if gap <= 0.0 {
                    f32::INFINITY
                } else {
                    radius * focus.height / (gap * focus.tan_half_fov)
                };
                metric.score(pixels, gap.max(0.0) * state.meters_per_unit)
            }
            // A hint of only empty boxes means there is nothing to show.
            (None, Some(None)) => f32::NEG_INFINITY,
            // Without a hint a unit is loaded so that its content shows.
            (None, _) => f32::INFINITY,
        };
        let score = if score.is_nan() {
            f32::NEG_INFINITY
        } else {
            score
        };
        let (come, stay) = metric.thresholds(0, 1, full, settings.lod_step);
        let wanted = score >= if unit.loaded { stay } else { come };
        if unit.payload && wanted != unit.loaded {
            let action = if wanted {
                Action::Load(path.clone())
            } else {
                Action::Unload(path.clone())
            };
            actions.push((if wanted { score } else { -score }, action));
        }
        if let Some(lod) = &unit.lod
            && (unit.loaded || wanted)
        {
            let level = if wanted {
                wanted_level(settings, full, score, lod.current, lod.levels.len())
            } else {
                0
            };
            if level != lod.current {
                actions.push((score, Action::Switch(path.clone(), level)));
            }
        }
        markers.push((
            entity,
            UsdStreamingUnit {
                loaded: unit.loaded,
                level: unit
                    .lod
                    .as_ref()
                    .map(|lod| (lod.set.clone(), lod.levels[lod.current].clone())),
                bounds,
            },
        ));
    }
    // Unloads first, then the most wanted loads and switches.
    actions.sort_by(|a, b| {
        let unload = |action: &Action| matches!(action, Action::Unload(_));
        unload(&a.1).cmp(&unload(&b.1)).then(a.0.total_cmp(&b.0))
    });
    state.actions = actions;
    for (entity, marker) in markers {
        if world.get::<UsdStreamingUnit>(entity) != Some(&marker)
            && let Ok(mut entity) = world.get_entity_mut(entity)
        {
            entity.insert(marker);
        }
    }
}

/// Changes applied together, so the load rules are rebuilt once per batch.
const BATCH: usize = 64;

/// Applies a batch of planned changes and queues the projection of what
/// they bring in.
fn apply(world: &mut World, runtime: &mut InstanceRuntime, batch: Vec<(f32, Action)>) {
    let stage = runtime.live.stage.clone();
    let (mut loads, mut unloads, mut switches) = (Vec::new(), Vec::new(), Vec::new());
    for (_, action) in batch {
        match action {
            Action::Load(path) => loads.push(path),
            Action::Unload(path) => unloads.push(path),
            Action::Switch(path, level) => switches.push((path, level)),
        }
    }
    if (!loads.is_empty() || !unloads.is_empty())
        && let Err(error) = stage.load_and_unload(
            loads
                .iter()
                .map(|path| (path.clone(), LoadPolicy::WithoutDescendants)),
            unloads.iter().cloned(),
        )
    {
        warn!("streaming loads: {error}");
        return;
    }
    for path in &unloads {
        mark(runtime, path, |unit| unit.loaded = false);
        collapse(world, runtime, path, false);
    }
    for path in &loads {
        mark(runtime, path, |unit| unit.loaded = true);
    }
    let mut switched = Vec::new();
    for (path, level) in switches {
        let Some(lod) = runtime
            .streaming
            .as_ref()
            .and_then(|state| state.units.get(&path))
            .and_then(|unit| unit.lod.as_ref())
        else {
            continue;
        };
        let (set, variant) = (lod.set.clone(), lod.levels[level].clone());
        if let Err(error) = crate::authoring::set_variant(&stage, path.as_str(), &set, &variant) {
            warn!("streaming switch of {path} to {set}={variant}: {error}");
            continue;
        }
        mark(runtime, &path, |unit| {
            if let Some(lod) = &mut unit.lod {
                lod.current = level;
            }
        });
        switched.push(path);
    }
    // These changes are projected below; the live-edit pass must not redo them.
    let _ = runtime.live.drain_changes();
    for path in loads {
        expand(world, runtime, &path, false);
    }
    // A switched unit keeps its old level on screen until the new one is in.
    for path in switched {
        expand(world, runtime, &path, true);
    }
}

fn mark(runtime: &mut InstanceRuntime, path: &Path, change: impl FnOnce(&mut Unit)) {
    if let Some(unit) = runtime
        .streaming
        .as_mut()
        .and_then(|state| state.units.get_mut(path))
    {
        change(unit);
    }
}

/// Removes `path`'s projected descendants from the prim map, forgets the
/// units among them, stops their projections and reprojects `path` itself.
/// The descendants are despawned, or with `keep` their top entities are
/// returned still spawned, so the old content stays visible while its
/// replacement projects.
fn collapse(
    world: &mut World,
    runtime: &mut InstanceRuntime,
    path: &Path,
    keep: bool,
) -> Vec<Entity> {
    let prefix = path.as_str();
    let depth = prefix.matches('/').count() + usize::from(prefix != "/");
    let mut below: Vec<_> = runtime
        .map
        .subtree(prefix)
        .into_iter()
        .filter(|(child, _)| child != prefix)
        .collect();
    below.sort_by_key(|(child, _)| std::cmp::Reverse(child.matches('/').count()));
    let mut kept = Vec::new();
    for (child, entity) in below {
        runtime.map.remove_path(&child);
        if keep {
            if child.matches('/').count() == depth {
                kept.push(entity);
            }
        } else if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }
    let state = runtime.streaming.as_mut().unwrap();
    state
        .units
        .retain(|unit, _| !unit.has_prefix(path) || unit == path);
    state
        .discovered
        .retain(|found| !found.has_prefix(path) || found == path);
    let (cancelled, jobs): (Vec<_>, Vec<_>) = std::mem::take(&mut state.jobs)
        .into_iter()
        .partition(|pending| pending.job.covers(path));
    state.jobs = jobs.into();
    for pending in cancelled {
        pending.retire(world);
    }
    reproject(world, runtime, path);
    kept
}

/// Collapses `path` and queues the projection of its new descendants. With
/// `keep` the old descendants stay until the new ones are projected.
fn expand(world: &mut World, runtime: &mut InstanceRuntime, path: &Path, keep: bool) {
    let retiring = collapse(world, runtime, path, keep);
    let Some(entity) = runtime.map.entity(path.as_str()) else {
        Pending::despawn(world, retiring);
        return;
    };
    let mut job = ProjectionJob::subtree(&runtime.live.stage, path, entity);
    job.record_projected();
    runtime
        .streaming
        .as_mut()
        .unwrap()
        .jobs
        .push_back(Pending { job, retiring });
}

/// A subtree projection and the entities it replaces, which stay visible
/// until it finishes.
pub(crate) struct Pending {
    job: ProjectionJob,
    retiring: Vec<Entity>,
}

impl Pending {
    fn retire(self, world: &mut World) {
        Self::despawn(world, self.retiring);
    }

    fn despawn(world: &mut World, entities: Vec<Entity>) {
        for entity in entities {
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
    }
}

/// Reapplies every route to `path`'s entity, whose own opinions a load or a
/// switch can change.
fn reproject(world: &mut World, runtime: &mut InstanceRuntime, path: &Path) {
    let Some(entity) = runtime.map.entity(path.as_str()) else {
        return;
    };
    with_projection(world, runtime, |world, runtime| {
        let registry = crate::live::registry_of(world);
        registry.patch_prim(&runtime.live.stage, path, world, entity, &[]);
        runtime
            .map
            .remember_type(&runtime.live.stage, path.as_str());
    });
}

/// Steps the queued subtree projections within `budget`, round robin: each
/// gets a slice and goes to the back if unfinished, so small loads finish in
/// their first turn while large scaffolds keep advancing.
fn project(world: &mut World, runtime: &mut InstanceRuntime, budget: Duration) {
    let started = Instant::now();
    let slice = (budget / 8).max(Duration::from_millis(1));
    let mut turns = runtime.streaming.as_ref().unwrap().jobs.len();
    let mut stepped = false;
    while turns > 0 && (!stepped || started.elapsed() < budget) {
        turns -= 1;
        stepped = true;
        let Some(job) = runtime.streaming.as_mut().unwrap().jobs.pop_front() else {
            break;
        };
        let remaining = budget.saturating_sub(started.elapsed()).min(slice);
        if let Some(job) = step_job(world, runtime, job, remaining) {
            runtime.streaming.as_mut().unwrap().jobs.push_back(job);
        }
    }
}

/// Steps `pending` for `budget`, handing it back if it is not done and
/// retiring what it replaces once it is.
fn step_job(
    world: &mut World,
    runtime: &mut InstanceRuntime,
    mut pending: Pending,
    budget: Duration,
) -> Option<Pending> {
    let done = with_projection(world, runtime, |world, runtime| {
        let stage = runtime.live.stage.clone();
        pending.job.step(world, &stage, &mut runtime.map, budget)
    });
    runtime
        .streaming
        .as_mut()
        .unwrap()
        .note_projected(pending.job.drain_projected());
    if done {
        pending.retire(world);
        return None;
    }
    Some(pending)
}

/// Runs `f` with the root's clock, textures and material memo installed, as
/// the routes expect while they project.
fn with_projection<T>(
    world: &mut World,
    runtime: &mut InstanceRuntime,
    f: impl FnOnce(&mut World, &mut InstanceRuntime) -> T,
) -> T {
    let previous_time = world.remove_resource::<StageTime>();
    let previous_textures = world.remove_resource::<crate::asset::SnapshotTextures>();
    let previous_materials = world.remove_non_send::<crate::route::material::ProjectionMaterials>();
    world.insert_resource(StageTime {
        current: runtime.sampled,
    });
    world.insert_resource(runtime.textures.clone());
    let memo = runtime
        .materials
        .take()
        .unwrap_or_else(|| crate::route::material::ProjectionMaterials::new(&runtime.live.stage));
    world.insert_non_send(memo);
    let result = f(world, runtime);
    runtime.materials = world.remove_non_send::<crate::route::material::ProjectionMaterials>();
    world.remove_resource::<crate::asset::SnapshotTextures>();
    world.remove_resource::<StageTime>();
    if let Some(previous) = previous_time {
        world.insert_resource(previous);
    }
    if let Some(previous) = previous_textures {
        world.insert_resource(previous);
    }
    if let Some(previous) = previous_materials {
        world.insert_non_send(previous);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{UsdScene, UsdSceneRoot, spawn_usd_scenes};

    const ROOT: &str = r#"#usda 1.0
(
    metersPerUnit = 1
)
def Xform "Near" (
    prepend payload = @near.usda@</Model>
) {
    float3[] extentsHint = [(-1, -1, -1), (1, 1, 1)]
    double3 xformOp:translate = (0, 0, -5)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "Far" (
    prepend payload = @near.usda@</Model>
) {
    float3[] extentsHint = [(-1, -1, -1), (1, 1, 1)]
    double3 xformOp:translate = (0, 0, -50000)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "Empty" (
    prepend payload = @near.usda@</Model>
) {
    float3[] extentsHint = [(3.4e38, 3.4e38, 3.4e38), (-3.4e38, -3.4e38, -3.4e38)]
    double3 xformOp:translate = (0, 0, -3)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "Render" (
    prepend payload = @near.usda@</Model>
) {
    float3[] extentsHint = [(3.4e38, 3.4e38, 3.4e38), (-3.4e38, -3.4e38, -3.4e38), (-1, -1, -1), (1, 1, 1)]
    double3 xformOp:translate = (0, 0, -3)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "Switch" (
    variants = { string lod = "proxy" }
    prepend variantSets = "lod"
) {
    float3[] extentsHint = [(-1, -1, -1), (1, 1, 1)]
    double3 xformOp:translate = (2, 0, -3)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    variantSet "lod" = {
        "full" { def Sphere "Detail" {} }
        "proxy" { def Cube "Proxy" {} def Cube "Proxy2" {} }
    }
}
"#;

    const NEAR: &str = r#"#usda 1.0
def Xform "Model" {
    def Cube "Box" {}
    def Xform "Inner" (
        prepend payload = @inner.usda@</Inner>
    ) {
        float3[] extentsHint = [(-1, -1, -1), (1, 1, 1)]
    }
}
"#;

    const INNER: &str = "#usda 1.0\ndef Xform \"Inner\" { def Sphere \"Ball\" {} }\n";

    fn streaming_world() -> (World, Entity, Entity) {
        let mut world = World::new();
        world.insert_resource(crate::SchemaRegistry::builtin());
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let source = crate::UsdSource::from_memory(
            "root.usda",
            [
                ("root.usda", ROOT.as_bytes()),
                ("near.usda", NEAR.as_bytes()),
                ("inner.usda", INNER.as_bytes()),
            ],
        )
        .unwrap();
        let mut scenes = Assets::<UsdScene>::default();
        let handle = scenes.add(UsdScene {
            source,
            textures: default(),
        });
        world.insert_resource(scenes);
        let root = world
            .spawn((
                UsdSceneRoot(handle),
                UsdStreaming {
                    interval: Duration::ZERO,
                    budget: Duration::from_secs(5),
                    ..default()
                },
            ))
            .id();
        let camera = world
            .spawn((
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                GlobalTransform::default(),
            ))
            .id();
        (world, root, camera)
    }

    fn settle(world: &mut World) {
        for _ in 0..8 {
            spawn_usd_scenes(world);
        }
    }

    fn projected(world: &World, root: Entity, path: &str) -> bool {
        world
            .non_send::<UsdInstances>()
            .entity(root, path)
            .is_some()
    }

    #[test]
    fn near_payloads_load_and_unload_as_the_camera_leaves() {
        let (mut world, root, camera) = streaming_world();
        settle(&mut world);
        assert!(
            projected(&world, root, "/Near/Box"),
            "the near payload loads"
        );
        assert!(
            projected(&world, root, "/Near/Inner/Ball"),
            "its nested payload loads too"
        );
        assert!(projected(&world, root, "/Far"));
        assert!(
            !projected(&world, root, "/Far/Box"),
            "the far payload stays unloaded"
        );
        assert!(
            !projected(&world, root, "/Empty/Box"),
            "an empty hint never loads"
        );
        assert!(
            projected(&world, root, "/Render/Box"),
            "any purpose's box counts"
        );
        let near = world
            .non_send::<UsdInstances>()
            .entity(root, "/Near")
            .unwrap();
        assert!(world.get::<UsdStreamingUnit>(near).unwrap().loaded);

        *world.get_mut::<GlobalTransform>(camera).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 100_000.0));
        settle(&mut world);
        assert!(projected(&world, root, "/Near"));
        assert!(
            !projected(&world, root, "/Near/Box"),
            "the near payload unloads"
        );
        assert!(!projected(&world, root, "/Near/Inner"));
        assert!(!world.get::<UsdStreamingUnit>(near).unwrap().loaded);
    }

    #[test]
    fn lod_variants_follow_the_camera() {
        let (mut world, root, camera) = streaming_world();
        settle(&mut world);
        assert!(
            projected(&world, root, "/Switch/Detail"),
            "close up selects the finest level"
        );
        assert!(!projected(&world, root, "/Switch/Proxy"));

        *world.get_mut::<GlobalTransform>(camera).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 100_000.0));
        settle(&mut world);
        assert!(
            projected(&world, root, "/Switch/Proxy"),
            "far away selects the coarsest level"
        );
        assert!(!projected(&world, root, "/Switch/Detail"));
    }

    #[test]
    fn a_switched_level_stays_until_its_replacement_is_in() {
        let (mut world, root, camera) = streaming_world();
        settle(&mut world);
        let detail = world
            .non_send::<UsdInstances>()
            .entity(root, "/Switch/Detail")
            .unwrap();
        world.get_mut::<UsdStreaming>(root).unwrap().budget = Duration::ZERO;
        *world.get_mut::<GlobalTransform>(camera).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 100_000.0));
        spawn_usd_scenes(&mut world);
        assert!(projected(&world, root, "/Switch/Proxy"));
        assert!(
            !projected(&world, root, "/Switch/Proxy2"),
            "one prim per tick"
        );
        assert!(
            world.get_entity(detail).is_ok(),
            "the old level stays meanwhile"
        );
        spawn_usd_scenes(&mut world);
        assert!(projected(&world, root, "/Switch/Proxy2"));
        assert!(
            world.get_entity(detail).is_err(),
            "and goes once the new one is in"
        );
    }

    #[test]
    fn lod_names_rank_from_coarse_to_fine() {
        let mut names = vec!["full", "lod0", "proxy", "lod2", "high", "low"];
        names.sort_by_key(|name| detail_rank(name));
        assert_eq!(names, ["proxy", "low", "high", "full", "lod2", "lod0"]);
    }
}
