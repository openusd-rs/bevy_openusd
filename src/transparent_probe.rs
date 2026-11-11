use bevy::{
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        camera::TemporalJitter,
        render_phase::ViewSortedRenderPhases,
        sync_world::{MainEntity, MainEntityHashMap},
        view::ExtractedView,
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
    subsets: Extract<Query<(Entity, &usd_bevy::route::subset::UsdSubset, &ChildOf)>>,
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
    for (entity, subset, parent) in &subsets {
        let parent = labels
            .names
            .get(&MainEntity::from(parent.parent()))
            .map_or("<unlabelled>", String::as_str);
        let label = format!("{parent} [subset:{}]", subset.0);
        labels.names.insert(entity.into(), label);
    }
    labels.grids.clear();
    for (transform, settings) in &grids {
        labels.grids.push_str(&grid_report(transform, settings));
    }
}

pub(crate) fn grid_report(
    transform: &GlobalTransform,
    settings: &bevy::dev_tools::infinite_grid::InfiniteGridSettings,
) -> String {
    format!(
        "transparent_grid world={:?} scale={:?} fade={:?} dot_fade={:?} major_interval={} x_color={:?} z_color={:?} minor_color={:?} major_color={:?}\n",
        transform.to_matrix(),
        settings.scale,
        settings.fadeout_distance,
        settings.dot_fadeout_strength,
        settings.major_line_interval,
        settings.x_axis_color,
        settings.z_axis_color,
        settings.minor_line_color,
        settings.major_line_color,
    )
}

fn view_report(view: &ExtractedView, jitter: Option<Vec2>) -> String {
    format!(
        "transparent_camera scope=extracted-view-before-prepare world_from_view={:?} clip_from_view={:?} clip_from_world={:?} viewport={:?} format={:?} jitter={jitter:?}\n",
        view.world_from_view.to_matrix(),
        view.clip_from_view,
        view.clip_from_world,
        view.viewport,
        view.target_format,
    )
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
    views: Query<(&ExtractedView, Option<&TemporalJitter>)>,
) {
    let mut text = labels.grids.clone();
    for (view, phase) in &phases.0 {
        if let Some((extracted, jitter)) = views
            .iter()
            .find(|(extracted, _)| extracted.retained_view_entity == *view)
        {
            text.push_str(&view_report(extracted, jitter.map(|jitter| jitter.offset)));
        }
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
fn grid_report_preserves_transform_and_display_settings() {
    use bevy::dev_tools::infinite_grid::InfiniteGridSettings;
    let transform = GlobalTransform::from_translation(Vec3::new(1.0, -2.0, 3.0));
    let mut settings = InfiniteGridSettings {
        scale: 0.25,
        fadeout_distance: 42.0,
        dot_fadeout_strength: 0.75,
        major_line_interval: 8,
        ..default()
    };
    let before = grid_report(&transform, &settings);
    assert!(before.contains(&format!("world={:?}", transform.to_matrix())));
    assert!(before.contains("scale=0.25 fade=42.0 dot_fade=0.75 major_interval=8"));
    assert!(before.contains(&format!("x_color={:?}", settings.x_axis_color)));
    assert!(before.contains(&format!("z_color={:?}", settings.z_axis_color)));
    assert!(before.contains(&format!("minor_color={:?}", settings.minor_line_color)));
    assert!(before.contains(&format!("major_color={:?}", settings.major_line_color)));
    settings.fadeout_distance = f32::from_bits(settings.fadeout_distance.to_bits() + 1);
    assert_ne!(before, grid_report(&transform, &settings));
}

#[test]
fn view_report_preserves_extracted_matrices_and_jitter() {
    use bevy::render::{render_resource::TextureFormat, view::RetainedViewEntity};
    let view = ExtractedView {
        retained_view_entity: RetainedViewEntity::new(Entity::PLACEHOLDER.into(), None, 0),
        clip_from_view: Mat4::from_scale(Vec3::new(2.0, 3.0, 4.0)),
        world_from_view: GlobalTransform::from_translation(Vec3::new(5.0, 6.0, 7.0)),
        clip_from_world: Some(Mat4::IDENTITY),
        target_format: TextureFormat::Rgba16Float,
        viewport: UVec4::new(4, 5, 640, 480),
        color_grading: default(),
        invert_culling: false,
    };
    let report = view_report(&view, Some(Vec2::new(0.25, -0.25)));
    assert!(report.contains("scope=extracted-view-before-prepare"));
    assert!(report.contains(&format!(
        "world_from_view={:?}",
        view.world_from_view.to_matrix()
    )));
    assert!(report.contains(&format!("clip_from_view={:?}", view.clip_from_view)));
    assert!(report.contains(&format!("clip_from_world={:?}", view.clip_from_world)));
    assert!(report.contains(
        "viewport=UVec4(4, 5, 640, 480) format=Rgba16Float jitter=Some(Vec2(0.25, -0.25))"
    ));
    assert!(view_report(&view, None).contains("jitter=None"));
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
