use bevy::{
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        render_phase::ViewSortedRenderPhases,
        sync_world::{MainEntity, MainEntityHashMap},
    },
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Snapshot {
    frames: u64,
    text: String,
}

#[derive(Clone, Resource, Default)]
pub struct TransparentProbe(Arc<Mutex<Snapshot>>);

impl TransparentProbe {
    pub fn report(&self) -> String {
        let snapshot = self.0.lock().unwrap();
        format!(
            "transparent_probe_scope=latest-render-phase-after-sort-before-batching\ntransparent_probe_frames={}\n{}",
            snapshot.frames, snapshot.text
        )
    }
}

#[derive(Resource, Default)]
struct Labels {
    names: MainEntityHashMap<String>,
    grids: String,
}

type SourceLabels<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        Option<&'static usd_bevy::UsdPrimRef>,
        Option<&'static Name>,
    ),
    Or<(With<usd_bevy::UsdPrimRef>, With<Name>)>,
>;

pub fn configure(app: &mut App) {
    let probe = TransparentProbe::default();
    app.insert_resource(probe.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(probe)
        .init_resource::<Labels>()
        .add_systems(ExtractSchedule, extract_labels)
        .add_systems(
            Render,
            report
                .after(RenderSystems::PhaseSort)
                .before(RenderSystems::Prepare),
        );
}

fn extract_labels(
    mut labels: ResMut<Labels>,
    sources: Extract<SourceLabels>,
    grids: Extract<
        Query<(
            &GlobalTransform,
            &bevy::dev_tools::infinite_grid::InfiniteGridSettings,
        )>,
    >,
) {
    labels.names = sources
        .iter()
        .map(|(entity, prim, name)| {
            let label = prim
                .map(|prim| prim.path.to_string())
                .or_else(|| name.map(|name| name.as_str().to_owned()))
                .unwrap_or_default();
            (entity.into(), label)
        })
        .collect();
    labels.grids.clear();
    for (transform, settings) in &grids {
        labels.grids.push_str(&format!(
            "transparent_grid world={:?} scale={:?} fade={:?}\n",
            transform.to_matrix(),
            settings.scale,
            settings.fadeout_distance
        ));
    }
}

fn item_report(
    index: usize,
    entity: MainEntity,
    label: &str,
    sorting: TransparentSortingInfo3d,
    distance: f32,
) -> String {
    let key = match sorting {
        TransparentSortingInfo3d::Sorted {
            mesh_center,
            depth_bias,
        } => format!("center={mesh_center:?} depth_bias={depth_bias:?}"),
        TransparentSortingInfo3d::AlwaysOnTop => "always_on_top=true".to_owned(),
    };
    format!(
        "transparent_item index={index} main_entity={entity:?} label={label:?} distance={distance:?} {key}\n"
    )
}

fn report(
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    labels: Res<Labels>,
    probe: Res<TransparentProbe>,
) {
    let mut text = labels.grids.clone();
    for (view, phase) in &phases.0 {
        text.push_str(&format!(
            "transparent_view={view:?} items={}\n",
            phase.items.len()
        ));
        for (index, item) in phase.items.values().enumerate() {
            text.push_str(&item_report(
                index,
                item.entity.1,
                labels
                    .names
                    .get(&item.entity.1)
                    .map_or("<unlabelled>", String::as_str),
                item.sorting_info,
                item.distance,
            ));
        }
    }
    let mut snapshot = probe.0.lock().unwrap();
    snapshot.frames += 1;
    snapshot.text = text;
}

#[test]
fn probe_keeps_actual_phase_keys_and_latest_snapshot() {
    let entity = Entity::PLACEHOLDER.into();
    let row = item_report(
        3,
        entity,
        "/wing\"blur",
        TransparentSortingInfo3d::Sorted {
            mesh_center: Vec3::new(1.0, 2.0, 3.0),
            depth_bias: 0.25,
        },
        -4.0,
    );
    assert!(row.contains("index=3"));
    assert!(row.contains("label=\"/wing\\\"blur\""));
    assert!(row.contains("distance=-4.0 center=Vec3(1.0, 2.0, 3.0) depth_bias=0.25"));
    assert!(
        item_report(
            0,
            entity,
            "overlay",
            TransparentSortingInfo3d::AlwaysOnTop,
            f32::NEG_INFINITY
        )
        .contains("always_on_top=true")
    );
    let probe = TransparentProbe::default();
    let render = probe.clone();
    *render.0.lock().unwrap() = Snapshot {
        frames: 7,
        text: row,
    };
    assert!(probe.report().contains("transparent_probe_frames=7"));
    *render.0.lock().unwrap() = Snapshot {
        frames: 8,
        text: "replacement\n".into(),
    };
    let latest = probe.report();
    assert!(latest.contains("replacement"));
    assert!(!latest.contains("wing"));
}
