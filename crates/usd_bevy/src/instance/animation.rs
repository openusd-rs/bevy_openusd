//! Bounded root-local animation classification for the current composition.

use crate::live::{LiveStage, animated_only_by_deformation, prim_is_animated};
use openusd::{sdf::Path, usd::Stage};
use std::collections::HashMap;

#[derive(PartialEq, Eq)]
struct Revision {
    authored: u64,
    layers: Vec<String>,
    load_rules: openusd::pcp::LoadRules,
    mask: openusd::usd::StagePopulationMask,
    muted: Vec<String>,
}

impl Revision {
    fn read(live: &LiveStage) -> Option<Self> {
        live.stage.flush().ok()?;
        Self::at(&live.stage, live.composition_revision())
    }

    fn at(stage: &Stage, authored: u64) -> Option<Self> {
        (authored != u64::MAX).then(|| Self {
            authored,
            layers: stage.layer_identifiers(),
            load_rules: stage.load_rules(),
            mask: stage.mask(),
            muted: stage.muted_layers(),
        })
    }
}

/// Whether a prim animates, and whether only its deformation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Animation {
    pub animated: bool,
    pub deformation_only: bool,
}

impl Animation {
    fn classify(stage: &Stage, path: &Path) -> Self {
        let animated = prim_is_animated(stage, path);
        Self {
            animated,
            deformation_only: animated && animated_only_by_deformation(stage, path),
        }
    }
}

/// Animated prims of a root map with whether only their deformation animates.
pub(super) type AnimatedPrims = Vec<(Path, bevy::prelude::Entity, bool)>;

pub(crate) struct AnimationCache {
    revision: Option<Revision>,
    entries: HashMap<Path, Animation>,
    animated: Option<(usize, AnimatedPrims)>,
    classified: u64,
    path_bytes: usize,
    max_paths: usize,
    max_path_bytes: usize,
}

impl Default for AnimationCache {
    fn default() -> Self {
        Self {
            revision: None,
            entries: HashMap::new(),
            animated: None,
            classified: 0,
            path_bytes: 0,
            max_paths: 65_536,
            max_path_bytes: 8 * 1024 * 1024,
        }
    }
}

impl AnimationCache {
    fn clear(&mut self) {
        self.revision = None;
        self.entries = HashMap::new();
        self.animated = None;
        self.path_bytes = 0;
    }

    /// Validates retained classifications against the current composition;
    /// without `retain` every later lookup classifies afresh.
    pub(super) fn begin(&mut self, live: &LiveStage, retain: bool) {
        let revision = retain.then(|| Revision::read(live)).flatten();
        if self.revision != revision || revision.is_none() {
            self.clear();
            self.revision = revision;
        }
    }

    fn lookup(&mut self, stage: &Stage, path: &Path) -> (Animation, bool) {
        if let Some(&animated) = self.entries.get(path) {
            return (animated, true);
        }
        let animated = Animation::classify(stage, path);
        if self.revision.is_some()
            && self.entries.len() < self.max_paths
            && let Some(bytes) = self.path_bytes.checked_add(path.as_str().len())
            && bytes <= self.max_path_bytes
        {
            self.entries.insert(path.clone(), animated);
            self.path_bytes = bytes;
        }
        (animated, false)
    }

    pub(super) fn sample(
        &mut self,
        live: &LiveStage,
        path: &Path,
        retain: bool,
    ) -> (Animation, bool) {
        self.begin(live, retain);
        self.lookup(&live.stage, path)
    }

    /// Animated prims of `map`, reclassified only after [`Self::begin`] sees
    /// a composition change; `counts` receives fresh and reused lookups.
    pub(super) fn animated_prims(
        &mut self,
        stage: &Stage,
        map: &crate::live::PrimEntities,
        counts: &mut [u64; 2],
    ) -> AnimatedPrims {
        if self.revision.is_some()
            && let Some((len, prims)) = &self.animated
            && *len == map.len()
        {
            counts[1] += self.classified;
            return prims.clone();
        }
        let before = counts[0] + counts[1];
        let prims: AnimatedPrims = map
            .iter()
            .filter_map(|(path, entity)| {
                let path = openusd::sdf::path(path).ok()?;
                if path.as_str() == "/" {
                    return None;
                }
                let (animation, cached) = self.lookup(stage, &path);
                counts[usize::from(cached)] += 1;
                animation
                    .animated
                    .then_some((path, entity, animation.deformation_only))
            })
            .collect();
        if self.revision.is_some() {
            self.classified = counts[0] + counts[1] - before;
            self.animated = Some((map.len(), prims.clone()));
        }
        prims
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{UsdInstanceTime, UsdInstances};
    use bevy::prelude::*;
    use openusd::{usd::TimeCode, vt::Value};

    fn source(text: &str) -> crate::UsdSource {
        crate::UsdSource::new("animation-cache.usda", text.as_bytes()).unwrap()
    }

    fn check(cache: &mut AnimationCache, live: &LiveStage, name: &str, expected: bool) {
        live.stage.flush().unwrap();
        let path = openusd::sdf::path(name).unwrap();
        assert_eq!(
            prim_is_animated(&live.stage, &path),
            expected,
            "fresh {name}, prior entries {:?}, layers {:?}",
            cache.entries,
            live.stage.layer_identifiers()
        );
        assert_eq!(
            cache.sample(live, &path, true).0.animated,
            expected,
            "cached {name}"
        );
        assert_eq!(
            cache.sample(live, &path, true),
            (Animation::classify(&live.stage, &path), true)
        );
    }

    fn animated((animation, cached): (Animation, bool)) -> (bool, bool) {
        (animation.animated, cached)
    }

    #[test]
    fn classification_matches_fresh_deformation_clips_and_prototype_inputs() {
        for fixture in [
            "inherited_normals.usda",
            "material_subsets.usda",
            "skel_morph_blended_animated.usda",
            "retimed_instances.usda",
            "switching_clips.usda",
            "point_curve_animation.usda",
            "point_deformation.usda",
        ] {
            let file = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))
                .join(fixture);
            let live = LiveStage::new(
                crate::UsdSource::from_file(&file)
                    .unwrap()
                    .open_stage()
                    .unwrap(),
            );
            let mut paths = Vec::new();
            live.stage
                .traverse(
                    openusd::usd::PrimPredicate::default().with_instance_proxies(true),
                    |path| paths.push(path.clone()),
                )
                .unwrap();
            let expected: Vec<_> = paths
                .iter()
                .map(|path| prim_is_animated(&live.stage, path))
                .collect();
            assert!(!paths.is_empty());
            let mut cache = AnimationCache::default();
            for (&animated, path) in expected.iter().zip(&paths) {
                check(&mut cache, &live, path.as_str(), animated);
            }
            for path in paths.iter().rev().chain(&paths) {
                assert_eq!(
                    cache.sample(&live, path, true),
                    (Animation::classify(&live.stage, path), true)
                );
            }
        }
    }

    #[test]
    fn animated_prim_lists_are_reused_until_composition_changes() {
        let live = LiveStage::new(
            source(
                "#usda 1.0\ndef Sphere \"A\" { double radius = 1 }\ndef Sphere \"B\" { double radius.timeSamples = {0: 1, 10: 2} }\n",
            )
            .open_stage()
            .unwrap(),
        );
        let mut world = World::new();
        let mut map = crate::live::PrimEntities::default();
        for path in ["/", "/A", "/B"] {
            map.insert(path, world.spawn_empty().id());
        }
        let names = |prims: AnimatedPrims| {
            let mut names: Vec<_> = prims
                .iter()
                .map(|(path, _, _)| path.as_str().to_owned())
                .collect();
            names.sort();
            names
        };
        let mut cache = AnimationCache::default();
        let mut counts = [0; 2];
        for reused in [0, 2] {
            cache.begin(&live, true);
            assert_eq!(
                names(cache.animated_prims(&live.stage, &map, &mut counts)),
                ["/B"]
            );
            assert_eq!(counts, [2, reused]);
        }
        live.stage
            .attribute("/A.radius")
            .unwrap()
            .set_at(Value::Double(2.0), TimeCode::new(5.0))
            .unwrap();
        live.try_drain_changes().unwrap();
        cache.begin(&live, true);
        assert_eq!(
            names(cache.animated_prims(&live.stage, &map, &mut counts)),
            ["/A", "/B"]
        );
        assert_eq!(counts, [4, 2]);
    }

    #[test]
    fn direct_layer_samples_and_drained_notices_invalidate_both_values() {
        let live = LiveStage::new(
            source("#usda 1.0\ndef Sphere \"Box\" { double radius = 1 }\n")
                .open_stage()
                .unwrap(),
        );
        let mut cache = AnimationCache::default();
        check(&mut cache, &live, "/Box", false);
        let revision = live.composition_revision();
        live.stage
            .root_layer()
            .edit(|layer| {
                layer.data_mut().set_field(
                    &openusd::sdf::path("/Box.radius")?,
                    "timeSamples",
                    Value::TimeSamples(openusd::sdf::normalize_time_samples(vec![(
                        0.0,
                        Value::Double(2.0),
                    )])),
                );
                Ok(())
            })
            .unwrap();
        live.try_drain_changes().unwrap();
        assert!(live.composition_revision() > revision);
        assert_eq!(
            animated(cache.sample(&live, &openusd::sdf::path("/Box").unwrap(), true)),
            (true, false)
        );
        check(&mut cache, &live, "/Box", true);
        live.stage
            .attribute("/Box.radius")
            .unwrap()
            .clear_at(TimeCode::new(0.0))
            .unwrap();
        live.try_drain_changes().unwrap();
        assert_eq!(
            animated(cache.sample(&live, &openusd::sdf::path("/Box").unwrap(), true)),
            (false, false)
        );
        check(&mut cache, &live, "/Box", false);
    }

    #[test]
    fn variants_payloads_and_muting_invalidate_cached_animation() {
        let weak = source(
            "#usda 1.0\ndef Sphere \"Weak\" { double radius.timeSamples = {0: 1, 10: 2} }\n",
        );
        let payload = crate::UsdSource::new(
            "payload-cache.usda",
            b"#usda 1.0\ndef Sphere \"Payload\" { double radius.timeSamples = {0: 1, 10: 2} }\n"
                .as_slice(),
        )
        .unwrap();
        let root = crate::UsdSource::snapshot(
            "root-cache.usda",
            br#"#usda 1.0
(subLayers = [@animation-cache.usda@])
def Sphere "Root" (prepend payload = @payload-cache.usda@</Payload>) {}
def Sphere "Variant" (variants = {string mode = "still"} prepend variantSets = "mode") {
    variantSet "mode" = {
        "still" { double radius = 1 }
        "moving" { double radius.timeSamples = {0: 1, 10: 2} }
    }
}
"#
            .as_slice(),
        )
        .unwrap()
        .with_dependency(&weak)
        .unwrap()
        .with_dependency(&payload)
        .unwrap();
        let live = LiveStage::new(root.open_stage().unwrap());
        crate::UsdSource::validate_composition(&live.stage).unwrap();
        let mut cache = AnimationCache::default();
        check(&mut cache, &live, "/Root", true);
        live.stage.set_load_rules(openusd::pcp::LoadRules::none());
        check(&mut cache, &live, "/Root", false);
        live.stage.set_load_rules(openusd::pcp::LoadRules::all());
        check(&mut cache, &live, "/Root", true);
        check(&mut cache, &live, "/Weak", true);
        live.stage.mute_layer(weak.identifier());
        check(&mut cache, &live, "/Weak", false);
        live.stage.unmute_layer(weak.identifier());
        check(&mut cache, &live, "/Weak", true);
        for (choice, expected) in [("still", false), ("moving", true), ("still", false)] {
            crate::authoring::set_variant(&live.stage, "/Variant", "mode", choice).unwrap();
            live.try_drain_changes().unwrap();
            check(&mut cache, &live, "/Variant", expected);
        }
    }

    #[test]
    fn collection_binding_and_shader_edits_refresh_consumers() {
        let live = LiveStage::new(
            source(
                r#"#usda 1.0
def Xform "Bindings" (prepend apiSchemas = ["CollectionAPI:paint", "MaterialBindingAPI"]) {
    rel collection:paint:includes = []
    rel material:binding:collection:paint = [</Bindings.collection:paint>, </Mat>]
    def Cube "Box" {}
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        float inputs:roughness.timeSamples = {0: 0.2, 10: 0.8}
        token outputs:surface
    }
}
"#,
            )
            .open_stage()
            .unwrap(),
        );
        let mut cache = AnimationCache::default();
        check(&mut cache, &live, "/Bindings/Box", false);
        live.stage
            .relationship("/Bindings.collection:paint:includes")
            .unwrap()
            .set_targets([openusd::sdf::path("/Bindings/Box").unwrap()])
            .unwrap();
        check(&mut cache, &live, "/Bindings/Box", true);
        live.stage
            .attribute("/Mat/Surface.inputs:roughness")
            .unwrap()
            .clear()
            .unwrap()
            .set(Value::Float(0.5))
            .unwrap();
        check(&mut cache, &live, "/Bindings/Box", false);
    }

    #[test]
    fn limits_custom_bypass_and_saturated_revisions_do_not_retain_stale_entries() {
        let live = LiveStage::new(
            source("#usda 1.0\ndef Xform \"A\" {}\ndef Xform \"B\" {}\n")
                .open_stage()
                .unwrap(),
        );
        for (max_paths, max_path_bytes) in [(0, usize::MAX), (usize::MAX, 0), (1, 2)] {
            let mut cache = AnimationCache {
                max_paths,
                max_path_bytes,
                ..default()
            };
            for _ in 0..3 {
                for name in ["/A", "/B"] {
                    let path = openusd::sdf::path(name).unwrap();
                    assert!(!cache.sample(&live, &path, true).0.animated);
                    assert!(cache.entries.len() <= max_paths);
                    assert!(cache.path_bytes <= max_path_bytes);
                }
            }
            cache.sample(&live, &openusd::sdf::path("/A").unwrap(), false);
            assert!(cache.entries.is_empty());
            assert_eq!(cache.entries.capacity(), 0);
            assert_eq!(cache.path_bytes, 0);
            assert!(cache.revision.is_none());
        }
        assert!(Revision::at(&live.stage, u64::MAX).is_none());
    }

    #[test]
    fn population_masks_and_root_local_variants_do_not_share_classifications() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("masked.usda");
        std::fs::write(&file, "#usda 1.0\ndef Sphere \"A\" { double radius.timeSamples = {0: 1, 10: 2} }\ndef Xform \"B\" {}\n").unwrap();
        let mut cache = AnimationCache::default();
        for (mask, expected) in [("/A", true), ("/B", false), ("/A", true)] {
            let stage = Stage::builder()
                .mask(openusd::usd::StagePopulationMask::new([mask]).unwrap())
                .open(file.to_str().unwrap())
                .unwrap();
            check(&mut cache, &LiveStage::new(stage), "/A", expected);
        }
        let model = source(
            r#"#usda 1.0
def Sphere "Model" (variants = {string mode = "still"} prepend variantSets = "mode") {
    variantSet "mode" = { "still" { double radius = 1 } "moving" { double radius.timeSamples = {0: 1, 10: 2} } }
}
"#,
        );
        let a = LiveStage::new(model.open_stage().unwrap());
        let b = LiveStage::new(model.open_stage().unwrap());
        let mut ac = AnimationCache::default();
        let mut bc = AnimationCache::default();
        crate::authoring::set_variant(&a.stage, "/Model", "mode", "moving").unwrap();
        for _ in 0..3 {
            check(&mut ac, &a, "/Model", true);
            check(&mut bc, &b, "/Model", false);
        }
    }

    fn world(text: &str) -> (World, Handle<crate::UsdScene>, [Entity; 2]) {
        let mut world = World::new();
        world.insert_resource(crate::SchemaRegistry::builtin());
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::UsdScene>>();
        world.insert_resource(crate::UsdProjectionBudget(std::time::Duration::MAX));
        let handle = world
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source: source(text),
                textures: default(),
            });
        let roots = [0, 1].map(|_| {
            world
                .spawn((
                    crate::UsdSceneRoot(handle.clone()),
                    UsdInstanceTime::default(),
                ))
                .id()
        });
        crate::asset::spawn_usd_scenes(&mut world);
        for root in roots {
            assert_eq!(
                world.get::<crate::UsdSceneState>(root),
                Some(&crate::UsdSceneState::Ready)
            );
        }
        (world, handle, roots)
    }

    fn seek(world: &mut World, roots: [Entity; 2], clocks: [f64; 2]) {
        for (root, current) in roots.into_iter().zip(clocks) {
            world.get_mut::<UsdInstanceTime>(root).unwrap().current = current;
        }
        crate::asset::spawn_usd_scenes(world);
    }

    const MOVING: &str = r#"#usda 1.0
def Xform "A" {
    double3 xformOp:translate.timeSamples = {0: (0,0,0), 10: (10,0,0)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "B" {
    double3 xformOp:translate = (0,0,0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#;

    #[test]
    fn custom_routes_observe_mutations_before_classifying_later_prims() {
        struct AnimateLater;
        impl crate::PrimRoute for AnimateLater {
            fn matches(&self, ctx: &crate::RouteCtx) -> bool {
                ctx.path.as_str() == "/A"
            }
            fn project(&self, ctx: &crate::RouteCtx, _: &mut World, _: Entity) {
                ctx.stage
                    .attribute("/B.xformOp:translate")
                    .unwrap()
                    .set_at(
                        Value::Vec3d(openusd::gf::Vec3d::from([0.0, 0.0, 0.0])),
                        TimeCode::new(0.0),
                    )
                    .unwrap()
                    .set_at(
                        Value::Vec3d(openusd::gf::Vec3d::from([10.0, 0.0, 0.0])),
                        TimeCode::new(10.0),
                    )
                    .unwrap();
            }
        }
        let (mut world, _, roots) = world(MOVING);
        seek(&mut world, roots, [1.0, 2.0]);
        world
            .resource_mut::<crate::SchemaRegistry>()
            .register(AnimateLater);
        seek(&mut world, roots, [5.0, 8.0]);
        let instances = world.get_non_send::<UsdInstances>().unwrap();
        for (root, clock) in roots.into_iter().zip([5.0, 8.0]) {
            let entity = instances.entity(root, "/B").unwrap();
            assert_eq!(world.get::<Transform>(entity).unwrap().translation.x, clock);
            assert!(instances.roots[&root].animation.entries.is_empty());
        }
    }

    #[test]
    fn roots_reload_failure_recovery_and_teardown_keep_classification_local() {
        let (mut world, handle, roots) = world(MOVING);
        seek(&mut world, roots, [2.0, 8.0]);
        let entities = roots.map(|root| {
            world
                .get_non_send::<UsdInstances>()
                .unwrap()
                .entity(root, "/A")
                .unwrap()
        });
        for entity in entities {
            world.entity_mut(entity).insert(Name::new("runtime"));
        }
        let stage = world
            .get_non_send::<UsdInstances>()
            .unwrap()
            .stage(roots[0])
            .unwrap()
            .clone();
        stage
            .attribute("/A.xformOp:translate")
            .unwrap()
            .clear()
            .unwrap()
            .set(Value::Vec3d(openusd::gf::Vec3d::from([3.0, 0.0, 0.0])))
            .unwrap();
        seek(&mut world, roots, [7.0, 4.0]);
        assert_eq!(
            entities.map(|entity| world.get::<Transform>(entity).unwrap().translation.x),
            [3.0, 4.0]
        );
        world
            .resource_mut::<Assets<crate::UsdScene>>()
            .get_mut(&handle)
            .unwrap()
            .source = source("not USD");
        seek(&mut world, roots, [5.0, 9.0]);
        assert!(roots.iter().all(|root| matches!(
            world.get::<crate::UsdSceneState>(*root),
            Some(crate::UsdSceneState::Failed(_))
        )));
        assert_eq!(
            entities.map(|entity| world.get::<Transform>(entity).unwrap().translation.x),
            [3.0, 9.0]
        );
        world
            .resource_mut::<Assets<crate::UsdScene>>()
            .get_mut(&handle)
            .unwrap()
            .source = source(&MOVING.replace("(10,0,0)", "(20,0,0)"));
        seek(&mut world, roots, [5.0, 9.0]);
        assert_eq!(
            entities.map(|entity| world.get::<Transform>(entity).unwrap().translation.x),
            [10.0, 18.0]
        );
        seek(&mut world, roots, [1.0, 6.0]);
        assert_eq!(
            entities.map(|entity| world.get::<Transform>(entity).unwrap().translation.x),
            [2.0, 12.0]
        );
        for (root, entity) in roots.into_iter().zip(entities) {
            assert_eq!(
                world
                    .get_non_send::<UsdInstances>()
                    .unwrap()
                    .entity(root, "/A"),
                Some(entity)
            );
            assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "runtime");
        }
        world.despawn(roots[0]);
        crate::asset::spawn_usd_scenes(&mut world);
        assert!(
            world
                .get_non_send::<UsdInstances>()
                .unwrap()
                .stage(roots[0])
                .is_none()
        );
        assert_eq!(world.get_non_send::<UsdInstances>().unwrap().len(), 1);
    }
}
