use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::{
    asset::{
        AssetLoader, LoadContext, LoadState,
        io::{
            AssetSourceBuilder, Reader,
            memory::{Dir, MemoryAssetReader},
        },
    },
    prelude::*,
};
use usd_bevy::{
    UsdAssetPlugin, UsdPlugin, UsdProjectionBudget, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::{UsdInstanceTime, UsdInstances},
};

const SCENE: &str = r#"#usda 1.0
(
    startTimeCode = 0
    endTimeCode = 10
)
def Xform "Old" {}
def Xform "Mover" {
    double3 xformOp:translate.timeSamples = {0: (0,0,0), 10: (10,0,0)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Mesh "Shape" {
        point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
        uniform token subdivisionScheme = "none"
    }
}
"#;

fn app() -> (App, Dir) {
    let directory = Dir::default();
    let reader = directory.clone();
    let mut app = App::new();
    app.register_asset_source(
        "fixture",
        AssetSourceBuilder::new(move || {
            Box::new(MemoryAssetReader {
                root: reader.clone(),
            })
        }),
    );
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        UsdPlugin,
        UsdAssetPlugin,
    ));
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(UsdProjectionBudget(Duration::MAX));
    app.finish();
    app.cleanup();
    (app, directory)
}

#[track_caller]
fn until(app: &mut App, condition: impl Fn(&World) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.update();
        if condition(app.world()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "downstream asset operation timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn entity(world: &World, root: Entity, path: &str) -> Entity {
    world.non_send::<UsdInstances>().entity(root, path).unwrap()
}

#[test]
fn consumer_roots_seek_reload_recover_and_release_independently() {
    #[derive(Component)]
    struct RuntimeMarker;

    let (mut app, directory) = app();
    directory.insert_asset_text(Path::new("live.usda"), SCENE);
    let server = app.world().resource::<AssetServer>().clone();
    let handle: Handle<UsdScene> = server.load("fixture://live.usda");
    let a = app.world_mut().spawn(UsdSceneRoot(handle.clone())).id();
    let b = app
        .world_mut()
        .spawn((
            UsdSceneRoot(handle.clone()),
            UsdInstanceTime { current: 10.0 },
        ))
        .id();
    let unrelated = app.world_mut().spawn(ChildOf(a)).id();
    until(&mut app, |world| {
        [a, b]
            .into_iter()
            .all(|root| world.get::<UsdSceneState>(root) == Some(&UsdSceneState::Ready))
    });
    let mover_a = entity(app.world(), a, "/Mover");
    let mover_b = entity(app.world(), b, "/Mover");
    let mesh_a = entity(app.world(), a, "/Mover/Shape");
    let mesh_b = entity(app.world(), b, "/Mover/Shape");
    let geometry = app.world().get::<Mesh3d>(mesh_a).unwrap().0.clone();
    assert_ne!(mover_a, mover_b);
    assert_eq!(
        app.world().get::<Mesh3d>(mesh_b).unwrap().id(),
        geometry.id()
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Mesh>>()
            .get(&geometry)
            .unwrap()
            .count_vertices(),
        3
    );
    assert_eq!(
        app.world().get::<Transform>(mover_a).unwrap().translation.x,
        0.0
    );
    assert_eq!(
        app.world().get::<Transform>(mover_b).unwrap().translation.x,
        10.0
    );
    app.world_mut().entity_mut(mover_a).insert(RuntimeMarker);
    app.world_mut()
        .get_mut::<UsdInstanceTime>(a)
        .unwrap()
        .current = 5.0;
    app.update();
    assert_eq!(
        app.world().get::<Transform>(mover_a).unwrap().translation.x,
        5.0
    );
    assert_eq!(
        app.world().get::<Transform>(mover_b).unwrap().translation.x,
        10.0
    );

    directory.insert_asset_text(Path::new("live.usda"), &SCENE.replace("Old", "New"));
    server.reload("fixture://live.usda");
    until(&mut app, |world| {
        [a, b].into_iter().all(|root| {
            world
                .non_send::<UsdInstances>()
                .entity(root, "/New")
                .is_some()
        })
    });
    for (root, mover, time) in [(a, mover_a, 5.0), (b, mover_b, 10.0)] {
        assert_eq!(entity(app.world(), root, "/Mover"), mover);
        assert_eq!(
            app.world().get::<Transform>(mover).unwrap().translation.x,
            time
        );
        assert!(
            app.world()
                .non_send::<UsdInstances>()
                .entity(root, "/New")
                .is_some()
        );
    }
    assert!(app.world().get::<RuntimeMarker>(mover_a).is_some());
    assert_eq!(
        app.world().get::<Mesh3d>(mesh_a).unwrap().id(),
        geometry.id()
    );

    let prims = |world: &World, root| {
        let mut entries: Vec<_> = world
            .non_send::<UsdInstances>()
            .prims(root)
            .map(|(path, entity)| (path.to_owned(), entity))
            .collect();
        entries.sort();
        entries
    };
    let retained = [a, b].map(|root| prims(app.world(), root));
    directory.insert_asset_text(Path::new("live.usda"), "invalid USD");
    server.reload("fixture://live.usda");
    until(&mut app, |_| {
        matches!(
            server.get_load_state(handle.id()),
            Some(LoadState::Failed(_))
        )
    });
    for (root, expected) in [a, b].into_iter().zip(retained) {
        assert_eq!(prims(app.world(), root), expected);
        assert!(
            matches!(app.world().get::<UsdSceneState>(root), Some(UsdSceneState::Failed(message)) if message.contains("live.usda"))
        );
        assert!(
            app.world()
                .non_send::<UsdInstances>()
                .entity(root, "/New")
                .is_some()
        );
    }
    directory.insert_asset_text(Path::new("live.usda"), &SCENE.replace("Old", "Repaired"));
    server.reload("fixture://live.usda");
    until(&mut app, |world| {
        [a, b].into_iter().all(|root| {
            world
                .non_send::<UsdInstances>()
                .entity(root, "/Repaired")
                .is_some()
        })
    });
    assert_eq!(entity(app.world(), a, "/Mover"), mover_a);
    assert_eq!(entity(app.world(), b, "/Mover"), mover_b);
    for root in [a, b] {
        assert_eq!(
            app.world().get::<UsdSceneState>(root),
            Some(&UsdSceneState::Ready)
        );
    }
    assert!(app.world().get::<RuntimeMarker>(mover_a).is_some());
    assert_eq!(
        app.world().get::<Transform>(mover_b).unwrap().translation.x,
        10.0
    );

    app.world_mut().entity_mut(a).remove::<UsdSceneRoot>();
    app.update();
    assert!(app.world().non_send::<UsdInstances>().stage(a).is_none());
    assert!(app.world().get_entity(mover_a).is_err());
    assert!(app.world().get_entity(unrelated).is_ok());
    assert!(app.world().get_entity(mover_b).is_ok());
    app.world_mut().despawn(b);
    app.update();
    assert!(app.world().non_send::<UsdInstances>().is_empty());
}

#[derive(Asset, TypePath)]
struct Probe(usize);

#[derive(Default)]
struct Gate {
    fail_older: bool,
    calls: AtomicUsize,
    release: [AtomicBool; 2],
    waker: [Mutex<Option<std::task::Waker>>; 2],
}

#[derive(TypePath)]
struct ProbeLoader(Arc<Gate>);

impl AssetLoader for ProbeLoader {
    type Asset = Probe;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        _: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<Probe, std::io::Error> {
        if context.path().path() == Path::new("other.probe") {
            return Ok(Probe(42));
        }
        let call = self.0.calls.fetch_add(1, Ordering::SeqCst);
        if call == 1 || call == 3 {
            let slot = usize::from(call == 3);
            std::future::poll_fn(|cx| {
                let mut waker = self.0.waker[slot].lock().unwrap();
                if self.0.release[slot].load(Ordering::SeqCst) {
                    std::task::Poll::Ready(())
                } else {
                    *waker = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            })
            .await;
            if call == 1 && self.0.fail_older {
                return Err(std::io::Error::other("older load failed"));
            }
        }
        Ok(Probe(call))
    }

    fn extensions(&self) -> &[&str] {
        &["probe"]
    }
}

fn serialized_reload(fail_older: bool) {
    let (mut app, directory) = app();
    let gate = Arc::new(Gate {
        fail_older,
        ..default()
    });
    app.init_asset::<Probe>()
        .register_asset_loader(ProbeLoader(gate.clone()));
    directory.insert_asset_text(Path::new("root.probe"), "probe");
    let server = app.world().resource::<AssetServer>().clone();
    let handle: Handle<Probe> = server.load("fixture://root.probe");
    until(&mut app, |world| {
        world.resource::<Assets<Probe>>().get(&handle).is_some()
    });
    server.reload("fixture://root.probe");
    until(&mut app, |_| gate.calls.load(Ordering::SeqCst) >= 2);
    server.reload("fixture://root.probe");
    directory.insert_asset_text(Path::new("other.probe"), "other");
    let other: Handle<Probe> = server.load("fixture://other.probe");
    until(&mut app, |world| {
        world.resource::<Assets<Probe>>().get(&other).is_some()
    });
    let held_calls = gate.calls.load(Ordering::SeqCst);
    for slot in 0..2 {
        gate.release[slot].store(true, Ordering::SeqCst);
        if let Some(waker) = gate.waker[slot].lock().unwrap().take() {
            waker.wake();
        }
    }
    assert_eq!(
        held_calls, 2,
        "same-source reload overtook the older request"
    );
    let final_call = if fail_older { 3 } else { 2 };
    until(&mut app, |world| {
        world
            .resource::<Assets<Probe>>()
            .get(&handle)
            .is_some_and(|probe| probe.0 == final_call)
            && matches!(server.get_load_state(handle.id()), Some(LoadState::Loaded))
    });
}

#[test]
fn consumer_serializes_older_success_without_blocking_other_sources() {
    serialized_reload(false);
}

#[test]
fn consumer_serializes_older_failure_without_blocking_other_sources() {
    serialized_reload(true);
}
