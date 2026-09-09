use std::sync::{Arc, Mutex};
use bevy::prelude::*;
use mara::ui::modules::bevy as mara_bevy;
use mara::ui::mara_core::{pane::PaneBody, pod::Pod, vocab::Id};
use usd_bevy::route::dome_environment::{UsdDomeEnvironmentPlugin, UsdDomeEnvironmentSource, UsdDomeEnvironmentState};

#[derive(Clone)]
struct State {
    selected: Option<String>,
    studio: bool,
    domes: Vec<String>,
    status: String,
}

impl Default for State {
    fn default() -> Self { Self { selected: None, studio: true, domes: Vec::new(), status: "Studio lighting".into() } }
}

#[derive(Resource, Clone, Default)]
pub struct LightingBridge(Arc<Mutex<State>>);

impl LightingBridge {
    pub fn from_env() -> Self {
        let mut state = State::default();
        if let Ok(path) = std::env::var("USD_VIEWER_DOME") {
            state.selected = Some(path);
            state.studio = false;
        }
        Self(Arc::new(Mutex::new(state)))
    }
}

#[derive(Component)]
struct StudioAmbient(AmbientLight);

pub fn configure(app: &mut App, bridge: LightingBridge) {
    app.insert_resource(bridge).add_plugins(UsdDomeEnvironmentPlugin).add_systems(Update, apply);
}

fn apply(world: &mut World) {
    let bridge = world.resource::<LightingBridge>().clone();
    let Ok(mut state) = bridge.0.lock() else { return };
    let mut domes: Vec<_> = world.query_filtered::<(Entity, &usd_bevy::UsdPrimRef), With<usd_bevy::route::dome::UsdDomeLight>>()
        .iter(world).map(|(entity, prim)| (entity, prim.path.clone())).collect();
    domes.sort_by(|a, b| a.1.cmp(&b.1));
    state.domes = domes.iter().map(|(_, path)| path.clone()).collect();
    let selected = state.selected.as_ref().and_then(|path| domes.iter().find(|(_, candidate)| candidate == path).map(|(e, _)| *e));
    let cameras: Vec<_> = world.query_filtered::<Entity, With<mara_bevy::ChaseCamera>>().iter(world).collect();
    state.status = if state.selected.is_some() && selected.is_none() { "Selected dome is missing".into() }
        else if selected.is_some() { "Waiting for dome maps".into() }
        else if state.studio { "Studio lighting".into() }
        else { "Studio lights disabled".into() };
    for camera in cameras {
        if world.get::<StudioAmbient>(camera).is_none() {
            let ambient = world.get::<AmbientLight>(camera).cloned().unwrap_or_default();
            world.entity_mut(camera).insert(StudioAmbient(ambient));
        }
        let mut ambient = world.get::<StudioAmbient>(camera).unwrap().0.clone();
        if !state.studio { ambient.brightness = 0.0; }
        world.entity_mut(camera).insert(ambient);
        if let Some(dome) = selected {
            if world.get::<UsdDomeEnvironmentSource>(camera).is_none_or(|source| source.dome != dome) {
                world.entity_mut(camera).insert(UsdDomeEnvironmentSource::new(dome));
            }
            if let Some(status) = world.get::<UsdDomeEnvironmentState>(camera) {
                state.status = match status {
                    UsdDomeEnvironmentState::Attached => "Dome maps attached".into(),
                    UsdDomeEnvironmentState::WaitingForMaps => "Waiting for dome maps".into(),
                    UsdDomeEnvironmentState::Unavailable(error) => error.clone(),
                };
            }
        } else { world.entity_mut(camera).remove::<UsdDomeEnvironmentSource>(); }
    }
    for (mut light, studio) in world.query::<(&mut DirectionalLight, &super::environment::StudioLight)>().iter_mut(world) {
        light.illuminance = if state.studio { studio.0 } else { 0.0 };
    }
}

pub fn show(body: &mut PaneBody, bridge: &LightingBridge) {
    let Ok(state) = bridge.0.lock().map(|state| state.clone()) else { return };
    let controls = bridge.clone();
    let studio = state.studio;
    let lines = status_lines(&state.status);
    let mut pods = vec![Pod::new("lighting.status").with_custom_units(lines.len() + 1, move |ui| {
            ui.label("Status");
            for line in lines { ui.label(&line); }
        }),
        Pod::new("lighting.selection").with_readout("Dome", state.selected.as_deref().unwrap_or("None")),
        Pod::new("lighting.controls").with_custom_units(3, move |ui| {
            if ui.button("Use studio only").clicked && let Ok(mut state) = controls.0.lock() {
                state.selected = None;
                state.studio = true;
            }
            if ui.button(if studio { "Disable studio lights" } else { "Enable studio lights" }).clicked
                && let Ok(mut state) = controls.0.lock() { state.studio = !studio; }
        })];
    for path in state.domes {
        let controls = bridge.clone();
        pods.push(Pod::new(Id::new(("lighting.dome", &path))).with_custom_units(1, move |ui| {
            if ui.button(&format!("Use {path}")).clicked && let Ok(mut state) = controls.0.lock() {
                state.selected = Some(path);
                state.studio = false;
            }
        }));
    }
    body.add_normal("lighting.settings", "Viewport lighting", "options", pods);
}

pub(crate) fn status_lines(status: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in status.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > 40 {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() { line.push(' '); }
        for character in word.chars() {
            if line.chars().count() == 40 { lines.push(std::mem::take(&mut line)); }
            line.push(character);
        }
    }
    if !line.is_empty() { lines.push(line); }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_status_messages_fit_narrow_panes_without_losing_words() {
        let status = "GPU device cannot filter dome maps: requires 6 storage textures per shader stage and compute support";
        let lines = status_lines(status);
        assert_eq!(lines.join(" "), status);
        assert!(lines.iter().all(|line| line.chars().count() <= 40));
        let token = "é".repeat(85);
        assert_eq!(status_lines(&token).concat(), token);
        assert!(status_lines(&token).iter().all(|line| line.chars().count() <= 40));
    }

    #[test]
    fn selection_and_studio_switch_do_not_modify_authored_lights() {
        let mut world = World::new();
        let bridge = LightingBridge::default();
        world.insert_resource(bridge.clone());
        let camera = world.spawn((mara_bevy::ChaseCamera::default(), AmbientLight { brightness: 160.0, ..default() })).id();
        let studio = world.spawn((DirectionalLight::default(), super::super::environment::StudioLight(7500.0))).id();
        let authored = world.spawn(DirectionalLight { illuminance: 42.0, ..default() }).id();
        let dome = world.spawn((usd_bevy::UsdPrimRef::new("/Env"), usd_bevy::route::dome::UsdDomeLight::default())).id();
        bridge.0.lock().unwrap().selected = Some("/Env".into());
        bridge.0.lock().unwrap().studio = false;
        apply(&mut world);
        assert_eq!(world.get::<UsdDomeEnvironmentSource>(camera).unwrap().dome, dome);
        assert_eq!(world.get::<AmbientLight>(camera).unwrap().brightness, 0.0);
        assert_eq!(world.get::<DirectionalLight>(studio).unwrap().illuminance, 0.0);
        assert_eq!(world.get::<DirectionalLight>(authored).unwrap().illuminance, 42.0);
        world.despawn(dome);
        apply(&mut world);
        assert!(world.get::<UsdDomeEnvironmentSource>(camera).is_none());
        assert_eq!(bridge.0.lock().unwrap().status, "Selected dome is missing");
        *bridge.0.lock().unwrap() = State::default();
        apply(&mut world);
        assert_eq!(world.get::<AmbientLight>(camera).unwrap().brightness, 160.0);
        assert_eq!(world.get::<DirectionalLight>(studio).unwrap().illuminance, 7500.0);
    }
}
