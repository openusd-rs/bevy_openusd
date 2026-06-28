//! `usdview` — mara UI host (ribbons + panes) embedding a Bevy USD viewport.
//!
//! mara (egui/eframe) owns the window + ribbons + panes; the USD scene renders
//! in an embedded Bevy viewport. A left ribbon toggles an **Outliner** (prim
//! tree) and a **Properties** pane; a Save action writes the stage back.

use bevy::camera::RenderTarget;
use bevy::prelude::*;

use mara::host::{MaraHostCtx, RibbonRail};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use mara::window::{CreationContext, WindowApp};
use mara_core::container::SeparatorStyle;
use mara_core::pane::{PaneAnchor, PaneBody, RailZone};
use mara_core::pod::Pod;
use mara_core::ribbon::RibbonAction;
use mara_core::style::{AccentColor, GlassOpacity, Mode, active_accent};
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use mara_core::widget::{TreeBody, TreeIconKind, TreeIconSlot};
use mara_core::{RibbonAvoidance, WorkspaceStack};

use std::cell::RefCell;

use openusd::usd::Stage;
use usd_bevy::UsdPlugin;
use usd_bevy::live::{LiveStage, LiveStagePlugin};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    mara::window::run::<UsdApp>()
}

// ─── Ribbon / pane ids ──────────────────────────────────────────────

const RIBBON_LEFT: &str = "usd_ribbon_left";
const PANE_OUTLINER: &str = "usd_pane_outliner";
const PANE_PROPERTIES: &str = "usd_pane_properties";
const ACTION_SAVE: &str = "usd_action_save";

fn ribbon_action(id: &'static str) -> RibbonAction {
    RibbonAction::Command(MaraId::new(id))
}

// ─── App ────────────────────────────────────────────────────────────

struct PrimRow {
    path: String,
    name: String,
}

/// The mara window app.
struct UsdApp {
    bevy_view: mara_bevy::MaraBevyViewport,
    workspace: WorkspaceStack,
    /// A read-side stage for the outliner/properties panes (the embedded
    /// viewport renders its own live copy).
    stage: Option<Stage>,
    prims: Vec<PrimRow>,
    selected: Option<String>,
}

impl WindowApp for UsdApp {
    fn new(ctx: CreationContext<'_>) -> Self {
        let path = std::env::args().nth(1);
        let viewport_path = path.clone();
        let bevy_view = mara_bevy::MaraBevyViewport::with_render_state_and_content(
            ctx.render_state,
            move |app: &mut App| configure_usd_app(app, viewport_path.clone()),
        );

        let stage = path.as_deref().and_then(|p| Stage::open(p).ok());
        let prims = stage.as_ref().map(collect_prims).unwrap_or_default();

        Self {
            bevy_view,
            workspace: WorkspaceStack::new("usd-workspace"),
            stage,
            prims,
            selected: None,
        }
    }

    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        let Self {
            bevy_view,
            workspace,
            stage,
            prims,
            selected,
        } = self;
        // Apply the mara theme every frame (without this the panes/ribbons
        // render with raw-egui defaults).
        mara_core::style::set_theme(mara_core::style::theme_pro(Mode::Dark));
        host.apply_theme(AccentColor::default(), GlassOpacity::default());
        let accent = active_accent();

        // Viewport (root, behind the ribbon-avoiding panes).
        {
            let mut vctx = host.view_ctx(workspace, accent, RibbonAvoidance::all());
            bevy_view.show(&mut vctx, host.render_state(), accent);
        }

        // Panes + ribbon rail. Mara owns the pane/ribbon wiring,
        // open-state, pane-id publication, and paint ordering.
        let selected = RefCell::new(selected);
        let rail = RibbonRail::view_left(RIBBON_LEFT, "usdview.ribbons")
            .default_open(PANE_OUTLINER)
            .pane(
                PANE_OUTLINER,
                "list",
                "Outliner",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| {
                    let mut selected = selected.borrow_mut();
                    outliner_pane(body, prims, &mut **selected, accent);
                },
            )
            .pane(
                PANE_PROPERTIES,
                "options",
                "Properties",
                PaneAnchor::LeftRail(RailZone::Middle),
                |body| {
                    let selected = selected.borrow();
                    properties_pane(body, stage, &**selected);
                },
            )
            .action(
                ACTION_SAVE,
                "document",
                "Save stage",
                ribbon_action(ACTION_SAVE),
            );
        for click in host.show_ribbon_rail(rail, accent) {
            if click.action == ribbon_action(ACTION_SAVE) {
                if let Some(stage) = stage.as_ref() {
                    match usd_bevy::authoring::save_stage_as(stage, "usdview_out.usda") {
                        Ok(()) => info!("saved stage to usdview_out.usda"),
                        Err(e) => error!("save failed: {e:#}"),
                    }
                }
            }
        }
    }
}

fn collect_prims(stage: &Stage) -> Vec<PrimRow> {
    let mut out = Vec::new();
    let _ = stage.traverse(
        openusd::usd::PrimPredicate::default(),
        |path: &openusd::sdf::Path| {
            let s = path.as_str();
            let name = s.rsplit('/').next().unwrap_or(s).to_string();
            out.push(PrimRow {
                path: s.to_string(),
                name,
            });
        },
    );
    out
}

/// A node in the prim hierarchy (built from the flat traversal list).
struct UsdNode {
    path: String,
    name: String,
    children: Vec<usize>,
}

/// Build the prim hierarchy + the root indices from the flat, depth-ordered
/// prim list. A prim's parent is the path up to its last `/`.
fn build_usd_tree(prims: &[PrimRow]) -> (Vec<UsdNode>, Vec<usize>) {
    let mut nodes: Vec<UsdNode> = prims
        .iter()
        .map(|p| UsdNode {
            path: p.path.clone(),
            name: p.name.clone(),
            children: Vec::new(),
        })
        .collect();
    let index: std::collections::HashMap<&str, usize> =
        prims.iter().enumerate().map(|(i, p)| (p.path.as_str(), i)).collect();
    let mut roots = Vec::new();
    for (i, p) in prims.iter().enumerate() {
        let parent = &p.path[..p.path.rfind('/').unwrap_or(0)];
        match (!parent.is_empty()).then(|| index.get(parent)).flatten() {
            Some(&pi) => nodes[pi].children.push(i),
            None => roots.push(i),
        }
    }
    (nodes, roots)
}

fn outliner_pane(
    body: &mut PaneBody,
    prims: &[PrimRow],
    selected: &mut Option<String>,
    accent: MaraColor32,
) {
    let tree_root = MaraId::new(("usd.outliner", "tree_root"));
    let sel_key = tree_root.with("selected");
    // Selection from last frame (the tree writes it during render).
    let sel = body.temp_string(sel_key).unwrap_or_default();
    *selected = (!sel.is_empty()).then(|| sel.clone());

    let search_id = MaraId::new(("usd.outliner", "scene", 0usize));
    let filter = body.search_query(search_id, 0).to_lowercase();
    let (nodes, roots) = build_usd_tree(prims);

    body.add_normal(
        "usd.outliner",
        "Scene",
        "folder",
        vec![
            Pod::new(search_id)
                .with_separator(SeparatorStyle::Line)
                .with_search("filter by name / path…", accent),
            Pod::new(MaraId::new(("usd.outliner", "scene", 1usize)))
                .with_separator(SeparatorStyle::Line)
                .fill()
                .with_tree(7, move |tree| {
                    usd_tree(tree, tree_root, accent, &filter, &nodes, &roots)
                }),
            Pod::new(MaraId::new(("usd.outliner", "scene", 2usize))).with_readout(
                "selected",
                if sel.is_empty() { "—".to_string() } else { sel },
            ),
        ],
    );
}

fn usd_tree(
    tree: &mut TreeBody,
    root_id: MaraId,
    accent: MaraColor32,
    filter: &str,
    nodes: &[UsdNode],
    roots: &[usize],
) {
    let sel_key = root_id.with("selected");
    let mut selected = tree.temp_string(sel_key).unwrap_or_default();
    let mut clicked: Option<String> = None;
    for &r in roots {
        walk_usd_tree(tree, root_id, nodes, r, 0, &selected, accent, filter, &mut clicked);
    }
    if let Some(p) = clicked {
        selected = p;
        tree.set_temp_string(sel_key, selected);
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_usd_tree(
    tree: &mut TreeBody,
    root_id: MaraId,
    nodes: &[UsdNode],
    i: usize,
    depth: u32,
    selected: &str,
    accent: MaraColor32,
    filter: &str,
    clicked: &mut Option<String>,
) {
    if !usd_tree_passes(nodes, i, filter) {
        return;
    }
    let node = &nodes[i];
    let is_branch = !node.children.is_empty();
    let exp_key = root_id.with(("exp", node.path.as_str()));
    let eye_key = root_id.with(("eye", node.path.as_str()));
    let mut expanded = tree.persisted_bool(exp_key).unwrap_or(true);
    let mut eye_on = tree.persisted_bool(eye_key).unwrap_or(true);
    let mut slots =
        [TreeIconSlot::new(TreeIconKind::Eye, &mut eye_on).with_tooltip("Toggle visibility")];
    let resp = tree.row(
        i,
        depth,
        if is_branch { Some(&mut expanded) } else { None },
        Some("cube"),
        &node.name,
        selected == node.path,
        accent,
        &mut slots,
    );
    if resp.body.clicked {
        *clicked = Some(node.path.clone());
    }
    tree.set_persisted_bool(exp_key, expanded);
    tree.set_persisted_bool(eye_key, eye_on);
    if is_branch && expanded {
        for &c in &node.children {
            walk_usd_tree(tree, root_id, nodes, c, depth + 1, selected, accent, filter, clicked);
        }
    }
}

/// A node passes when it (or any descendant) matches the lowercase `filter`.
fn usd_tree_passes(nodes: &[UsdNode], i: usize, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let node = &nodes[i];
    if node.name.to_lowercase().contains(filter) || node.path.to_lowercase().contains(filter) {
        return true;
    }
    node.children.iter().any(|&c| usd_tree_passes(nodes, c, filter))
}

fn properties_pane(body: &mut PaneBody, stage: &Option<Stage>, selected: &Option<String>) {
    let pods = match (stage, selected) {
        (Some(stage), Some(path)) => {
            let ty = openusd::sdf::path(path)
                .ok()
                .and_then(|p| stage.prim(p).type_name().ok().flatten())
                .map(|t| t.as_str().to_string())
                .unwrap_or_else(|| "—".to_string());
            vec![
                Pod::new("usd.prop.path").with_readout("Path", path.clone()),
                Pod::new("usd.prop.type").with_readout("Type", ty),
            ]
        }
        _ => vec![Pod::new("usd.prop.none").with_readout("Selection", "none")],
    };
    body.add_normal("usd.properties", "Properties", "options", pods);
}

// ─── Embedded Bevy viewport (the USD scene) ─────────────────────────

#[derive(Resource, Clone)]
struct UsdArg(Option<String>);

fn configure_usd_app(app: &mut App, path: Option<String>) {
    // The mara viewport already adds GroundGridPlugin + the core/render
    // plugins; we add only our own and configure the grid resource.
    app.add_plugins((UsdPlugin, LiveStagePlugin))
        .init_resource::<mara_bevy::BevyViewportInput>()
        .insert_resource(mara_bevy::GroundGrid {
            visible: true,
            color: Color::srgba(0.30, 0.38, 0.50, 0.42),
        })
        .insert_resource(ClearColor(Color::srgb_u8(12, 14, 18)))
        .insert_resource(UsdArg(path))
        .add_systems(
            Startup,
            setup_camera.after(mara_bevy::BevyViewportSet::SetupTarget),
        )
        .add_systems(Startup, open_usd)
        .add_systems(Update, mara_bevy::apply_viewport_camera_input_system);
}

fn setup_camera(
    mut commands: Commands,
    render_target: Option<Res<mara_bevy::BevyViewportRenderTarget>>,
) {
    let chase = mara_bevy::ChaseCamera::default();
    let mut transform = Transform::default();
    mara_bevy::apply_rig(&chase, &mut transform);
    let mut camera = commands.spawn((
        Camera3d::default(),
        transform,
        AmbientLight {
            brightness: 220.0,
            ..default()
        },
        chase,
    ));
    if let Some(render_target) = render_target {
        camera.insert(RenderTarget::from(render_target.0.clone()));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn open_usd(world: &mut World) {
    let path = world.resource::<UsdArg>().0.clone();
    let Some(path) = path else {
        info!("usage: usdview <file.usd|usda|usdz>");
        return;
    };
    match Stage::open(&path) {
        Ok(stage) => {
            info!("opened USD stage: {path}");
            world.insert_non_send(LiveStage::new(stage));
        }
        Err(e) => error!("failed to open {path}: {e:#}"),
    }
}
