//! `usdview` — mara UI host (ribbons + panes) embedding a Bevy USD viewport.
//!
//! mara (egui/eframe) owns the window + ribbons + panes; the USD scene renders
//! in an embedded Bevy viewport. A left ribbon toggles an **Outliner** (prim
//! tree) and a **Properties** pane; a Save action writes the stage back.

use bevy::camera::RenderTarget;
use bevy::prelude::*;

mod environment;
mod curve_quality;
mod lighting;
mod inspector;
mod capture;
mod framing;
mod timeline;
mod ui_replay;
mod render_settings;
mod file_dialog;

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

use usd_bevy::UsdPlugin;
use usd_bevy::live::LiveStagePlugin;
use usd_bevy::editor::{EditorBridge, EditorCommand, EditorPlugin, SaveMode};

/// Everything (trace + panics + backtraces) is mirrored here so a hard crash
/// is still recoverable after the window dies.
const LOG_FILE: &str = "/tmp/usdview.log";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    curve_quality::from_env()?;
    capture::CaptureConfig::from_env()?;
    let watch = match std::env::var("USD_WATCH_TEXTURES") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    texture_watch_enabled(
        watch.as_deref(),
        cfg!(all(feature = "file_watcher", not(target_arch = "wasm32"))),
    )?;
    init_tracing();
    install_panic_logger();
    tracing::info!(target: "usdview", "usdview starting — full log at {LOG_FILE}");
    mara::window::run::<UsdApp>()
}

fn texture_watch_enabled(value: Option<&str>, supported: bool) -> Result<bool, String> {
    match value {
        None | Some("0") => Ok(false),
        Some("1") if supported => Ok(true),
        Some("1") => Err("USD_WATCH_TEXTURES=1 requires the file_watcher build feature".into()),
        _ => Err("USD_WATCH_TEXTURES must be 0 or 1".into()),
    }
}

#[cfg(all(feature = "file_watcher", not(target_arch = "wasm32")))]
fn report_texture_watch(status: Res<usd_bevy::editor::texture_watch::EditorTextureWatchStatus>) {
    if !status.is_changed() {
        return;
    }
    if let Some(error) = &status.error {
        tracing::error!("{error}");
    } else {
        tracing::info!(files = status.files, "editor texture watcher active");
    }
}

#[test]
fn texture_watch_configuration_is_explicit() {
    for supported in [false, true] {
        assert_eq!(texture_watch_enabled(None, supported), Ok(false));
        assert_eq!(texture_watch_enabled(Some("0"), supported), Ok(false));
        assert!(texture_watch_enabled(Some("true"), supported).is_err());
        assert!(texture_watch_enabled(Some(""), supported).is_err());
    }
    assert!(texture_watch_enabled(Some("1"), false).is_err());
    assert_eq!(texture_watch_enabled(Some("1"), true), Ok(true));
}

/// Tracing to BOTH stderr and [`LOG_FILE`]. The embedded Bevy app has no
/// `LogPlugin`, so without this the logs go nowhere; the file copy survives a
/// crash that eats stderr. Override the filter with `RUST_LOG`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let _ = std::fs::write(LOG_FILE, ""); // truncate per run
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,usdview=trace,usd_bevy=trace,openusd=info"));
    let to_file = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
            .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap())
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_ansi(false)
        .with_writer(std::io::stderr.and(to_file))
        .try_init();
}

/// Capture panics (message + full backtrace) to the log file and stderr —
/// otherwise a panic inside the embedded app vanishes with the window.
fn install_panic_logger() {
    unsafe {
        if std::env::var_os("RUST_BACKTRACE").is_none() {
            std::env::set_var("RUST_BACKTRACE", "full");
        }
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!("\n==== usdview PANIC ====\n{info}\n{backtrace}\n");
        tracing::error!(target: "usdview", "PANIC: {info}");
        eprint!("{msg}");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
        {
            let _ = f.write_all(msg.as_bytes());
        }
        prev(info);
    }));
}

// ─── Ribbon / pane ids ──────────────────────────────────────────────

const RIBBON_LEFT: &str = "usd_ribbon_left";
const PANE_OUTLINER: &str = "usd_pane_outliner";
const PANE_PROPERTIES: &str = "usd_pane_properties";
const PANE_TIMELINE: &str = "usd_pane_timeline";
const PANE_LIGHTING: &str = "usd_pane_lighting";
const PANE_RENDERING: &str = "usd_pane_rendering";
const ACTION_SAVE: &str = "usd_action_save";
const ACTION_OPEN: &str = "usd_action_open";
const ACTION_UNDO: &str = "usd_action_undo";
const ACTION_REDO: &str = "usd_action_redo";
const ACTION_FLATTEN: &str = "usd_action_flatten";
const ACTION_SAVE_LAYER: &str = "usd_action_save_layer";
const ACTION_REFRESH_TEXTURES: &str = "usd_action_refresh_textures";

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
    editor: EditorBridge,
    drafts: inspector::Drafts,
    timeline_draft: timeline::Draft,
    lighting: lighting::LightingBridge,
    rendering: render_settings::RenderSettingsBridge,
    file_dialogs: file_dialog::FileDialogs,
    capture_handshake: bool,
}

impl WindowApp for UsdApp {
    fn new(ctx: CreationContext<'_>) -> Self {
        ui_replay::install(ctx.__internal_egui_ctx()).expect("invalid USD_UI_REPLAY script");
        // Initial file: `USD_FILE` env var, else argv[1], else none.
        let path = std::env::var("USD_FILE")
            .ok()
            .or_else(|| std::env::args().nth(1));
        let editor = EditorBridge::default();
        if let Some(path) = path { send(&editor, EditorCommand::Open(path)); }
        if let Ok(path) = std::env::var("USD_VIEWER_SELECT") { send(&editor, EditorCommand::Select(Some(path))); }
        let bridge = editor.clone();
        let lighting = lighting::LightingBridge::from_env();
        let lighting_bridge = lighting.clone();
        let rendering = render_settings::RenderSettingsBridge::default();
        let rendering_bridge = rendering.clone();
        let bevy_view = mara_bevy::MaraBevyViewport::with_render_state_and_content(
            ctx.gpu(),
            move |app: &mut App| {
                configure_usd_app(app, bridge.clone());
                lighting::configure(app, lighting_bridge.clone());
                render_settings::configure(app, rendering_bridge.clone());
            },
        );

        let capture_handshake = std::env::var_os("USD_UI_CAPTURE_HANDSHAKE").is_some();
        if capture_handshake {
            eprintln!("USD_VIEWER_STARTED");
        }

        Self {
            bevy_view,
            workspace: WorkspaceStack::new("usd-workspace"),
            editor,
            drafts: Default::default(),
            timeline_draft: Default::default(),
            lighting,
            rendering,
            file_dialogs: file_dialog::FileDialogs::new(ctx.__internal_egui_ctx()),
            capture_handshake,
        }
    }

    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        let Self {
            bevy_view,
            workspace,
            editor,
            drafts,
            timeline_draft,
            lighting,
            rendering,
            file_dialogs,
            capture_handshake,
            ..
        } = self;
        if let Some(command) = file_dialogs.poll() { send(editor, command); }
        // Apply the mara theme every frame (without this the panes/ribbons
        // render with raw-egui defaults).
        mara_core::style::set_theme(mara_core::style::theme_pro(Mode::Dark));
        host.apply_theme(AccentColor::default(), GlassOpacity::default());
        let accent = active_accent();

        // Viewport (root, behind the ribbon-avoiding panes).
        {
            let mut vctx = host.view_ctx(workspace, accent, RibbonAvoidance::all());
            vctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
            bevy_view.show(&mut vctx, host.gpu(), accent);
        }

        // Panes + ribbon rail. Mara owns the pane/ribbon wiring,
        // open-state, pane-id publication, and paint ordering.
        let view = match editor.view() { Ok(view) => view, Err(error) => { error!("{error}"); return; } };
        let renderer_error = rendering.renderer_error();
        let prims: Vec<_> = view.document.prims.iter().map(|path| PrimRow {
            path: path.clone(), name: path.rsplit('/').next().unwrap_or(path).to_string(),
        }).collect();
        let rail = RibbonRail::view_left(RIBBON_LEFT, "usdview.ribbons")
            .default_open(match std::env::var("USD_VIEWER_PANE").as_deref() {
                Ok("lighting") => PANE_LIGHTING,
                Ok("inspector") => PANE_PROPERTIES,
                Ok("timeline") => PANE_TIMELINE,
                Ok("rendering") => PANE_RENDERING,
                _ => PANE_OUTLINER,
            })
            .pane(
                PANE_OUTLINER,
                "list",
                "Outliner",
                PaneAnchor::LeftRail(RailZone::Start),
                |body| {
                    outliner_pane(body, &prims, editor, view.document.selected.as_deref(), renderer_error.as_deref().or(file_dialogs.status()).unwrap_or(&view.status), accent, &view.document.visibility);
                },
            )
            .pane(
                PANE_PROPERTIES,
                "options",
                "Properties",
                PaneAnchor::LeftRail(RailZone::Middle),
                |body| {
                    inspector::show(body, &view.document, editor, drafts, view.timeline.current);
                },
            )
            .pane(PANE_TIMELINE, "options", "Timeline", PaneAnchor::LeftRail(RailZone::Middle), |body| {
                timeline::show(body, &view.timeline, editor, timeline_draft);
            })
            .pane(PANE_LIGHTING, "options", "Lighting", PaneAnchor::LeftRail(RailZone::Middle), |body| {
                lighting::show(body, lighting);
            })
            .pane(PANE_RENDERING, "options", "Rendering", PaneAnchor::LeftRail(RailZone::Middle), |body| {
                render_settings::show(body, rendering);
            })
            .action(
                ACTION_OPEN,
                "folder",
                "Open USD…",
                ribbon_action(ACTION_OPEN),
            )
            .action(
                ACTION_SAVE,
                "document",
                "Save root layer as…",
                ribbon_action(ACTION_SAVE),
            )
            .action(ACTION_SAVE_LAYER, "document", "Save edit layer as…", ribbon_action(ACTION_SAVE_LAYER))
            .action(ACTION_FLATTEN, "document", "Export flattened…", ribbon_action(ACTION_FLATTEN))
            .action(ACTION_UNDO, "arrow-left", "Undo", ribbon_action(ACTION_UNDO))
            .action(ACTION_REDO, "arrow-right", "Redo", ribbon_action(ACTION_REDO))
            .action(ACTION_REFRESH_TEXTURES, "image", "Refresh textures", ribbon_action(ACTION_REFRESH_TEXTURES));
        for click in host.show_ribbon_rail(rail, accent) {
            if click.action == ribbon_action(ACTION_SAVE) || click.action == ribbon_action(ACTION_SAVE_LAYER)
                || click.action == ribbon_action(ACTION_FLATTEN) {
                let mode = if click.action == ribbon_action(ACTION_SAVE_LAYER) { SaveMode::EditLayer }
                    else if click.action == ribbon_action(ACTION_FLATTEN) { SaveMode::Flattened } else { SaveMode::RootLayer };
                file_dialogs.start(file_dialog::Request::save(mode, &view.document));
            } else if click.action == ribbon_action(ACTION_UNDO) {
                send(editor, EditorCommand::Undo);
            } else if click.action == ribbon_action(ACTION_REDO) {
                send(editor, EditorCommand::Redo);
            } else if click.action == ribbon_action(ACTION_OPEN) {
                file_dialogs.start(file_dialog::Request::Open);
            } else if click.action == ribbon_action(ACTION_REFRESH_TEXTURES) {
                send(editor, EditorCommand::RefreshTextures);
            }
        }
        if *capture_handshake {
            eprintln!("USD_VIEWER_UI_UPDATED");
            *capture_handshake = false;
        }
    }
}

fn send(editor: &EditorBridge, command: EditorCommand) {
    if let Err(error) = editor.send(command) { error!("{error}"); }
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
    let index: std::collections::HashMap<&str, usize> = prims
        .iter()
        .enumerate()
        .map(|(i, p)| (p.path.as_str(), i))
        .collect();
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
    editor: &EditorBridge,
    selected: Option<&str>,
    status: &str,
    accent: MaraColor32,
    visibility: &std::collections::HashMap<String, bool>,
) {
    let lines = lighting::status_lines(status);
    let status_pod = Pod::new(MaraId::new(("usd.outliner", "status")));
    let status_pod = if lines.len() > 1 {
        status_pod.with_custom_units(lines.len(), move |ui| {
            for line in lines { ui.label(&line); }
        })
    } else { status_pod.with_readout("state", status) };
    body.add_normal(
        "usd.status",
        "Status",
        "list",
        vec![status_pod],
    );

    let tree_root = MaraId::new(("usd.outliner", "tree_root"));
    let sel = selected.unwrap_or_default().to_string();
    let tree_selected = sel.clone();
    let editor = editor.clone();
    let visibility = visibility.clone();

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
                    usd_tree(tree, tree_root, accent, &filter, &nodes, &roots, &tree_selected, &editor, &visibility)
                }),
            Pod::new(MaraId::new(("usd.outliner", "scene", 2usize))).with_readout(
                "selected",
                if sel.is_empty() {
                    "—".to_string()
                } else {
                    sel
                },
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
    selected: &str,
    editor: &EditorBridge,
    visibility: &std::collections::HashMap<String, bool>,
) {
    let mut clicked: Option<String> = None;
    for &r in roots {
        walk_usd_tree(
            tree,
            root_id,
            nodes,
            r,
            0,
            selected,
            accent,
            filter,
            &mut clicked,
            editor,
            visibility,
        );
    }
    if let Some(p) = clicked {
        send(editor, EditorCommand::Select(Some(p)));
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
    editor: &EditorBridge,
    visibility: &std::collections::HashMap<String, bool>,
) {
    if !usd_tree_passes(nodes, i, filter) {
        return;
    }
    let node = &nodes[i];
    let is_branch = !node.children.is_empty();
    let exp_key = root_id.with(("exp", node.path.as_str()));
    let mut expanded = tree.persisted_bool(exp_key).unwrap_or(true);
    let previous_eye = visibility.get(&node.path).copied().unwrap_or(true);
    let mut eye_on = previous_eye;
    let mut slots =
        [TreeIconSlot::new(TreeIconKind::Eye, &mut eye_on).with_tooltip("Toggle local visibility; animated values use the current time")];
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
    if eye_on != previous_eye {
        send(editor, EditorCommand::Visibility { prim: node.path.clone(), visible: eye_on });
    }
    if is_branch && expanded {
        for &c in &node.children {
            walk_usd_tree(
                tree,
                root_id,
                nodes,
                c,
                depth + 1,
                selected,
                accent,
                filter,
                clicked,
                editor,
                visibility,
            );
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
    node.children
        .iter()
        .any(|&c| usd_tree_passes(nodes, c, filter))
}


// ─── Embedded Bevy viewport (the USD scene) ─────────────────────────

fn configure_usd_app(app: &mut App, editor: EditorBridge) {
    app.insert_resource(curve_quality::from_env().expect("validated USD_CURVE_STEPS"));
    app.insert_resource(editor);
    app.add_plugins((
        UsdPlugin,
        LiveStagePlugin,
        EditorPlugin,
        environment::ViewerEnvironmentPlugin,
    ))
        .init_resource::<mara_bevy::BevyViewportInput>()
        .insert_resource(mara_bevy::GroundGrid {
            visible: false,
            ..default()
        })
        .add_systems(
            Startup,
            setup_camera.after(mara_bevy::BevyViewportSet::SetupTarget),
        )
        .add_systems(Update, mara_bevy::apply_viewport_camera_input_system);
    #[cfg(all(feature = "file_watcher", not(target_arch = "wasm32")))]
    if texture_watch_enabled(std::env::var("USD_WATCH_TEXTURES").ok().as_deref(), true)
        .unwrap_or(false)
    {
        app.add_plugins(usd_bevy::editor::texture_watch::EditorTextureWatchPlugin)
            .add_systems(Update, report_texture_watch);
    }
    if std::env::var_os("USD_CPU_SKINNING").is_none() {
        app.add_plugins(usd_bevy::route::gpu_skin::UsdGpuSkinningPlugin);
    }
    capture::configure(app);
    if let Ok(levels) = std::env::var("USD_SUBDIVISION_LEVELS") {
        app.insert_resource(usd_bevy::route::subdivision::UsdSubdivisionSettings::new(
            levels.parse().expect("USD_SUBDIVISION_LEVELS must be an integer")).expect("invalid subdivision levels"));
    }
    framing::configure(app);
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
            color: Color::srgb(0.78, 0.85, 1.0),
            brightness: 160.0,
            ..default()
        },
        chase,
    ));
    if let Some(render_target) = render_target {
        camera.insert(RenderTarget::from(render_target.0.clone()));
    }
}
