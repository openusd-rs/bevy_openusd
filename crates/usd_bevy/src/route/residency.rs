//! On-demand preparation of initially hidden mesh prims.

use bevy::prelude::*;
use super::{PrimRoute, RouteCtx, SchemaRegistry};

/// Opt-in initial mesh deferral for built-in registries; resident meshes remain retained.
#[derive(Resource, Default)]
pub struct DeferHiddenMeshes;

/// A logical mesh prim whose render geometry has not yet been prepared.
#[derive(Component)]
pub struct UsdDeferredMesh;

/// Soft per-update budget for promoting deferred meshes; one mesh may exceed it.
#[derive(Resource)]
pub struct UsdResidencyBudget(pub std::time::Duration);

/// Deferred mesh queued for preparation from the current stage and clock.
#[derive(Component)]
pub struct UsdMeshPreparationQueued;

#[derive(Component)]
struct PreparationQueue(std::collections::VecDeque<Entity>);

/// Accumulated promotion work and the slowest attempted prim.
#[derive(Resource, Default)]
pub struct ResidencyPreparationTiming {
    pub attempts: usize,
    pub elapsed: std::time::Duration,
    pub slowest: Option<(String, std::time::Duration)>,
}

#[derive(Resource, Default)]
struct PreparationTurn {
    spent: std::time::Duration,
    attempts: usize,
    active: Option<Entity>,
}

#[derive(Resource)]
struct PreparationConfigured;

pub(crate) fn configure(app: &mut App) {
    if app.world().contains_resource::<PreparationConfigured>() { return; }
    app.insert_resource(PreparationConfigured).init_resource::<PreparationTurn>().add_systems(First, reset_turn);
}

fn reset_turn(mut turn: ResMut<PreparationTurn>) { *turn = default(); }

#[derive(Resource, Default)]
pub(crate) struct AppliedDeferMode(Option<bool>);

pub(crate) fn enabled(world: &World) -> bool {
    world.contains_resource::<DeferHiddenMeshes>() && world.get_resource::<SchemaRegistry>().is_none_or(|registry| registry.builtin_only)
}

pub(crate) fn refresh_mode(world: &mut World) {
    let current = enabled(world);
    let changed = world.resource::<AppliedDeferMode>().0 != Some(current);
    if !changed && world.query_filtered::<Entity, With<UsdMeshPreparationQueued>>().iter(world).next().is_none() { return; }
    let Some(live) = world.remove_non_send::<crate::live::LiveStage>() else { return; };
    let map = world.remove_resource::<crate::live::PrimEntities>().unwrap_or_default();
    if changed { materialize(world, &live.stage, &map); }
    else { resume(world, &live.stage, &map); }
    world.insert_resource(map);
    world.insert_non_send(live);
    world.resource_mut::<AppliedDeferMode>().0 = Some(current);
}

pub(super) fn should_defer(ctx: &RouteCtx, world: &World, entity: Entity) -> bool {
    if ctx.type_name.as_deref() != Some("Mesh") { return false; }
    if world.contains_resource::<UsdResidencyBudget>() && world.get::<UsdDeferredMesh>(entity).is_some()
        && world.get_resource::<PreparationTurn>().is_none_or(|turn| turn.active != Some(entity)) { return true; }
    world.contains_resource::<DeferHiddenMeshes>() && ctx.type_name.as_deref() == Some("Mesh")
        && world.contains_resource::<Assets<Mesh>>() && world.contains_resource::<Assets<StandardMaterial>>()
        && world.get::<Mesh3d>(entity).is_none() && super::hierarchy_hidden(world, entity) == Some(true)
}

pub(super) fn geometry_route(name: &str) -> bool {
    [super::geom::MeshRoute.name(), super::material::MaterialRoute.name(), super::skel::SkinRoute.name(),
        super::subdivision::SubdivisionRoute.name(), super::subset::SubsetRoute.name()].contains(&name)
}

pub(crate) fn materialize(world: &mut World, stage: &openusd::usd::Stage, map: &crate::live::PrimEntities) {
    if !world.contains_resource::<DeferHiddenMeshes>()
        && world.query_filtered::<Entity, With<UsdDeferredMesh>>().iter(world).next().is_none() { return; }
    let registry = world.get_resource::<SchemaRegistry>().cloned().unwrap_or_else(SchemaRegistry::builtin);
    let eager = !world.contains_resource::<DeferHiddenMeshes>() || !registry.builtin_only;
    let pending: Vec<_> = map.iter().filter(|(_, entity)| world.get::<UsdDeferredMesh>(*entity).is_some())
        .filter(|(_, entity)| eager || super::hierarchy_hidden(world, *entity) != Some(true))
        .map(|(_, entity)| entity).collect();
    for entity in pending { world.entity_mut(entity).insert(UsdMeshPreparationQueued); }
    let queue = ordered_queue(world, map);
    if let Some(root) = map.entity("/").filter(|root| world.get_entity(*root).is_ok()) {
        if queue.is_empty() { world.entity_mut(root).remove::<PreparationQueue>(); }
        else { world.entity_mut(root).insert(PreparationQueue(queue)); }
    }
    resume(world, stage, map);
}

pub(crate) fn resume(world: &mut World, stage: &openusd::usd::Stage, map: &crate::live::PrimEntities) {
    if world.get_resource::<SchemaRegistry>().is_none_or(|registry| registry.builtin_only) && exhausted(world) { return; }
    let root = map.entity("/").filter(|root| world.get_entity(*root).is_ok());
    let mut pending = if let Some(root) = root {
        let Some(queue) = world.entity_mut(root).take::<PreparationQueue>() else { return; };
        queue.0
    } else { ordered_queue(world, map) };
    if pending.is_empty() { return; }
    let registry = world.get_resource::<SchemaRegistry>().cloned().unwrap_or_else(SchemaRegistry::builtin);
    prepare(world, stage, &registry, map, &mut pending);
    if !pending.is_empty() && let Some(root) = root && let Ok(mut root) = world.get_entity_mut(root) {
        root.insert(PreparationQueue(pending));
    }
}

fn ordered_queue(world: &World, map: &crate::live::PrimEntities) -> std::collections::VecDeque<Entity> {
    let mut pending: Vec<_> = map.iter().filter(|(_, entity)| world.get::<UsdMeshPreparationQueued>(*entity).is_some()).collect();
    pending.sort_unstable_by(|a, b| a.0.cmp(b.0));
    pending.into_iter().map(|(_, entity)| entity).collect()
}

fn prepare(world: &mut World, stage: &openusd::usd::Stage, registry: &SchemaRegistry, map: &crate::live::PrimEntities,
    pending: &mut std::collections::VecDeque<Entity>) {
    if registry.builtin_only && exhausted(world) { return; }
    let eager = !world.contains_resource::<DeferHiddenMeshes>() || !registry.builtin_only;
    let budget = if registry.builtin_only { world.get_resource::<UsdResidencyBudget>().map(|budget| budget.0) } else { None };
    world.init_resource::<PreparationTurn>();
    while let Some(&entity) = pending.front() {
        if world.get::<UsdMeshPreparationQueued>(entity).is_none() || world.get::<UsdDeferredMesh>(entity).is_none()
            || map.path(entity).is_none() || (!eager && super::hierarchy_hidden(world, entity) == Some(true)) {
            pending.pop_front();
            if let Ok(mut entity) = world.get_entity_mut(entity) { entity.remove::<UsdMeshPreparationQueued>(); }
            continue;
        }
        let turn = world.resource::<PreparationTurn>();
        if budget.is_some_and(|budget| budget != std::time::Duration::MAX && turn.attempts > 0 && turn.spent >= budget) { break; }
        pending.pop_front();
        let path = map.path(entity).unwrap();
        world.resource_mut::<PreparationTurn>().active = Some(entity);
        let started = std::time::Instant::now();
        if let Ok(path) = openusd::sdf::path(path) { registry.project_prim(stage, &path, world, entity); }
        let elapsed = started.elapsed();
        let mut turn = world.resource_mut::<PreparationTurn>();
        turn.active = None;
        turn.spent += elapsed;
        turn.attempts += 1;
        if let Some(mut timing) = world.get_resource_mut::<ResidencyPreparationTiming>() {
            timing.attempts += 1;
            timing.elapsed += elapsed;
            if timing.slowest.as_ref().is_none_or(|(_, previous)| elapsed > *previous) {
                timing.slowest = Some((path.to_owned(), elapsed));
            }
        }
        world.entity_mut(entity).remove::<UsdMeshPreparationQueued>();
    }
}

fn exhausted(world: &World) -> bool {
    world.get_resource::<UsdResidencyBudget>().is_some_and(|budget| budget.0 != std::time::Duration::MAX
        && world.get_resource::<PreparationTurn>().is_some_and(|turn| turn.attempts > 0 && turn.spent >= budget.0))
}

pub(crate) fn attempts(world: &World) -> usize {
    world.get_resource::<PreparationTurn>().map_or(0, |turn| turn.attempts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSource, instance::{UsdInstanceTime, UsdInstances}};

    fn setup() -> (App, Handle<UsdScene>) {
        let source = UsdSource::snapshot("deferred.usda", br#"#usda 1.0
def Xform "Hidden" {
    token visibility = "invisible"
    def Mesh "M" {
        point3f[] points.timeSamples = { 0: [(0,0,0),(1,0,0),(0,1,0)], 10: [(0,0,0),(3,0,0),(0,3,0)] }
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
}
"#.as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), UsdPlugin, UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>().init_resource::<DeferHiddenMeshes>();
        app.insert_resource(crate::asset::UsdProjectionBudget(std::time::Duration::MAX));
        let asset = app.world_mut().resource_mut::<Assets<UsdScene>>().add(UsdScene { source, textures: default() });
        (app, asset)
    }

    fn mesh_entity(app: &App, root: Entity) -> Entity {
        app.world().non_send::<UsdInstances>().entity(root, "/Hidden/M").unwrap()
    }

    #[test]
    fn promotion_budget_is_shared_across_roots_and_samples_latest_clock() {
        let (mut app, asset) = setup();
        app.insert_resource(UsdResidencyBudget(std::time::Duration::ZERO));
        let roots = [0, 1, 2].map(|_| app.world_mut().spawn((UsdSceneRoot(asset.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let entities = roots.map(|root| mesh_entity(&app, root));
        for root in roots {
            let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap().clone();
            stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token("inherited".into())).unwrap();
        }
        app.update();
        assert_eq!(entities.iter().filter(|entity| app.world().get::<Mesh3d>(**entity).is_some()).count(), 1);
        assert_eq!(entities.iter().filter(|entity| app.world().get::<UsdMeshPreparationQueued>(**entity).is_some()).count(), 2);
        let pending: Vec<_> = roots.into_iter().zip(entities).filter(|(_, entity)| app.world().get::<Mesh3d>(*entity).is_none()).collect();
        for (root, _) in &pending { app.world_mut().get_mut::<UsdInstanceTime>(*root).unwrap().current = 10.0; }
        app.update();
        assert_eq!(entities.iter().filter(|entity| app.world().get::<Mesh3d>(**entity).is_some()).count(), 2);
        app.update();
        for (_, entity) in pending {
            let handle = &app.world().get::<Mesh3d>(entity).unwrap().0;
            let mesh = app.world().resource::<Assets<Mesh>>().get(handle).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
            assert!(points.iter().any(|point| point[0] == 3.0));
            assert!(app.world().get::<UsdMeshPreparationQueued>(entity).is_none());
        }
    }

    #[test]
    fn rehidden_queued_mesh_is_not_prepared_and_budget_removal_drains_queue() {
        let (mut app, asset) = setup();
        app.insert_resource(UsdResidencyBudget(std::time::Duration::ZERO));
        let roots = [0, 1, 2].map(|_| app.world_mut().spawn(UsdSceneRoot(asset.clone())).id());
        app.update();
        let entities = roots.map(|root| mesh_entity(&app, root));
        app.world_mut().remove_resource::<DeferHiddenMeshes>();
        app.update();
        let (root, entity) = roots.into_iter().zip(entities).find(|(_, entity)| app.world().get::<Mesh3d>(*entity).is_none()).unwrap();
        app.insert_resource(DeferHiddenMeshes);
        let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap().clone();
        stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token("invisible".into())).unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(entity).is_none());
        assert!(app.world().get::<UsdMeshPreparationQueued>(entity).is_none());
        app.world_mut().remove_resource::<UsdResidencyBudget>();
        app.world_mut().remove_resource::<DeferHiddenMeshes>();
        app.update();
        for entity in entities {
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(app.world().get::<UsdMeshPreparationQueued>(entity).is_none());
        }
    }

    #[test]
    fn retained_queue_discards_despawned_entities() {
        let (mut app, asset) = setup();
        app.insert_resource(UsdResidencyBudget(std::time::Duration::ZERO));
        let roots = [0, 1, 2].map(|_| app.world_mut().spawn(UsdSceneRoot(asset.clone())).id());
        app.update();
        let entities = roots.map(|root| mesh_entity(&app, root));
        app.world_mut().remove_resource::<DeferHiddenMeshes>();
        app.update();
        let removed = entities.into_iter().find(|entity| app.world().get::<UsdMeshPreparationQueued>(*entity).is_some()).unwrap();
        app.world_mut().despawn(removed);
        for _ in 0..3 { app.update(); }
        assert!(app.world().get_entity(removed).is_err());
        for entity in entities.into_iter().filter(|entity| *entity != removed) {
            assert!(app.world().get::<Mesh3d>(entity).is_some());
            assert!(app.world().get::<UsdMeshPreparationQueued>(entity).is_none());
        }
        assert_eq!(app.world_mut().query::<&PreparationQueue>().iter(app.world()).count(), 0);
    }

    #[test]
    fn budget_rotates_between_busy_roots_before_draining_one() {
        let (mut app, asset) = setup();
        app.insert_resource(UsdResidencyBudget(std::time::Duration::ZERO));
        app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&asset).unwrap().source =
            UsdSource::snapshot("fair.usda", br#"#usda 1.0
def Xform "Hidden" {
    token visibility = "invisible"
    def Mesh "M" {
        point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
    def Mesh "N" (prepend references = </Hidden/M>) {}
    def Mesh "O" (prepend references = </Hidden/M>) {}
}
"#.as_slice()).unwrap();
        let roots = [0, 1, 2].map(|_| app.world_mut().spawn(UsdSceneRoot(asset.clone())).id());
        app.update();
        let entities = roots.map(|root| ["/Hidden/M", "/Hidden/N", "/Hidden/O"].map(|path|
            app.world().non_send::<UsdInstances>().entity(root, path).unwrap()));
        app.world_mut().remove_resource::<DeferHiddenMeshes>();
        for round in 1..=3 {
            for _ in 0..3 { app.update(); }
            for root_entities in entities {
                assert_eq!(root_entities.iter().filter(|entity| app.world().get::<Mesh3d>(**entity).is_some()).count(), round);
            }
        }
    }

    #[test]
    fn hidden_edits_and_clock_changes_are_read_on_reveal_and_residency_is_retained() {
        let (mut app, asset) = setup();
        let roots = [0, 1].map(|_| app.world_mut().spawn((UsdSceneRoot(asset.clone()), UsdInstanceTime { current: 0.0 })).id());
        app.update();
        let entities = roots.map(|root| mesh_entity(&app, root));
        for entity in entities {
            assert!(app.world().get::<UsdDeferredMesh>(entity).is_some());
            assert!(app.world().get::<Mesh3d>(entity).is_none());
        }
        app.world_mut().get_mut::<UsdInstanceTime>(roots[0]).unwrap().current = 10.0;
        let stage = app.world().non_send::<UsdInstances>().stage(roots[0]).unwrap().clone();
        stage.attribute("/Hidden/M.points").unwrap().set_at(openusd::sdf::Value::Vec3fVec(
            vec![[0.0,0.0,0.0].into(), [7.0,0.0,0.0].into(), [0.0,7.0,0.0].into()]), openusd::usd::TimeCode::new(10.0)).unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(entities[0]).is_none());
        stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token("inherited".into())).unwrap();
        app.update();
        let handle = app.world().get::<Mesh3d>(entities[0]).unwrap().0.clone();
        assert!(app.world().get::<UsdDeferredMesh>(entities[0]).is_none());
        assert!(app.world().get::<Mesh3d>(entities[1]).is_none());
        let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
        assert!(points.iter().any(|point| point[0] == 7.0));
        for value in ["invisible", "inherited"] {
            stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token(value.into())).unwrap();
            app.update();
            assert_eq!(app.world().get::<Mesh3d>(entities[0]).unwrap().0, handle);
        }
        app.world_mut().remove_resource::<DeferHiddenMeshes>();
        app.update();
        assert!(app.world().get::<Mesh3d>(entities[1]).is_some());
    }

    #[test]
    fn purpose_changes_materialize_deferred_instance_meshes() {
        let (mut app, asset) = setup();
        let root = app.world_mut().spawn(UsdSceneRoot(asset)).id();
        app.update();
        let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap().clone();
        stage.create_attribute(openusd::sdf::path("/Hidden.purpose").unwrap(), "token").unwrap()
            .set(openusd::sdf::Value::Token("guide".into())).unwrap();
        stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token("inherited".into())).unwrap();
        app.update();
        assert!(app.world().get::<Mesh3d>(mesh_entity(&app, root)).is_none());
        app.insert_resource(super::super::DisplayPurposes { guide: true, ..default() });
        app.update();
        assert!(app.world().get::<Mesh3d>(mesh_entity(&app, root)).is_some());
    }

    struct RequiresMesh;
    impl PrimRoute for RequiresMesh {
        fn matches(&self, ctx: &RouteCtx) -> bool { ctx.prim_str() == "/Hidden/M" }
        fn project(&self, _: &RouteCtx, world: &mut World, entity: Entity) { assert!(world.get::<Mesh3d>(entity).is_some()); }
    }

    #[test]
    fn registering_custom_route_materializes_existing_deferred_meshes() {
        let (mut app, asset) = setup();
        let root = app.world_mut().spawn(UsdSceneRoot(asset)).id();
        app.update();
        let entity = mesh_entity(&app, root);
        assert!(app.world().get::<UsdDeferredMesh>(entity).is_some());
        app.world_mut().resource_mut::<SchemaRegistry>().register(RequiresMesh);
        app.update();
        assert!(app.world().get::<Mesh3d>(entity).is_some());
        assert!(app.world().get::<UsdDeferredMesh>(entity).is_none());
    }

    #[test]
    fn hidden_source_reload_preserves_entities_and_reveals_new_geometry() {
        let (mut app, asset) = setup();
        let root = app.world_mut().spawn(UsdSceneRoot(asset.clone())).id();
        app.update();
        let entity = mesh_entity(&app, root);
        app.world_mut().entity_mut(entity).insert(Name::new("runtime marker"));
        let replacement = UsdSource::snapshot("deferred.usda", br#"#usda 1.0
def Xform "Hidden" {
    token visibility = "invisible"
    def Mesh "M" {
        point3f[] points = [(0,0,0),(9,0,0),(0,9,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
}
"#.as_slice()).unwrap();
        app.world_mut().resource_mut::<Assets<UsdScene>>().get_mut(&asset).unwrap().source = replacement;
        for _ in 0..3 { app.update(); }
        assert_eq!(mesh_entity(&app, root), entity);
        assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime marker");
        assert!(app.world().get::<UsdDeferredMesh>(entity).is_some());
        assert!(app.world().get::<Mesh3d>(entity).is_none());
        let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap().clone();
        stage.attribute("/Hidden.visibility").unwrap().set(openusd::sdf::Value::Token("inherited".into())).unwrap();
        app.update();
        let handle = &app.world().get::<Mesh3d>(entity).unwrap().0;
        let mesh = app.world().resource::<Assets<Mesh>>().get(handle).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
        assert!(points.iter().any(|point| point[0] == 9.0));
    }

    #[test]
    fn animated_ancestor_visibility_promotes_at_current_clock() {
        let (mut app, asset) = setup();
        let root = app.world_mut().spawn((UsdSceneRoot(asset), UsdInstanceTime { current: 0.0 })).id();
        app.update();
        let entity = mesh_entity(&app, root);
        let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap().clone();
        for (time, value) in [(0.0, "invisible"), (10.0, "inherited")] {
            stage.attribute("/Hidden.visibility").unwrap().set_at(openusd::sdf::Value::Token(value.into()), openusd::usd::TimeCode::new(time)).unwrap();
        }
        app.update();
        assert!(app.world().get::<Mesh3d>(entity).is_none());
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 10.0;
        app.update();
        let handle = app.world().get::<Mesh3d>(entity).unwrap().0.clone();
        let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
        assert!(points.iter().any(|point| point[0] == 3.0));
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 0.0;
        app.update();
        assert!(app.world().get::<Mesh3d>(entity).is_some());
        assert!(app.world().get::<UsdDeferredMesh>(entity).is_none());
    }

    #[test]
    fn custom_registry_keeps_eager_geometry_contract() {
        let (mut app, asset) = setup();
        app.world_mut().resource_mut::<SchemaRegistry>().register(RequiresMesh);
        let root = app.world_mut().spawn(UsdSceneRoot(asset)).id();
        app.update();
        assert!(app.world().get::<Mesh3d>(mesh_entity(&app, root)).is_some());
        assert!(app.world().get::<UsdDeferredMesh>(mesh_entity(&app, root)).is_none());
    }
}
