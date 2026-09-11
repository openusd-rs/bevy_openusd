use std::sync::{Arc, Mutex};
use bevy::prelude::*;
use bevy::render::{RenderApp, RenderStartup, mesh::allocator::MeshAllocatorSettings, renderer::RenderDevice};
use bevy::render::error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy};
use mara::ui::mara_core::{pane::PaneBody, pod::Pod, vocab::Id};
use usd_bevy::route::subdivision::{UsdSubdivisionApplied, UsdSubdivisionError, UsdSubdivisionSettings};
use usd_bevy::route::curves::{UsdCurveSettings, UsdCurveError};

#[derive(Clone, Default)]
struct State {
    requested: Option<u32>,
    requested_curve_steps: Option<usize>,
    requested_curve_surface_sides: Option<Option<usize>>,
    curve_steps: usize,
    curve_surface_sides: Option<usize>,
    level: u32,
    refined: usize,
    mesh_entities: usize,
    unique_meshes: usize,
    standard_materials: usize,
    errors: Vec<String>,
    renderer_error: Option<String>,
}

#[derive(Resource, Clone, Default)]
pub struct RenderSettingsBridge(Arc<Mutex<State>>);

impl RenderSettingsBridge {
    pub fn renderer_error(&self) -> Option<String> {
        self.0.lock().ok().and_then(|state| state.renderer_error.clone())
    }
}

pub fn configure(app: &mut App, bridge: RenderSettingsBridge) {
    app.insert_resource(bridge).insert_resource(RenderErrorHandler(stop_rendering))
        .add_systems(PreUpdate, apply).add_systems(Last, publish);
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(RenderStartup, limit_mesh_slabs);
    }
}

fn stop_rendering(error: &RenderError, main: &mut World, _: &mut World) -> RenderErrorPolicy {
    if let Some(bridge) = main.get_resource::<RenderSettingsBridge>()
        && let Ok(mut state) = bridge.0.lock()
        && state.renderer_error.is_none()
    {
        let mut characters = error.description.chars();
        let mut description: String = characters.by_ref().take(2048).collect();
        if characters.next().is_some() { description.push('…'); }
        state.renderer_error = Some(format!("Renderer stopped ({:?}). Save edits before restarting the viewer.\n{description}", error.ty));
    }
    RenderErrorPolicy::StopRendering
}

fn limit_mesh_slabs(device: Res<RenderDevice>, mut settings: ResMut<MeshAllocatorSettings>) {
    bound_mesh_slabs(&mut settings, device.limits().max_buffer_size);
}

fn bound_mesh_slabs(settings: &mut MeshAllocatorSettings, device_limit: u64) {
    let maximum = (device_limit / 2).max(1);
    settings.max_slab_size = settings.max_slab_size.min(maximum);
    settings.min_slab_size = settings.min_slab_size.min(settings.max_slab_size);
    settings.large_threshold = settings.large_threshold.min(settings.max_slab_size);
}

fn apply(world: &mut World) {
    let bridge = world.resource::<RenderSettingsBridge>().clone();
    let Some((level, steps, sides)) = bridge.0.lock().ok().map(|mut state| (
        state.requested.take(), state.requested_curve_steps.take(), state.requested_curve_surface_sides.take(),
    )) else { return };
    if let Some(level) = level {
        if level == 0 { world.remove_resource::<UsdSubdivisionSettings>(); }
        else if let Ok(settings) = UsdSubdivisionSettings::new(level) { world.insert_resource(settings); }
    }
    if let Some(steps) = steps && let Ok(settings) = UsdCurveSettings::new(steps) {
        let sides = world.get_resource::<UsdCurveSettings>().copied().unwrap_or_default().surface_sides();
        world.insert_resource(settings.with_surface_sides(sides).unwrap());
    }
    if let Some(sides) = sides {
        let current = world.get_resource::<UsdCurveSettings>().copied().unwrap_or_default();
        if let Ok(settings) = current.with_surface_sides(sides) { world.insert_resource(settings); }
    }
}

fn publish(world: &mut World) {
    let bridge = world.resource::<RenderSettingsBridge>().clone();
    let Ok(mut state) = bridge.0.lock() else { return };
    state.level = world.get_resource::<UsdSubdivisionSettings>().map_or(0, |settings| settings.levels());
    state.curve_steps = world.get_resource::<UsdCurveSettings>().copied().unwrap_or_default().cubic_steps();
    state.curve_surface_sides = world.get_resource::<UsdCurveSettings>().copied().unwrap_or_default().surface_sides();
    state.refined = world.query::<&UsdSubdivisionApplied>().iter(world).count();
    let mut meshes = std::collections::HashSet::new();
    let mut materials = std::collections::HashSet::new();
    state.mesh_entities = 0;
    for (mesh, material) in world.query_filtered::<(&Mesh3d, Option<&MeshMaterial3d<StandardMaterial>>), With<usd_bevy::UsdPrimRef>>().iter(world) {
        state.mesh_entities += 1;
        meshes.insert(mesh.0.id());
        if let Some(material) = material { materials.insert(material.0.id()); }
    }
    state.unique_meshes = meshes.len();
    state.standard_materials = materials.len();
    state.errors = world.query::<(&usd_bevy::UsdPrimRef, &UsdSubdivisionError)>().iter(world)
        .map(|(prim, error)| format!("{}: {}", prim.path, error.0)).collect();
    state.errors.extend(world.query::<(&usd_bevy::UsdPrimRef, &usd_bevy::route::instancer::UsdInstancerWarning)>().iter(world)
        .map(|(prim, error)| format!("{}: {}", prim.path, error.0)));
    state.errors.extend(world.query::<(&usd_bevy::UsdPrimRef, &UsdCurveError)>().iter(world)
        .map(|(prim, error)| format!("{}: {}", prim.path, error.0)));
    state.errors.sort();
    state.errors.dedup();
}

pub fn show(body: &mut PaneBody, bridge: &RenderSettingsBridge) {
    let Ok(state) = bridge.0.lock().map(|state| state.clone()) else { return };
    if let Some(error) = &state.renderer_error {
        let lines = super::lighting::status_lines(error);
        body.add_normal("rendering.failure", "Renderer stopped", "options", vec![
            Pod::new("rendering.failure.message").with_custom_units(lines.len(), move |ui| {
                for line in lines { ui.label(&line); }
            }),
        ]);
    }
    let current = if state.level == 0 { "Control cage".into() } else { format!("Subdivision level {}", state.level) };
    let mut pods = vec![Pod::new("rendering.status").with_custom_units(4, move |ui| {
        ui.label(&current);
        ui.label(&format!("Refined mesh prims: {}", state.refined));
        ui.label("Finite CPU refinement, not limit surfaces");
        ui.label("Higher levels can exceed geometry budgets");
    })];
    for level in 0..=6 {
        let bridge = bridge.clone();
        pods.push(Pod::new(Id::new(("rendering.level", level))).with_custom_units(1, move |ui| {
            let label = if level == 0 { "Use control cage".into() } else { format!("Use subdivision level {level}") };
            if ui.button(&label).clicked && let Ok(mut state) = bridge.0.lock() { state.requested = Some(level); }
        }));
    }
    body.add_normal("rendering.subdivision", "Subdivision", "options", pods);
    let mut curve_pods = vec![
        Pod::new("rendering.curves.status").with_custom_units(2, move |ui| {
            ui.label(&format!("Samples per segment: {}", state.curve_steps));
            ui.label(&match state.curve_surface_sides {
                Some(sides) => format!("Width surfaces; {sides} tube sides"),
                None => "Fixed sampling; one-pixel lines".into(),
            });
        }),
    ];
    for steps in [1, 8, 32, 64] {
        let bridge = bridge.clone();
        curve_pods.push(Pod::new(Id::new(("rendering.curves.steps", steps))).with_custom_units(1, move |ui| {
            if ui.button(&format!("Samples: {steps}")).clicked && let Ok(mut state) = bridge.0.lock() {
                state.requested_curve_steps = Some(steps);
            }
        }));
    }
    for sides in [None, Some(8), Some(16), Some(32)] {
        let bridge = bridge.clone();
        curve_pods.push(Pod::new(Id::new(("rendering.curves.surface", sides))).with_custom_units(1, move |ui| {
            let label = match sides { None => "Use lines".into(), Some(sides) => format!("Width surfaces: {sides} sides") };
            if ui.button(&label).clicked && let Ok(mut state) = bridge.0.lock() {
                state.requested_curve_surface_sides = Some(sides);
            }
        }));
    }
    body.add_normal("rendering.curves", "Curves", "options", curve_pods);
    body.add_normal("rendering.scene", "Scene assets", "options", vec![
        Pod::new("rendering.scene.counts").with_custom_units(5, move |ui| {
            ui.label(&format!("USD mesh entities: {}", state.mesh_entities));
            ui.label(&format!("Referenced mesh handles: {}", state.unique_meshes));
            ui.label(&format!("Standard material handles: {}", state.standard_materials));
            ui.label("Includes hidden USD mesh entities");
            ui.label("Handle counts, not GPU memory usage");
        }),
    ]);
    if state.errors.is_empty() { return; }
    let errors = state.errors.into_iter().enumerate().map(|(index, error)| {
        let lines = super::lighting::status_lines(&error);
        Pod::new(Id::new(("rendering.error", index))).with_custom_units(lines.len(), move |ui| {
            for line in lines { ui.label(&line); }
        })
    }).collect();
    body.add_normal("rendering.errors", "Projection errors", "options", errors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_counts_track_shared_handles_hidden_entities_and_cleanup() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bridge = RenderSettingsBridge::default();
        configure(&mut app, bridge.clone());
        let mut meshes = Assets::<Mesh>::default();
        let shared = meshes.add(Cuboid::default());
        let distinct = meshes.add(Cuboid::default());
        let mut materials = Assets::<StandardMaterial>::default();
        let material = materials.add(StandardMaterial::default());
        let other_material = materials.add(StandardMaterial::default());
        let first = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/First"), Mesh3d(shared.clone()), MeshMaterial3d(material.clone()))).id();
        let second = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/Second"), Mesh3d(shared), MeshMaterial3d(material), Visibility::Hidden)).id();
        let third = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/Third"), Mesh3d(distinct.clone()))).id();
        app.world_mut().spawn((Mesh3d(distinct), MeshMaterial3d(other_material.clone())));
        app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/NoMesh"), MeshMaterial3d(other_material.clone())));
        app.update();
        let counts = || {
            let state = bridge.0.lock().unwrap();
            (state.mesh_entities, state.unique_meshes, state.standard_materials)
        };
        assert_eq!(counts(), (3, 2, 1));
        app.world_mut().entity_mut(third).insert(MeshMaterial3d(other_material));
        app.update();
        assert_eq!(counts(), (3, 2, 2));
        app.world_mut().despawn(first);
        app.update();
        assert_eq!(counts(), (2, 2, 2));
        app.world_mut().despawn(second);
        app.world_mut().despawn(third);
        app.update();
        assert_eq!(counts(), (0, 0, 0));
    }

    #[test]
    fn renderer_failure_is_retained_without_exiting_or_resuming_rendering() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bridge = RenderSettingsBridge::default();
        configure(&mut app, bridge.clone());
        let entity = app.world_mut().spawn(Name::new("unsaved document entity")).id();
        let mut render = World::new();
        let error = RenderError {
            ty: bevy::render::error_handler::ErrorType::Validation,
            description: "mesh buffer exceeds device limit".into(), source: None,
        };
        let handler = app.world().resource::<RenderErrorHandler>().0;
        assert!(matches!(handler(&error, app.world_mut(), &mut render), RenderErrorPolicy::StopRendering));
        let first = bridge.renderer_error().unwrap();
        assert!(first.contains("Validation") && first.contains("mesh buffer exceeds device limit"));
        assert!(first.contains("Save edits before restarting"));
        let subsequent = RenderError { description: "later symptom".into(), ..error };
        assert!(matches!(handler(&subsequent, app.world_mut(), &mut render), RenderErrorPolicy::StopRendering));
        app.update();
        assert_eq!(bridge.renderer_error().as_deref(), Some(first.as_str()));
        assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "unsaved document entity");
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        assert!(matches!(stop_rendering(&subsequent, &mut World::new(), &mut render), RenderErrorPolicy::StopRendering));
        let mut main = World::new();
        let bridge = RenderSettingsBridge::default();
        main.insert_resource(bridge.clone());
        let long = RenderError { description: "é".repeat(4096), ..subsequent };
        assert!(matches!(stop_rendering(&long, &mut main, &mut render), RenderErrorPolicy::StopRendering));
        let message = bridge.renderer_error().unwrap();
        assert_eq!(message.matches('é').count(), 2048);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn mesh_slabs_respect_shared_device_limits_and_smaller_settings() {
        let mut settings = MeshAllocatorSettings::default();
        bound_mesh_slabs(&mut settings, 256 * 1024 * 1024);
        assert_eq!(settings.max_slab_size, 128 * 1024 * 1024);
        assert_eq!(settings.min_slab_size, 1024 * 1024);
        assert_eq!(settings.large_threshold, 128 * 1024 * 1024);
        assert_eq!(settings.growth_factor, 1.5);
        settings.max_slab_size = 64 * 1024;
        settings.large_threshold = 32 * 1024;
        bound_mesh_slabs(&mut settings, 256 * 1024 * 1024);
        assert_eq!(settings.max_slab_size, 64 * 1024);
        assert_eq!(settings.min_slab_size, 64 * 1024);
        assert_eq!(settings.large_threshold, 32 * 1024);
        bound_mesh_slabs(&mut settings, 32 * 1024);
        assert_eq!(settings.max_slab_size, 16 * 1024);
        assert_eq!(settings.min_slab_size, 16 * 1024);
        assert_eq!(settings.large_threshold, 16 * 1024);
    }

    #[test]
    fn curve_surface_controls_preserve_sampling_and_consume_requests() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bridge = RenderSettingsBridge::default();
        configure(&mut app, bridge.clone());
        for sides in [Some(8), Some(16), None, Some(32)] {
            {
                let mut state = bridge.0.lock().unwrap();
                state.requested_curve_steps = Some(32);
                state.requested_curve_surface_sides = Some(sides);
            }
            app.update();
            let settings = app.world().resource::<UsdCurveSettings>();
            assert_eq!(settings.cubic_steps(), 32);
            assert_eq!(settings.surface_sides(), sides);
            let state = bridge.0.lock().unwrap();
            assert_eq!(state.curve_surface_sides, sides);
            assert_eq!(state.requested_curve_surface_sides, None);
        }
        bridge.0.lock().unwrap().requested_curve_surface_sides = Some(Some(2));
        app.update();
        assert_eq!(app.world().resource::<UsdCurveSettings>().surface_sides(), Some(32));
        app.world_mut().insert_resource(UsdCurveSettings::default());
        app.update();
        assert_eq!(bridge.0.lock().unwrap().curve_surface_sides, None);
    }

    #[test]
    fn curve_controls_preserve_startup_quality_and_publish_errors() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bridge = RenderSettingsBridge::default();
        configure(&mut app, bridge.clone());
        app.insert_resource(UsdCurveSettings::new(32).unwrap().with_surface_sides(Some(12)).unwrap());
        app.update();
        assert_eq!(bridge.0.lock().unwrap().curve_steps, 32);
        bridge.0.lock().unwrap().requested_curve_steps = Some(1);
        app.update();
        assert_eq!(app.world().resource::<UsdCurveSettings>().cubic_steps(), 1);
        assert_eq!(app.world().resource::<UsdCurveSettings>().surface_sides(), Some(12));
        assert_eq!(bridge.0.lock().unwrap().curve_steps, 1);
        let prim = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/Curve"), UsdCurveError("curve budget exceeded".into()))).id();
        app.update();
        assert_eq!(bridge.0.lock().unwrap().errors, ["/Curve: curve budget exceeded"]);
        bridge.0.lock().unwrap().requested_curve_steps = Some(64);
        app.world_mut().entity_mut(prim).remove::<UsdCurveError>();
        app.update();
        assert_eq!(bridge.0.lock().unwrap().curve_steps, 64);
        assert!(bridge.0.lock().unwrap().errors.is_empty());
        assert!(!app.world().contains_resource::<UsdSubdivisionSettings>());
    }

    #[test]
    fn controls_preserve_initial_settings_and_publish_recovery() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let bridge = RenderSettingsBridge::default();
        configure(&mut app, bridge.clone());
        app.insert_resource(UsdSubdivisionSettings::new(2).unwrap());
        let prim = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/M"), UsdSubdivisionError("unsupported rule".into()))).id();
        app.update();
        assert_eq!(bridge.0.lock().unwrap().level, 2);
        assert_eq!(bridge.0.lock().unwrap().errors, ["/M: unsupported rule"]);
        bridge.0.lock().unwrap().requested = Some(1);
        app.world_mut().entity_mut(prim).remove::<UsdSubdivisionError>().insert(UsdSubdivisionApplied { levels: 1 });
        app.update();
        assert_eq!(bridge.0.lock().unwrap().level, 1);
        assert_eq!(bridge.0.lock().unwrap().refined, 1);
        assert!(bridge.0.lock().unwrap().errors.is_empty());
        bridge.0.lock().unwrap().requested = Some(0);
        app.update();
        assert!(!app.world().contains_resource::<UsdSubdivisionSettings>());
    }
}
