use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderSystems,
    render_resource::{CachedPipelineState, PipelineCache},
};
use bevy::shader::ShaderCacheError;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct PipelineSnapshot {
    compiled: usize,
    pending: usize,
    errors: Vec<String>,
}

impl PipelineSnapshot {
    fn observe(&mut self, state: &CachedPipelineState) {
        match state {
            CachedPipelineState::Ok(_) => self.compiled += 1,
            CachedPipelineState::Err(
                ShaderCacheError::ShaderNotLoaded(_)
                | ShaderCacheError::ShaderImportNotYetAvailable,
            ) => self.pending += 1,
            CachedPipelineState::Err(error) => self.errors.push(error.to_string()),
            _ => self.pending += 1,
        }
    }

    fn ready_count(&self) -> Option<usize> {
        (self.compiled > 0 && self.pending == 0 && self.errors.is_empty()).then_some(self.compiled)
    }
}

#[derive(Clone, Resource, Default)]
pub struct PipelineProbe(Arc<Mutex<PipelineSnapshot>>);

impl PipelineProbe {
    pub fn snapshot(&self) -> PipelineSnapshot {
        self.0.lock().unwrap().clone()
    }

    pub fn ready_count(&self) -> Option<usize> {
        self.0.lock().unwrap().ready_count()
    }
}

pub fn configure(app: &mut App) {
    let probe = PipelineProbe::default();
    app.insert_resource(probe.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(probe)
        .add_systems(Render, report.in_set(RenderSystems::Cleanup));
}

fn report(cache: Res<PipelineCache>, probe: Res<PipelineProbe>) {
    let mut snapshot = PipelineSnapshot::default();
    for pipeline in cache.pipelines() {
        snapshot.observe(&pipeline.state);
    }
    *probe.0.lock().unwrap() = snapshot;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_unobserved_cache_is_not_ready() {
        assert_eq!(PipelineSnapshot::default().ready_count(), None);
        assert_eq!(PipelineProbe::default().ready_count(), None);
    }

    #[test]
    fn pending_imports_and_permanent_errors_prevent_readiness() {
        for state in [
            CachedPipelineState::Queued,
            CachedPipelineState::Err(ShaderCacheError::ShaderImportNotYetAvailable),
            CachedPipelineState::Err(ShaderCacheError::ShaderNotLoaded(
                Handle::<bevy::shader::Shader>::default().id(),
            )),
            CachedPipelineState::Err(ShaderCacheError::ProcessShaderError(
                "invalid shader".into(),
            )),
            CachedPipelineState::Err(ShaderCacheError::CreateShaderModule(
                "validation failure".into(),
            )),
        ] {
            let mut snapshot = PipelineSnapshot {
                compiled: 3,
                ..default()
            };
            snapshot.observe(&state);
            assert_eq!(snapshot.ready_count(), None);
        }
    }

    #[test]
    fn readiness_tracks_the_latest_complete_render_report() {
        let probe = PipelineProbe::default();
        let render = probe.clone();
        *render.0.lock().unwrap() = PipelineSnapshot {
            compiled: 3,
            ..default()
        };
        assert_eq!(probe.ready_count(), Some(3));
        render.0.lock().unwrap().pending = 1;
        assert_eq!(probe.ready_count(), None);
        *render.0.lock().unwrap() = PipelineSnapshot {
            compiled: 4,
            ..default()
        };
        assert_eq!(probe.ready_count(), Some(4));
    }
}
