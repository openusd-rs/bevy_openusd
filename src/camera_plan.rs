use std::sync::{Arc, Mutex};
use bevy::prelude::*;
use mara::ui::{mara_core::{pane::PaneBody, pod::Pod}, modules::bevy as mara_bevy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Waypoint {
    pub name: String,
    pub focus: [f32; 3],
    pub yaw: f32,
    pub elevation: f32,
    pub distance: f32,
    pub fov: f32,
    pub seconds: f32,
}

impl Waypoint {
    fn valid(&self) -> bool {
        self.focus.iter().all(|v| v.is_finite()) && self.yaw.is_finite()
            && self.elevation.is_finite() && self.elevation.abs() < std::f32::consts::FRAC_PI_2
            && self.distance.is_finite() && self.distance > 0.0
            && self.fov.is_finite() && self.fov > 0.0 && self.fov < std::f32::consts::PI
            && self.seconds.is_finite() && (0.01..=3600.0).contains(&self.seconds)
    }

    fn between(&self, next: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let angle = (next.yaw - self.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let mut result = self.clone();
        result.focus = Vec3::from_array(self.focus).lerp(Vec3::from_array(next.focus), t).to_array();
        result.yaw += angle * t;
        result.elevation += (next.elevation - self.elevation) * t;
        result.distance += (next.distance - self.distance) * t;
        result.fov += (next.fov - self.fov) * t;
        result
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan { version: u32, points: Vec<Waypoint> }

impl Plan {
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let plan: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if plan.version != 1 || plan.points.len() > 1000 || !plan.points.iter().all(Waypoint::valid) {
            return Err("Invalid camera plan or unsupported version".into());
        }
        Ok(plan)
    }
}

struct State {
    points: Vec<Waypoint>,
    current: Option<Waypoint>,
    jump: Option<usize>,
    elapsed: Option<f32>,
    path: String,
    status: String,
}

impl Default for State {
    fn default() -> Self {
        Self { points: vec![], current: None, jump: None, elapsed: None,
            path: "camera-plan.json".into(), status: String::new() }
    }
}

#[derive(Resource, Clone, Default)]
pub struct CameraPlan(Arc<Mutex<State>>);

impl CameraPlan {
    pub fn playing(&self) -> bool { self.0.lock().unwrap().elapsed.is_some() }
}

fn sample(points: &[Waypoint], elapsed: f32) -> Option<(Waypoint, bool)> {
    let mut remaining = elapsed.max(0.0);
    for pair in points.windows(2) {
        let duration = pair[1].seconds;
        if remaining < duration { return Some((pair[0].between(&pair[1], remaining / duration), false)); }
        remaining -= duration;
    }
    points.last().cloned().map(|pose| (pose, true))
}

pub fn configure(app: &mut App, bridge: CameraPlan) {
    app.insert_resource(bridge).add_systems(PostUpdate, update.before(bevy::transform::TransformSystems::Propagate));
}

fn update(bridge: Res<CameraPlan>, time: Res<Time<Real>>,
    mut cameras: Query<(&Camera, &mut mara_bevy::ChaseCamera, &mut Transform, &mut Projection)>) {
    let mut state = bridge.0.lock().unwrap();
    let Some((_, mut rig, mut transform, mut projection)) = cameras.iter_mut().find(|(c, ..)| c.is_active) else {
        state.current = None;
        state.elapsed = None;
        return;
    };
    let Projection::Perspective(perspective) = &mut *projection else { state.current = None; state.elapsed = None; return; };
    let mut pose = state.jump.take().and_then(|i| state.points.get(i).cloned());
    if let Some(elapsed) = state.elapsed {
        if let Some((sampled, finished)) = sample(&state.points, elapsed) {
            pose = Some(sampled);
            state.elapsed = if finished { None } else { Some(elapsed + time.delta_secs()) };
        } else { state.elapsed = None; }
    }
    if let Some(pose) = pose {
        rig.focus = Vec3::from_array(pose.focus);
        rig.yaw = pose.yaw;
        rig.elevation = pose.elevation;
        rig.distance = pose.distance;
        rig.min_distance = rig.min_distance.min(pose.distance);
        rig.max_distance = rig.max_distance.max(pose.distance);
        perspective.fov = pose.fov;
        mara_bevy::apply_rig(&rig, &mut transform);
    }
    state.current = Some(Waypoint { name: String::new(), focus: rig.focus.to_array(), yaw: rig.yaw,
        elevation: rig.elevation, distance: rig.distance, fov: perspective.fov, seconds: 3.0 });
}

pub fn show(body: &mut PaneBody, bridge: &CameraPlan) {
    let bridge = bridge.clone();
    let rows = bridge.0.lock().unwrap().points.len();
    body.add_normal("camera.plan", "Camera path", "camera", vec![
        Pod::new("camera.plan.controls").with_custom_units(8 + rows * 3, move |ui| {
            let mut state = bridge.0.lock().unwrap();
            let playing = state.elapsed.is_some();
            ui.label("Move the camera, then add a waypoint.");
            ui.horizontal(|ui| {
                if !playing && state.current.is_some() && ui.button("Add view").clicked {
                    let mut pose = state.current.clone().unwrap();
                    pose.name = format!("View {}", state.points.len() + 1);
                    state.points.push(pose);
                }
                if !playing && state.points.len() >= 2 && ui.button("Play path").clicked {
                    state.elapsed = Some(0.0);
                }
                if playing && ui.button("Stop").clicked { state.elapsed = None; }
            });
            let mut jump = None;
            let mut remove = None;
            let mut swap = None;
            if !playing {
                let current = state.current.clone();
                for (i, point) in state.points.iter_mut().enumerate() {
                    ui.in_id_scope(mara::ui::mara_core::vocab::Id::new(("camera.point", i)), &mut |ui| {
                        ui.text_input(&mut point.name, "Waypoint name");
                        ui.horizontal(|ui| {
                            if ui.button("Go").clicked { jump = Some(i); }
                            if current.is_some() && ui.button("Update").clicked {
                                let mut replacement = current.clone().unwrap();
                                replacement.name = point.name.clone(); replacement.seconds = point.seconds; *point = replacement;
                            }
                            if i > 0 && ui.button("Up").clicked { swap = Some(i); }
                            if ui.button("Delete").clicked { remove = Some(i); }
                        });
                        if i > 0 {
                            let mut seconds = point.seconds as f64;
                            ui.drag_value("Travel", &mut seconds, 0.1, 0.01..=3600.0, 2, "s");
                            point.seconds = seconds as f32;
                        }
                        else { ui.label("Starting view"); }
                    });
                }
                if let Some(i) = swap { state.points.swap(i, i - 1); }
                if let Some(i) = remove { state.points.remove(i); }
                state.jump = jump;
                ui.text_input(&mut state.path, "Plan file path");
                ui.horizontal(|ui| {
                    if ui.button("Save new plan").clicked {
                        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                            let bytes = serde_json::to_vec_pretty(&Plan { version: 1, points: state.points.clone() })?;
                            use std::io::Write;
                            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&state.path)?;
                            file.write_all(&bytes)?; file.sync_all()?; Ok(())
                        })();
                        state.status = result.map(|_| "Plan saved".into()).unwrap_or_else(|e| e.to_string());
                    }
                    if ui.button("Load plan").clicked {
                        let result = std::fs::read(&state.path).map_err(|e| e.to_string()).and_then(|bytes| Plan::decode(&bytes));
                        match result {
                            Ok(plan) => { state.points = plan.points; state.jump = None; state.status = "Plan loaded".into(); }
                            Err(error) => state.status = error,
                        }
                    }
                });
            }
            ui.label(&state.status);
        }),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(yaw: f32) -> Waypoint { Waypoint { name: "view".into(), focus: [0.0; 3], yaw,
        elevation: 0.3, distance: 10.0, fov: 0.8, seconds: 2.0 } }
    #[test]
    fn path_uses_short_yaw_arc_and_reaches_final_view() {
        let a = point(179_f32.to_radians()); let b = point(-179_f32.to_radians());
        let points = [a.clone(), b.clone()];
        let (middle, finished) = sample(&points, 1.0).unwrap();
        assert!(!finished); assert!((middle.yaw - std::f32::consts::PI).abs() < 0.001);
        assert_eq!(sample(&points, 0.0).unwrap().0.yaw, a.yaw);
        let (last, finished) = sample(&points, 2.0).unwrap();
        assert!(finished); assert_eq!(last.yaw, b.yaw);
        assert!(sample(&[], 0.0).is_none());
    }
    #[test]
    fn persisted_plans_validate_before_replacement() {
        let mut plan = Plan { version: 1, points: vec![point(0.0)] };
        assert!(Plan::decode(&serde_json::to_vec(&plan).unwrap()).is_ok());
        plan.points[0].seconds = 0.0;
        assert!(Plan::decode(&serde_json::to_vec(&plan).unwrap()).is_err());
        plan.points.clear(); plan.version = 2;
        assert!(Plan::decode(&serde_json::to_vec(&plan).unwrap()).is_err());
    }
    #[test]
    fn playback_updates_the_camera_and_stops_at_the_endpoint() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>();
        let bridge = CameraPlan::default();
        configure(&mut app, bridge.clone());
        let camera = app.world_mut().spawn((Camera3d::default(), mara_bevy::ChaseCamera::default(),
            Transform::default(), Projection::Perspective(PerspectiveProjection::default()))).id();
        let mut last = point(1.0); last.focus = [1000.0, 20.0, 30.0]; last.fov = 1.2;
        {
            let mut state = bridge.0.lock().unwrap();
            state.points = vec![point(0.0), last.clone()]; state.elapsed = Some(2.0);
        }
        app.update();
        assert!(!bridge.playing());
        let rig = app.world().get::<mara_bevy::ChaseCamera>(camera).unwrap();
        assert_eq!(rig.focus.to_array(), last.focus);
        assert_eq!(rig.yaw, last.yaw);
        assert!(app.world().get::<Transform>(camera).unwrap().translation.is_finite());
        let Projection::Perspective(projection) = app.world().get::<Projection>(camera).unwrap() else { panic!() };
        assert_eq!(projection.fov, last.fov);
    }
}
