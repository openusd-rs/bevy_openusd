//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).
//!
//! The composed USD stage is the single source of truth. We hold it live
//! (not baked to a one-shot `Scene`), project it into Bevy entities, and
//! keep them in sync off openusd's `StageSink` (`UsdNotice`) change stream:
//! every committed edit fires the sink, we copy the changed paths out, and a
//! Bevy system patches local changes and reconciles dependency-affecting edits.
//!
//! The openusd `Stage` is `Rc`/`RefCell`-backed (`!Send`), so [`LiveStage`]
//! is a **non-send** resource (main thread only). The path↔entity index
//! [`PrimEntities`] is plain data and is a normal `Resource`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bevy::prelude::*;
use openusd::usd::{CommittedChange, Stage, StageSinkId};

/// One committed stage change, copied out of the borrowed [`CommittedChange`]
/// so it can outlive the sink callback and be drained on a later frame.
///
/// * `resynced` — composition restructured (define / remove / reparent /
///   variant / reference / layer-mute …); the subtree must be reprojected.
/// * `changed_info` — a field/value/target changed, namespace intact; the
///   corresponding component(s) can be patched in place.
#[derive(Clone, Debug, Default)]
pub struct StageChange {
    pub resynced: Vec<String>,
    pub changed_info: Vec<String>,
}

impl StageChange {
    /// All paths mentioned by this change (resynced ∪ changed-info).
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.resynced.iter().chain(self.changed_info.iter())
    }
}

/// The live, editable USD stage and its change queue. **Non-send** — insert
/// via `world.insert_non_send(LiveStage::new(stage))`.
///
/// Authoring goes through `live.stage` (every method is `&self`); each commit
/// fires the installed sink, which records a [`StageChange`] onto the queue.
/// A reprojection system drains the queue once per frame.
pub struct LiveStage {
    pub stage: Stage,
    queue: Rc<RefCell<Vec<StageChange>>>,
    // Prim paths whose *next* change was caused by our own author-back and
    // should be swallowed once (the echo guard, PLAN P2). Author-back writes
    // the value the component already holds, so a re-project would be a no-op
    // — but skipping it avoids redundant work and any mid-edit churn.
    suppressed: Rc<RefCell<std::collections::HashSet<String>>>,
    // Kept so the sink lives as long as the stage; removed on drop.
    sink: Option<StageSinkId>,
}

impl LiveStage {
    /// Wrap a stage and install the change sink.
    pub fn new(stage: Stage) -> Self {
        let queue: Rc<RefCell<Vec<StageChange>>> = Rc::new(RefCell::new(Vec::new()));
        let q = queue.clone();
        let sink = stage.add_sink(move |_stage: &Stage, change: &CommittedChange<'_>| {
            q.borrow_mut().push(StageChange {
                resynced: change
                    .resynced
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
                changed_info: change
                    .changed_info_only
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
            });
        });
        Self {
            stage,
            queue,
            suppressed: Rc::new(RefCell::new(std::collections::HashSet::new())),
            sink: Some(sink),
        }
    }

    /// Take and clear all changes recorded since the last drain.
    pub fn drain_changes(&self) -> Vec<StageChange> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Whether any change is pending (cheap check before doing work).
    pub fn has_changes(&self) -> bool {
        !self.queue.borrow().is_empty()
    }

    /// Mark `prim` as self-authored: the next change mentioning it (fired by
    /// our own author-back) is swallowed by [`apply_changes`] rather than
    /// re-projected. Call immediately before authoring.
    pub fn mark_authored(&self, prim: impl Into<String>) {
        self.suppressed.borrow_mut().insert(prim.into());
    }

    /// Take and clear the set of self-authored prim paths.
    fn take_suppressed(&self) -> std::collections::HashSet<String> {
        std::mem::take(&mut *self.suppressed.borrow_mut())
    }

    /// Load `prim`'s payload (and everything beneath it). This is a composition
    /// change — it fires the change sink, so the next `apply_changes` reconciles
    /// and the newly-composed subtree is projected. The reversible counterpart
    /// of BSN's `queue_spawn_scene`.
    pub fn load_payload(&self, prim: &str) {
        if let Ok(p) = openusd::sdf::path(prim) {
            match self.stage.load(p, openusd::usd::LoadPolicy::WithDescendants) {
                Ok(()) => self.enqueue_resync(prim),
                Err(error) => warn!("failed to load payload {prim}: {error}"),
            }
        }
    }

    /// Unload `prim`'s payload — the projected subtree is despawned on the next
    /// `apply_changes` and the prim is marked
    /// [`UsdPayloadUnloaded`](crate::route::payload::UsdPayloadUnloaded).
    pub fn unload_payload(&self, prim: &str) {
        if let Ok(p) = openusd::sdf::path(prim) {
            match self.stage.unload(p) {
                Ok(()) => self.enqueue_resync(prim),
                Err(error) => warn!("failed to unload payload {prim}: {error}"),
            }
        }
    }

    /// Enqueue a `resynced` change for `prim`. openusd's `load`/`unload` change
    /// composition but do **not** fire the authoring change sink (they are
    /// stage load-rule changes, not layer-edit commits), so we synthesize the
    /// notice ourselves — the reconcile then materializes/despawns the subtree.
    pub(crate) fn enqueue_resync(&self, prim: &str) {
        self.queue.borrow_mut().push(StageChange {
            resynced: vec![prim.to_string()],
            changed_info: Vec::new(),
        });
    }
}

impl Drop for LiveStage {
    fn drop(&mut self) {
        if let Some(id) = self.sink.take() {
            self.stage.remove_sink(id);
        }
    }
}

/// Bidirectional `SdfPath ↔ Entity` index — the reprojection key. Plain
/// `Resource` (the paths are owned `String`s, the entities are ids).
#[derive(Resource, Default)]
pub struct PrimEntities {
    by_path: HashMap<String, Entity>,
    by_entity: HashMap<Entity, String>,
}

impl PrimEntities {
    pub fn insert(&mut self, path: impl Into<String>, entity: Entity) {
        let path = path.into();
        self.by_entity.insert(entity, path.clone());
        self.by_path.insert(path, entity);
    }

    pub fn entity(&self, path: &str) -> Option<Entity> {
        self.by_path.get(path).copied()
    }

    pub fn path(&self, entity: Entity) -> Option<&str> {
        self.by_entity.get(&entity).map(String::as_str)
    }

    /// Remove a path's mapping, returning the entity it pointed at.
    pub fn remove_path(&mut self, path: &str) -> Option<Entity> {
        let e = self.by_path.remove(path)?;
        self.by_entity.remove(&e);
        Some(e)
    }

    /// Remove an entity's mapping (e.g. on despawn).
    pub fn remove_entity(&mut self, entity: Entity) -> Option<String> {
        let p = self.by_entity.remove(&entity)?;
        self.by_path.remove(&p);
        Some(p)
    }

    /// Every `(path, entity)` currently mapped.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Entity)> {
        self.by_path.iter().map(|(p, e)| (p.as_str(), *e))
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Every `(path, entity)` whose path is `prefix` or a descendant of it —
    /// the set a `resynced` parent invalidates.
    pub fn subtree(&self, prefix: &str) -> Vec<(String, Entity)> {
        let with_slash = format!("{prefix}/");
        self.by_path
            .iter()
            .filter(|(p, _)| p.as_str() == prefix || p.starts_with(&with_slash))
            .map(|(p, e)| (p.clone(), *e))
            .collect()
    }
}

// ─── Projection + reprojection (v1: transforms) ─────────────────────
//
// Minimal slice of the project/sync loop: one entity per prim carrying
// `UsdPrimRef` + `Transform`. Mesh / material / the full field→component
// routing (RETHINK §12) layer on top of this same shape.

use crate::prim_ref::UsdPrimRef;
use crate::read::xform::read_transform;
use crate::route::{SchemaRegistry, StageTime};

/// Prim paths with animated inputs, revisited when [`StageTime`] changes.
/// Refreshed by projection and live change processing.
#[derive(Resource, Default, Clone)]
pub struct AnimatedPrims(pub std::collections::HashSet<String>);

/// The [`DisplayPurposes`] the projected entities were last filtered against,
/// so the purpose reprojector only reruns when the toggle actually changes.
#[derive(Resource, Default)]
struct AppliedPurposes(Option<crate::route::DisplayPurposes>);

/// The [`StageTime`] the projected entities were last sampled at, so the
/// resampler only reruns when the time actually moves.
#[derive(Resource, Default)]
struct SampledTime(Option<f64>);

fn has_time_samples(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    let Ok(prim) = stage.prim(path.clone()) else { return false };
    prim.authored_attributes()
        .map(|attrs| {
            attrs.iter().any(|a| {
                a.time_sample_times()
                    .map(|times| !times.is_empty())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn subsets_are_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    stage.prim(path.clone()).ok().and_then(|prim| prim.child_names().ok())
        .is_some_and(|names| names.iter().any(|name| {
            let Ok(child) = path.append_path(name.as_str()) else { return false };
            stage.prim(child.clone()).ok().and_then(|prim| prim.type_name().ok().flatten())
                .is_some_and(|name| name == "GeomSubset")
                && (has_time_samples(stage, &child)
                    || crate::read::shade::bound_material_is_time_varying(stage, &child))
        }))
}

fn inherited_primvar_is_animated(stage: &Stage, path: &openusd::sdf::Path, name: &str) -> bool {
    let Ok(owner) = crate::read::geom::inherited_primvar_owner(stage, path, name) else { return false };
    if owner == *path { return false; }
    let Ok(prim) = stage.prim(owner) else { return false };
    [name.to_owned(), format!("{name}:indices")].iter().any(|name|
        prim.attribute(name.as_str()).time_sample_times().is_ok_and(|times| !times.is_empty()))
}

/// Whether a prim, its deformation inputs or its point-instancer prototypes animate.
pub(crate) fn prim_is_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    let prototypes = stage.prim(path.clone()).ok().filter(|prim| {
        prim.type_name().ok().flatten().as_deref() == Some("PointInstancer")
    }).and_then(|prim| prim.relationship("prototypes").targets().ok())
        .is_some_and(|targets| targets.iter().any(|target| prototype_is_animated(stage, target)));
    prototypes || prim_inputs_are_animated(stage, path)
}

fn prim_inputs_are_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    has_time_samples(stage, path) || subsets_are_animated(stage, path) || crate::read::skel::deformation_is_time_varying(stage, path)
        || ["primvars:normals", "primvars:displayColor", "primvars:displayOpacity", "primvars:st", "primvars:st0"].iter()
            .any(|name| inherited_primvar_is_animated(stage, path, name))
        || crate::read::shade::bound_material_is_time_varying(stage, path)
}

fn prototype_is_animated(stage: &Stage, root: &openusd::sdf::Path) -> bool {
    let mut pending = vec![root.clone()];
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 4096 || prim_inputs_are_animated(stage, &path) { return true; }
        let Ok(prim) = stage.prim(path.clone()) else { continue };
        if let Ok(names) = prim.child_names() {
            pending.extend(names.iter().filter_map(|name| path.append_path(name.as_str()).ok()));
        }
    }
    false
}

fn to_bevy_transform(t: crate::read::xform::Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

/// Rotation mapping the stage's authored up-axis onto Bevy's Y-up world. USD
/// defaults to Y-up; Z-up content (common for robotics / CAD assets) is rotated
/// -90° about X so +Z becomes +Y. Applied once on the stage-root entity so the
/// whole composed scene stands upright on the ground grid.
pub(crate) fn stage_up_axis(stage: &Stage) -> Quat {
    let is_z = matches!(
        stage.stage_metadata("upAxis").ok().flatten(),
        Some(openusd::sdf::Value::Token(t)) if t.as_str() == "Z"
    );
    if is_z {
        Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    }
}

/// The namespace parent of a prim path — the pseudo-root `/` for a top-level
/// prim, so it parents onto the stage-root entity.
fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// The prim path owning a (possibly property) path: `/Foo.bar` → `/Foo`.
fn prim_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// The property part of a (possibly property) path: `/Foo.xformOp:x` →
/// `Some("xformOp:x")`; a bare prim path → `None`.
fn property_of(path: &str) -> Option<&str> {
    path.split_once('.').map(|(_, prop)| prop)
}

/// The traversal predicate for projection: active + defined + non-abstract, but
/// **not** requiring `LOADED` (unlike `PrimPredicate::default()`). This projects
/// a prim whose payload is *unloaded* as a placeholder (its payloaded children
/// stay absent until [`LiveStage::load_payload`]). For fully-loaded stages this
/// is identical to the default predicate.
fn traverse_predicate() -> openusd::usd::PrimPredicate {
    use openusd::usd::PrimStatus;
    openusd::usd::PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    ).with_instance_proxies(true)
}

/// Snapshot the registry out of the world (Arc-cheap `Clone`), falling back to
/// the built-in routes when none is installed — so direct `project_stage` /
/// `apply_changes` calls in tests work without wiring a registry.
fn registry_of(world: &World) -> SchemaRegistry {
    world
        .get_resource::<SchemaRegistry>()
        .cloned()
        .unwrap_or_else(SchemaRegistry::builtin)
}

/// Project every prim in the stage into an entity (`UsdPrimRef` +
/// `Transform`), recording the path↔entity bimap. Idempotent only on an
/// empty world — call once on load.
pub fn project_stage(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let registry = registry_of(world);
    // The stage-root entity (the pseudo-root `/`) carries the up-axis rotation;
    // every top-level prim hangs off it, so Bevy's transform propagation
    // composes prim-local transforms into correct world transforms and the
    // whole scene stands upright on the grid. It is not a real prim, so no
    // routes run on it (they would clobber the up-axis rotation).
    let root = world
        .spawn((
            UsdPrimRef {
                path: "/".to_string(),
            },
            Transform::from_rotation(stage_up_axis(stage)),
            Visibility::default(),
        ))
        .id();
    map.insert("/", root);

    let mut prim_count = 0usize;
    let mut animated: std::collections::HashSet<String> = std::collections::HashSet::new();
    let _ = stage.traverse(
        traverse_predicate(),
        |path: &openusd::sdf::Path| {
            // Traversal is pre-order, so the parent prim's entity already exists.
            let parent = map.entity(parent_path(path.as_str())).unwrap_or(root);
            let entity = world
                .spawn((
                    UsdPrimRef {
                        path: path.as_str().to_string(),
                    },
                    ChildOf(parent),
                ))
                .id();
            map.insert(path.as_str().to_string(), entity);
            prim_count += 1;
            if prim_is_animated(stage, path) {
                animated.insert(path.as_str().to_string());
            }
            // Every prim→component mapping goes through the registry.
            registry.project_prim(stage, path, world, entity);
        },
    );
    bevy::log::info!(
        target: "usd_bevy::live",
        "projected {prim_count} prims ({} animated)",
        animated.len()
    );
    world.insert_resource(AnimatedPrims(animated));
    // Projecting authored the initial read; clear so the first sync starts clean.
    let _ = live.drain_changes();
}

/// Project `stage` as a **static subtree** parented under `parent` — the asset
/// path (PLAN: USD as a Bevy asset). Unlike [`project_stage`] this doesn't touch
/// the live-session resources ([`PrimEntities`]/[`AnimatedPrims`]) or a change
/// stream: it's for spawning a loaded USD file as one instance, so many
/// instances can hang off different roots at once.
///
/// A stage-root child (carrying the up-axis rotation) is created under `parent`,
/// and every prim is projected beneath it through the same [`SchemaRegistry`]
/// the live path uses. Returns the local prim→entity map for the caller.
pub fn project_stage_under(world: &mut World, stage: &Stage, parent: Entity) -> PrimEntities {
    let registry = registry_of(world);
    let mut map = PrimEntities::default();
    // The stage-root carries the up-axis rotation; the caller's `parent` keeps
    // its own transform (placement of this instance).
    let root = world
        .spawn((
            UsdPrimRef {
                path: "/".to_string(),
            },
            Transform::from_rotation(stage_up_axis(stage)),
            Visibility::default(),
            ChildOf(parent),
        ))
        .id();
    map.insert("/", root);

    let _ = stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
        let parent = map.entity(parent_path(path.as_str())).unwrap_or(root);
        let entity = world
            .spawn((
                UsdPrimRef {
                    path: path.as_str().to_string(),
                },
                ChildOf(parent),
            ))
            .id();
        map.insert(path.as_str().to_string(), entity);
        registry.project_prim(stage, path, world, entity);
    });
    map
}

fn affects_projection_consumers(stage: &Stage, map: &PrimEntities, paths: &[&str]) -> bool {
    if paths.is_empty() { return false; }
    if paths.iter().any(|path| {
        let parent = parent_path(prim_of(path));
        let mesh_child = map.entity(parent).is_some() && stage.prim(parent).ok()
            .is_some_and(|prim| prim.type_name().ok().flatten().as_deref() == Some("Mesh")
                || ["points", "faceVertexCounts", "faceVertexIndices"].iter()
                    .all(|name| prim.attribute(*name).is_defined().unwrap_or(false)));
        mesh_child || property_of(path).is_some_and(|property| {
            property.starts_with("material:binding") || property.starts_with("collection:")
                || property == "primvars:normals" || property.starts_with("primvars:normals:")
                || property == "primvars:displayColor" || property.starts_with("primvars:displayColor:")
                || property == "primvars:displayOpacity" || property.starts_with("primvars:displayOpacity:")
                || property == "primvars:st" || property.starts_with("primvars:st:")
                || property == "primvars:st0" || property.starts_with("primvars:st0:")
        }) || stage.prim(prim_of(path)).ok()
            .and_then(|prim| prim.type_name().ok().flatten())
            .is_some_and(|name| matches!(name.as_str(), "Material" | "Shader" | "NodeGraph" | "GeomSubset" | "Skeleton" | "SkelAnimation" | "BlendShape"))
    }) {
        return true;
    }
    for (path, _) in map.iter() {
        let Ok(prim) = stage.prim(path) else { continue };
        if prim.type_name().ok().flatten().as_deref() != Some("PointInstancer") {
            continue;
        }
        let Ok(targets) = prim.relationship("prototypes").targets() else { continue };
        if targets.iter().any(|target| paths.iter().any(|changed| {
            let changed = prim_of(changed);
            let target = target.as_str();
            changed == target || changed.strip_prefix(target).is_some_and(|rest| rest.starts_with('/'))
                || target.strip_prefix(changed).is_some_and(|rest| rest.starts_with('/'))
        })) {
            return true;
        }
    }
    false
}

/// Drain the change queue and reproject affected entities.
///
/// * Any `resynced` change → reconcile the entity set against the stage
///   (spawn entities for new prims, despawn entities for removed prims,
///   patch the rest). v1 reconciles the whole stage; a later version scopes
///   to the resynced subtree.
/// * Material graph, binding, collection or prototype changes → reconcile consumers.
/// * Other `changed_info` changes → patch the touched prims in place.
pub fn apply_changes(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let mut changes = live.drain_changes();
    if changes.is_empty() {
        return;
    }
    let suppressed = live.take_suppressed();
    for change in &mut changes {
        change.resynced.retain(|path| {
            property_of(path).is_none() || !suppressed.contains(prim_of(path))
        });
    }
    if changes.iter().any(|c| !c.resynced.is_empty()) {
        reconcile(world, live, map, true);
        return;
    }
    let changed_paths: Vec<_> = changes.iter().flat_map(|change| change.changed_info.iter())
        .filter(|path| !suppressed.contains(prim_of(path))).map(String::as_str).collect();
    if affects_projection_consumers(&live.stage, map, &changed_paths) {
        reconcile(world, live, map, true);
        return;
    }
    // `changed_info` only: group the changed *property* paths by owning prim so
    // each route sees exactly which properties changed and can patch sparsely.
    let registry = registry_of(world);
    // Echo guard: prims we just authored ourselves are swallowed this round.
    let mut by_prim: HashMap<String, Vec<String>> = HashMap::new();
    for change in &changes {
        for path in change.paths() {
            let prim = prim_of(path).to_string();
            let entry = by_prim.entry(prim).or_default();
            if let Some(prop) = property_of(path) {
                entry.push(prop.to_string());
            }
        }
    }
    for (prim, props) in by_prim {
        if suppressed.contains(&prim) {
            continue;
        }
        let Some(entity) = map.entity(&prim) else {
            continue;
        };
        let Ok(p) = openusd::sdf::path(&prim) else {
            continue;
        };
        let prop_refs: Vec<&str> = props.iter().map(String::as_str).collect();
        registry.patch_prim(&live.stage, &p, world, entity, &prop_refs);
        if world.contains_resource::<AnimatedPrims>() {
            let animated = prim_is_animated(&live.stage, &p);
            if let Some(mut index) = world.get_resource_mut::<AnimatedPrims>() {
                if animated { index.0.insert(prim); } else { index.0.remove(&prim); }
            }
        }
    }
}

/// Updates the projected prim paths after a successful editor namespace edit.
pub(crate) fn remap_namespace(world: &mut World, old: &str, new: &str) {
    let Some(mut map) = world.remove_resource::<PrimEntities>() else { return };
    let moved: Vec<_> = map.iter().filter_map(|(path, entity)| {
        let suffix = path.strip_prefix(old)?;
        (suffix.is_empty() || suffix.starts_with('/')).then(|| (path.to_string(), format!("{new}{suffix}"), entity))
    }).collect();
    for (path, _, _) in &moved { map.remove_path(path); }
    for (_, path, entity) in &moved {
        if let Ok(mut entity_mut) = world.get_entity_mut(*entity) {
            entity_mut.insert(UsdPrimRef::new(path));
            map.insert(path.clone(), *entity);
        }
    }
    for (_, path, entity) in moved {
        if let Some(parent) = map.entity(parent_path(&path)).or_else(|| map.entity("/")) {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) { entity_mut.insert(ChildOf(parent)); }
        }
    }
    world.insert_resource(map);
}

/// Reconciles paths, hierarchy and routed components, optionally rebuilding the live animation index.
pub(crate) fn reconcile(world: &mut World, live: &LiveStage, map: &mut PrimEntities, collect_animation: bool) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let mut current: std::collections::HashSet<String> = std::collections::HashSet::new();
    let traversal = stage.traverse(
        traverse_predicate(),
        |p: &openusd::sdf::Path| {
            current.insert(p.as_str().to_string());
        },
    );
    if let Err(error) = traversal {
        bevy::log::warn!("USD reconciliation skipped: {error}");
        return;
    }

    // Despawn entities for prims no longer present (never the `/` stage root).
    let stale: Vec<(String, Entity)> = map
        .iter()
        .filter(|(p, _)| *p != "/" && !current.contains(*p))
        .map(|(p, e)| (p.to_string(), e))
        .collect();
    for (path, entity) in stale {
        world.despawn(entity);
        map.remove_path(&path);
    }

    // Spawn new prims (shallowest first, so a child finds its parent) and
    // reapply routes on existing ones. Both go through the registry: new prims
    // get a full `project_prim`; existing prims get a full re-patch (empty
    // `changed` = "reapply everything", the conservative choice on a resync).
    let root = map.entity("/");
    let mut ordered: Vec<&String> = current.iter().collect();
    ordered.sort_by_key(|p| p.matches('/').count());
    let mut animated: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in ordered {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        if collect_animation && prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        if let Some(entity) = map.entity(path).filter(|entity| world.get_entity(*entity).is_ok()) {
            if let Some(parent) = map.entity(parent_path(path)).or(root) {
                if world.get::<ChildOf>(entity).map(ChildOf::parent) != Some(parent) {
                    world.entity_mut(entity).insert(ChildOf(parent));
                }
            }
            registry.patch_prim(stage, &p, world, entity, &[]);
        } else {
            map.remove_path(path);
            let parent = map.entity(parent_path(path)).or(root);
            let mut e = world.spawn(UsdPrimRef {
                path: path.clone(),
            });
            if let Some(parent) = parent {
                e.insert(ChildOf(parent));
            }
            let entity = e.id();
            map.insert(path.clone(), entity);
            registry.project_prim(stage, &p, world, entity);
        }
    }
    // Refresh the animated set for the reconciled prim set.
    if collect_animation { world.insert_resource(AnimatedPrims(animated)); }
}

// ─── Bevy plugin + systems ──────────────────────────────────────────
//
// `LiveStage` is `!Send`, and `apply_changes`/`project_stage` need `&mut
// World` (to spawn/despawn) plus `&LiveStage` plus `&mut PrimEntities` at
// once — which would alias `World`. So the exclusive systems below
// temporarily *remove* the live stage + bimap from the world, run, and
// re-insert. An app does: `app.add_plugins(LiveStagePlugin)` then
// `world.insert_non_send(LiveStage::new(stage))` to start a session.

use bevy::app::{App, Plugin, Update};

/// Registers the `PrimEntities` bimap and the per-frame reprojection system.
/// Insert a `LiveStage` non-send resource to begin a live session.
pub struct LiveStagePlugin;

#[derive(Resource, Default)]
struct AppliedSubdivision(Option<u32>);

#[derive(Resource, Default)]
struct AppliedCurveSteps((usize, Option<usize>));

impl Plugin for LiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrimEntities>()
            .init_resource::<StageTime>()
            .init_resource::<AnimatedPrims>()
            .init_resource::<SampledTime>()
            .init_resource::<crate::route::DisplayPurposes>()
            .init_resource::<AppliedPurposes>()
            .init_resource::<AppliedSubdivision>()
            .init_resource::<AppliedCurveSteps>()
            .add_systems(
                Update,
                (
                    project_on_load_system,
                    reproject_system,
                    apply_subdivision_settings_system,
                    apply_curve_settings_system,
                    resample_animation_system,
                    apply_display_purposes_system,
                )
                    .chain(),
            );
        // Ensure the routing registry exists even if `UsdPlugin` wasn't added.
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
    }
}

/// One-shot projection the first frame a `LiveStage` is present.
fn project_on_load_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    // Only project once per session: skip if the bimap is already populated.
    if !world.resource::<PrimEntities>().is_empty() {
        return;
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let purposes = world.get_resource::<crate::route::DisplayPurposes>().copied();
    let time = world.get_resource::<StageTime>().map(|time| time.current);
    project_stage(world, &live, &mut map);
    world.resource_mut::<AppliedPurposes>().0 = purposes;
    world.resource_mut::<SampledTime>().0 = time;
    let subdivision_levels = crate::route::subdivision::current_levels(world);
    world.resource_mut::<AppliedSubdivision>().0 = subdivision_levels;
    world.resource_mut::<AppliedCurveSteps>().0 = crate::route::curves::current_geometry_key(world);
    world.insert_resource(map);
    world.insert_non_send(live);
}

fn apply_subdivision_settings_system(world: &mut World) {
    let current = crate::route::subdivision::current_levels(world);
    if world.resource::<AppliedSubdivision>().0 == current { return; }
    let Some(live) = world.remove_non_send::<LiveStage>() else { return };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    crate::route::subdivision::refresh_geometry(world, &live.stage, &map);
    world.insert_resource(map);
    world.insert_non_send(live);
    world.resource_mut::<AppliedSubdivision>().0 = current;
}

fn apply_curve_settings_system(world: &mut World) {
    let current = crate::route::curves::current_geometry_key(world);
    if world.resource::<AppliedCurveSteps>().0 == current { return; }
    let Some(live) = world.remove_non_send::<LiveStage>() else { return };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    crate::route::curves::refresh_geometry(world, &live.stage, &map);
    world.insert_resource(map);
    world.insert_non_send(live);
    world.resource_mut::<AppliedCurveSteps>().0 = current;
}

/// Resample animated prims when [`StageTime`] moves. Only revisits the prims
/// that actually carry time samples ([`AnimatedPrims`]), re-patching them at
/// the new time (the routes read `StageTime` when resolving values).
fn resample_animation_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world.get_resource::<StageTime>().map(|t| t.current);
    let last = world.get_resource::<SampledTime>().and_then(|t| t.0);
    if current == last {
        return; // time hasn't moved
    }
    let animated = world.get_resource::<AnimatedPrims>().cloned().unwrap_or_default();
    if animated.0.is_empty() {
        // Nothing animated; still record the time so we don't re-check.
        if let Some(mut sampled) = world.get_resource_mut::<SampledTime>() {
            sampled.0 = current;
        }
        return;
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let registry = registry_of(world);
    for path in &animated.0 {
        if let Some(entity) = map.entity(path)
            && let Ok(p) = openusd::sdf::path(path)
        {
            // patch_prim resolves values at the world's StageTime.
            registry.patch_prim(&live.stage, &p, world, entity, &[]);
        }
    }
    world.insert_resource(map);
    world.insert_non_send(live);
    if let Some(mut sampled) = world.get_resource_mut::<SampledTime>() {
        sampled.0 = current;
    }
}

/// Re-filter every prim's visibility when [`DisplayPurposes`] changes (a
/// viewport toggling proxy↔render, or revealing guides). Purpose is inherited,
/// so a toggle can flip any prim — re-patch them all with a synthetic `purpose`
/// change; routes that don't own `purpose` ignore it.
fn apply_display_purposes_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world.get_resource::<crate::route::DisplayPurposes>().copied();
    let last = world.get_resource::<AppliedPurposes>().and_then(|a| a.0);
    if current == last {
        return; // toggle hasn't moved
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let registry = registry_of(world);
    let entries: Vec<(String, Entity)> =
        map.iter().map(|(p, e)| (p.to_string(), e)).collect();
    for (path, entity) in entries {
        if let Ok(p) = openusd::sdf::path(&path) {
            registry.patch_prim(&live.stage, &p, world, entity, &["purpose"]);
        }
    }
    world.insert_resource(map);
    world.insert_non_send(live);
    if let Some(mut applied) = world.get_resource_mut::<AppliedPurposes>() {
        applied.0 = current;
    }
}

/// Drain the live stage's change queue and reproject affected entities.
fn reproject_system(world: &mut World) {
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    if live.has_changes() {
        let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
        apply_changes(world, &live, &mut map);
        world.insert_resource(map);
    }
    world.insert_non_send(live);
}

// ─── Authoring back (entity edit → stage) ───────────────────────────
//
// The write direction: an entity's `Transform` (e.g. after a gizmo drag)
// authored back onto the prim as a single `xformOp:transform` matrix under
// the stage's current edit target. The commit fires the sink, so the edit
// re-projects like any other change (idempotent — the entity already holds
// the value). Authoring one matrix op (instead of decomposed T/R/S) keeps a
// clean round-trip with `read_transform`.

/// Author `transform` onto `prim_path` as `xformOp:transform`. Errors if the
/// path is malformed or the layer rejects the edit.
pub fn author_transform(
    stage: &Stage,
    prim_path: &str,
    transform: &Transform,
) -> anyhow::Result<()> {
    use openusd::sdf::Value;
    let prim = openusd::sdf::path(prim_path)?;
    let cols = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
    .to_cols_array();
    let m: [f64; 16] = std::array::from_fn(|i| cols[i] as f64);

    let xop = prim.append_property("xformOp:transform")?;
    stage
        .create_attribute(xop, "matrix4d")?
        .set(Value::Matrix4d(openusd::gf::Matrix4d(m)))?;
    let order = prim.append_property("xformOpOrder")?;
    stage
        .create_attribute(order, "token[]")?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

/// Current authored transform of a prim, if any.
pub fn current_transform(stage: &Stage, prim_path: &str) -> Option<Transform> {
    openusd::sdf::path(prim_path)
        .ok()
        .and_then(|p| read_transform(stage, &p).ok().flatten())
        .map(to_bevy_transform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_projection_records_purpose_and_sample_without_repeating_routes() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Cube "Box" {
    uniform token purpose = "guide"
    double size.timeSamples = {0: 2, 10: 4}
}
"#).open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::UsdPlugin, LiveStagePlugin));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
        app.init_resource::<crate::route::ProjectionTimings>();
        app.world_mut().resource_mut::<StageTime>().current = 5.0;
        app.insert_non_send(LiveStage::new(stage));
        let projections = |world: &World| world.resource::<crate::route::ProjectionTimings>().0
            .get(std::any::type_name::<crate::route::shapes::ShapesRoute>()).unwrap().matches;
        app.update();
        let entity = app.world().resource::<PrimEntities>().entity("/Box").unwrap();
        let half_width = |world: &World| {
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            points.iter().map(|point| point[0]).fold(f32::NEG_INFINITY, f32::max)
        };
        assert_eq!(projections(app.world()), 1);
        assert_eq!(half_width(app.world()), 1.5);
        assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        app.update();
        assert_eq!(projections(app.world()), 1);
        app.world_mut().resource_mut::<StageTime>().current = 10.0;
        app.update();
        assert_eq!(projections(app.world()), 2);
        assert_eq!(half_width(app.world()), 2.0);
        app.world_mut().resource_mut::<crate::route::DisplayPurposes>().guide = true;
        app.update();
        assert_eq!(projections(app.world()), 3);
        assert_ne!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Box"), Some(entity));
    }

    #[test]
    fn shader_connection_edits_refresh_live_animation_membership() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Cube "Box" { rel material:binding = </Mat> }
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:roughness = 0.5
        token outputs:surface
    }
    def Shader "Animated" {
        uniform token info:id = "ND_constant_float"
        float inputs:value.timeSamples = {0: 0.2, 10: 0.8}
        float outputs:out
    }
}
"#).open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::UsdPlugin, LiveStagePlugin));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
        app.insert_non_send(LiveStage::new(stage.clone()));
        app.update();
        let entity = app.world().resource::<PrimEntities>().entity("/Box").unwrap();
        app.world_mut().entity_mut(entity).insert(Name::new("runtime annotation"));
        assert!(!app.world().resource::<AnimatedPrims>().0.contains("/Box"));
        let roughness = |world: &World| {
            let handle = &world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0;
            world.resource::<Assets<StandardMaterial>>().get(handle).unwrap().perceptual_roughness
        };
        assert_eq!(roughness(app.world()), 0.5);
        let input = stage.prim("/Mat/Surface").unwrap().attribute("inputs:roughness");
        let input = input.set_connections([openusd::sdf::path("/Mat/Animated.outputs:out").unwrap()]).unwrap();
        app.update();
        assert!(app.world().resource::<AnimatedPrims>().0.contains("/Box"));
        for (current, expected) in [(0.0, 0.2), (10.0, 0.8)] {
            app.world_mut().resource_mut::<StageTime>().current = current;
            app.update();
            assert!((roughness(app.world()) - expected).abs() < 1e-6);
            assert_eq!(app.world().resource::<PrimEntities>().entity("/Box"), Some(entity));
            assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime annotation");
        }
        input.set_connections(std::iter::empty::<openusd::sdf::Path>()).unwrap();
        app.update();
        assert!(!app.world().resource::<AnimatedPrims>().0.contains("/Box"));
        assert_eq!(roughness(app.world()), 0.5);
        app.world_mut().resource_mut::<StageTime>().current = 0.0;
        app.update();
        assert_eq!(roughness(app.world()), 0.5);
        assert_eq!(app.world().resource::<PrimEntities>().entity("/Box"), Some(entity));
        assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime annotation");
    }

    #[test]
    fn combined_deformation_animation_scan_matches_individual_queries() {
        for fixture in ["skel_test_simple.usda", "blendshape_test.usda", "point_shapes.usda", "skel_morph_normals.usda"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets").join(fixture);
            let source = crate::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap();
            let stage = source.open_stage().unwrap();
            let mut pending = stage.prim("/").unwrap().children().unwrap();
            while let Some(prim) = pending.pop() {
                pending.extend(prim.children().unwrap());
                assert_eq!(crate::read::skel::deformation_is_time_varying(&stage, prim.path()),
                    crate::read::skel::skin_is_time_varying(&stage, prim.path())
                        || crate::read::skel::blend_is_time_varying(&stage, prim.path()), "{fixture}: {}", prim.path());
            }
        }
    }

    #[test]
    fn authored_animation_scan_matches_composed_scan_for_proxies_and_edits() {
        let source = crate::UsdSource::new("animation-scan.usda", br#"#usda 1.0
def Cube "Static" {}
def Xform "Template" {
    def Cube "Animated" { double size.timeSamples = {0: 1, 10: 2} }
}
def Xform "Instance" (
    instanceable = true
    prepend references = </Template>
) {}
def Camera "Camera" { float focalLength.timeSamples = {0: 20, 10: 40} }
def Xform "Custom" { custom float bevy:Speed:value.timeSamples = {0: 1, 10: 3} }
"#.as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let static_prim = stage.prim("/Static").unwrap();
        assert!(static_prim.authored_attributes().unwrap().is_empty());
        assert!(!static_prim.attributes().unwrap().is_empty());
        for edited in [false, true] {
            if edited { stage.attribute("/Template/Animated.size").unwrap().clear().unwrap(); }
            for (path, expected) in [("/Static", false), ("/Template/Animated", !edited),
                ("/Instance/Animated", !edited), ("/Camera", true), ("/Custom", true)] {
                let path = openusd::sdf::path(path).unwrap();
                let old_scan = stage.prim(path.clone()).unwrap().attributes().unwrap().iter()
                    .any(|attribute| !attribute.time_sample_times().unwrap().is_empty());
                assert_eq!(has_time_samples(&stage, &path), expected, "{path}");
                assert_eq!(old_scan, expected, "{path}");
            }
        }
    }

    #[test]
    fn consumer_invalidation_distinguishes_namespace_boundaries() {
        let source = crate::UsdSource::new("dependencies.usda", &br#"#usda 1.0
def PointInstancer "PI" { rel prototypes = [</Group/Proto>] }
def Xform "Group" {
    def Mesh "Proto" {}
    def Mesh "PrototypeSibling" {}
}
def Shader "Shader" {}
def NodeGraph "Graph" {}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let mut world = World::new();
        let mut map = PrimEntities::default();
        map.insert("/PI", world.spawn_empty().id());
        for path in ["/Group/Proto.points", "/Group/Proto/Child.points", "/Group.visibility",
            "/Shader.inputs:roughness", "/Graph.inputs:weight", "/Group.material:binding",
            "/Group.collection:binding:includes"] {
            assert!(affects_projection_consumers(&stage, &map, &[path]), "{path}");
        }
        for path in ["/Group/PrototypeSibling.points", "/Elsewhere.visibility", "/PI.positions"] {
            assert!(!affects_projection_consumers(&stage, &map, &[path]), "{path}");
        }
        assert!(!affects_projection_consumers(&stage, &map, &[]));
    }

    /// The asset-spawn engine: projecting a stage under a given parent produces
    /// a parented subtree (stage-root `/` → prims), with routes applied, without
    /// touching the live-session resources.
    #[test]
    fn project_stage_under_parents_a_subtree() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("under.usda").unwrap();
        stage.define_prim("/Root").unwrap().set_type_name("Xform").unwrap();
        stage.define_prim("/Root/Child").unwrap().set_type_name("Xform").unwrap();

        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let parent = world.spawn(Transform::default()).id();
        let map = project_stage_under(&mut world, &stage, parent);

        let sroot = map.entity("/").expect("stage root");
        let root = map.entity("/Root").expect("/Root");
        let child = map.entity("/Root/Child").expect("/Root/Child");

        assert_eq!(
            world.get::<ChildOf>(sroot).map(|c| c.parent()),
            Some(parent),
            "stage-root hangs off the caller's parent"
        );
        assert_eq!(world.get::<ChildOf>(root).map(|c| c.parent()), Some(sroot));
        assert_eq!(world.get::<ChildOf>(child).map(|c| c.parent()), Some(root));
        // A route ran: the Xform prims carry a Transform.
        assert!(world.get::<Transform>(root).is_some(), "xform route applied");
        // The live-session resource is untouched by the asset path.
        assert!(world.get_resource::<PrimEntities>().is_none());
    }

    /// Kitchen_set.usdz's root layer is `Kitchen_set.usd`, so this exercises
    /// the openusd USDZ `.usd`-layer content-sniff fix (without it the stage
    /// won't even open). NOTE: its geometry is behind references to other
    /// files *inside* the usdz, which openusd doesn't resolve yet — so the
    /// composed stage is structure-only (0 meshes). See OPENUSD_ISSUE.md.
    #[test]
    fn loads_kitchen_usdz() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/external/Kitchen_set.usdz"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("Kitchen_set.usdz should open");
        let mut meshes = 0usize;
        let _ = stage.traverse(
            openusd::usd::PrimPredicate::default(),
            |p: &openusd::sdf::Path| {
                if let Ok(Some(m)) = crate::read::geom::read_mesh(&stage, p)
                    && !m.points.is_empty()
                {
                    meshes += 1;
                }
            },
        );
        // Kitchen's geometry is behind references to other files *inside* the
        // usdz; this asserts the openusd package-resolution fix resolves them.
        println!("KITCHEN meshes: {meshes}");
        assert!(
            meshes > 1000,
            "kitchen packaged meshes should resolve, got {meshes}"
        );
    }
    use openusd::sdf::Value;

    fn tx(stage: &Stage, prim: &str) -> Option<Vec3> {
        current_transform(stage, prim).map(|t| t.translation)
    }

    /// The `UsdNotice` loop: authoring an edit fires the sink, and the change
    /// (mentioning the edited path) lands on the drainable queue. This is the
    /// foundation the whole live-editor reprojection is built on.
    #[test]
    fn sink_records_authored_edits() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .in_memory("live_test.usda")
            .expect("in-memory stage");
        stage
            .define_prim("/Foo")
            .expect("define")
            .set_type_name("Xform")
            .expect("type");

        let live = LiveStage::new(stage);
        assert!(!live.has_changes(), "no changes before any edit");

        // Author an attribute on /Foo — this commits and should fire the sink.
        live.stage
            .create_attribute("/Foo.size", "double")
            .expect("create attr")
            .set(Value::Double(2.0))
            .expect("set value");

        let changes = live.drain_changes();
        assert!(
            !changes.is_empty(),
            "the edit should have recorded a change"
        );
        let mentioned: Vec<String> = changes.iter().flat_map(|c| c.paths().cloned()).collect();
        assert!(
            mentioned.iter().any(|p| p.starts_with("/Foo")),
            "change should mention /Foo, got {mentioned:?}"
        );
        assert!(
            live.drain_changes().is_empty(),
            "queue is empty after drain"
        );
    }

    /// Namespace edits (define/remove) surface as `resynced` — the signal to
    /// reproject a subtree.
    #[test]
    fn define_and_remove_resync() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("resync.usda").unwrap();
        let live = LiveStage::new(stage);

        live.stage.define_prim("/World").unwrap();
        live.stage.define_prim("/World/Child").unwrap();
        let after_define = live.drain_changes();
        let resynced: Vec<String> = after_define
            .iter()
            .flat_map(|c| c.resynced.clone())
            .collect();
        assert!(
            resynced.iter().any(|p| p.starts_with("/World")),
            "defining prims should resync /World, got resynced={resynced:?}"
        );

        live.stage.remove_prim("/World/Child").unwrap();
        let after_remove = live.drain_changes();
        assert!(
            !after_remove.is_empty(),
            "removing a prim should record a change"
        );
    }

    /// The full loop: project a prim's transform into an entity, author a
    /// new translate on the stage, sync, and confirm the entity's
    /// `Transform` was reprojected from the edit.
    #[test]
    fn edit_reprojects_transform() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("e2e.usda").unwrap();
        stage
            .define_prim("/Foo")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        stage
            .create_attribute("/Foo.xformOp:translate", "double3")
            .unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([1.0, 0.0, 0.0])))
            .unwrap();
        stage
            .create_attribute("/Foo.xformOpOrder", "token[]")
            .unwrap()
            .set(Value::TokenVec(vec!["xformOp:translate".into()]))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let foo = map.entity("/Foo").expect("/Foo projected");
        assert_eq!(
            world.get::<Transform>(foo).unwrap().translation,
            Vec3::new(1.0, 0.0, 0.0),
            "initial projection reads the authored translate"
        );

        // Author a new translate; the sink records it; sync reprojects.
        live.stage
            .attribute("/Foo.xformOp:translate").unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([2.0, 5.0, 0.0])))
            .unwrap();
        assert!(live.has_changes(), "the edit fired the sink");
        apply_changes(&mut world, &live, &mut map);

        assert_eq!(
            world.get::<Transform>(foo).unwrap().translation,
            Vec3::new(2.0, 5.0, 0.0),
            "sync reprojected the edited transform onto the entity"
        );
    }

    /// Namespace edits reconcile the entity set: a new prim spawns an
    /// entity, a removed prim despawns it.
    #[test]
    fn resync_spawns_and_despawns_entities() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("rs.usda").unwrap();
        stage.define_prim("/World").unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let base = map.len();

        live.stage.define_prim("/World/NewChild").unwrap();
        apply_changes(&mut world, &live, &mut map);
        let child = map.entity("/World/NewChild").expect("new prim projected");
        assert_eq!(map.len(), base + 1);
        assert!(world.get_entity(child).is_ok(), "child entity exists");

        live.stage.remove_prim("/World/NewChild").unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(
            map.entity("/World/NewChild").is_none(),
            "removed prim despawned"
        );
        assert_eq!(map.len(), base);
        assert!(world.get_entity(child).is_err(), "child entity despawned");
    }

    /// The write path: authoring an entity transform back onto the prim
    /// round-trips through `read_transform`, and fires the sink.
    #[test]
    fn author_transform_roundtrips_and_notifies() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("auth.usda").unwrap();
        stage
            .define_prim("/Foo")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = LiveStage::new(stage);

        let t = Transform::from_xyz(3.0, 4.0, 5.0).with_scale(Vec3::splat(2.0));
        author_transform(&live.stage, "/Foo", &t).unwrap();
        assert!(live.has_changes(), "authoring fires the sink");

        let read = read_transform(&live.stage, &openusd::sdf::path("/Foo").unwrap())
            .unwrap()
            .expect("transform authored");
        let back = to_bevy_transform(read);
        assert!(
            (back.translation - Vec3::new(3.0, 4.0, 5.0)).length() < 1e-4,
            "translation round-trips, got {:?}",
            back.translation
        );
        assert!(
            (back.scale - Vec3::splat(2.0)).length() < 1e-4,
            "scale round-trips, got {:?}",
            back.scale
        );
    }

    #[test]
    fn transform_undo_redo() {
        use crate::editor::{EditorEdit, EditorSession};
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("undo.usda").unwrap();
        stage
            .define_prim("/Foo")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut hist = EditorSession::new(stage.clone());
        for x in [1.0, 2.0] {
            hist.edit(EditorEdit::TransformMatrix {
                prim: "/Foo".into(),
                matrix: bevy::math::DMat4::from_translation(bevy::math::DVec3::new(x, 0.0, 0.0)).to_cols_array(),
                reset: false,
            }).unwrap();
        }
        let after = stage.root_layer().export_to_string().unwrap();
        assert_eq!(tx(&stage, "/Foo"), Some(Vec3::new(2.0, 0.0, 0.0)));

        assert!(hist.undo().unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            "undo → previous"
        );
        assert!(hist.undo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
        assert_eq!(
            tx(&stage, "/Foo"),
            None,
            "undo past the first edit clears the transform"
        );
        assert!(!hist.undo().unwrap(), "nothing left to undo");

        assert!(hist.redo().unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            "redo → first edit"
        );
        assert!(hist.redo().unwrap());
        assert_eq!(stage.root_layer().export_to_string().unwrap(), after);
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(2.0, 0.0, 0.0)),
            "redo → second edit"
        );
        assert!(!hist.redo().unwrap(), "nothing left to redo");
    }

    /// The plugin wires it together: projecting on load and reprojecting on
    /// edit, run through a real Bevy `Update` schedule.
    #[test]
    fn plugin_projects_and_reprojects() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("app.usda").unwrap();
        stage
            .define_prim("/World")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = LiveStage::new(stage);

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(live);

        app.world_mut().run_schedule(Update);
        assert!(
            app.world()
                .resource::<PrimEntities>()
                .entity("/World")
                .is_some(),
            "projected on load"
        );

        // Author a new prim on the stage; next update reprojects it.
        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .stage
            .define_prim("/World/Child")
            .unwrap();
        app.world_mut().run_schedule(Update);
        assert!(
            app.world()
                .resource::<PrimEntities>()
                .entity("/World/Child")
                .is_some(),
            "reprojected the new prim through the schedule"
        );
    }

    /// Visibility routes like transforms: authoring `visibility = invisible`
    /// reprojects the entity's `Visibility` to `Hidden`.
    #[test]
    fn edit_reprojects_visibility() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("vis.usda").unwrap();
        stage
            .define_prim("/Foo")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let foo = map.entity("/Foo").unwrap();
        assert_eq!(
            *world.get::<Visibility>(foo).unwrap(),
            Visibility::Inherited
        );

        live.stage
            .create_attribute("/Foo.visibility", "token")
            .unwrap()
            .set(Value::Token("invisible".into()))
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(
            *world.get::<Visibility>(foo).unwrap(),
            Visibility::Hidden,
            "visibility=invisible reprojected to Hidden"
        );
    }

    /// Open a real `.usda` from disk and project it — the full load path the
    /// viewer uses, minus the GPU.
    #[test]
    fn project_real_usda_file() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/two_xforms.usda");
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open two_xforms.usda");
        let live = LiveStage::new(stage);
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        for p in ["/World", "/World/ChildA", "/World/ChildB"] {
            assert!(map.entity(p).is_some(), "{p} should project to an entity");
        }
    }

    /// With the render `Assets` present, mesh prims project `Mesh3d` —
    /// the geometry the viewer renders.
    #[test]
    fn project_mesh_attaches_render_components() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_test_simple.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open skel_test_simple.usda");
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let mut q = world.query::<&Mesh3d>();
        let mesh_count = q.iter(&world).count();
        assert!(
            mesh_count > 0,
            "at least one mesh prim should project a Mesh3d"
        );
    }

    // ─── Reflect route (PLAN P1.3): arbitrary components from USD ────────

    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::reflect::TypeRegistry;

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Health {
        current: f64,
        max: f64,
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Boss {
        enraged: bool,
        rage: f32,
    }

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    enum MotionState {
        #[default]
        Idle,
        Moving(f32),
        Warp { x: i32, y: i32 },
    }

    #[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
    #[reflect(Component, Default)]
    struct Motion {
        state: MotionState,
    }

    /// A `World` with an `AppTypeRegistry` (populated by `register`) and the
    /// built-in `SchemaRegistry`, ready for `project_stage`.
    fn reflect_world(register: impl FnOnce(&mut TypeRegistry)) -> World {
        let mut world = World::new();
        let type_registry = AppTypeRegistry::default();
        register(&mut type_registry.write());
        world.insert_resource(type_registry);
        world.insert_resource(SchemaRegistry::builtin());
        world
    }

    fn author_double(stage: &Stage, attr: &str, v: f64) {
        stage
            .create_attribute(attr, "double")
            .unwrap()
            .set(Value::Double(v))
            .unwrap();
    }

    /// Project a prim carrying `bevy:` attributes → the reflect route builds
    /// and populates the matching component. This is the importer→scene-system
    /// line: an arbitrary gameplay component authored purely in USD.
    #[test]
    fn reflect_route_projects_component() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("reflect.usda").unwrap();
        stage
            .define_prim("/Enemy")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        author_double(&stage, "/Enemy.bevy:Health:current", 50.0);
        author_double(&stage, "/Enemy.bevy:Health:max", 100.0);

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Enemy").unwrap();
        assert_eq!(
            world.get::<Health>(e),
            Some(&Health {
                current: 50.0,
                max: 100.0
            }),
            "both authored fields projected onto the component"
        );
    }

    /// A data-carrying enum field (PLAN 4c): a token selects the variant and
    /// sibling `:_0` / `:name` attributes fill its tuple / struct payload.
    #[test]
    fn reflect_route_data_enum_variants() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("enum.usda").unwrap();
        stage.define_prim("/Mover").unwrap().set_type_name("Xform").unwrap();
        stage.define_prim("/Warper").unwrap().set_type_name("Xform").unwrap();
        // Tuple variant `Moving(f32)`.
        stage
            .create_attribute("/Mover.bevy:Motion:state", "token")
            .unwrap()
            .set(Value::Token("Moving".into()))
            .unwrap();
        stage
            .create_attribute("/Mover.bevy:Motion:state:_0", "float")
            .unwrap()
            .set(Value::Float(5.0))
            .unwrap();
        // Struct variant `Warp { x, y }`.
        stage
            .create_attribute("/Warper.bevy:Motion:state", "token")
            .unwrap()
            .set(Value::Token("Warp".into()))
            .unwrap();
        stage
            .create_attribute("/Warper.bevy:Motion:state:x", "int")
            .unwrap()
            .set(Value::Int(3))
            .unwrap();
        stage
            .create_attribute("/Warper.bevy:Motion:state:y", "int")
            .unwrap()
            .set(Value::Int(7))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Motion>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        assert_eq!(
            world.get::<Motion>(map.entity("/Mover").unwrap()).unwrap().state,
            MotionState::Moving(5.0),
            "tuple-variant payload authored from :_0"
        );
        assert_eq!(
            world.get::<Motion>(map.entity("/Warper").unwrap()).unwrap().state,
            MotionState::Warp { x: 3, y: 7 },
            "struct-variant payload authored from :x/:y"
        );
    }

    /// A `changed_info` edit to one `bevy:` field patches *only* that field —
    /// BSN's `TemplatePatch` semantics, driven by USD. The untouched field
    /// keeps its value.
    #[test]
    fn reflect_route_sparse_patch() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("sparse.usda").unwrap();
        stage.define_prim("/Enemy").unwrap();
        author_double(&stage, "/Enemy.bevy:Health:current", 50.0);
        author_double(&stage, "/Enemy.bevy:Health:max", 100.0);

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Enemy").unwrap();

        // Change only `max`.
        live.stage
            .attribute("/Enemy.bevy:Health:max").unwrap()
            .set(Value::Double(250.0))
            .unwrap();
        apply_changes(&mut world, &live, &mut map);

        assert_eq!(
            world.get::<Health>(e),
            Some(&Health {
                current: 50.0,
                max: 250.0
            }),
            "only `max` changed; `current` untouched"
        );
    }

    /// Clearing an authored opinion reverts that field to the type default;
    /// clearing every `bevy:` opinion of a type removes the component.
    #[test]
    fn reflect_route_clear_reverts_and_removes() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("clear.usda").unwrap();
        stage.define_prim("/Enemy").unwrap();
        author_double(&stage, "/Enemy.bevy:Health:current", 50.0);
        author_double(&stage, "/Enemy.bevy:Health:max", 100.0);

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Enemy").unwrap();

        // Clear just `max` → reverts to Health::default().max (0.0).
        live.stage
            .remove_property(openusd::sdf::path("/Enemy.bevy:Health:max").unwrap())
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(
            world.get::<Health>(e),
            Some(&Health {
                current: 50.0,
                max: 0.0
            }),
            "cleared `max` reverts to default; `current` stays"
        );

        // Clear the last opinion → component removed.
        live.stage
            .remove_property(openusd::sdf::path("/Enemy.bevy:Health:current").unwrap())
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert!(
            world.get::<Health>(e).is_none(),
            "clearing the last bevy: opinion removes the component"
        );
    }

    /// A `Handle<T>` reflect field resolves an asset-path value through the
    /// `AssetServer` at project time (PLAN 4a — the `HandleTemplate` analog).
    #[test]
    fn reflect_handle_field_loads_asset() {
        #[derive(Component, Reflect, Default)]
        #[reflect(Component, Default)]
        struct Icon {
            image: Handle<Image>,
        }

        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("handle.usda").unwrap();
        stage.define_prim("/I").unwrap();
        stage
            .create_attribute("/I.bevy:Icon:image", "asset")
            .unwrap()
            .set(Value::AssetPath("textures/icon.png".into()))
            .unwrap();
        let live = LiveStage::new(stage);

        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ));
        app.init_asset::<Image>();
        app.world_mut().insert_resource(SchemaRegistry::builtin());
        {
            let reg = app.world().resource::<AppTypeRegistry>().clone();
            reg.write().register::<Icon>();
        }
        let mut map = PrimEntities::default();
        project_stage(app.world_mut(), &live, &mut map);

        let e = map.entity("/I").unwrap();
        let handle = app.world().get::<Icon>(e).expect("Icon component").image.clone();
        assert!(
            handle.path().is_some(),
            "Handle<Image> field resolved an asset path via the AssetServer"
        );
        assert_eq!(
            handle.path().unwrap().path().to_str(),
            Some("textures/icon.png"),
        );
    }

    /// Nested field paths descend struct fields (`bevy:Boss:rage`), and a
    /// second component on the same prim projects independently.
    #[test]
    fn reflect_route_multiple_components() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("multi.usda").unwrap();
        stage.define_prim("/Enemy").unwrap();
        author_double(&stage, "/Enemy.bevy:Health:max", 100.0);
        stage
            .create_attribute("/Enemy.bevy:Boss:enraged", "bool")
            .unwrap()
            .set(Value::Bool(true))
            .unwrap();
        stage
            .create_attribute("/Enemy.bevy:Boss:rage", "float")
            .unwrap()
            .set(Value::Float(0.75))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| {
            r.register::<Health>();
            r.register::<Boss>();
        });
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Enemy").unwrap();

        assert_eq!(world.get::<Health>(e).unwrap().max, 100.0);
        assert_eq!(
            world.get::<Boss>(e),
            Some(&Boss {
                enraged: true,
                rage: 0.75
            })
        );
    }

    /// Composition-proof: a stronger opinion in the session layer wins over the
    /// root layer, and it's the *composed* (LIVERPS-resolved) value that lands
    /// on the component. This is USD standing in for BSN's `ResolvedScene`.
    #[test]
    fn reflect_route_stronger_layer_wins() {
        // Root layer authors `max = 100`; a session layer (stronger) overrides
        // it with `250`. Opening them composes to 250 — and *that* composed
        // value is what the reflect route projects.
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/reflect_compose_root.usda"
        );
        let session = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/reflect_compose_session.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .session_layer(session)
            .open(root)
            .expect("open root + session");

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Enemy").unwrap();

        assert_eq!(
            world.get::<Health>(e).unwrap().max,
            250.0,
            "the session-layer opinion (stronger) is what USD composed and projected"
        );
    }

    /// A full Rust type path (encoded with `__` for `::`, the path-legal form)
    /// resolves via the reflect route's short→full fallback — the escape hatch
    /// for short-name collisions.
    #[test]
    fn reflect_full_type_path_disambiguates() {
        // e.g. usd_bevy::live::tests::Health → usd_bevy__live__tests__Health
        let tp = std::any::type_name::<Health>().replace("::", "__");
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("fullpath.usda").unwrap();
        stage.define_prim("/E").unwrap();
        author_double(&stage, &format!("/E.bevy:{tp}:max"), 50.0);

        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/E").unwrap();
        assert_eq!(
            world.get::<Health>(e).map(|h| h.max),
            Some(50.0),
            "component authored via full type path projected"
        );
    }

    /// A `resynced` reconcile (triggered by a sibling prim appearing) must not
    /// drop reflect-routed components on the *existing* entities — reconcile
    /// re-applies routes on the survivors.
    #[test]
    fn resync_preserves_reflect_components() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("resync_reflect.usda").unwrap();
        stage.define_prim("/Enemy").unwrap();
        author_double(&stage, "/Enemy.bevy:Health:max", 100.0);
        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let enemy = map.entity("/Enemy").unwrap();
        assert_eq!(world.get::<Health>(enemy).unwrap().max, 100.0);

        // Defining a new prim resyncs → reconcile runs over the whole stage.
        live.stage.define_prim("/Other").unwrap();
        apply_changes(&mut world, &live, &mut map);

        assert!(map.entity("/Other").is_some(), "new prim spawned");
        assert_eq!(
            world.get::<Health>(enemy).map(|h| h.max),
            Some(100.0),
            "existing prim's reflect component survived the reconcile"
        );
    }

    /// A custom app-registered route runs on project, patch, and reconcile —
    /// the extension point apps use for their own schemas.
    #[test]
    fn custom_route_runs_on_all_paths() {
        #[derive(Component, Default)]
        struct Tagged(u32);

        struct TagRoute;
        impl crate::route::PrimRoute for TagRoute {
            fn matches(&self, ctx: &crate::route::RouteCtx) -> bool {
                ctx.type_name.as_deref() == Some("Xform")
            }
            fn project(&self, _c: &crate::route::RouteCtx, world: &mut World, e: Entity) {
                if let Ok(mut ent) = world.get_entity_mut(e) {
                    ent.insert(Tagged(1));
                }
            }
            fn patch(
                &self,
                _c: &crate::route::RouteCtx,
                world: &mut World,
                e: Entity,
                _changed: &[&str],
            ) {
                if let Some(mut t) = world.get_mut::<Tagged>(e) {
                    t.0 += 1;
                }
            }
        }

        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("custom.usda").unwrap();
        stage
            .define_prim("/A")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = LiveStage::new(stage);

        let mut world = World::new();
        let mut registry = SchemaRegistry::builtin();
        registry.register(TagRoute);
        world.insert_resource(registry);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let a = map.entity("/A").unwrap();
        assert_eq!(world.get::<Tagged>(a).map(|t| t.0), Some(1), "project ran");

        // A changed_info edit → patch.
        author_double(&live.stage, "/A.foo", 1.0);
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(world.get::<Tagged>(a).map(|t| t.0), Some(2), "patch ran");

        // A resync (new sibling prim) → reconcile re-patches existing /A.
        live.stage
            .define_prim("/B")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(
            world.get::<Tagged>(a).map(|t| t.0),
            Some(3),
            "reconcile re-patched existing prim"
        );
        assert_eq!(
            world.get::<Tagged>(map.entity("/B").unwrap()).map(|t| t.0),
            Some(1),
            "reconcile projected the new prim"
        );
    }

    /// The `usd!` macro (PLAN P3): an inline, compile-time-validated usda
    /// snippet with `${expr}` interpolation opens as a stage and projects
    /// through the routing registry — including a `bevy:` component.
    #[test]
    fn usd_macro_snippet_projects() {
        let hp = 175.0_f64;
        let name = "Goblin";
        let snippet = crate::usd!(
            "#usda 1.0\n\
             def Xform \"${name}\"\n\
             {\n\
                 custom double bevy:Health:max = ${hp}\n\
             }\n"
        );
        assert!(snippet.text().contains("Goblin"), "interpolated name");
        assert!(snippet.text().contains("175"), "interpolated hp");

        let stage = snippet.open_stage().expect("snippet opens as a stage");
        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Goblin").expect("interpolated prim projected");
        assert_eq!(
            world.get::<Health>(e).unwrap().max,
            175.0,
            "the bevy: component from the inline snippet projected"
        );
    }

    /// The echo guard (PLAN P2): a prim marked as self-authored is skipped by
    /// the *next* `apply_changes`, so author-back doesn't drive a reprojection
    /// of the entity we just wrote.
    #[test]
    fn author_back_is_echo_suppressed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Component, Default)]
        struct Touches(usize);

        struct CountRoute(Arc<AtomicUsize>);
        impl crate::route::PrimRoute for CountRoute {
            fn matches(&self, _c: &crate::route::RouteCtx) -> bool {
                true
            }
            fn project(&self, _c: &crate::route::RouteCtx, world: &mut World, e: Entity) {
                if let Ok(mut ent) = world.get_entity_mut(e) {
                    ent.insert(Touches(0));
                }
            }
            fn patch(
                &self,
                _c: &crate::route::RouteCtx,
                world: &mut World,
                e: Entity,
                _changed: &[&str],
            ) {
                self.0.fetch_add(1, Ordering::SeqCst);
                if let Some(mut t) = world.get_mut::<Touches>(e) {
                    t.0 += 1;
                }
            }
        }

        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("echo.usda").unwrap();
        stage.define_prim("/A").unwrap();
        let live = LiveStage::new(stage);

        let patches = Arc::new(AtomicUsize::new(0));
        let mut world = World::new();
        let mut registry = SchemaRegistry::builtin();
        registry.register(CountRoute(patches.clone()));
        world.insert_resource(registry);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        // Author onto /A but mark it self-authored → the resulting change is
        // swallowed, so the route's patch does NOT run for /A.
        live.mark_authored("/A");
        author_double(&live.stage, "/A.some", 1.0);
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(
            patches.load(Ordering::SeqCst),
            0,
            "self-authored change was suppressed (no patch)"
        );

        // A subsequent, un-marked change is patched normally (guard is one-shot).
        author_double(&live.stage, "/A.some", 2.0);
        apply_changes(&mut world, &live, &mut map);
        assert_eq!(
            patches.load(Ordering::SeqCst),
            1,
            "the next unmarked change patches normally"
        );
    }

    /// The material route (PLAN P4): a mesh bound to a UsdPreviewSurface
    /// projects a `StandardMaterial` with the authored colour/roughness/metallic
    /// — replacing the mesh route's grey placeholder.
    #[test]
    fn project_material_binding() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/material_test.usda");
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open material_test.usda");
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let tri = map.entity("/World/Tri").expect("mesh prim projected");
        let handle = world
            .get::<MeshMaterial3d<StandardMaterial>>(tri)
            .expect("material attached")
            .0
            .clone();
        let mats = world.resource::<Assets<StandardMaterial>>();
        let m = mats.get(&handle).expect("material asset present");

        let c = m.base_color.to_linear();
        assert!(
            (c.red - 0.8).abs() < 1e-3 && (c.green - 0.1).abs() < 1e-3 && (c.blue - 0.1).abs() < 1e-3,
            "authored diffuseColor projected to base_color, got {c:?}"
        );
        assert!(
            (m.perceptual_roughness - 0.25).abs() < 1e-4,
            "roughness projected, got {}",
            m.perceptual_roughness
        );
        assert!(
            (m.metallic - 0.9).abs() < 1e-4,
            "metallic projected, got {}",
            m.metallic
        );
    }

    // ─── Time-sample animation (PLAN in-repo) ───────────────────────────

    use openusd::usd::TimeCode;

    /// Author a prim with an animated `xformOp:translate` (samples at t=0 and
    /// t=10), returning the stage.
    fn animated_translate_stage() -> Stage {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("anim.usda").unwrap();
        stage
            .define_prim("/Mover")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let attr = stage
            .create_attribute("/Mover.xformOp:translate", "double3")
            .unwrap();
        attr.set_at(
            Value::Vec3d(openusd::gf::Vec3d::from([0.0, 0.0, 0.0])),
            TimeCode::new(0.0),
        )
        .unwrap()
        .set_at(
            Value::Vec3d(openusd::gf::Vec3d::from([10.0, 0.0, 0.0])),
            TimeCode::new(10.0),
        )
        .unwrap();
        stage
            .create_attribute("/Mover.xformOpOrder", "token[]")
            .unwrap()
            .set(Value::TokenVec(vec!["xformOp:translate".into()]))
            .unwrap();
        stage
    }

    #[test]
    fn sparse_sample_edits_refresh_the_live_animation_index() {
        let stage = animated_translate_stage();
        stage.prim("/Mover").unwrap().attribute("xformOp:translate").clear().unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([1.0, 0.0, 0.0]))).unwrap();
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(StageTime { current: 0.0 });
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let mover = map.entity("/Mover").unwrap();
        assert!(world.resource::<AnimatedPrims>().0.is_empty());
        live.stage.prim("/Mover").unwrap().attribute("xformOp:translate")
            .set_at(Value::Vec3d(openusd::gf::Vec3d::from([0.0, 0.0, 0.0])), TimeCode::new(0.0)).unwrap()
            .set_at(Value::Vec3d(openusd::gf::Vec3d::from([10.0, 0.0, 0.0])), TimeCode::new(10.0)).unwrap();
        assert!(live.queue.borrow().iter().all(|change| change.resynced.is_empty()));
        apply_changes(&mut world, &live, &mut map);
        assert!(world.resource::<AnimatedPrims>().0.contains("/Mover"));
        world.insert_resource(map);
        world.insert_non_send(live);
        world.resource_mut::<StageTime>().current = 5.0;
        resample_animation_system(&mut world);
        assert_eq!(world.get::<Transform>(mover).unwrap().translation.x, 5.0);
        let live = world.remove_non_send::<LiveStage>().unwrap();
        let mut map = world.remove_resource::<PrimEntities>().unwrap();
        live.stage.prim("/Mover").unwrap().attribute("xformOp:translate")
            .clear_at(TimeCode::new(0.0)).unwrap().clear_at(TimeCode::new(10.0)).unwrap();
        assert!(live.queue.borrow().iter().all(|change| change.resynced.is_empty()));
        apply_changes(&mut world, &live, &mut map);
        assert!(world.resource::<AnimatedPrims>().0.is_empty());
        assert_eq!(map.entity("/Mover"), Some(mover));
        assert_eq!(world.get::<Transform>(mover).unwrap().translation.x, 1.0);
    }

    #[test]
    fn reconciliation_can_skip_the_live_animation_index() {
        let live = LiveStage::new(animated_translate_stage());
        let mut world = World::new();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let mover = map.entity("/Mover").unwrap();
        let sentinel = std::collections::HashSet::from(["/OtherSession".to_string()]);
        world.insert_resource(AnimatedPrims(sentinel.clone()));
        world.insert_resource(StageTime { current: 5.0 });
        reconcile(&mut world, &live, &mut map, false);
        assert_eq!(world.resource::<AnimatedPrims>().0, sentinel);
        assert_eq!(map.entity("/Mover"), Some(mover));
        assert_eq!(world.get::<Transform>(mover).unwrap().translation.x, 5.0);
        world.insert_resource(StageTime { current: 10.0 });
        reconcile(&mut world, &live, &mut map, true);
        assert_eq!(world.resource::<AnimatedPrims>().0, std::collections::HashSet::from(["/Mover".to_string()]));
        assert_eq!(world.get::<Transform>(mover).unwrap().translation.x, 10.0);
    }

    /// Projection resolves animated transforms at the world's `StageTime`, and
    /// re-patching after moving the time samples the new frame.
    #[test]
    fn animated_transform_follows_stage_time() {
        let live = LiveStage::new(animated_translate_stage());
        let mut world = World::new();
        world.insert_resource(StageTime { current: 0.0 });
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let mover = map.entity("/Mover").unwrap();
        assert_eq!(
            world.get::<Transform>(mover).unwrap().translation,
            Vec3::new(0.0, 0.0, 0.0),
            "projected at t=0"
        );

        // The prim is flagged as animated.
        assert!(
            world.resource::<AnimatedPrims>().0.contains("/Mover"),
            "animated prim detected"
        );

        // Move time and re-patch → samples t=10.
        world.resource_mut::<StageTime>().current = 10.0;
        let registry = SchemaRegistry::builtin();
        let p = openusd::sdf::path("/Mover").unwrap();
        registry.patch_prim(&live.stage, &p, &mut world, mover, &[]);
        assert_eq!(
            world.get::<Transform>(mover).unwrap().translation,
            Vec3::new(10.0, 0.0, 0.0),
            "re-sampled at t=10"
        );

        // Halfway is linearly interpolated.
        world.resource_mut::<StageTime>().current = 5.0;
        registry.patch_prim(&live.stage, &p, &mut world, mover, &[]);
        assert!(
            (world.get::<Transform>(mover).unwrap().translation.x - 5.0).abs() < 1e-4,
            "t=5 interpolates to x≈5"
        );
    }

    /// The full animation loop through the plugin schedule: scrubbing
    /// `StageTime` resamples the animated entity on the next update.
    #[test]
    fn plugin_resamples_on_time_scrub() {
        let live = LiveStage::new(animated_translate_stage());
        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(live);

        app.world_mut().run_schedule(Update); // projects at t=0
        let mover = app.world().resource::<PrimEntities>().entity("/Mover").unwrap();
        assert_eq!(
            app.world().get::<Transform>(mover).unwrap().translation.x,
            0.0
        );

        // Scrub to t=10 and tick.
        app.world_mut().resource_mut::<StageTime>().current = 10.0;
        app.world_mut().run_schedule(Update);
        assert!(
            (app.world().get::<Transform>(mover).unwrap().translation.x - 10.0).abs() < 1e-4,
            "plugin resampled the animated transform after the scrub"
        );
    }

    #[test]
    fn point_and_curve_geometry_sample_independent_clocks() {
        use crate::instance::{UsdInstances, UsdInstanceTime};
        let source = crate::UsdSource::new("point-curve.usda", include_bytes!("../../../assets/point_curve_animation.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let entities = roots.map(|root| ["/Points", "/Curve"].map(|path|
            app.world().get_non_send::<UsdInstances>().unwrap().entity(root, path).unwrap()));
        let children = entities.map(|pair| pair.map(|entity| app.world_mut().spawn((Name::new("runtime"), ChildOf(entity))).id()));
        for times in [[0.0,10.0], [5.0,0.0], [10.0,5.0]] {
            for (root,time) in roots.into_iter().zip(times) {
                app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time;
            }
            app.update();
            for (index, (root,time)) in roots.into_iter().zip(times).enumerate() {
                for (kind,path) in ["/Points", "/Curve"].into_iter().enumerate() {
                    let entity = app.world().get_non_send::<UsdInstances>().unwrap().entity(root, path).unwrap();
                    assert_eq!(entity, entities[index][kind]);
                    assert_eq!(app.world().get::<ChildOf>(children[index][kind]).unwrap().parent(), entity);
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
                    let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("positions") };
                    assert_eq!(positions.len(), 4);
                    let height = time as f32 / 5.0 + if kind == 0 { 0.5 } else { 0.0 };
                    assert!(positions.iter().all(|position| (position[1] - height).abs() < 1e-6), "{path} at {time}: {positions:?}");
                    let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
                    assert!(material.unlit);
                    if kind == 1 {
                        let expected = if time < 10.0 { vec![0,1,1,2,2,3] } else { vec![0,1,2,3] };
                        assert_eq!(mesh.indices().unwrap().iter().collect::<Vec<_>>(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn inherited_uv_indices_sample_per_root_and_reconcile_parent_edits() {
        use crate::instance::{UsdInstances, UsdInstanceTime};
        let fixture = r#"#usda 1.0
def Xform "Root" {
    texCoord2f[] primvars:st = [(0.25,0.25), (0.75,0.75)] (interpolation = "constant")
    int[] primvars:st:indices.timeSamples = {0: [0], 10: [1]}
    def Mesh "M" {
        point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
}
def PointInstancer "PI" {
    rel prototypes = </Root/M>
    int[] protoIndices = [0]
    point3f[] positions = [(2,0,0)]
}
"#;
        for name in ["st", "st0"] {
            let fixture = fixture.replace("primvars:st", &format!("primvars:{name}"));
            let source = crate::UsdSource::new("uv.usda", fixture.into_bytes()).unwrap();
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
            app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
            let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
            let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
            let check = |app: &App, root, expected: [f32;2]| {
                let instances = app.world().get_non_send::<UsdInstances>().unwrap();
                let mesh = instances.entity(root, "/Root/M").unwrap();
                let pi = instances.entity(root, "/PI").unwrap();
                let copy = app.world().get::<Children>(pi).unwrap().iter().find(|entity| app.world().get::<crate::route::instancer::UsdInstance>(*entity).is_some()).unwrap();
                for entity in [mesh,copy] {
                    let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
                    let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { panic!("uvs") };
                    assert!(uvs.iter().all(|uv| *uv == expected), "{name} expected={expected:?} actual={uvs:?}");
                }
                mesh
            };
            for times in [[0.0,10.0], [10.0,0.0]] {
                for (root,time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
                app.update();
                for (root,time) in roots.into_iter().zip(times) { check(&app, root, if time == 0.0 { [0.25,0.75] } else { [0.75,0.25] }); }
            }
            let entity = check(&app, roots[0], [0.75,0.25]);
            let child = app.world_mut().spawn(ChildOf(entity)).id();
            let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(roots[0]).unwrap().clone();
            stage.attribute(format!("/Root.primvars:{name}:indices")).unwrap().set_at(Value::IntVec(vec![0]), openusd::usd::TimeCode::new(10.0)).unwrap();
            app.update();
            assert_eq!(check(&app, roots[0], [0.25,0.75]), entity);
            assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
            let local = stage.create_attribute(format!("/Root/M.primvars:{name}"), "texCoord2f[]").unwrap();
            local.clone().set(Value::Vec2fVec(vec![openusd::gf::Vec2f::from([0.5,0.5])])).unwrap();
            app.update();
            check(&app, roots[0], [0.5,0.5]);
            local.clear().unwrap();
            app.update();
            check(&app, roots[0], [0.25,0.75]);
        }
    }

    #[test]
    fn inherited_display_edits_and_local_overrides_preserve_entities() {
        let source = crate::UsdSource::new("inherited-display.usda", include_bytes!("../../../assets/inherited_display.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        app.world_mut().insert_non_send(LiveStage::new(stage.clone()));
        app.world_mut().run_schedule(Update);
        let entity = app.world().resource::<PrimEntities>().entity("/Scene/Mesh").unwrap();
        let runtime = app.world_mut().spawn(ChildOf(entity)).id();
        let check = |app: &App, expected: [f32;4]| {
            assert_eq!(app.world().resource::<PrimEntities>().entity("/Scene/Mesh"), Some(entity));
            assert_eq!(app.world().get::<ChildOf>(runtime).unwrap().parent(), entity);
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
            assert!(colors.iter().all(|color| Vec4::from_array(*color).abs_diff_eq(Vec4::from_array(expected), 1e-5)), "expected={expected:?} actual={colors:?}");
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.alpha_mode, if expected[3] < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque });
        };
        check(&app, [0.8,0.2,0.1,0.2]);
        app.world_mut().resource_mut::<StageTime>().current = 10.0;
        app.world_mut().run_schedule(Update);
        check(&app, [0.0,0.2,0.8,1.0]);
        stage.attribute("/Scene.primvars:displayColor").unwrap().set_at(Value::Vec3fVec(vec![
            openusd::gf::Vec3f::from([0.0,1.0,0.0]), openusd::gf::Vec3f::from([1.0,1.0,0.0])]), openusd::usd::TimeCode::new(10.0)).unwrap();
        stage.create_attribute("/Scene.primvars:displayColor:indices", "int[]").unwrap().set(Value::IntVec(vec![1])).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,1.0,0.0,1.0]);
        stage.attribute("/Scene.primvars:displayOpacity").unwrap().set_at(Value::FloatVec(vec![0.25,1.0]), openusd::usd::TimeCode::new(10.0)).unwrap();
        let opacity_indices = stage.create_attribute("/Scene.primvars:displayOpacity:indices", "int[]").unwrap();
        opacity_indices.clone().set(Value::IntVec(vec![0])).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,1.0,0.0,0.25]);
        opacity_indices.set(Value::IntVec(vec![1])).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,1.0,0.0,1.0]);
        let local = stage.create_attribute("/Scene/Mesh.primvars:displayColor", "color3f[]").unwrap();
        local.clone().set(Value::Vec3fVec(vec![openusd::gf::Vec3f::from([0.0,0.0,1.0])])).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [0.0,0.0,1.0,1.0]);
        local.clear().unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,1.0,0.0,1.0]);
        stage.attribute("/Scene.primvars:displayColor").unwrap().set_metadata("interpolation", Value::Token("vertex".into())).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,1.0,1.0,1.0]);
    }

    #[test]
    fn inherited_normals_refresh_on_scrub_and_parent_edit() {
        let stage = crate::snippet::UsdSnippet::new(r#"#usda 1.0
def Xform "Root" {
    normal3f[] primvars:normals.timeSamples = { 0: [(0,0,1)], 10: [(0,1,0)] }
    def Mesh "M" {
        point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
}
"#).open_stage().unwrap();
        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_resource(Assets::<Mesh>::default());
        app.world_mut().insert_resource(Assets::<StandardMaterial>::default());
        app.world_mut().insert_non_send(LiveStage::new(stage.clone()));
        app.world_mut().run_schedule(Update);
        let entity = app.world().resource::<PrimEntities>().entity("/Root/M").unwrap();
        let runtime_child = app.world_mut().spawn(ChildOf(entity)).id();
        let check = |app: &App, expected: [f32; 3]| {
            assert_eq!(app.world().resource::<PrimEntities>().entity("/Root/M"), Some(entity));
            let handle = &app.world().get::<Mesh3d>(entity).unwrap().0;
            let mesh = app.world().resource::<Assets<Mesh>>().get(handle).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
            assert_eq!(normals, &[expected; 3]);
            assert_eq!(app.world().get::<ChildOf>(runtime_child).unwrap().parent(), entity);
        };
        check(&app, [0.0,0.0,1.0]);
        app.world_mut().resource_mut::<StageTime>().current = 10.0;
        app.world_mut().run_schedule(Update);
        check(&app, [0.0,1.0,0.0]);
        stage.attribute("/Root.primvars:normals").unwrap().set_at(
            Value::Vec3fVec(vec![openusd::gf::Vec3f::from([1.0,0.0,0.0])]), openusd::usd::TimeCode::new(10.0)).unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [1.0,0.0,0.0]);
        stage.attribute("/Root.primvars:normals").unwrap().block().unwrap();
        app.world_mut().run_schedule(Update);
        check(&app, [0.0,0.0,1.0]);
    }

    /// A static (unanimated) prim is not flagged animated and ignores StageTime.
    #[test]
    fn static_prim_not_animated() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("static.usda").unwrap();
        stage
            .define_prim("/Fixed")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        stage
            .create_attribute("/Fixed.xformOp:translate", "double3")
            .unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([1.0, 2.0, 3.0])))
            .unwrap();
        stage
            .create_attribute("/Fixed.xformOpOrder", "token[]")
            .unwrap()
            .set(Value::TokenVec(vec!["xformOp:translate".into()]))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(StageTime { current: 999.0 });
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Fixed").unwrap();
        assert!(
            !world.resource::<AnimatedPrims>().0.contains("/Fixed"),
            "no time samples → not animated"
        );
        assert_eq!(
            world.get::<Transform>(e).unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0),
            "static value regardless of StageTime"
        );
    }

    /// UsdSkel end-to-end: a skinned mesh projects a `Mesh3d`, is flagged
    /// animated, and re-deforms (new mesh handle) when `StageTime` scrubs.
    #[test]
    fn skinned_mesh_projects_and_resamples() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/skel_test_simple.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open skel_test_simple.usda");
        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(StageTime { current: 0.0 });
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let bar = map.entity("/Test/Bar").expect("skinned mesh projected");
        let h0 = world
            .get::<Mesh3d>(bar)
            .expect("skinned mesh has geometry")
            .0
            .clone();
        assert!(
            world.resource::<AnimatedPrims>().0.contains("/Test/Bar"),
            "skinned mesh flagged animated (its samples live on the SkelAnimation)"
        );

        // Scrub to t=30 and re-patch → a freshly deformed mesh.
        world.resource_mut::<StageTime>().current = 30.0;
        let registry = SchemaRegistry::builtin();
        let p = openusd::sdf::path("/Test/Bar").unwrap();
        registry.patch_prim(&live.stage, &p, &mut world, bar, &[]);
        let h30 = world.get::<Mesh3d>(bar).unwrap().0.clone();
        assert_ne!(h0, h30, "resampling produced a new deformed mesh");
    }

    /// Payload lazy load/unload (PLAN Phase 3) — the `queue_spawn_scene`
    /// analog. Unloading despawns the payloaded subtree and marks the prim
    /// `UsdPayloadUnloaded`; loading re-materializes it.
    #[test]
    fn payload_load_unload_streams_subtree() {
        use crate::route::payload::UsdPayloadUnloaded;
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/payload_test.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open payload_test.usda");
        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        // Payload loads by default → subtree present.
        let root = map.entity("/Root").expect("payload prim projected");
        let child = map.entity("/Root/Child").expect("payloaded child projected");
        assert_eq!(world.get::<Health>(child).unwrap().max, 42.0);
        assert!(world.get::<UsdPayloadUnloaded>(root).is_none(), "loaded → no marker");

        // Unload → child despawns, root becomes a placeholder.
        live.unload_payload("/Root");
        assert!(live.has_changes(), "unload fired the change sink");
        apply_changes(&mut world, &live, &mut map);
        assert!(map.entity("/Root/Child").is_none(), "payloaded subtree despawned");
        let root = map.entity("/Root").expect("payload prim stays as placeholder");
        assert!(
            world.get::<UsdPayloadUnloaded>(root).is_some(),
            "unloaded payload marked"
        );

        // Load → subtree re-materializes, marker cleared.
        live.load_payload("/Root");
        apply_changes(&mut world, &live, &mut map);
        let child = map.entity("/Root/Child").expect("payloaded child re-materialized");
        assert_eq!(world.get::<Health>(child).unwrap().max, 42.0);
        assert!(
            world.get::<UsdPayloadUnloaded>(map.entity("/Root").unwrap()).is_none(),
            "reload cleared the marker"
        );
    }

    /// Variant switching (PLAN Phase 2) — a capability BSN has no equivalent
    /// for. Authoring a variant selection is a composition change, so the live
    /// loop reconciles and the reflect component reprojects to the new variant.
    #[test]
    fn variant_switch_reprojects() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/variant_test.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).expect("open variant_test.usda");
        let live = LiveStage::new(stage);
        let mut world = reflect_world(|r| r.register::<Health>());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Prop").expect("prim projected");
        assert_eq!(
            world.get::<Health>(e).unwrap().max,
            100.0,
            "default 'red' variant projected"
        );
        assert_eq!(
            crate::read::variants::variant_selection(
                &live.stage,
                &openusd::sdf::path("/Prop").unwrap(),
                "look"
            ),
            Some("red".to_string()),
        );

        // Switch the variant — fires the sink; the live loop reconciles.
        crate::authoring::set_variant(&live.stage, "/Prop", "look", "blue").unwrap();
        assert!(live.has_changes(), "variant switch fired the change sink");
        apply_changes(&mut world, &live, &mut map);

        let e = map.entity("/Prop").expect("prim still projected");
        assert_eq!(
            world.get::<Health>(e).unwrap().max,
            250.0,
            "'blue' variant reprojected the component"
        );
    }

    /// Variant selection round-trips through editor transaction undo/redo.
    #[test]
    fn variant_undo_redo() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/variant_test.usda"
        );
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).open(path).unwrap();
        let mut hist = crate::editor::EditorSession::new(stage.clone());
        let prim = openusd::sdf::path("/Prop").unwrap();
        let sel =
            |s: &Stage| crate::read::variants::variant_selection(s, &prim, "look");

        hist.edit(crate::editor::EditorEdit::Variant { prim: "/Prop".into(), set: "look".into(), selection: "blue".into() }).unwrap();
        assert_eq!(sel(&stage).as_deref(), Some("blue"));
        assert!(hist.undo().unwrap());
        assert_eq!(sel(&stage).as_deref(), Some("red"), "undo → prior variant");
        assert!(hist.redo().unwrap());
        assert_eq!(sel(&stage).as_deref(), Some("blue"), "redo → switched variant");
    }

    #[test]
    fn path_splitting_helpers() {
        assert_eq!(prim_of("/A/B.foo"), "/A/B");
        assert_eq!(prim_of("/A/B"), "/A/B");
        assert_eq!(prim_of("/"), "/");
        assert_eq!(property_of("/A/B.foo:bar"), Some("foo:bar"));
        assert_eq!(property_of("/A/B"), None);
        assert_eq!(parent_path("/A/B/C"), "/A/B");
        assert_eq!(parent_path("/A"), "/");
    }

    #[test]
    fn prim_entities_bimap_subtree() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let c = world.spawn_empty().id();
        let mut map = PrimEntities::default();
        map.insert("/World", a);
        map.insert("/World/Mesh", b);
        map.insert("/Other", c);

        assert_eq!(map.entity("/World/Mesh"), Some(b));
        assert_eq!(map.path(a), Some("/World"));

        let sub: Vec<String> = map.subtree("/World").into_iter().map(|(p, _)| p).collect();
        assert_eq!(sub.len(), 2, "subtree of /World = {{/World, /World/Mesh}}");
        assert!(sub.iter().all(|p| p.starts_with("/World")));

        assert_eq!(map.remove_path("/World/Mesh"), Some(b));
        assert_eq!(map.entity("/World/Mesh"), None);
        assert_eq!(map.path(b), None);
    }
}
