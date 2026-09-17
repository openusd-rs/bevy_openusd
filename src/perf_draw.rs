use std::{collections::HashMap, marker::PhantomData, sync::Mutex};
use bevy::{prelude::*, ecs::{query::ROQueryItem, system::{lifetimeless::SRes, ReadOnlySystemParam, SystemParamItem}}};
use bevy::render::{render_phase::{DrawFunctions, PhaseItem, RenderCommand, RenderCommandResult, RenderCommandState, TrackedRenderPass}, Render, RenderSystems};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Counts {
    fn record(&mut self, result: &RenderCommandResult) {
        match result {
            RenderCommandResult::Success => self.succeeded += 1,
            RenderCommandResult::Skip => self.skipped += 1,
            RenderCommandResult::Failure(_) => self.failed += 1,
        }
    }

    pub(super) fn ready(&self, expected: usize) -> bool {
        self.skipped == 0 && self.failed == 0 && (expected == 0 || self.succeeded > 0)
    }
}

#[derive(Resource, Default)]
pub(super) struct DrawProbe(Mutex<HashMap<(Entity, &'static str), Counts>>);

impl DrawProbe {
    pub(super) fn for_view(&self, view: Entity) -> Counts {
        let mut total = Counts::default();
        for ((entity, _), count) in self.0.lock().expect("draw probe").iter() {
            if *entity == view {
                total.succeeded += count.succeeded;
                total.skipped += count.skipped;
                total.failed += count.failed;
            }
        }
        total
    }
}

struct Observe<C>(PhantomData<C>);

impl<P: PhaseItem, C: RenderCommand<P>> RenderCommand<P> for Observe<C> {
    type Param = (C::Param, SRes<DrawProbe>);
    type ViewQuery = (C::ViewQuery, Entity);
    type ItemQuery = C::ItemQuery;

    fn render<'w>(item: &P, (view_query, view): ROQueryItem<'w, '_, Self::ViewQuery>,
        entity: Option<ROQueryItem<'w, '_, Self::ItemQuery>>, (param, probe): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        let result = C::render(item, view_query, entity, param, pass);
        probe.0.lock().expect("draw probe").entry((view, std::any::type_name::<P>())).or_default().record(&result);
        result
    }
}

fn install<P: PhaseItem, C: RenderCommand<P> + Send + Sync + 'static>(app: &mut SubApp)
where C::Param: ReadOnlySystemParam {
    let state = RenderCommandState::<P, Observe<C>>::new(app.world_mut());
    app.world().resource::<DrawFunctions<P>>().write().add_with::<C, _>(state);
}

pub(super) fn configure(app: &mut SubApp) {
    use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transparent3d};
    use bevy::pbr::Transmissive3d;
    app.init_resource::<DrawProbe>().add_systems(Render,
        (|probe: Res<DrawProbe>| probe.0.lock().expect("draw probe").clear()).before(RenderSystems::Render));
    install::<Opaque3d, bevy::pbr::DrawMaterial>(app);
    install::<AlphaMask3d, bevy::pbr::DrawMaterial>(app);
    install::<Transmissive3d, bevy::pbr::DrawMaterial>(app);
    install::<Transparent3d, bevy::pbr::DrawMaterial>(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_results_are_scoped_to_views_and_require_success_for_nonempty_views() {
        let mut world = World::new();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let probe = DrawProbe::default();
        assert!(!probe.for_view(first).ready(1));
        assert!(probe.for_view(first).ready(0));
        {
            let mut rows = probe.0.lock().unwrap();
            rows.entry((first, "opaque")).or_default().record(&RenderCommandResult::Success);
            rows.entry((first, "transparent")).or_default().record(&RenderCommandResult::Success);
            rows.entry((second, "opaque")).or_default().record(&RenderCommandResult::Skip);
        }
        assert_eq!(probe.for_view(first).succeeded, 2);
        assert!(probe.for_view(first).ready(100));
        assert!(!probe.for_view(second).ready(1));
        probe.0.lock().unwrap().entry((first, "opaque")).or_default().record(&RenderCommandResult::Failure("test"));
        assert!(!probe.for_view(first).ready(100));
        probe.0.lock().unwrap().clear();
        assert!(!probe.for_view(first).ready(1));
    }
}
