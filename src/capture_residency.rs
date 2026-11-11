use bevy::{ecs::system::SystemParam, prelude::*};
use usd_bevy::{
    editor::{EditorBridge, EditorCommand, EditorView},
    live::PrimEntities,
    route::residency::{DeferHiddenMeshes, UsdDeferredMesh},
};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    prim: String,
    mesh: String,
    steps: usize,
}

impl Config {
    pub fn parse(value: Option<String>) -> Result<Option<Self>, String> {
        let Some(value) = value else { return Ok(None) };
        if value.len() > 8192 {
            return Err("USD_CAPTURE_RESIDENCY exceeds 8192 bytes".into());
        }
        let config: Self = serde_json::from_str(&value).map_err(|error| error.to_string())?;
        if !(1..=4).contains(&config.steps)
            || [&config.prim, &config.mesh].into_iter().any(|path| {
                !path.starts_with('/')
                    || path.len() == 1
                    || path.chars().any(char::is_control)
                    || openusd::sdf::path(path).is_err()
            })
            || !(config.mesh == config.prim
                || config.mesh.starts_with(&format!("{}/", config.prim)))
        {
            return Err(
                "USD_CAPTURE_RESIDENCY requires absolute prim/descendant mesh paths and 1..4 steps"
                    .into(),
            );
        }
        Ok(Some(config))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Observation {
    document: u64,
    revision: u64,
    time: f64,
    entity: Entity,
    selected: Option<String>,
    visible: bool,
    inherited_visible: bool,
    undo: bool,
    redo: bool,
    resident: bool,
    deferred: bool,
    policy: bool,
    camera: String,
}

#[derive(SystemParam)]
pub struct Inputs<'w, 's> {
    prims: Res<'w, PrimEntities>,
    meshes: Query<
        'w,
        's,
        (
            Option<&'static Mesh3d>,
            Option<&'static UsdDeferredMesh>,
            Option<&'static InheritedVisibility>,
        ),
    >,
    policy: Option<Res<'w, DeferHiddenMeshes>>,
}

impl Inputs<'_, '_> {
    fn observe(
        &self,
        config: &Config,
        view: EditorView,
        camera: String,
    ) -> Result<Observation, String> {
        if view.status != "Ready" || view.timeline.playing {
            return Err(format!(
                "residency capture requires a ready paused editor: {}",
                view.status
            ));
        }
        let entity = self
            .prims
            .entity(&config.mesh)
            .ok_or("capture mesh is not projected")?;
        let (mesh, deferred, visibility) =
            self.meshes.get(entity).map_err(|error| error.to_string())?;
        Ok(Observation {
            document: view.document.document_id,
            revision: view.document.revision,
            time: view.timeline.current,
            entity,
            selected: view.document.selected,
            visible: *view
                .document
                .visibility
                .get(&config.prim)
                .ok_or("capture prim lacks visibility")?,
            inherited_visible: visibility
                .ok_or("capture mesh lacks inherited visibility")?
                .get(),
            undo: view.document.can_undo,
            redo: view.document.can_redo,
            resident: mesh.is_some(),
            deferred: deferred.is_some(),
            policy: self.policy.is_some(),
            camera,
        })
    }
}

#[derive(Default)]
pub struct State {
    settled: u32,
    observations: Vec<Observation>,
    pending: bool,
}

impl State {
    pub fn advance(
        &mut self,
        config: &Config,
        inputs: &Inputs,
        bridge: &EditorBridge,
        camera: String,
    ) -> Result<bool, String> {
        self.settled = self.settled.saturating_add(1);
        if self.settled < 60 {
            return Ok(false);
        }
        let observed = inputs.observe(
            config,
            bridge.view().map_err(|error| error.to_string())?,
            camera,
        )?;
        self.step(config, observed, bridge)
    }

    fn step(
        &mut self,
        config: &Config,
        observed: Observation,
        bridge: &EditorBridge,
    ) -> Result<bool, String> {
        if self.observations.is_empty() {
            if observed.visible
                || observed.inherited_visible
                || observed.undo
                || observed.redo
                || observed.selected.is_some()
                || observed.resident == observed.policy
                || observed.deferred != observed.policy
            {
                return Err(format!(
                    "residency capture initial state is not hidden and pristine: {observed:?}"
                ));
            }
            self.observations.push(observed);
        } else {
            let initial = &self.observations[0];
            if observed.document != initial.document
                || observed.time != initial.time
                || observed.entity != initial.entity
                || observed.policy != initial.policy
                || observed.camera != initial.camera
            {
                return Err(
                    "residency capture changed document, clock, entity, policy or camera".into(),
                );
            }
            if self.pending {
                if self.observations.last() == Some(&observed) {
                    return Ok(false);
                }
                let step = self.observations.len();
                let visible = matches!(step, 2 | 4);
                let resident = !initial.policy || step > 1;
                if observed.revision != initial.revision + (step - 1) as u64
                    || observed.selected.as_deref() != Some(config.mesh.as_str())
                    || observed.visible != visible
                    || observed.inherited_visible != visible
                    || observed.undo != visible
                    || observed.redo != (step == 3)
                    || observed.resident != resident
                    || observed.deferred == resident
                {
                    return Err(format!(
                        "residency step {step} acknowledgement differs: {observed:?}"
                    ));
                }
                self.observations.push(observed);
                self.pending = false;
            } else if self.observations.last() != Some(&observed) {
                return Err("residency capture state changed after the final command".into());
            }
        }
        if self.observations.len() > config.steps {
            return Ok(true);
        }
        let command = match self.observations.len() {
            1 => EditorCommand::Select(Some(config.mesh.clone())),
            2 => EditorCommand::Visibility {
                prim: config.prim.clone(),
                visible: true,
            },
            3 => EditorCommand::Undo,
            4 => EditorCommand::Redo,
            _ => return Err("invalid residency capture step".into()),
        };
        bridge.send(command).map_err(|error| error.to_string())?;
        self.pending = true;
        Ok(false)
    }

    pub fn report(&self, config: &Config) -> String {
        let mut report = format!(
            "residency_scope=live-editor-select-reveal-undo-redo\nresidency_prim={}\nresidency_mesh={}\nresidency_steps_requested={}\nresidency_steps_observed={}\n",
            config.prim,
            config.mesh,
            config.steps,
            self.observations.len().saturating_sub(1)
        );
        for (step, observation) in self.observations.iter().enumerate() {
            report.push_str(&format!("residency_step_{step}=revision:{},time:{},selected:{:?},visible:{},inherited:{},resident:{},deferred:{},undo:{},redo:{},policy:{},entity:{:?}\n",
                observation.revision, observation.time, observation.selected, observation.visible,
                observation.inherited_visible, observation.resident, observation.deferred,
                observation.undo, observation.redo, observation.policy, observation.entity));
        }
        if let Some(initial) = self.observations.first() {
            for line in initial.camera.lines() {
                report.push_str(&format!("residency_initial_{line}\n"));
            }
        }
        report
    }
}

#[cfg(test)]
mod tests;
