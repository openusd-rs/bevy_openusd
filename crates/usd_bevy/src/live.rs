//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).
//!
//! The composed USD stage is the single source of truth. We hold it live
//! (not baked to a one-shot `Scene`), project it into Bevy entities, and
//! keep them in sync off openusd's `StageSink` (`UsdNotice`) change stream:
//! every committed edit fires the sink, we copy the changed paths out, and a
//! Bevy system reprojects exactly the affected entities.
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
use crate::read::geom::{VisibilityState, read_visibility};
use crate::read::xform::read_transform;

fn to_bevy_transform(t: crate::read::xform::Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

fn transform_at(stage: &Stage, prim_path: &str) -> Transform {
    openusd::sdf::path(prim_path)
        .ok()
        .and_then(|p| read_transform(stage, &p).ok().flatten())
        .map(to_bevy_transform)
        .unwrap_or_default()
}

fn visibility_at(stage: &Stage, prim_path: &str) -> Visibility {
    match openusd::sdf::path(prim_path)
        .ok()
        .and_then(|p| read_visibility(stage, &p).ok())
    {
        Some(VisibilityState::Invisible) => Visibility::Hidden,
        _ => Visibility::default(),
    }
}

/// Re-read and patch the per-prim components we project (the §12 routing
/// target). v1: `Transform` + `Visibility`; mesh/material/light extend here.
fn patch_prim(world: &mut World, stage: &Stage, entity: Entity, prim: &str) {
    let t = transform_at(stage, prim);
    if let Some(mut tr) = world.get_mut::<Transform>(entity) {
        *tr = t;
    }
    let v = visibility_at(stage, prim);
    if let Some(mut vis) = world.get_mut::<Visibility>(entity) {
        *vis = v;
    }
}

/// If `prim` is a mesh, build a Bevy mesh + a default material and attach
/// `Mesh3d`/`MeshMaterial3d`. No-op when the render `Assets` aren't present
/// (headless) or the prim has no mesh. (Real material binding lands when the
/// material reader is ported into `live`.)
fn attach_mesh(world: &mut World, stage: &Stage, entity: Entity, prim: &str) {
    let Some(p) = openusd::sdf::path(prim).ok() else {
        return;
    };
    let Ok(Some(read)) = crate::read::geom::read_mesh(stage, &p) else {
        return;
    };
    if world.get_resource::<Assets<Mesh>>().is_none()
        || world.get_resource::<Assets<StandardMaterial>>().is_none()
    {
        return;
    }
    let mesh = crate::mesh::mesh_from_usd(&read);
    let mesh_handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    world
        .entity_mut(entity)
        .insert((Mesh3d(mesh_handle), MeshMaterial3d(material)));
}

/// The prim path owning a (possibly property) path: `/Foo.bar` → `/Foo`.
fn prim_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// Project every prim in the stage into an entity (`UsdPrimRef` +
/// `Transform`), recording the path↔entity bimap. Idempotent only on an
/// empty world — call once on load.
pub fn project_stage(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let _ = stage.traverse(
        openusd::usd::PrimPredicate::default(),
        |path: &openusd::sdf::Path| {
            let entity = world
                .spawn((
                    UsdPrimRef {
                        path: path.as_str().to_string(),
                        ..Default::default()
                    },
                    transform_at(stage, path.as_str()),
                    visibility_at(stage, path.as_str()),
                ))
                .id();
            map.insert(path.as_str().to_string(), entity);
            attach_mesh(world, stage, entity, path.as_str());
        },
    );
    // Projecting authored the initial read; clear so the first sync starts clean.
    let _ = live.drain_changes();
}

/// Drain the change queue and reproject affected entities.
///
/// * Any `resynced` change → reconcile the entity set against the stage
///   (spawn entities for new prims, despawn entities for removed prims,
///   patch the rest). v1 reconciles the whole stage; a later version scopes
///   to the resynced subtree.
/// * `changed_info` only → patch the touched prims' transforms in place.
pub fn apply_changes(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let changes = live.drain_changes();
    if changes.is_empty() {
        return;
    }
    if changes.iter().any(|c| !c.resynced.is_empty()) {
        reconcile(world, live, map);
        return;
    }
    for change in &changes {
        for path in change.paths() {
            let prim = prim_of(path);
            if let Some(entity) = map.entity(prim) {
                patch_prim(world, &live.stage, entity, prim);
            }
        }
    }
}

/// Reconcile the projected entities against the stage's current prims:
/// despawn entities whose prim was removed, spawn entities for new prims,
/// patch transforms on the rest.
fn reconcile(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let mut current: std::collections::HashSet<String> = std::collections::HashSet::new();
    let _ = stage.traverse(
        openusd::usd::PrimPredicate::default(),
        |p: &openusd::sdf::Path| {
            current.insert(p.as_str().to_string());
        },
    );

    // Despawn entities for prims no longer present.
    let stale: Vec<(String, Entity)> = map
        .iter()
        .filter(|(p, _)| !current.contains(*p))
        .map(|(p, e)| (p.to_string(), e))
        .collect();
    for (path, entity) in stale {
        world.despawn(entity);
        map.remove_path(&path);
    }

    // Spawn new prims; patch components on existing ones.
    for path in &current {
        if let Some(entity) = map.entity(path) {
            patch_prim(world, stage, entity, path);
        } else {
            let entity = world
                .spawn((
                    UsdPrimRef {
                        path: path.clone(),
                        ..Default::default()
                    },
                    transform_at(stage, path),
                    visibility_at(stage, path),
                ))
                .id();
            map.insert(path.clone(), entity);
            attach_mesh(world, stage, entity, path);
        }
    }
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

impl Plugin for LiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrimEntities>()
            .add_systems(Update, (project_on_load_system, reproject_system).chain());
    }
}

/// One-shot projection the first frame a `LiveStage` is present.
fn project_on_load_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    // Only project once per session: skip if the bimap is already populated.
    if world.resource::<PrimEntities>().len() > 0 {
        return;
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    project_stage(world, &live, &mut map);
    world.insert_resource(map);
    world.insert_non_send(live);
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

fn clear_transform(stage: &Stage, prim_path: &str) -> anyhow::Result<()> {
    let prim = openusd::sdf::path(prim_path)?;
    let _ = stage.remove_property(prim.append_property("xformOp:transform")?);
    let _ = stage.remove_property(prim.append_property("xformOpOrder")?);
    Ok(())
}

// ─── Undo / redo for transform edits (RETHINK P6, gizmo slice) ───────
//
// Typed-action history: each edit captures the prim's transform before +
// after, so undo re-authors the prior state (or clears it if there was
// none) and redo re-applies. General attribute / namespace undo via
// openusd `Diff` inverses is the next layer.

struct TransformEdit {
    prim: String,
    before: Option<Transform>,
    after: Transform,
}

/// Undo/redo stack for transform edits.
#[derive(Default)]
pub struct TransformHistory {
    undo: Vec<TransformEdit>,
    redo: Vec<TransformEdit>,
}

impl TransformHistory {
    /// Author `after` onto `prim`, recording the prior transform for undo.
    pub fn author(&mut self, stage: &Stage, prim: &str, after: Transform) -> anyhow::Result<()> {
        let before = current_transform(stage, prim);
        author_transform(stage, prim, &after)?;
        self.undo.push(TransformEdit {
            prim: prim.to_string(),
            before,
            after,
        });
        self.redo.clear();
        Ok(())
    }

    /// Undo the most recent edit. Returns `false` if nothing to undo.
    pub fn undo(&mut self, stage: &Stage) -> anyhow::Result<bool> {
        let Some(edit) = self.undo.pop() else {
            return Ok(false);
        };
        match &edit.before {
            Some(t) => author_transform(stage, &edit.prim, t)?,
            None => clear_transform(stage, &edit.prim)?,
        }
        self.redo.push(edit);
        Ok(true)
    }

    /// Redo the most recently undone edit. Returns `false` if nothing to redo.
    pub fn redo(&mut self, stage: &Stage) -> anyhow::Result<bool> {
        let Some(edit) = self.redo.pop() else {
            return Ok(false);
        };
        author_transform(stage, &edit.prim, &edit.after)?;
        self.undo.push(edit);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openusd::sdf::Value;

    fn tx(stage: &Stage, prim: &str) -> Option<Vec3> {
        current_transform(stage, prim).map(|t| t.translation)
    }

    /// The `UsdNotice` loop: authoring an edit fires the sink, and the change
    /// (mentioning the edited path) lands on the drainable queue. This is the
    /// foundation the whole live-editor reprojection is built on.
    #[test]
    fn sink_records_authored_edits() {
        let stage = Stage::builder()
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
        let stage = Stage::builder().in_memory("resync.usda").unwrap();
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
        let stage = Stage::builder().in_memory("e2e.usda").unwrap();
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
            .attribute("/Foo.xformOp:translate")
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
        let stage = Stage::builder().in_memory("rs.usda").unwrap();
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
        let stage = Stage::builder().in_memory("auth.usda").unwrap();
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

    /// Undo/redo walks the transform history: undo restores the prior value
    /// (or clears it when there was none), redo re-applies.
    #[test]
    fn transform_undo_redo() {
        let stage = Stage::builder().in_memory("undo.usda").unwrap();
        stage
            .define_prim("/Foo")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let mut hist = TransformHistory::default();

        hist.author(&stage, "/Foo", Transform::from_xyz(1.0, 0.0, 0.0))
            .unwrap();
        hist.author(&stage, "/Foo", Transform::from_xyz(2.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(tx(&stage, "/Foo"), Some(Vec3::new(2.0, 0.0, 0.0)));

        assert!(hist.undo(&stage).unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            "undo → previous"
        );
        assert!(hist.undo(&stage).unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            None,
            "undo past the first edit clears the transform"
        );
        assert!(!hist.undo(&stage).unwrap(), "nothing left to undo");

        assert!(hist.redo(&stage).unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            "redo → first edit"
        );
        assert!(hist.redo(&stage).unwrap());
        assert_eq!(
            tx(&stage, "/Foo"),
            Some(Vec3::new(2.0, 0.0, 0.0)),
            "redo → second edit"
        );
        assert!(!hist.redo(&stage).unwrap(), "nothing left to redo");
    }

    /// The plugin wires it together: projecting on load and reprojecting on
    /// edit, run through a real Bevy `Update` schedule.
    #[test]
    fn plugin_projects_and_reprojects() {
        let stage = Stage::builder().in_memory("app.usda").unwrap();
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
        let stage = Stage::builder().in_memory("vis.usda").unwrap();
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
        let stage = Stage::open(path).expect("open two_xforms.usda");
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
        let stage = Stage::open(path).expect("open skel_test_simple.usda");
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
