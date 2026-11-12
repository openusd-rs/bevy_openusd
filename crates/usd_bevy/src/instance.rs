//! Main-thread live stages owned by individual USD asset roots.

use std::collections::HashMap;

use bevy::prelude::*;
use openusd::usd::Stage;

use crate::asset::SnapshotTextures;
use crate::live::{AnimatedPrims, LiveStage, PrimEntities, apply_changes};
use crate::route::{SchemaRegistry, StageTime};

mod animation;

/// An attribute opinion reapplied after source reloads.
#[derive(Debug, Clone, PartialEq)]
pub struct UsdAttributeOverride {
    pub prim: String,
    pub name: String,
    pub type_name: String,
    pub value: openusd::vt::Value,
}

/// Root-local opinions applied in order: variants, then attributes.
/// Removing an opinion restores the source snapshot on the next update.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdInstanceOverrides {
    pub variants: Vec<(String, String, String)>,
    pub attributes: Vec<UsdAttributeOverride>,
}

impl UsdInstanceOverrides {
    /// Patches encodable component opinions that are reapplied after source reloads.
    /// Absent options retain earlier overrides; stage validation occurs during projection.
    pub fn set_component<T: Component + Reflect + TypePath>(
        &mut self,
        registry: &bevy::reflect::TypeRegistry,
        prim: &str,
        component: &T,
    ) -> anyhow::Result<Vec<String>> {
        let opinions = crate::sync::component_value_overrides(registry, prim, component)?;
        let names: Vec<_> = opinions
            .iter()
            .map(|opinion| opinion.name.clone())
            .collect();
        self.attributes
            .retain(|opinion| opinion.prim != prim || !names.contains(&opinion.name));
        self.attributes.extend(opinions);
        Ok(names)
    }

    pub(crate) fn apply(&self, stage: &Stage) -> anyhow::Result<()> {
        for (prim, set, selection) in &self.variants {
            crate::authoring::set_variant(stage, prim, set, selection)?;
        }
        for attribute in &self.attributes {
            crate::authoring::set_attribute(
                stage,
                &attribute.prim,
                &attribute.name,
                &attribute.type_name,
                attribute.value.clone(),
            )?;
        }
        Ok(())
    }
}

/// Animation position in USD time codes. Each root has its own clock.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdInstanceTime {
    pub current: f64,
}

/// Per-root playback controls. Speed is a signed multiplier of stage time codes
/// per second; loop ranges are half-open and measured in USD time codes.
#[derive(Component, Debug, Clone, Copy)]
pub struct UsdPlayback {
    pub playing: bool,
    pub speed: f64,
    pub looping: bool,
    pub range: Option<(f64, f64)>,
}

impl Default for UsdPlayback {
    fn default() -> Self {
        Self {
            playing: false,
            speed: 1.0,
            looping: true,
            range: None,
        }
    }
}

impl UsdPlayback {
    pub(crate) fn advance(&mut self, current: f64, delta: f64, stage: &Stage) -> f64 {
        if !self.playing || !delta.is_finite() || delta <= 0.0 {
            return current;
        }
        let rate = stage.time_codes_per_second();
        let (start, end) = self
            .range
            .unwrap_or((stage.start_time_code(), stage.end_time_code()));
        let next = current + delta * rate * self.speed;
        if !next.is_finite()
            || !rate.is_finite()
            || rate <= 0.0
            || !start.is_finite()
            || !end.is_finite()
            || end < start
            || !(end - start).is_finite()
        {
            self.playing = false;
            return current;
        }
        if end == start {
            self.playing = false;
            return start;
        }
        if self.looping {
            start + (next - start).rem_euclid(end - start)
        } else {
            if (self.speed > 0.0 && next >= end) || (self.speed < 0.0 && next <= start) {
                self.playing = false;
            }
            next.clamp(start, end)
        }
    }
}

/// Independent live stages, accessed as a Bevy non-send resource.
/// Direct stage edits are transient and are replaced on source reload.
#[derive(Default)]
pub struct UsdInstances {
    pub(crate) roots: HashMap<Entity, InstanceRuntime>,
}

/// Opt-in asset-root animation timings; classification is included in total time.
#[derive(Resource, Default)]
pub struct UsdInstanceAnimationTiming {
    pub updates: u64,
    pub classified: u64,
    pub reused: u64,
    pub animated: u64,
    pub classification: std::time::Duration,
    pub total: std::time::Duration,
}

impl UsdInstances {
    pub fn stage(&self, root: Entity) -> Option<&Stage> {
        self.roots.get(&root).map(|runtime| &runtime.live.stage)
    }

    pub fn entity(&self, root: Entity, path: &str) -> Option<Entity> {
        self.roots.get(&root)?.map.entity(path)
    }

    /// The prim path of an entity projected under `root`.
    pub fn path(&self, root: Entity, entity: Entity) -> Option<&str> {
        self.roots.get(&root)?.map.path(entity)
    }

    /// Every `(prim path, entity)` projected under `root`, the stage root
    /// `/` included.
    pub fn prims(&self, root: Entity) -> impl Iterator<Item = (&str, Entity)> {
        self.roots
            .get(&root)
            .into_iter()
            .flat_map(|runtime| runtime.map.iter())
    }

    /// The scene roots with a projected stage.
    pub fn roots(&self) -> impl Iterator<Item = Entity> + '_ {
        self.roots.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

pub(crate) struct InstanceRuntime {
    pub asset: bevy::asset::AssetId<crate::asset::UsdScene>,
    pub live: LiveStage,
    pub map: PrimEntities,
    pub textures: SnapshotTextures,
    pub sampled: f64,
    pub animation: animation::AnimationCache,
    pub purposes: crate::route::DisplayPurposes,
    pub defer_hidden: bool,
    /// The projection still running for this root, spread over frames.
    pub job: Option<crate::live::ProjectionJob>,
    pub subdivision_levels: Option<u32>,
    pub curve_steps: (usize, Option<usize>),
    /// Mesh and material handles of prims a variant switch turned off, kept
    /// so switching back finds them interned instead of uploading again.
    pub parked: bevy::platform::collections::HashMap<
        bevy::asset::UntypedAssetId,
        bevy::asset::UntypedHandle,
    >,
    /// Material memo for variant switches on this runtime's own stage.
    pub materials: Option<crate::route::material::ProjectionMaterials>,
    /// Units and pending work of a root with [`UsdStreaming`](crate::streaming::UsdStreaming).
    pub streaming: Option<crate::streaming::StreamingState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_timing_is_optional_and_counts_only_changed_root_clocks() {
        let source = crate::UsdSource::new(
            "instance-animation-timing.usda",
            br#"#usda 1.0
def Xform "Fixed" {}
def Xform "Mover" {
    double3 xformOp:translate.timeSamples = {0: (0,0,0), 10: (10,0,0)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#
            .to_vec(),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::UsdPlugin,
            crate::UsdAssetPlugin,
        ));
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(crate::UsdProjectionBudget(std::time::Duration::MAX));
        let handle = app
            .world_mut()
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source,
                textures: default(),
            });
        let roots = [0, 1].map(|_| {
            app.world_mut()
                .spawn((
                    crate::UsdSceneRoot(handle.clone()),
                    UsdInstanceTime::default(),
                ))
                .id()
        });
        for _ in 0..8 {
            app.update();
            if roots.iter().all(|root| {
                app.world().get::<crate::asset::UsdSceneState>(*root)
                    == Some(&crate::asset::UsdSceneState::Ready)
            }) {
                break;
            }
        }
        for root in roots {
            assert_eq!(
                app.world().get::<crate::asset::UsdSceneState>(root),
                Some(&crate::asset::UsdSceneState::Ready),
            );
        }
        app.world_mut()
            .get_mut::<UsdInstanceTime>(roots[0])
            .unwrap()
            .current = 2.0;
        app.update();
        assert!(
            !app.world()
                .contains_resource::<UsdInstanceAnimationTiming>()
        );
        app.init_resource::<UsdInstanceAnimationTiming>();
        for (root, clock) in roots.into_iter().zip([4.0, 7.0]) {
            app.world_mut()
                .get_mut::<UsdInstanceTime>(root)
                .unwrap()
                .current = clock;
        }
        app.update();
        let timing = app.world().resource::<UsdInstanceAnimationTiming>();
        assert_eq!(
            (
                timing.updates,
                timing.classified,
                timing.reused,
                timing.animated
            ),
            (2, 2, 2, 2)
        );
        assert!(timing.total >= timing.classification);
        for (root, clock) in roots.into_iter().zip([4.0, 7.0]) {
            let mover = app
                .world()
                .get_non_send::<UsdInstances>()
                .unwrap()
                .entity(root, "/Mover")
                .unwrap();
            assert_eq!(
                app.world().get::<Transform>(mover).unwrap().translation.x,
                clock
            );
        }
        app.update();
        assert_eq!(
            app.world().resource::<UsdInstanceAnimationTiming>().updates,
            2
        );
        app.world_mut()
            .get_mut::<UsdInstanceTime>(roots[0])
            .unwrap()
            .current = 5.0;
        app.update();
        let timing = app.world().resource::<UsdInstanceAnimationTiming>();
        assert_eq!(
            (
                timing.updates,
                timing.classified,
                timing.reused,
                timing.animated
            ),
            (3, 2, 4, 3)
        );
    }

    #[test]
    fn inherited_normals_stay_independent_across_roots_and_prototypes() {
        use crate::route::{instancer::UsdInstance, subset::UsdSubset};
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/inherited_normals.usda"
        );
        let source = crate::UsdSource::new(
            "inherited-instance.usda",
            format!(
                r#"#usda 1.0
( subLayers = [@{fixture}@] )
over "Root" {{
    over "M" {{
        def GeomSubset "Part" {{
            uniform token familyName = "materialBind"
            uniform token elementType = "face"
            int[] indices = [0]
        }}
    }}
}}
def PointInstancer "PI" {{
    point3f[] positions = [(0,0,0),(3,0,0)]
    int[] protoIndices = [0,0]
    rel prototypes = [</Root/M>]
}}
"#
            )
            .into_bytes(),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::UsdPlugin,
            crate::UsdAssetPlugin,
        ));
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source,
                textures: default(),
            });
        let roots = [0, 1].map(|_| {
            app.world_mut()
                .spawn((
                    crate::UsdSceneRoot(handle.clone()),
                    UsdInstanceTime { current: 0.0 },
                ))
                .id()
        });
        app.insert_resource(crate::UsdProjectionBudget(std::time::Duration::MAX));
        app.update();
        for root in roots {
            assert_eq!(
                app.world().get::<crate::asset::UsdSceneState>(root),
                Some(&crate::asset::UsdSceneState::Ready),
            );
        }
        let lookup = |world: &World, root, path| {
            world
                .get_non_send::<UsdInstances>()
                .unwrap()
                .entity(root, path)
                .unwrap()
        };
        let parents = roots.map(|root| lookup(app.world(), root, "/Root/M"));
        let child = app.world_mut().spawn(ChildOf(parents[0])).id();
        let check = |world: &World, expected: [[f32; 3]; 2]| {
            for (index, root) in roots.into_iter().enumerate() {
                assert_eq!(lookup(world, root, "/Root/M"), parents[index]);
                let pi = lookup(world, root, "/PI");
                let prototypes: Vec<_> = world
                    .get::<Children>(pi)
                    .unwrap()
                    .iter()
                    .filter(|entity| world.get::<UsdInstance>(*entity).is_some())
                    .collect();
                assert_eq!(prototypes.len(), 2);
                assert_eq!(
                    world.get::<Mesh3d>(prototypes[0]).unwrap().0,
                    world.get::<Mesh3d>(prototypes[1]).unwrap().0
                );
                for parent in std::iter::once(parents[index]).chain(prototypes) {
                    let subsets: Vec<_> = world
                        .get::<Children>(parent)
                        .unwrap()
                        .iter()
                        .filter(|entity| world.get::<UsdSubset>(*entity).is_some())
                        .collect();
                    assert_eq!(subsets.len(), 1);
                    for entity in std::iter::once(parent).chain(subsets) {
                        let mesh = world
                            .resource::<Assets<Mesh>>()
                            .get(&world.get::<Mesh3d>(entity).unwrap().0)
                            .unwrap();
                        let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) =
                            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
                        else {
                            panic!("normals")
                        };
                        assert_eq!(normals.len(), mesh.count_vertices());
                        if entity != parent {
                            assert!(normals.len() >= 4);
                        }
                        assert!(normals.iter().all(|normal| *normal == expected[index]));
                        assert_eq!(
                            mesh.indices().unwrap().len(),
                            if entity == parent { 0 } else { 6 }
                        );
                    }
                }
            }
            assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), parents[0]);
        };
        for times in [[0.0, 10.0], [10.0, 0.0], [0.0, 0.0]] {
            for (root, current) in roots.into_iter().zip(times) {
                app.world_mut()
                    .get_mut::<UsdInstanceTime>(root)
                    .unwrap()
                    .current = current;
            }
            app.update();
            check(
                app.world(),
                times.map(|time| {
                    if time == 0.0 {
                        [0.0, 0.0, 1.0]
                    } else {
                        [1.0, 0.0, 0.0]
                    }
                }),
            );
        }
        let untouched = app.world().get::<Mesh3d>(parents[1]).unwrap().0.clone();
        let stage = app
            .world()
            .get_non_send::<UsdInstances>()
            .unwrap()
            .stage(roots[0])
            .unwrap()
            .clone();
        stage
            .attribute("/Root.primvars:normals")
            .unwrap()
            .set_at(
                openusd::vt::Value::Vec3fVec(vec![openusd::gf::Vec3f::from([0.0, 1.0, 0.0])]),
                openusd::usd::TimeCode::new(0.0),
            )
            .unwrap();
        app.update();
        check(app.world(), [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(app.world().get::<Mesh3d>(parents[1]).unwrap().0, untouched);
        stage
            .create_attribute("/Root/M.primvars:normals", "normal3f[]")
            .unwrap()
            .set(openusd::vt::Value::Vec3fVec(vec![
                openusd::gf::Vec3f::from([0.0, 0.0, -1.0]),
            ]))
            .unwrap();
        app.update();
        check(app.world(), [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0]]);
        crate::authoring::clear_attribute(&stage, "/Root/M", "primvars:normals").unwrap();
        app.update();
        check(app.world(), [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        stage
            .attribute("/Root.primvars:normals")
            .unwrap()
            .block()
            .unwrap();
        app.update();
        check(app.world(), [[0.0, 0.0, 1.0]; 2]);
        assert_eq!(app.world().get::<Mesh3d>(parents[1]).unwrap().0, untouched);
    }

    #[test]
    fn typed_override_encoding_failures_do_not_mutate_existing_opinions() {
        #[derive(Component, Reflect)]
        #[reflect(Component)]
        struct Invalid {
            value: f64,
            bytes: Vec<u8>,
        }
        let mut registry = bevy::reflect::TypeRegistry::default();
        registry.register::<Invalid>();
        let mut overrides = UsdInstanceOverrides {
            variants: vec![("/Prim".into(), "shape".into(), "a".into())],
            attributes: vec![UsdAttributeOverride {
                prim: "/Prim".into(),
                name: "custom".into(),
                type_name: "int".into(),
                value: openusd::vt::Value::Int(7),
            }],
        };
        let before = overrides.clone();
        for prim in ["/Prim", "/", "relative"] {
            assert!(
                overrides
                    .set_component(
                        &registry,
                        prim,
                        &Invalid {
                            value: 12.0,
                            bytes: vec![1]
                        }
                    )
                    .is_err()
            );
            assert_eq!(overrides, before);
        }
    }

    #[test]
    fn playback_handles_rate_reverse_boundaries_and_invalid_ranges() {
        let stage = crate::UsdSource::new("clock.usda", &b"#usda 1.0\n"[..])
            .unwrap()
            .open_stage()
            .unwrap();
        stage.set_time_codes_per_second(10.0).unwrap();
        stage.set_start_time_code(2.0).unwrap();
        stage.set_end_time_code(12.0).unwrap();
        let mut playback = UsdPlayback {
            playing: true,
            ..default()
        };
        assert_eq!(playback.advance(2.0, 0.5, &stage), 7.0);
        assert_eq!(playback.advance(2.0, 1.0, &stage), 2.0);
        playback.speed = -1.0;
        assert_eq!(playback.advance(2.0, 0.5, &stage), 7.0);
        playback.looping = false;
        assert_eq!(playback.advance(3.0, 0.5, &stage), 2.0);
        assert!(!playback.playing);
        playback.playing = true;
        playback.range = Some((5.0, 5.0));
        assert_eq!(playback.advance(3.0, 0.5, &stage), 5.0);
        assert!(!playback.playing);
        for range in [(10.0, 0.0), (f64::NAN, 2.0), (0.0, f64::INFINITY)] {
            playback.playing = true;
            playback.range = Some(range);
            assert_eq!(playback.advance(3.0, 0.5, &stage), 3.0);
            assert!(!playback.playing);
        }
    }
}

#[derive(Resource, Default)]
struct ResidencyTurn(Option<Entity>);

pub(crate) fn tick(world: &mut World, instances: &mut UsdInstances) {
    let delta = world
        .get_resource::<Time>()
        .map_or(0.0, Time::delta_secs_f64);
    let subdivision_levels = crate::route::subdivision::current_levels(world);
    let curve_steps = crate::route::curves::current_geometry_key(world);
    let revealed = crate::route::residency::revealed(world);
    let mut roots: Vec<_> = instances.roots.keys().copied().collect();
    roots.sort_unstable();
    if let Some(last) = world
        .get_resource::<ResidencyTurn>()
        .and_then(|turn| turn.0)
    {
        let start = roots.partition_point(|root| *root <= last);
        roots.rotate_left(start);
    }
    for root in roots {
        let runtime = instances.roots.get_mut(&root).unwrap();
        // A stage still projecting is not synced or animated yet.
        if runtime.job.is_some() {
            continue;
        }
        let attempts = crate::route::residency::attempts(world);
        let mut current = world
            .get::<UsdInstanceTime>(root)
            .map_or(0.0, |time| time.current);
        if let Some(mut playback) = world.get_mut::<UsdPlayback>(root) {
            current = playback.advance(current, delta, &runtime.live.stage);
        }
        if let Some(mut time) = world.get_mut::<UsdInstanceTime>(root) {
            time.current = current;
        }
        let old_time = world.remove_resource::<StageTime>();
        let old_animated = world.remove_resource::<AnimatedPrims>();
        let old_textures = world.remove_resource::<SnapshotTextures>();
        world.insert_resource(StageTime { current });
        world.insert_resource(runtime.textures.clone());
        let defer_hidden = crate::route::residency::enabled(world);
        if defer_hidden != runtime.defer_hidden {
            crate::route::residency::materialize(world, &runtime.live.stage, &runtime.map);
            runtime.defer_hidden = defer_hidden;
        }
        apply_changes(world, &runtime.live, &mut runtime.map);
        if revealed {
            crate::route::residency::materialize(world, &runtime.live.stage, &runtime.map);
        }
        let purposes = world
            .get_resource::<crate::route::DisplayPurposes>()
            .copied()
            .unwrap_or_default();
        if purposes != runtime.purposes {
            let registry = world.resource::<SchemaRegistry>().clone();
            for (path, entity) in runtime.map.iter() {
                if let Ok(path) = openusd::sdf::path(path) {
                    registry.patch_prim(&runtime.live.stage, &path, world, entity, &["purpose"]);
                }
            }
            crate::route::residency::materialize(world, &runtime.live.stage, &runtime.map);
            runtime.purposes = purposes;
        }
        if subdivision_levels != runtime.subdivision_levels {
            crate::route::subdivision::refresh_geometry(world, &runtime.live.stage, &runtime.map);
            runtime.subdivision_levels = subdivision_levels;
        }
        if curve_steps != runtime.curve_steps {
            crate::route::curves::refresh_geometry(world, &runtime.live.stage, &runtime.map);
            runtime.curve_steps = curve_steps;
        }
        if current != runtime.sampled {
            let started = world
                .contains_resource::<UsdInstanceAnimationTiming>()
                .then(bevy::platform::time::Instant::now);
            let mut classified = 0;
            let mut reused = 0;
            let mut animated = 0;
            let mut classification = std::time::Duration::ZERO;
            let registry = world.resource::<SchemaRegistry>().clone();
            let cacheable = registry.is_builtin_only();
            registry.with_materials(world, &runtime.live, |world| {
                let prims = runtime.map.iter().filter_map(|(path, entity)| {
                    let path = openusd::sdf::path(path).ok()?;
                    if path.as_str() == "/" {
                        return None;
                    }
                    let begin = started.map(|_| bevy::platform::time::Instant::now());
                    let (sample, cached) =
                        runtime.animation.sample(&runtime.live, &path, cacheable);
                    if let Some(begin) = begin {
                        classified += u64::from(!cached);
                        reused += u64::from(cached);
                        animated += u64::from(sample.animated);
                        classification += begin.elapsed();
                    }
                    sample
                        .animated
                        .then_some((path, entity, sample.deformation_only))
                });
                if !cacheable {
                    let prims = prims.map(|(path, entity, _)| (path, entity));
                    registry.patch_animation(&runtime.live.stage, prims, world);
                    return;
                }
                let prims: Vec<_> = prims.collect();
                for (_, entity, deformation_only) in &prims {
                    let Ok(mut entity) = world.get_entity_mut(*entity) else {
                        continue;
                    };
                    let marked = entity.contains::<crate::live::DeformationOnlyAnimation>();
                    if *deformation_only && !marked {
                        entity.insert(crate::live::DeformationOnlyAnimation);
                    } else if !*deformation_only && marked {
                        entity.remove::<crate::live::DeformationOnlyAnimation>();
                    }
                }
                let prims = prims.into_iter().map(|(path, entity, _)| (path, entity));
                registry.patch_animation(&runtime.live.stage, prims, world);
            });
            if let Some(started) = started {
                let elapsed = started.elapsed();
                let mut timing = world.resource_mut::<UsdInstanceAnimationTiming>();
                timing.updates += 1;
                timing.classified += classified;
                timing.reused += reused;
                timing.animated += animated;
                timing.classification += classification;
                timing.total += elapsed;
            }
            runtime.sampled = current;
            crate::route::residency::materialize(world, &runtime.live.stage, &runtime.map);
        }
        crate::route::residency::resume(world, &runtime.live.stage, &runtime.map);
        if crate::route::residency::attempts(world) != attempts {
            world.insert_resource(ResidencyTurn(Some(root)));
        }
        world.remove_resource::<StageTime>();
        world.remove_resource::<AnimatedPrims>();
        world.remove_resource::<SnapshotTextures>();
        if let Some(value) = old_time {
            world.insert_resource(value);
        }
        if let Some(value) = old_animated {
            world.insert_resource(value);
        }
        if let Some(value) = old_textures {
            world.insert_resource(value);
        }
    }
}
