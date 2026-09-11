use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use openusd::usd::{CommittedChange, Stage, StageSink, StageSinkId};

#[derive(Clone, Default)]
struct Changes(Rc<RefCell<BTreeMap<String, u64>>>);

impl StageSink for Changes {
    fn after_commit(&self, _stage: &Stage, change: &CommittedChange<'_>) {
        let mut revisions = self.0.borrow_mut();
        for (layer, changes) in change.layer_changes {
            if !changes.is_empty() {
                let revision = revisions.entry(layer.clone()).or_default();
                *revision = revision.saturating_add(1);
            }
        }
    }
}

pub(super) struct LayerChanges {
    stage: Stage,
    sink: StageSinkId,
    changes: Changes,
}

impl LayerChanges {
    pub fn new(stage: &Stage) -> Self {
        let changes = Changes::default();
        let sink = stage.add_sink(changes.clone());
        Self { stage: stage.clone(), sink, changes }
    }

    pub fn revisions(&self) -> BTreeMap<String, u64> {
        let _ = self.stage.layer_count();
        self.changes.0.borrow().clone()
    }
}

impl Drop for LayerChanges {
    fn drop(&mut self) { self.stage.remove_sink(self.sink); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_tracker_detaches_sink_from_surviving_stage() {
        let stage = Stage::builder().in_memory("tracking.usda").unwrap();
        let tracker = LayerChanges::new(&stage);
        let changes = tracker.changes.clone();
        drop(tracker);
        assert_eq!(Rc::strong_count(&changes.0), 1);
        stage.define_prim("/After").unwrap();
        assert!(changes.0.borrow().is_empty());
    }
}
