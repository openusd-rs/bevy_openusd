//! USD as a first-class Bevy asset (PLAN: "USD as a strong asset contender").
//!
//! This is the glTF-equivalent entry point: `asset_server.load("scene.usdz")`
//! yields a [`Handle<UsdScene>`], and spawning an entity with
//! [`UsdSceneRoot`]`(handle)` projects the composed stage as a child subtree —
//! the same way `SceneRoot(gltf_scene)` works, but USD-native and with **zero
//! `bevy_scene` dependency** (USD composition is the scene system).
//!
//! Layers and external asset bytes are read through Bevy and retained in a snapshot.

use std::path::Path;

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetId, AssetLoader, AssetPath, LoadContext, LoadState};
use bevy::prelude::*;

use crate::live::project_stage_under;
use crate::instance::{InstanceRuntime, UsdInstanceOverrides, UsdInstanceTime, UsdInstances, UsdPlayback};
use crate::live::{AnimatedPrims, LiveStage, reconcile, stage_up_axis};
use crate::route::StageTime;
use crate::{SchemaRegistry, UsdSource};

/// A composed source snapshot ready for main-thread projection.
#[derive(Asset, TypePath, Debug, Clone)]
pub struct UsdScene {
    pub source: UsdSource,
    #[dependency]
    pub textures: bevy::platform::collections::HashMap<(String, bool), Handle<Image>>,
}

#[derive(Resource, Clone, Default)]
pub(crate) struct SnapshotTextures(
    pub bevy::platform::collections::HashMap<(String, bool), Handle<Image>>,
);

/// Spawn a loaded [`UsdScene`] as a child subtree of this entity — the USD-native
/// analog of `SceneRoot`. The entity keeps its own `Transform` (placement); the
/// USD content, up-axis-corrected, hangs beneath it.
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility, UsdInstanceTime, UsdInstanceOverrides, UsdPlayback)]
pub struct UsdSceneRoot(pub Handle<UsdScene>);

/// Marks a [`UsdSceneRoot`] that has already been projected, so the spawn system
/// doesn't re-project it every frame.
#[derive(Component, Debug, Clone)]
pub struct UsdSceneInstance {
    asset: AssetId<UsdScene>,
    revision: u64,
    subtree: Option<Entity>,
    overrides: UsdInstanceOverrides,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub enum UsdSceneState {
    Loading,
    Ready,
    Failed(String),
}

/// Optional cumulative timings for source/override publication attempts.
#[derive(Resource, Default, Debug, Clone)]
pub struct UsdSceneTimings {
    pub attempts: usize,
    pub failures: usize,
    pub open: std::time::Duration,
    pub overrides: std::time::Duration,
    pub validation: std::time::Duration,
    pub projection: std::time::Duration,
}

fn timed<T>(enabled: bool, elapsed: &mut std::time::Duration, operation: impl FnOnce() -> T) -> T {
    if !enabled { return operation(); }
    let start = std::time::Instant::now();
    let result = operation();
    *elapsed += start.elapsed();
    result
}

/// Reads a source snapshot and its discovered dependencies through Bevy.
#[derive(Default, TypePath)]
pub struct UsdAssetLoader;

impl AssetLoader for UsdAssetLoader {
    type Asset = UsdScene;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<UsdScene, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let root = std::env::current_dir()?.join("__bevy_usd_assets__");
        let path = load_context.path().path().to_path_buf();
        if path.is_absolute() {
            return Err(std::io::Error::other(
                "USD asset paths must be source-relative",
            ));
        }
        let source_id = load_context.path().source().clone_owned();
        let mut source = UsdSource::snapshot(&root.join(path), bytes)?;
        if !Path::new(source.identifier()).starts_with(&root) {
            return Err(std::io::Error::other("USD root escapes asset source"));
        }
        for _ in 0..128 {
            let (result, missing) = source.probe();
            if missing.is_empty() {
                result.map_err(std::io::Error::other)?;
                let mut textures = bevy::platform::collections::HashMap::default();
                for (index, (path, srgb)) in source
                    .texture_requests()
                    .map_err(std::io::Error::other)?
                    .into_iter()
                    .enumerate()
                {
                    let bytes = source.read_asset(&path)?;
                    let inner = openusd::ar::split_package_relative_path_inner(&path)
                        .map(|(_, inner)| inner)
                        .unwrap_or_else(|| path.clone());
                    let extension = Path::new(&inner)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .ok_or_else(|| {
                            std::io::Error::other(format!("texture has no extension: {path}"))
                        })?;
                    let image = Image::from_buffer(
                        &bytes,
                        bevy::image::ImageType::Extension(extension),
                        bevy::image::CompressedImageFormats::NONE,
                        srgb,
                        bevy::image::ImageSampler::default(),
                        bevy::asset::RenderAssetUsages::default(),
                    )
                    .map_err(|error| std::io::Error::other(format!("texture {path}: {error}")))?;
                    let handle = load_context.add_labeled_asset(format!("texture_{index}"), image);
                    textures.insert((path, srgb), handle);
                }
                return Ok(UsdScene { source, textures });
            }
            for identifier in missing {
                if openusd::ar::is_package_relative_path(&identifier) {
                    return Err(std::io::Error::other(format!(
                        "missing USD package entry: {identifier}"
                    )));
                }
                let dependency = Path::new(&identifier).strip_prefix(&root).map_err(|_| {
                    std::io::Error::other(format!(
                        "USD dependency escapes asset source: {identifier}"
                    ))
                })?;
                let asset_path =
                    AssetPath::from(dependency.to_path_buf()).with_source(source_id.clone());
                let bytes = load_context
                    .read_asset_bytes(asset_path)
                    .await
                    .map_err(std::io::Error::other)?;
                source.insert_dependency(identifier, bytes);
            }
        }
        Err(std::io::Error::other(
            "USD dependency discovery exceeded 128 rounds",
        ))
    }

    fn extensions(&self) -> &[&str] {
        &["usd", "usda", "usdc", "usdz"]
    }
}

/// Registers the USD asset loader + the spawn system. Add alongside
/// [`crate::UsdPlugin`] (which provides the [`SchemaRegistry`]); this plugin
/// installs one too if it's missing, so it also works standalone.
pub struct UsdAssetPlugin;

impl Plugin for UsdAssetPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<Assets<Image>>() {
            app.init_asset::<Image>();
        }
        app.init_asset::<UsdScene>()
            .init_resource::<crate::route::cache::ProjectionCache>()
            .register_asset_loader(UsdAssetLoader)
            .add_systems(Update, spawn_usd_scenes);
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
    }
}

/// Project any `UsdSceneRoot` whose asset has finished loading and hasn't been
/// spawned yet. Exclusive (`&mut World`) because projection spawns a hierarchy
/// and runs the routes, which need `&mut World`.
fn spawn_usd_scenes(world: &mut World) {
    let mut instances = world.remove_non_send::<UsdInstances>().unwrap_or_default();
    instances.roots.retain(|root, _| world.get::<UsdSceneRoot>(*root).is_some());
    let orphaned: Vec<_> = world
        .query_filtered::<(Entity, &UsdSceneInstance), Without<UsdSceneRoot>>()
        .iter(world)
        .map(|(entity, instance)| (entity, instance.subtree))
        .collect();
    for (entity, subtree) in orphaned {
        if let Some(subtree) = subtree {
            world.despawn(subtree);
        }
        world
            .entity_mut(entity)
            .remove::<(UsdSceneInstance, UsdSceneState)>();
    }
    let mut query = world.query::<(Entity, &UsdSceneRoot, Option<&UsdSceneInstance>)>();
    let pending: Vec<_> = query
        .iter(world)
        .map(|(e, r, instance)| (e, r.0.clone(), instance.cloned()))
        .collect();
    for (entity, handle, previous) in pending {
        let overrides = world.get::<UsdInstanceOverrides>(entity).cloned().unwrap_or_default();
        if let Some(LoadState::Failed(error)) = world
            .get_resource::<AssetServer>()
            .and_then(|server| server.get_load_state(handle.id()))
        {
            world
                .entity_mut(entity)
                .insert(UsdSceneState::Failed(error.to_string()));
            continue;
        }
        // Skip until the asset has actually loaded.
        let Some((source, textures)) = world
            .resource::<Assets<UsdScene>>()
            .get(&handle)
            .map(|s| (s.source.clone(), s.textures.clone()))
        else {
            let state = match world
                .get_resource::<AssetServer>()
                .and_then(|server| server.get_load_state(handle.id()))
            {
                Some(LoadState::Failed(error)) => UsdSceneState::Failed(error.to_string()),
                _ => UsdSceneState::Loading,
            };
            world.entity_mut(entity).insert(state);
            continue;
        };

        if previous.as_ref().is_some_and(|old| old.asset == handle.id()
            && old.revision == source.revision() && old.overrides == overrides)
        {
            continue;
        }
        let profiled = world.contains_resource::<UsdSceneTimings>();
        let mut timing = UsdSceneTimings { attempts: 1, ..default() };
        let opened = timed(profiled, &mut timing.open, || source.open_stage()).map_err(anyhow::Error::from).and_then(|stage| {
            timed(profiled, &mut timing.overrides, || overrides.apply(&stage))?;
            timed(profiled, &mut timing.validation, || UsdSource::validate_composition(&stage))?;
            Ok(stage)
        });
        timing.failures = usize::from(opened.is_err());
        match opened {
            Ok(stage) => {
                let retained = instances.roots.remove(&entity)
                    .filter(|runtime| runtime.asset == handle.id());
                if retained.is_none() {
                    if let Some(subtree) = previous.as_ref().and_then(|old| old.subtree) {
                        world.despawn(subtree);
                    }
                }
                let current = world.get::<UsdInstanceTime>(entity).map_or(0.0, |time| time.current);
                let previous_time = world.remove_resource::<StageTime>();
                let previous_animated = world.remove_resource::<AnimatedPrims>();
                let previous_textures = world.remove_resource::<SnapshotTextures>();
                world.insert_resource(StageTime { current });
                world.insert_resource(SnapshotTextures(textures.clone()));
                let live = LiveStage::new(stage);
                let map = timed(profiled, &mut timing.projection, || {
                    if let Some(mut runtime) = retained {
                        reconcile(world, &live, &mut runtime.map, false);
                        if let Some(root) = runtime.map.entity("/") {
                            world.entity_mut(root).insert(Transform::from_rotation(stage_up_axis(&live.stage)));
                        }
                        runtime.map
                    } else {
                        project_stage_under(world, &live.stage, entity)
                    }
                });
                world.remove_resource::<SnapshotTextures>();
                world.remove_resource::<StageTime>();
                world.remove_resource::<AnimatedPrims>();
                if let Some(previous) = previous_time { world.insert_resource(previous); }
                if let Some(previous) = previous_animated { world.insert_resource(previous); }
                if let Some(previous) = previous_textures {
                    world.insert_resource(previous);
                }
                world.entity_mut(entity).insert((
                    UsdSceneInstance {
                        asset: handle.id(),
                        revision: source.revision(),
                        subtree: map.entity("/"),
                        overrides,
                    },
                    UsdSceneState::Ready,
                ));
                instances.roots.insert(entity, InstanceRuntime {
                    asset: handle.id(), live, map, textures: SnapshotTextures(textures), sampled: current,
                    subdivision_levels: crate::route::subdivision::current_levels(world),
                });
            }
            Err(error) => {
                world.entity_mut(entity).insert((
                    UsdSceneInstance {
                        asset: handle.id(),
                        revision: source.revision(),
                        subtree: previous.and_then(|old| old.subtree),
                        overrides,
                    },
                    UsdSceneState::Failed(error.to_string()),
                ));
            }
        }
        if profiled && let Some(mut total) = world.get_resource_mut::<UsdSceneTimings>() {
            total.attempts += timing.attempts;
            total.failures += timing.failures;
            total.open += timing.open;
            total.overrides += timing.overrides;
            total.validation += timing.validation;
            total.projection += timing.projection;
        }
    }
    crate::instance::tick(world, &mut instances);
    world.insert_non_send(instances);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UsdPrimRef;

    const ANIMATED: &str = r#"#usda 1.0
def Xform "Mover" {
    double3 xformOp:translate.timeSamples = { 0: (0, 0, 0), 10: (10, 0, 0) }
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
def Xform "Old" {}
"#;

    fn instance_world() -> (World, Handle<UsdScene>) {
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut scenes = Assets::<UsdScene>::default();
        let handle = scenes.add(UsdScene {
            source: UsdSource::new("instances.usda", ANIMATED.as_bytes()).unwrap(),
            textures: default(),
        });
        world.insert_resource(scenes);
        (world, handle)
    }

    fn instance_entity(world: &World, root: Entity, path: &str) -> Entity {
        world.non_send::<UsdInstances>().entity(root, path).unwrap()
    }

    #[test]
    fn incomplete_composed_source_fails_before_replacing_live_entities() {
        let (mut world, handle) = instance_world();
        world.init_resource::<UsdSceneTimings>();
        let root = world.spawn(UsdSceneRoot(handle.clone())).id();
        spawn_usd_scenes(&mut world);
        let old = instance_entity(&world, root, "/Old");
        let good = world.resource::<Assets<UsdScene>>().get(&handle).unwrap().source.clone();
        let directory = tempfile::tempdir().unwrap();
        let broken = UsdSource::snapshot(directory.path().join("root.usda"),
            &b"#usda 1.0\ndef Xform \"Broken\" (prepend references = @missing.usda@</Model>) {}\n"[..]).unwrap();
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = broken;
        spawn_usd_scenes(&mut world);
        let Some(UsdSceneState::Failed(error)) = world.get::<UsdSceneState>(root) else { panic!("missing composition must fail") };
        assert!(error.contains("missing.usda"), "{error}");
        assert_eq!(instance_entity(&world, root, "/Old"), old);
        assert!(world.non_send::<UsdInstances>().entity(root, "/Broken").is_none());
        assert_eq!(world.resource::<UsdSceneTimings>().attempts, 2);
        assert_eq!(world.resource::<UsdSceneTimings>().failures, 1);
        let fresh = world.spawn(UsdSceneRoot(handle.clone())).id();
        spawn_usd_scenes(&mut world);
        assert!(matches!(world.get::<UsdSceneState>(fresh), Some(UsdSceneState::Failed(_))));
        assert!(world.non_send::<UsdInstances>().stage(fresh).is_none());
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = good;
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
        assert_eq!(instance_entity(&world, root, "/Old"), old);
        assert_eq!(world.get::<UsdSceneState>(fresh), Some(&UsdSceneState::Ready));
        assert_ne!(instance_entity(&world, fresh, "/Old"), old);
    }

    #[test]
    fn typed_component_overrides_survive_source_reload_and_removal() {
        #[derive(Component, Reflect, Default, Debug, PartialEq)]
        #[reflect(Component, Default)]
        struct Score { value: f64 }
        #[derive(Component)]
        struct RuntimeMarker;
        let (mut world, handle) = instance_world();
        let registry = AppTypeRegistry::default();
        registry.write().register::<Score>();
        world.insert_resource(registry.clone());
        let source = |value| UsdSource::new("instances.usda", format!("#usda 1.0\ndef Xform \"Model\" {{ custom double bevy:Score:value = {value}\n}}\n").into_bytes()).unwrap();
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = source(10);
        let mut overrides = UsdInstanceOverrides::default();
        overrides.set_component(&registry.read(), "/Model", &Score { value: 7.0 }).unwrap();
        overrides.set_component(&registry.read(), "/Model", &Score { value: 8.0 }).unwrap();
        assert_eq!(overrides.attributes.len(), 1);
        let a = world.spawn((UsdSceneRoot(handle.clone()), overrides)).id();
        let b = world.spawn(UsdSceneRoot(handle.clone())).id();
        spawn_usd_scenes(&mut world);
        let ea = instance_entity(&world, a, "/Model");
        let eb = instance_entity(&world, b, "/Model");
        world.entity_mut(ea).insert(RuntimeMarker);
        assert_eq!(world.get::<Score>(ea), Some(&Score { value: 8.0 }));
        assert_eq!(world.get::<Score>(eb), Some(&Score { value: 10.0 }));
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = source(20);
        spawn_usd_scenes(&mut world);
        assert_eq!(instance_entity(&world, a, "/Model"), ea);
        assert_eq!(instance_entity(&world, b, "/Model"), eb);
        assert_eq!(world.get::<Score>(ea), Some(&Score { value: 8.0 }));
        assert_eq!(world.get::<Score>(eb), Some(&Score { value: 20.0 }));
        world.entity_mut(a).insert(UsdInstanceOverrides::default());
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Score>(ea), Some(&Score { value: 20.0 }));
        assert!(world.get::<RuntimeMarker>(ea).is_some());
    }

    #[test]
    fn source_reload_and_scrub_detect_new_animation_without_a_shared_index() {
        let (mut world, handle) = instance_world();
        let static_source = UsdSource::snapshot("instances.usda", &b"#usda 1.0\ndef Xform \"Mover\" {}\n"[..]).unwrap();
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = static_source.clone();
        let sentinel = std::collections::HashSet::from(["/OtherSession".to_string()]);
        world.insert_resource(AnimatedPrims(sentinel.clone()));
        world.insert_resource(StageTime { current: 999.0 });
        let a = world.spawn((UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 2.0 })).id();
        let b = world.spawn((UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 8.0 })).id();
        spawn_usd_scenes(&mut world);
        let ea = instance_entity(&world, a, "/Mover");
        let eb = instance_entity(&world, b, "/Mover");
        for animated in [true, false, true] {
            world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source = if animated {
                UsdSource::snapshot("instances.usda", ANIMATED.as_bytes()).unwrap()
            } else { static_source.clone() };
            world.get_mut::<UsdInstanceTime>(a).unwrap().current = 2.0;
            world.get_mut::<UsdInstanceTime>(b).unwrap().current = 8.0;
            spawn_usd_scenes(&mut world);
            assert_eq!(world.get::<Transform>(ea).unwrap().translation.x, if animated { 2.0 } else { 0.0 });
            assert_eq!(world.get::<Transform>(eb).unwrap().translation.x, if animated { 8.0 } else { 0.0 });
            world.get_mut::<UsdInstanceTime>(a).unwrap().current = 5.0;
            spawn_usd_scenes(&mut world);
            assert_eq!(world.get::<Transform>(ea).unwrap().translation.x, if animated { 5.0 } else { 0.0 });
            assert_eq!(world.get::<Transform>(eb).unwrap().translation.x, if animated { 8.0 } else { 0.0 });
            assert_eq!(instance_entity(&world, a, "/Mover"), ea);
            assert_eq!(instance_entity(&world, b, "/Mover"), eb);
            assert_eq!(world.resource::<StageTime>().current, 999.0);
            assert_eq!(world.resource::<AnimatedPrims>().0, sentinel);
            assert_eq!(world.get::<UsdSceneState>(a), Some(&UsdSceneState::Ready));
            assert_eq!(world.get::<UsdSceneState>(b), Some(&UsdSceneState::Ready));
        }
    }

    #[test]
    fn live_instances_have_independent_time_and_edits() {
        let (mut world, handle) = instance_world();
        world.insert_resource(StageTime { current: 999.0 });
        let a = world.spawn(UsdSceneRoot(handle.clone())).id();
        let b = world.spawn((UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        spawn_usd_scenes(&mut world);
        let mover_a = instance_entity(&world, a, "/Mover");
        let mover_b = instance_entity(&world, b, "/Mover");
        assert_ne!(mover_a, mover_b);
        assert_eq!(world.get::<Transform>(mover_a).unwrap().translation.x, 0.0);
        assert_eq!(world.get::<Transform>(mover_b).unwrap().translation.x, 10.0);
        world.get_mut::<UsdInstanceTime>(a).unwrap().current = 5.0;
        crate::authoring::define_prim(world.non_send::<UsdInstances>().stage(a).unwrap(), "/OnlyA", "Xform").unwrap();
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Transform>(mover_a).unwrap().translation.x, 5.0);
        assert_eq!(world.get::<Transform>(mover_b).unwrap().translation.x, 10.0);
        assert!(world.non_send::<UsdInstances>().entity(a, "/OnlyA").is_some());
        assert!(world.non_send::<UsdInstances>().entity(b, "/OnlyA").is_none());
        assert_eq!(world.resource::<StageTime>().current, 999.0);
        assert!(!world.contains_resource::<AnimatedPrims>());
        assert!(!world.contains_resource::<SnapshotTextures>());
    }

    #[test]
    fn reload_preserves_matching_entities_and_runtime_components() {
        #[derive(Component, PartialEq, Debug)]
        struct Runtime(u32);
        let (mut world, handle) = instance_world();
        let root = world.spawn(UsdSceneRoot(handle.clone())).id();
        spawn_usd_scenes(&mut world);
        let mover = instance_entity(&world, root, "/Mover");
        let old = instance_entity(&world, root, "/Old");
        world.entity_mut(mover).insert(Runtime(42));
        let replacement = ANIMATED.replace("Old", "New");
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
            UsdSource::new("instances.usda", replacement.as_bytes()).unwrap();
        spawn_usd_scenes(&mut world);
        assert_eq!(instance_entity(&world, root, "/Mover"), mover);
        assert_eq!(world.get::<Runtime>(mover), Some(&Runtime(42)));
        assert!(world.get_entity(old).is_err());
        assert!(world.non_send::<UsdInstances>().entity(root, "/New").is_some());
        world.despawn(root);
        spawn_usd_scenes(&mut world);
        assert!(world.non_send::<UsdInstances>().is_empty());
    }

    #[test]
    fn failed_reload_keeps_last_good_live_stage() {
        let (mut world, handle) = instance_world();
        let root = world.spawn(UsdSceneRoot(handle.clone())).id();
        spawn_usd_scenes(&mut world);
        let mover = instance_entity(&world, root, "/Mover");
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
            UsdSource::new("instances.usda", &b"not USD"[..]).unwrap();
        spawn_usd_scenes(&mut world);
        assert!(matches!(world.get::<UsdSceneState>(root), Some(UsdSceneState::Failed(_))));
        assert_eq!(instance_entity(&world, root, "/Mover"), mover);
        world.get_mut::<UsdInstanceTime>(root).unwrap().current = 10.0;
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Transform>(mover).unwrap().translation.x, 10.0);
    }

    #[test]
    fn instance_opinions_are_isolated_reloadable_and_removable() {
        use crate::instance::UsdAttributeOverride;
        let (mut world, handle) = instance_world();
        let source = r#"#usda 1.0
def Xform "Model" (
    variants = { string shape = "a" }
    prepend variantSets = "shape"
) {
    variantSet "shape" = {
        "a" { def Xform "A" {} }
        "b" { def Xform "B" {} }
    }
}
"#;
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
            UsdSource::new("instances.usda", source.as_bytes()).unwrap();
        let a = world.spawn(UsdSceneRoot(handle.clone())).id();
        let b = world.spawn((UsdSceneRoot(handle.clone()), UsdInstanceOverrides {
            variants: vec![("/Model".into(), "shape".into(), "b".into())],
            attributes: vec![UsdAttributeOverride {
                prim: "/Model".into(), name: "visibility".into(), type_name: "token".into(),
                value: openusd::sdf::Value::Token("invisible".into()),
            }],
        })).id();
        spawn_usd_scenes(&mut world);
        assert!(world.non_send::<UsdInstances>().entity(a, "/Model/A").is_some());
        assert!(world.non_send::<UsdInstances>().entity(a, "/Model/B").is_none());
        let variant_b = instance_entity(&world, b, "/Model/B");
        let model_b = instance_entity(&world, b, "/Model");
        assert_eq!(world.get::<Visibility>(model_b), Some(&Visibility::Hidden));
        world.resource_mut::<Assets<UsdScene>>().get_mut(&handle).unwrap().source =
            UsdSource::new("instances.usda", source.as_bytes()).unwrap();
        spawn_usd_scenes(&mut world);
        assert_eq!(instance_entity(&world, b, "/Model/B"), variant_b);
        assert_eq!(world.get::<Visibility>(model_b), Some(&Visibility::Hidden));
        world.entity_mut(b).insert(UsdInstanceOverrides::default());
        spawn_usd_scenes(&mut world);
        assert_eq!(instance_entity(&world, b, "/Model"), model_b);
        assert!(world.get_entity(variant_b).is_err());
        assert!(world.non_send::<UsdInstances>().entity(b, "/Model/A").is_some());
        assert_ne!(world.get::<Visibility>(model_b), Some(&Visibility::Hidden));
    }

    #[test]
    fn playback_uses_stage_rate_and_independent_controls() {
        let (mut world, handle) = instance_world();
        let a = world.spawn((UsdSceneRoot(handle.clone()), UsdPlayback {
            playing: true, range: Some((0.0, 10.0)), ..default()
        })).id();
        let b = world.spawn((UsdSceneRoot(handle), UsdInstanceTime { current: 8.0 },
            UsdPlayback { playing: true, speed: -2.0, looping: false,
                range: Some((0.0, 10.0)) })).id();
        spawn_usd_scenes(&mut world);
        let a_entity = instance_entity(&world, a, "/Mover");
        let b_entity = instance_entity(&world, b, "/Mover");
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_millis(250));
        world.insert_resource(time);
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Transform>(a_entity).unwrap().translation.x, 6.0);
        assert_eq!(world.get::<Transform>(b_entity).unwrap().translation.x, 0.0);
        assert!(!world.get::<UsdPlayback>(b).unwrap().playing);
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Transform>(a_entity).unwrap().translation.x, 2.0);
        world.get_mut::<UsdPlayback>(a).unwrap().playing = false;
        spawn_usd_scenes(&mut world);
        assert_eq!(world.get::<Transform>(a_entity).unwrap().translation.x, 2.0);
    }

    #[test]
    fn failed_handle_replacement_does_not_transfer_runtime_components() {
        #[derive(Component)]
        struct Runtime;
        let (mut world, first) = instance_world();
        let second = world.resource_mut::<Assets<UsdScene>>().add(UsdScene {
            source: UsdSource::new("replacement.usda", &b"invalid"[..]).unwrap(),
            textures: default(),
        });
        let root = world.spawn(UsdSceneRoot(first)).id();
        spawn_usd_scenes(&mut world);
        let old = instance_entity(&world, root, "/Mover");
        world.entity_mut(old).insert(Runtime);
        world.entity_mut(root).insert(UsdSceneRoot(second.clone()));
        spawn_usd_scenes(&mut world);
        assert!(matches!(world.get::<UsdSceneState>(root), Some(UsdSceneState::Failed(_))));
        assert_eq!(instance_entity(&world, root, "/Mover"), old);
        world.resource_mut::<Assets<UsdScene>>().get_mut(&second).unwrap().source =
            UsdSource::new("replacement.usda", ANIMATED.as_bytes()).unwrap();
        spawn_usd_scenes(&mut world);
        let new = instance_entity(&world, root, "/Mover");
        assert_ne!(old, new);
        assert!(world.get_entity(old).is_err());
        assert!(world.get::<Runtime>(new).is_none());
    }

    #[test]
    fn asset_server_reload_preserves_two_live_instances_and_root_ownership() {
        #[derive(Component)]
        struct Runtime;
        let (mut app, directory, notify) = watched_memory_app();
        directory.insert_asset(Path::new("live.usda"), ANIMATED.as_bytes());
        directory.insert_asset(Path::new("replacement.usda"),
            b"#usda 1.0\ndef Xform \"Replacement\" {}\n".as_slice());
        let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("fixture://live.usda");
        let a = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        let b = app.world_mut().spawn((UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 10.0 })).id();
        let unrelated = app.world_mut().spawn(ChildOf(a)).id();
        tick_until(&mut app, |world| [a,b].iter().all(|root|
            world.get::<UsdSceneState>(*root) == Some(&UsdSceneState::Ready)));
        let mover_a = instance_entity(app.world(), a, "/Mover");
        let mover_b = instance_entity(app.world(), b, "/Mover");
        app.world_mut().entity_mut(mover_a).insert(Runtime);
        let revision = app.world().get::<UsdSceneInstance>(a).unwrap().revision;
        directory.insert_asset(Path::new("live.usda"), ANIMATED.replace("Old", "New").into_bytes());
        notify("live.usda");
        tick_until(&mut app, |world| [a,b].iter().all(|root|
            world.get::<UsdSceneInstance>(*root).is_some_and(|state| state.revision != revision)));
        assert_eq!(instance_entity(app.world(), a, "/Mover"), mover_a);
        assert_eq!(instance_entity(app.world(), b, "/Mover"), mover_b);
        assert!(app.world().get::<Runtime>(mover_a).is_some());
        assert_eq!(app.world().get::<Transform>(mover_b).unwrap().translation.x, 10.0);
        assert!(app.world().non_send::<UsdInstances>().entity(a, "/New").is_some());
        let replacement = app.world().resource::<AssetServer>().load("fixture://replacement.usda");
        app.world_mut().entity_mut(a).insert(UsdSceneRoot(replacement));
        tick_until(&mut app, |world| world.non_send::<UsdInstances>().entity(a, "/Replacement").is_some());
        assert!(app.world().get_entity(mover_a).is_err());
        assert!(app.world().get_entity(mover_b).is_ok());
        assert!(app.world().get_entity(unrelated).is_ok());
        app.world_mut().entity_mut(a).remove::<UsdSceneRoot>();
        app.update();
        assert!(app.world().non_send::<UsdInstances>().stage(a).is_none());
        assert!(app.world().get_entity(unrelated).is_ok());
        app.world_mut().despawn(b);
        app.update();
        assert!(app.world().non_send::<UsdInstances>().is_empty());
    }

    fn memory_app() -> (App, bevy::asset::io::memory::Dir) {
        let (app, directory, _) = watched_memory_app();
        (app, directory)
    }

    fn watched_memory_app() -> (App, bevy::asset::io::memory::Dir, impl Fn(&str)) {
        use bevy::asset::io::{
            AssetSourceBuilder,
            memory::{Dir, MemoryAssetReader},
        };
        let directory = Dir::default();
        let reader = directory.clone();
        let (send, receive) = std::sync::mpsc::channel();
        struct Watcher;
        impl bevy::asset::io::AssetWatcher for Watcher {}
        let mut app = App::new();
        app.register_asset_source(
            "fixture",
            AssetSourceBuilder::new(move || {
                Box::new(MemoryAssetReader {
                    root: reader.clone(),
                })
            })
            .with_watcher(move |events| {
                send.send(events).unwrap();
                Some(Box::new(Watcher))
            }),
        );
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                watch_for_changes_override: Some(true),
                ..default()
            },
            UsdAssetPlugin,
        ));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>();
        app.finish();
        app.cleanup();
        let events = receive.recv().unwrap();
        (app, directory, move |path| {
            events
                .try_send(bevy::asset::io::AssetSourceEvent::ModifiedAsset(
                    path.into(),
                ))
                .unwrap();
        })
    }

    fn tick_until(app: &mut App, condition: impl Fn(&World) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.update();
            if condition(app.world()) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "asset operation timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[cfg(all(feature = "file_watcher", not(target_arch = "wasm32")))]
    #[test]
    #[ignore = "requires native filesystem events"]
    fn native_file_watcher_reloads_layers_textures_and_recovers() {
        let directory = tempfile::tempdir().unwrap();
        let layer = directory.path().join("models/textured.usda");
        let texture = directory.path().join("textures/pixel.png");
        std::fs::create_dir_all(layer.parent().unwrap()).unwrap();
        std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
        std::fs::write(directory.path().join("root.usda"),
            "#usda 1.0\n( subLayers = [@models/textured.usda@] )\n").unwrap();
        std::fs::write(&layer, TEXTURED).unwrap();
        std::fs::write(&texture, pixel_png([255, 0, 0, 255])).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin {
            file_path: directory.path().to_string_lossy().into_owned(),
            watch_for_changes_override: Some(true),
            ..default()
        }, UsdAssetPlugin));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>();
        app.finish();
        app.cleanup();
        let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("root.usda");
        let roots = [
            app.world_mut().spawn(UsdSceneRoot(handle.clone())).id(),
            app.world_mut().spawn(UsdSceneRoot(handle.clone())).id(),
        ];
        tick_until(&mut app, |world| roots.iter().all(|root|
            world.get::<UsdSceneState>(*root) == Some(&UsdSceneState::Ready))
            && world.resource::<AssetServer>().is_loaded_with_dependencies(handle.id()));
        let entities = roots.map(|root| app.world().non_send::<UsdInstances>()
            .entity(root, "/Mesh").unwrap());
        let children = entities.map(|entity| {
            app.world_mut().entity_mut(entity).insert(Name::new("runtime name"));
            app.world_mut().spawn((Name::new("runtime child"), ChildOf(entity))).id()
        });
        let mesh_handles = |world: &World| entities.map(|entity|
            world.get::<Mesh3d>(entity).unwrap().0.clone());
        let image_handles = |world: &World| entities.map(|entity| {
            let material = &world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0;
            world.resource::<Assets<StandardMaterial>>().get(material).unwrap()
                .base_color_texture.clone().unwrap()
        });
        let initial_meshes = mesh_handles(app.world());
        let initial_images = image_handles(app.world());
        assert_eq!(initial_meshes[0], initial_meshes[1]);
        assert_eq!(initial_images[0], initial_images[1]);
        let edited = TEXTURED.replace("(1, 0, 0)", "(2, 0, 0)");
        std::fs::write(&layer, &edited).unwrap();
        tick_until(&mut app, |world| mesh_handles(world).iter().all(|handle| {
            let mesh = world.resource::<Assets<Mesh>>().get(handle).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return false; };
            positions.iter().any(|position| position[0] == 2.0)
        }));
        let updated_meshes = mesh_handles(app.world());
        assert_ne!(initial_meshes[0], updated_meshes[0]);
        assert_eq!(updated_meshes[0], updated_meshes[1]);
        std::fs::write(&texture, pixel_png([0, 0, 255, 255])).unwrap();
        tick_until(&mut app, |world| image_handles(world).iter().all(|handle|
            world.resource::<Assets<Image>>().get(handle).unwrap().data.as_deref()
                == Some(&[0, 0, 255, 255])));
        let updated_images = image_handles(app.world());
        assert_eq!(initial_images[0], updated_images[0]);
        assert_eq!(updated_images[0], updated_images[1]);
        std::fs::write(&layer, "#usda 1.0\ndef Mesh \"Mesh\" {").unwrap();
        tick_until(&mut app, |world| roots.iter().all(|root|
            matches!(world.get::<UsdSceneState>(*root), Some(UsdSceneState::Failed(_)))));
        assert_eq!(mesh_handles(app.world()), updated_meshes);
        assert_eq!(image_handles(app.world()), updated_images);
        std::fs::write(&layer, &edited).unwrap();
        tick_until(&mut app, |world| roots.iter().all(|root|
            world.get::<UsdSceneState>(*root) == Some(&UsdSceneState::Ready)));
        for ((root, entity), child) in roots.into_iter().zip(entities).zip(children) {
            assert_eq!(app.world().non_send::<UsdInstances>().entity(root, "/Mesh"), Some(entity));
            assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime name");
            assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
        }
        assert!(image_handles(app.world()).iter().all(|handle|
            app.world().resource::<Assets<Image>>().get(handle).unwrap().data.as_deref()
                == Some(&[0, 0, 255, 255])));
    }

    fn pixel_png(rgba: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
        }
        bytes
    }

    fn dome_hdr(red: u8) -> Vec<u8> {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
        bytes.extend_from_slice(&[red, 64, 32, 132, red, 64, 32, 132]);
        bytes
    }

    const DOME_TEXTURES: &str = r#"#usda 1.0
def DomeLight "Env" {
    token inputs:texture:format = "latlong"
    asset inputs:texture:file.timeSamples = {
        0: @../textures/first.hdr@,
        10: @../textures/second.hdr@
    }
}
"#;

    #[test]
    fn dome_hdr_samples_load_project_and_reload_from_named_source() {
        use crate::route::dome::UsdDomeTexture;
        let (mut app, directory, changed) = watched_memory_app();
        directory.insert_asset_text(Path::new("models/dome.usda"), DOME_TEXTURES);
        directory.insert_asset(Path::new("textures/first.hdr"), dome_hdr(128));
        directory.insert_asset(Path::new("textures/second.hdr"), dome_hdr(64));
        let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("fixture://models/dome.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
            && world.resource::<AssetServer>().is_loaded_with_dependencies(handle.id()));
        let scene = app.world().resource::<Assets<UsdScene>>().get(&handle).unwrap();
        assert_eq!(scene.textures.len(), 2);
        let revision = scene.source.revision();
        let entity = app.world_mut().query_filtered::<Entity, With<UsdDomeTexture>>().single(app.world()).unwrap();
        let radiance = |world: &World| {
            let texture = &world.get::<UsdDomeTexture>(entity).unwrap().0;
            world.resource::<Assets<Image>>().get(texture).unwrap().get_color_at(0, 0).unwrap().to_linear().red
        };
        assert_eq!(radiance(app.world()), 8.0);
        app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = 10.0;
        tick_until(&mut app, |world| radiance(world) == 4.0);
        directory.insert_asset(Path::new("textures/second.hdr"), dome_hdr(192));
        changed("textures/second.hdr");
        tick_until(&mut app, |world| world.resource::<Assets<UsdScene>>().get(&handle)
            .is_some_and(|scene| scene.source.revision() != revision) && radiance(world) == 12.0);
    }

    #[test]
    fn packaged_dome_hdr_samples_are_loaded_without_filesystem_paths() {
        let (mut app, directory) = memory_app();
        let mut archive = openusd::usdz::ArchiveWriter::new(std::io::Cursor::new(Vec::new()));
        archive.add_layer("models/dome.usda", DOME_TEXTURES.as_bytes()).unwrap();
        archive.add_layer("textures/first.hdr", &dome_hdr(128)).unwrap();
        archive.add_layer("textures/second.hdr", &dome_hdr(64)).unwrap();
        directory.insert_asset(Path::new("dome.usdz"), archive.finish().unwrap().into_inner());
        let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("fixture://dome.usdz");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
            && world.resource::<AssetServer>().is_loaded_with_dependencies(handle.id()));
        let scene = app.world().resource::<Assets<UsdScene>>().get(&handle).unwrap();
        assert_eq!(scene.textures.len(), 2);
        for ((path, srgb), image) in &scene.textures {
            assert!(openusd::ar::is_package_relative_path(path));
            assert!(!srgb);
            let image = app.world().resource::<Assets<Image>>().get(image).unwrap();
            assert!(image.get_color_at(0, 0).unwrap().to_linear().red > 1.0);
            assert!(crate::route::environment_map::latlong_cubemap(image, 1, [1.0; 3]).is_ok());
        }
    }

    const TEXTURED: &str = r#"#usda 1.0
def Mesh "Mesh" {
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0, 1, 2]
    point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    rel material:binding = </Mat>
}
def Material "Mat" {
    token outputs:surface.connect = </Mat/Shader.outputs:surface>
    def Shader "Shader" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor.connect = </Mat/Tex.outputs:rgb>
        normal3f inputs:normal.connect = </Mat/Tex.outputs:rgb>
        token outputs:surface
    }
    def Shader "Tex" {
        uniform token info:id = "UsdUVTexture"
        asset inputs:file = @../textures/pixel.png@
        float3 outputs:rgb
    }
}
"#;

    #[test]
    fn explicit_texture_color_spaces_override_usage_defaults() {
        for (space, srgb, expected) in [("raw", false, 128), ("sRGB", true, 55)] {
            let (mut app, directory) = memory_app();
            let text = TEXTURED.replace("normal3f inputs:normal.connect = </Mat/Tex.outputs:rgb>",
                "float inputs:roughness.connect = </Mat/Tex.outputs:r>")
                .replace("float3 outputs:rgb", &format!("float3 outputs:rgb\n        float outputs:r\n        token inputs:sourceColorSpace = \"{space}\""));
            directory.insert_asset_text(Path::new("models/color.usda"), &text);
            directory.insert_asset(Path::new("textures/pixel.png"), pixel_png([128, 128, 128, 255]));
            let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("fixture://models/color.usda");
            let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
            tick_until(&mut app, |world| world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready));
            let entity = instance_entity(app.world(), root, "/Mesh");
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            let images = app.world().resource::<Assets<Image>>();
            let diffuse = images.get(material.base_color_texture.as_ref().unwrap()).unwrap();
            assert_eq!(diffuse.texture_descriptor.format.is_srgb(), srgb);
            let packed = images.get(material.metallic_roughness_texture.as_ref().unwrap()).unwrap();
            assert_eq!(packed.data.as_deref(), Some([255, expected, 255, 255].as_slice()));
            assert!(app.world().get::<crate::route::material::UsdMaterialWarning>(entity).is_none());
        }
    }

    #[test]
    fn scalar_output_channels_are_packed_and_reloaded() {
        let (mut app, directory, notify) = watched_memory_app();
        let text = TEXTURED.replace("normal3f inputs:normal.connect = </Mat/Tex.outputs:rgb>",
            "float inputs:roughness.connect = </Mat/Tex.outputs:g>\n        float inputs:metallic.connect = </Mat/Tex.outputs:b>\n        float inputs:occlusion.connect = </Mat/Tex.outputs:g>\n        float inputs:opacity.connect = </Mat/Tex.outputs:a>")
            .replace("float3 outputs:rgb", "float3 outputs:rgb\n        float outputs:g\n        float outputs:b\n        float outputs:a");
        directory.insert_asset_text(Path::new("models/packed.usda"), &text);
        directory.insert_asset(Path::new("textures/pixel.png"), pixel_png([12, 64, 192, 31]));
        let handle: Handle<UsdScene> = app.world().resource::<AssetServer>().load("fixture://models/packed.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready));
        let entity = instance_entity(app.world(), root, "/Mesh");
        let packed = |world: &World| {
            let material = world.resource::<Assets<StandardMaterial>>().get(&world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.metallic, 1.0);
            assert_eq!(material.perceptual_roughness, 1.0);
            material.metallic_roughness_texture.clone().unwrap()
        };
        let first = packed(app.world());
        let base = |world: &World| {
            let material = world.resource::<Assets<StandardMaterial>>().get(&world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert_eq!(material.alpha_mode, AlphaMode::Blend);
            material.base_color_texture.clone().unwrap()
        };
        let first_base = base(app.world());
        let occlusion = |world: &World| {
            world.resource::<Assets<StandardMaterial>>().get(&world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap().occlusion_texture.clone().unwrap()
        };
        let first_occlusion = occlusion(app.world());
        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(images.get(&first_base).unwrap().data.as_deref(), Some([12, 64, 192, 31].as_slice()));
        assert_eq!(images.get(&first_occlusion).unwrap().data.as_deref(), Some([64, 255, 255, 255].as_slice()));
        assert_eq!(images.get(&first).unwrap().data.as_deref(), Some([255, 64, 192, 255].as_slice()));
        assert_eq!(images.get(&first).unwrap().texture_descriptor.format, bevy::render::render_resource::TextureFormat::Rgba8Unorm);
        assert!(app.world().get::<crate::route::material::UsdMaterialWarning>(entity).is_none());
        let revision = app.world().get::<UsdSceneInstance>(root).unwrap().revision;
        directory.insert_asset(Path::new("textures/pixel.png"), pixel_png([12, 128, 32, 200]));
        notify("textures/pixel.png");
        tick_until(&mut app, |world| world.get::<UsdSceneInstance>(root).is_some_and(|instance| instance.revision != revision));
        let second = packed(app.world());
        let second_base = base(app.world());
        assert_ne!(first_base, second_base);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&second_base).unwrap().data.as_deref(), Some([12, 128, 32, 200].as_slice()));
        let second_occlusion = occlusion(app.world());
        assert_ne!(first_occlusion, second_occlusion);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&second_occlusion).unwrap().data.as_deref(), Some([128, 255, 255, 255].as_slice()));
        assert_ne!(first, second);
        assert_eq!(app.world().resource::<Assets<Image>>().get(&second).unwrap().data.as_deref(), Some([255, 128, 32, 255].as_slice()));
    }

    #[test]
    fn packaged_material_images_are_labeled_assets() {
        let (mut app, directory) = memory_app();
        let mut archive = openusd::usdz::ArchiveWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .add_layer("models/textured.usda", TEXTURED.as_bytes())
            .unwrap();
        archive
            .add_layer("textures/pixel.png", &pixel_png([32, 64, 128, 255]))
            .unwrap();
        directory.insert_asset(
            Path::new("textured.usdz"),
            archive.finish().unwrap().into_inner(),
        );
        let handle: Handle<UsdScene> = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://textured.usdz");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| {
            world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
                && world
                    .resource::<AssetServer>()
                    .is_loaded_with_dependencies(handle.id())
        });
        let scene = app
            .world()
            .resource::<Assets<UsdScene>>()
            .get(&handle)
            .unwrap();
        assert_eq!(scene.textures.len(), 2);
        for handle in scene.textures.values() {
            let image = app.world().resource::<Assets<Image>>().get(handle).unwrap();
            assert_eq!(image.data.as_deref().unwrap(), &[32, 64, 128, 255]);
        }
    }

    #[test]
    fn texture_handles_use_color_spaces_and_dependency_changes_reload_owner() {
        let (mut app, directory, changed) = watched_memory_app();
        directory.insert_asset_text(Path::new("models/textured.usda"), TEXTURED);
        directory.insert_asset(Path::new("textures/pixel.png"), pixel_png([255, 0, 0, 255]));
        let handle: Handle<UsdScene> = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://models/textured.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| {
            world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
                && world
                    .resource::<AssetServer>()
                    .is_loaded_with_dependencies(handle.id())
        });
        let scene = app
            .world()
            .resource::<Assets<UsdScene>>()
            .get(&handle)
            .unwrap();
        let revision = scene.source.revision();
        assert_eq!(scene.textures.len(), 2);
        for ((_, srgb), handle) in &scene.textures {
            let image = app.world().resource::<Assets<Image>>().get(handle).unwrap();
            assert_eq!(image.texture_descriptor.format.is_srgb(), *srgb);
            assert_eq!(image.data.as_deref().unwrap(), &[255, 0, 0, 255]);
        }
        let mesh_material = app
            .world_mut()
            .query::<(&UsdPrimRef, &MeshMaterial3d<StandardMaterial>)>()
            .iter(app.world())
            .find(|(prim, _)| prim.path == "/Mesh")
            .unwrap()
            .1
            .0
            .clone();
        let material = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&mesh_material)
            .unwrap();
        assert!(material.base_color_texture.is_some());
        assert!(material.normal_map_texture.is_some());
        assert_ne!(material.base_color_texture, material.normal_map_texture);
        directory.insert_asset(Path::new("textures/pixel.png"), pixel_png([0, 255, 0, 255]));
        changed("textures/pixel.png");
        tick_until(&mut app, |world| {
            world
                .resource::<Assets<UsdScene>>()
                .get(&handle)
                .is_some_and(|scene| {
                    scene.source.revision() != revision
                        && scene.textures.values().all(|handle| {
                            world
                                .resource::<Assets<Image>>()
                                .get(handle)
                                .is_some_and(|image| {
                                    image.data.as_deref() == Some(&[0, 255, 0, 255])
                                })
                        })
                })
                && world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
        });
    }

    #[test]
    fn asset_server_reads_references_and_external_assets_from_named_source() {
        let (mut app, directory, changed) = watched_memory_app();
        directory.insert_asset_text(
            Path::new("models/scene.usda"),
            "#usda 1.0\ndef Xform \"Model\" (references = @../parts/model.usda@</Part>) {}\n",
        );
        directory.insert_asset_text(Path::new("parts/model.usda"),
            "#usda 1.0\ndef Xform \"Part\" { custom asset preview = @../textures/color.png@\n def Cube \"Box\" {} }\n");
        directory.insert_asset(
            Path::new("textures/color.png"),
            b"dependency bytes".to_vec(),
        );
        let handle: Handle<UsdScene> = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://models/scene.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
        tick_until(&mut app, |world| {
            world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
        });
        let scene = app
            .world()
            .resource::<Assets<UsdScene>>()
            .get(&handle)
            .unwrap();
        assert_eq!(scene.source.dependencies().count(), 2);
        assert!(
            scene
                .source
                .open_stage()
                .unwrap()
                .prim("/Model/Box")
                .unwrap()
                .is_valid()
                .unwrap()
        );
        let first_revision = scene.source.revision();
        directory.insert_asset_text(
            Path::new("parts/model.usda"),
            "#usda 1.0\ndef Xform \"Part\" { def Sphere \"Updated\" {} }\n",
        );
        changed("parts/model.usda");
        tick_until(&mut app, |world| {
            world
                .resource::<Assets<UsdScene>>()
                .get(&handle)
                .is_some_and(|scene| scene.source.revision() != first_revision)
                && world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
        });
        let stage = app
            .world()
            .resource::<Assets<UsdScene>>()
            .get(&handle)
            .unwrap()
            .source
            .open_stage()
            .unwrap();
        assert!(stage.prim("/Model/Updated").unwrap().is_valid().unwrap());
        assert!(!stage.prim("/Model/Box").unwrap().is_valid().unwrap());
    }

    #[test]
    fn missing_dependency_becomes_visible_failure() {
        let (mut app, directory) = memory_app();
        directory.insert_asset_text(
            Path::new("broken.usda"),
            "#usda 1.0\ndef Xform \"Model\" (references = @missing.usda@</Part>) {}\n",
        );
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://broken.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
        tick_until(&mut app, |world| {
            matches!(
                world.get::<UsdSceneState>(root),
                Some(UsdSceneState::Failed(_))
            )
        });
        assert!(app.world().get::<UsdSceneInstance>(root).is_none());
    }

    #[test]
    fn existing_layer_with_missing_reference_target_fails_asset_loading() {
        for target in ["/Absent", "/Present/Absent"] {
            let (mut app, directory) = memory_app();
            directory.insert_asset_text(Path::new("model.usda"), "#usda 1.0\ndef Scope \"Present\" {}\n");
            directory.insert_asset_text(Path::new("broken-target.usda"), &format!(
                "#usda 1.0\ndef Scope \"Mounted\" (prepend references = @model.usda@<{target}>) {{}}\n"));
            let handle = app.world().resource::<AssetServer>().load("fixture://broken-target.usda");
            let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
            tick_until(&mut app, |world| matches!(world.get::<UsdSceneState>(root), Some(UsdSceneState::Failed(_) | UsdSceneState::Ready)));
            let Some(UsdSceneState::Failed(error)) = app.world().get::<UsdSceneState>(root) else {
                panic!("missing prim target state: {:?}", app.world().get::<UsdSceneState>(root));
            };
            assert!(error.contains("Absent"), "{error}");
            assert!(app.world().get::<UsdSceneInstance>(root).is_none());
        }
    }

    #[test]
    fn variant_supplied_subroot_target_loads_without_false_diagnostics() {
        let (mut app, directory) = memory_app();
        directory.insert_asset_text(Path::new("variant-model.usda"), r#"#usda 1.0
def Scope "Model" (
    prepend variantSets = ["choice"]
    variants = { string choice = "show" }
) {
    variantSet "choice" = {
        "show" {
            def Scope "Child" {
                double score = 17
            }
        }
        "hide" {}
    }
}
"#);
        directory.insert_asset_text(Path::new("subroot.usda"),
            "#usda 1.0\ndef Scope \"Mounted\" (prepend references = @variant-model.usda@</Model/Child>) {}\n");
        let handle = app.world().resource::<AssetServer>().load("fixture://subroot.usda");
        let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
        tick_until(&mut app, |world| matches!(world.get::<UsdSceneState>(root), Some(UsdSceneState::Failed(_) | UsdSceneState::Ready)));
        assert_eq!(app.world().get::<UsdSceneState>(root), Some(&UsdSceneState::Ready));
        let stage = app.world().non_send::<UsdInstances>().stage(root).unwrap();
        assert_eq!(stage.prim("/Mounted").unwrap().attribute("score").get::<f64>().unwrap(), Some(17.0));
        assert!(stage.composition_errors().is_empty());
    }

    #[test]
    fn package_load_and_root_removal_preserve_unowned_children() {
        use std::io::Cursor;
        let (mut app, directory) = memory_app();
        let mut archive = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
        archive
            .add_layer(
                "scenes/root.usda",
                b"#usda 1.0\ndef Xform \"Model\" (references = @part.usda@</Part>) {}\n",
            )
            .unwrap();
        archive
            .add_layer(
                "scenes/part.usda",
                b"#usda 1.0\ndef Xform \"Part\" { def Cube \"Box\" {} }\n",
            )
            .unwrap();
        directory.insert_asset(
            Path::new("model.usdz"),
            archive.finish().unwrap().into_inner(),
        );
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://model.usdz");
        let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
        let child = app.world_mut().spawn(ChildOf(root)).id();
        tick_until(&mut app, |world| {
            world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready)
        });
        let subtree = app
            .world()
            .get::<UsdSceneInstance>(root)
            .unwrap()
            .subtree
            .unwrap();
        assert!(
            app.world_mut()
                .query::<&UsdPrimRef>()
                .iter(app.world())
                .any(|prim| prim.path == "/Model/Box")
        );
        app.world_mut().entity_mut(root).remove::<UsdSceneRoot>();
        app.update();
        assert!(app.world().get_entity(subtree).is_err());
        assert!(app.world().get_entity(child).is_ok());
        assert!(app.world().get::<UsdSceneInstance>(root).is_none());
    }

    #[test]
    fn missing_package_member_fails_loading() {
        use std::io::Cursor;
        let (mut app, directory) = memory_app();
        let mut archive = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
        archive
            .add_layer(
                "root.usda",
                b"#usda 1.0\ndef Xform \"Model\" (references = @missing.usda@</Part>) {}\n",
            )
            .unwrap();
        directory.insert_asset(
            Path::new("broken.usdz"),
            archive.finish().unwrap().into_inner(),
        );
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load("fixture://broken.usdz");
        let root = app.world_mut().spawn(UsdSceneRoot(handle)).id();
        tick_until(&mut app, |world| {
            matches!(
                world.get::<UsdSceneState>(root),
                Some(UsdSceneState::Failed(_))
            )
        });
    }

    #[test]
    fn loader_advertises_usd_extensions() {
        let exts = UsdAssetLoader.extensions();
        for e in ["usd", "usda", "usdc", "usdz"] {
            assert!(exts.contains(&e), "loader handles .{e}");
        }
    }

    /// End-to-end (minus the AssetServer): a `UsdScene` in `Assets` + a
    /// `UsdSceneRoot` on an entity → the spawn system opens and
    /// projects the stage as a child subtree.
    #[test]
    fn spawn_system_projects_scene_under_root() {
        let usda = b"#usda 1.0\ndef Xform \"Root\"\n{\n    def Cube \"Box\" {}\n}\n";

        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut scenes = Assets::<UsdScene>::default();
        let handle = scenes.add(UsdScene {
            source: UsdSource::new("test.usda", &usda[..]).unwrap(),
            textures: default(),
        });
        world.insert_resource(scenes);

        let root = world.spawn(UsdSceneRoot(handle)).id();
        // The system is a plain `fn(&mut World)`, so call it directly.
        spawn_usd_scenes(&mut world);

        // Root is marked spawned so it isn't reprojected.
        assert!(
            world.get::<UsdSceneInstance>(root).is_some(),
            "root marked as spawned"
        );
        // The stage projected: a Cube prim exists and has a mesh attached.
        let mut q = world.query::<(&UsdPrimRef, Option<&Mesh3d>)>();
        let paths: Vec<&str> = q.iter(&world).map(|(r, _)| r.path.as_str()).collect();
        assert!(
            paths.contains(&"/Root/Box"),
            "cube prim projected, got {paths:?}"
        );
        let cube_has_mesh = q
            .iter(&world)
            .any(|(r, mesh)| r.path == "/Root/Box" && mesh.is_some());
        assert!(cube_has_mesh, "cube prim got a Mesh3d");
    }
}
