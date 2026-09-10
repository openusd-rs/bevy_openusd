use std::sync::{Arc, Mutex};
use bevy::prelude::*;
use bevy::render::{RenderApp, RenderStartup, mesh::allocator::MeshAllocatorSettings, renderer::RenderDevice};
use bevy::render::error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy};
use mara::ui::mara_core::{pane::PaneBody, pod::Pod, vocab::Id};
use usd_bevy::route::subdivision::{UsdSubdivisionApplied, UsdSubdivisionError, UsdSubdivisionSettings};

#[derive(Clone, Default)]
struct State {
    requested: Option<u32>,
    level: u32,
    refined: usize,
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
    let Some(level) = bridge.0.lock().ok().and_then(|mut state| state.requested.take()) else { return };
    if level == 0 { world.remove_resource::<UsdSubdivisionSettings>(); }
    else if let Ok(settings) = UsdSubdivisionSettings::new(level) { world.insert_resource(settings); }
}

fn publish(world: &mut World) {
    let bridge = world.resource::<RenderSettingsBridge>().clone();
    let Ok(mut state) = bridge.0.lock() else { return };
    state.level = world.get_resource::<UsdSubdivisionSettings>().map_or(0, |settings| settings.levels());
    state.refined = world.query::<&UsdSubdivisionApplied>().iter(world).count();
    state.errors = world.query::<(&usd_bevy::UsdPrimRef, &UsdSubdivisionError)>().iter(world)
        .map(|(prim, error)| format!("{}: {}", prim.path, error.0)).collect();
    state.errors.extend(world.query::<(&usd_bevy::UsdPrimRef, &usd_bevy::route::instancer::UsdInstancerWarning)>().iter(world)
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
