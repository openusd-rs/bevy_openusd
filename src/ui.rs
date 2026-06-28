//! Viewer UI — pure mara: ribbons + rail-anchored panes + declarative
//! `Pod` containers. No raw egui, no legacy widgets — every panel is
//! built with `body.add_normal(...)` + `Pod` builders, and interactions
//! are read back from `body.render()`'s `PodResponse` map.
//!
//! Left rail is one `ThreeSided` panel ribbon. Primary tools in the
//! `Start` cluster, the play toggle in `Middle`, utility/help in `End`.
//! `RibbonOpen` drives pane visibility (one open pane per ribbon).

use bevy::asset::Assets;
use bevy::ecs::hierarchy::Children;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_mara::prelude::*;
use mara_core::pane::{Pane, PaneAnchor, PaneResize, RailZone};
use mara_core::pod::{Pod, PodResponse, TagItem};
use mara_core::ribbon::{
    RibbonAction, RibbonCluster, RibbonDrag, RibbonEdge, RibbonGlyph, RibbonMode, RibbonOpen,
    RibbonPlacement, RibbonRole, RibbonSlotClick, RibbonSlotItem, ResolvedSlotRibbon,
    draw_slot_ribbons_featureful,
};
use mara_core::style;
use mara_core::style::AccentColor;
use mara_core::widget::{TreeIconKind, TreeIconSlot};
use mara_core::{CommandPaletteState, PaletteItem, command_palette};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use usd_bevy::{UsdAsset, UsdDisplayName, UsdKind, UsdPrimRef, UsdProcedural, UsdSpatialAudio};

use crate::camera::ArcballCamera;
use crate::overlays::DisplayToggles;
use crate::state::{
    CameraBookmark, CameraBookmarks, CameraMount, FlyTo, LoadRequest, LoaderTuning,
    PendingAnimationClip, ReloadRequest, SelectedPrim, StageInfo, UsdStageTime,
};

// ─── Ribbon declaration ─────────────────────────────────────────────

pub const RIBBON_LEFT: &str = "viewer_left";

pub const RIB_SELECTION: &str = "viewer_selection";
pub const RIB_TREE: &str = "viewer_tree";
pub const RIB_INFO: &str = "viewer_info";
pub const RIB_VARIANTS: &str = "viewer_variants";
pub const RIB_CAMERAS: &str = "viewer_cameras";
pub const RIB_MATERIALS: &str = "viewer_materials";
pub const RIB_OVERLAYS: &str = "viewer_overlays";
pub const RIB_TIMELINE: &str = "viewer_timeline";
pub const RIB_KEYS: &str = "viewer_keys";
pub const RIB_LOG: &str = "viewer_log";
pub const RIB_PLAY: &str = "viewer_play";

#[derive(Clone, Copy)]
struct RibbonSpec {
    id: &'static str,
    edge: RibbonEdge,
    role: RibbonRole,
    mode: RibbonMode,
    accepts: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct RibbonButtonSpec {
    id: &'static str,
    ribbon: &'static str,
    cluster: RibbonCluster,
    glyph: RibbonGlyph,
    tooltip: &'static str,
    role: Option<RibbonRole>,
}

const RIBBONS: &[RibbonSpec] = &[RibbonSpec {
    id: RIBBON_LEFT,
    edge: RibbonEdge::Left,
    role: RibbonRole::Panel,
    mode: RibbonMode::ThreeSided,
    accepts: &[],
}];

const RIBBON_ITEMS: &[RibbonButtonSpec] = &[
    RibbonButtonSpec { id: RIB_SELECTION, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("cursor"), tooltip: "File / selection", role: None },
    RibbonButtonSpec { id: RIB_TREE, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("folder"), tooltip: "Prim tree (T)", role: None },
    RibbonButtonSpec { id: RIB_INFO, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("document"), tooltip: "Stage info (I)", role: None },
    RibbonButtonSpec { id: RIB_VARIANTS, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("options"), tooltip: "Variants", role: None },
    RibbonButtonSpec { id: RIB_CAMERAS, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("cube"), tooltip: "Cameras", role: None },
    RibbonButtonSpec { id: RIB_MATERIALS, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Start, glyph: RibbonGlyph::Icon("color"), tooltip: "Materials", role: None },
    RibbonButtonSpec { id: RIB_PLAY, ribbon: RIBBON_LEFT, cluster: RibbonCluster::Middle, glyph: RibbonGlyph::Icon("play"), tooltip: "Play / pause physics", role: Some(RibbonRole::Icon) },
    RibbonButtonSpec { id: RIB_OVERLAYS, ribbon: RIBBON_LEFT, cluster: RibbonCluster::End, glyph: RibbonGlyph::Icon("square-multiple"), tooltip: "Overlays (O)", role: None },
    RibbonButtonSpec { id: RIB_TIMELINE, ribbon: RIBBON_LEFT, cluster: RibbonCluster::End, glyph: RibbonGlyph::Icon("clock"), tooltip: "Timeline", role: None },
    RibbonButtonSpec { id: RIB_KEYS, ribbon: RIBBON_LEFT, cluster: RibbonCluster::End, glyph: RibbonGlyph::Icon("keyboard"), tooltip: "Controls (?)", role: None },
    RibbonButtonSpec { id: RIB_LOG, ribbon: RIBBON_LEFT, cluster: RibbonCluster::End, glyph: RibbonGlyph::Icon("list"), tooltip: "Log", role: None },
];

/// Free-text filter for the prim-tree panel.
#[derive(Resource, Default)]
pub struct TreeFilter(pub String);

/// Wrapper around mara's `CommandPaletteState`.
#[derive(Resource, Default)]
pub struct ViewerCommandPalette(pub CommandPaletteState);

const PALETTE_ITEMS: &[PaletteItem] = &[
    PaletteItem { id: "open_selection", label: "Open: Selection panel", hint: Some("F") },
    PaletteItem { id: "open_tree", label: "Open: Prim tree", hint: Some("T") },
    PaletteItem { id: "open_info", label: "Open: Stage info", hint: Some("I") },
    PaletteItem { id: "open_variants", label: "Open: Variants", hint: None },
    PaletteItem { id: "open_cameras", label: "Open: Cameras", hint: None },
    PaletteItem { id: "open_overlays", label: "Open: Overlays", hint: Some("O") },
    PaletteItem { id: "open_timeline", label: "Open: Timeline", hint: None },
    PaletteItem { id: "open_keys", label: "Open: Controls", hint: Some("?") },
    PaletteItem { id: "open_log", label: "Open: Log", hint: None },
    PaletteItem { id: "toggle_grid", label: "Toggle: Ground grid", hint: Some("G") },
    PaletteItem { id: "toggle_axes", label: "Toggle: World axes", hint: Some("X") },
    PaletteItem { id: "toggle_markers", label: "Toggle: Prim markers", hint: Some("P") },
    PaletteItem { id: "toggle_wireframe", label: "Toggle: Wireframe", hint: None },
    PaletteItem { id: "reload_stage", label: "Stage: Reload", hint: Some("R") },
    PaletteItem { id: "browse_usd", label: "Stage: Browse for USD…", hint: None },
];

// ─── Ribbon helpers ─────────────────────────────────────────────────

fn ribbon_action(id: &'static str) -> RibbonAction {
    RibbonAction::Command(egui::Id::new(id))
}

fn pane_anchor_for(item_id: &'static str) -> PaneAnchor {
    let zone = RIBBON_ITEMS
        .iter()
        .find(|i| i.id == item_id)
        .map(|i| match i.cluster {
            RibbonCluster::Start => RailZone::Start,
            RibbonCluster::Middle => RailZone::Middle,
            RibbonCluster::End => RailZone::End,
        })
        .unwrap_or(RailZone::Start);
    PaneAnchor::LeftRail(zone)
}

fn draw_unified_ribbons(
    ctx: &egui::Context,
    accent: egui::Color32,
    open: &mut RibbonOpen,
    placement: &mut RibbonPlacement,
    drag: &mut RibbonDrag,
    active: impl Fn(&'static str) -> bool,
) -> Vec<RibbonSlotClick> {
    let mut resolved = Vec::new();
    for ribbon in RIBBONS {
        for cluster in [RibbonCluster::Start, RibbonCluster::Middle, RibbonCluster::End] {
            let items: Vec<RibbonSlotItem> = RIBBON_ITEMS
                .iter()
                .filter(|item| item.ribbon == ribbon.id && item.cluster == cluster)
                .map(|item| {
                    let icon = match item.glyph {
                        RibbonGlyph::Icon(i) | RibbonGlyph::Text(i) | RibbonGlyph::Svg(i) => i,
                    };
                    let mut slot =
                        RibbonSlotItem::featureful(item.id, icon, item.id, item.tooltip, ribbon_action(item.id))
                            .with_role(item.role.unwrap_or(ribbon.role));
                    slot.active = active(item.id);
                    slot
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            resolved.push(ResolvedSlotRibbon {
                id: egui::Id::new((ribbon.id, cluster)),
                chrome_id: Some(ribbon.id),
                scope: mara_core::RibbonScope::Permanent,
                edge: ribbon.edge,
                role: ribbon.role,
                mode: ribbon.mode,
                cluster,
                accepts: ribbon.accepts,
                items,
            });
        }
    }
    draw_slot_ribbons_featureful(ctx, accent, &resolved, open, placement, drag)
}

// ─── Pod-response read helpers ──────────────────────────────────────

type RespMap = HashMap<egui::Id, Vec<PodResponse>>;

fn pod<'a>(r: &'a RespMap, container: egui::Id, idx: usize) -> Option<&'a PodResponse> {
    r.get(&container).and_then(|v| v.get(idx))
}

fn cid(suffix: &str) -> egui::Id {
    egui::Id::new(("usdview_pane", suffix))
}
fn pid(container: &str, idx: usize) -> egui::Id {
    egui::Id::new(("usdview_pod", container, idx))
}

// ─── Plugin ─────────────────────────────────────────────────────────

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MaraPlugin>() {
            app.add_plugins(MaraPlugin);
        }
        app.init_resource::<TreeFilter>()
            .init_resource::<ViewerCommandPalette>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    publish_pane_ids,
                    draw_selection_panel,
                    draw_tree_panel,
                    draw_info_panel,
                    draw_variants_panel,
                    draw_cameras_panel,
                    draw_materials_panel,
                    draw_overlays_panel,
                    draw_timeline_panel,
                    draw_keys_panel,
                    draw_log_panel,
                    draw_palette_panel,
                    draw_ribbons,
                )
                    .chain()
                    .after(RibbonGhostSet),
            );
    }
}

fn publish_pane_ids(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let ids: Vec<egui::Id> = RIBBON_ITEMS
        .iter()
        .filter(|i| i.role.is_none())
        .map(|i| egui::Id::new(i.id))
        .collect();
    mara_core::pane::publish_ribbon_pane_ids(ctx, ids);
}

fn is_panel_open(open: &RibbonOpen, item: &'static str) -> bool {
    open.is_open(RIBBON_LEFT, item)
}

// ─── Ribbon rail ────────────────────────────────────────────────────

fn draw_ribbons(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
    mut physics: ResMut<usd_bevy::physics::PhysicsActive>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let physics_on = physics.0;
    let clicks = draw_unified_ribbons(ctx, accent.0, &mut open, &mut placement, &mut drag, |id| {
        id == RIB_PLAY && physics_on
    });
    for click in clicks {
        if click.item == egui::Id::new(RIB_PLAY) {
            physics.0 = !physics.0;
        }
    }
}

// ─── Selection panel ────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_selection_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
    requested: Res<crate::RequestedAsset>,
    mut load_req: ResMut<LoadRequest>,
    mut selected: ResMut<SelectedPrim>,
    prims: Query<(Entity, &Name, &UsdPrimRef)>,
    mesh_q: Query<(), With<Mesh3d>>,
    kind_q: Query<&UsdKind>,
    audio_q: Query<&UsdSpatialAudio>,
    proc_q: Query<&UsdProcedural>,
    vis_q: Query<&Visibility>,
    children: Query<&Children>,
) {
    if !is_panel_open(&open, RIB_SELECTION) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let stage = cid("sel_stage");
    let prim = cid("sel_prim");

    let sel = selected.0.and_then(|e| {
        prims.get(e).ok().map(|(_, n, pr)| {
            let mut tags: Vec<TagItem> = Vec::new();
            if mesh_q.get(e).is_ok() {
                tags.push(TagItem::new("mesh"));
            }
            if let Ok(k) = kind_q.get(e) {
                tags.push(TagItem::new(format!("kind:{}", k.kind)));
            }
            if children.get(e).map(|c| !c.is_empty()).unwrap_or(false) {
                tags.push(TagItem::new("parent"));
            }
            if audio_q.get(e).is_ok() {
                tags.push(TagItem::new("audio"));
            }
            if proc_q.get(e).is_ok() {
                tags.push(TagItem::new("procedural"));
            }
            if matches!(vis_q.get(e), Ok(Visibility::Hidden)) {
                tags.push(TagItem::colored("hidden", style::WARNING));
            }
            (n.as_str().to_string(), pr.path.clone(), tags)
        })
    });
    let stale = selected.0.is_some() && sel.is_none();

    let mut clear = false;
    let mut browse = false;
    let mut reveal = false;
    Pane::new(RIB_SELECTION, "Selection", pane_anchor_for(RIB_SELECTION), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                stage,
                "Loaded stage",
                "folder",
                vec![
                    Pod::new(pid("sel_stage", 0)).with_readout("file", info.path.as_str()),
                    Pod::new(pid("sel_stage", 1)).with_button("Browse USD…", accent_col),
                    Pod::new(pid("sel_stage", 2)).with_button("Reveal in filesystem", accent_col),
                ],
            );
            match &sel {
                Some((name, path, tags)) => {
                    body.add_normal(
                        prim,
                        "Selected prim",
                        "cursor",
                        vec![
                            Pod::new(pid("sel_prim", 0)).with_readout("name", name.as_str()),
                            Pod::new(pid("sel_prim", 1)).with_readout("path", path.as_str()),
                            Pod::new(pid("sel_prim", 2)).with_tag_items(tags.clone(), accent_col),
                            Pod::new(pid("sel_prim", 3)).with_button("Clear selection", accent_col),
                        ],
                    );
                }
                None => {
                    let msg = if stale {
                        "(selection stale)"
                    } else {
                        "Click a prim in the Tree panel"
                    };
                    body.add_normal(
                        prim,
                        "Selected prim",
                        "cursor",
                        vec![Pod::new(pid("sel_prim", 0)).with_readout("prim", msg)],
                    );
                }
            }
            let r = body.render();
            browse = pod(&r, stage, 1).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
            reveal = pod(&r, stage, 2).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
            clear = pod(&r, prim, 3).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
        });

    if stale || clear {
        selected.0 = None;
    }
    if browse
        && let Some(picked) = rfd::FileDialog::new()
            .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
            .pick_file()
    {
        load_req.path = Some(PathBuf::from(picked));
    }
    if reveal {
        let full = requested.root.join(&info.path);
        let target = full.parent().unwrap_or(&requested.root).to_path_buf();
        let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
    }
}

// ─── Prim-tree panel ────────────────────────────────────────────────

/// Owned snapshot of one prim row for the (`'static`) tree closure.
#[derive(Clone)]
struct TreeNodeSnap {
    path: String,
    label: String,
    depth: u32,
    has_children: bool,
    swatch: Option<[f32; 3]>,
}

fn swatch_rgb_for(
    entity: Entity,
    mat_q: &Query<&MeshMaterial3d<StandardMaterial>>,
    children: &Query<&Children>,
    materials: &Assets<StandardMaterial>,
) -> Option<[f32; 3]> {
    let pick = |e: Entity| -> Option<[f32; 3]> {
        let mm = mat_q.get(e).ok()?;
        let mat = materials.get(&mm.0)?;
        let c = mat.base_color.to_linear();
        Some([c.red, c.green, c.blue])
    };
    pick(entity).or_else(|| children.get(entity).ok().and_then(|cs| cs.iter().find_map(pick)))
}

const TREE_SELECTED_KEY: &str = "usdview_tree_selected";

fn tree_expand_key(path: &str) -> egui::Id {
    egui::Id::new(("usdview_tree_expand", path))
}
fn tree_vis_key(path: &str) -> egui::Id {
    egui::Id::new(("usdview_tree_hidden", path))
}

#[allow(clippy::too_many_arguments)]
fn draw_tree_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    mut selected: ResMut<SelectedPrim>,
    mut fly: ResMut<FlyTo>,
    mut filter: ResMut<TreeFilter>,
    materials: Res<Assets<StandardMaterial>>,
    cameras: Query<&ArcballCamera>,
    gt_query: Query<&GlobalTransform>,
    extent_q: Query<&usd_bevy::UsdLocalExtent>,
    prims: Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    mat_q: Query<&MeshMaterial3d<StandardMaterial>>,
    mut visibility_q: Query<(Entity, &mut Visibility)>,
    children: Query<&Children>,
) {
    if !is_panel_open(&open, RIB_TREE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;

    // Owned, sorted snapshot of every prim (path hierarchy).
    let mut nodes: Vec<TreeNodeSnap> = Vec::new();
    let mut path_entity: HashMap<String, Entity> = HashMap::new();
    for (e, name, pref, dn) in prims.iter() {
        path_entity.entry(pref.path.clone()).or_insert(e);
        let label = dn.map(|d| d.0.clone()).unwrap_or_else(|| name.as_str().to_string());
        let depth = pref.path.matches('/').count().saturating_sub(1) as u32;
        nodes.push(TreeNodeSnap {
            path: pref.path.clone(),
            label,
            depth,
            has_children: false,
            swatch: swatch_rgb_for(e, &mat_q, &children, &materials),
        });
    }
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    let paths_sorted: Vec<String> = nodes.iter().map(|n| n.path.clone()).collect();
    for n in nodes.iter_mut() {
        let prefix = format!("{}/", n.path);
        n.has_children = paths_sorted.iter().any(|p| p.starts_with(&prefix));
    }

    let filter_lc = filter.0.to_lowercase();
    let prim_count = nodes.len();

    Pane::new(RIB_TREE, "Prim tree", pane_anchor_for(RIB_TREE), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                cid("tree"),
                "Hierarchy",
                "folder",
                vec![
                    Pod::new(pid("tree", 0)).with_readout("prims", prim_count.to_string()),
                    Pod::new(pid("tree", 1)).with_search("Search prims…", accent_col),
                    Pod::new(pid("tree", 2)).fill().with_tree(8, move |tree| {
                        render_tree(tree, &nodes, &filter_lc, accent_col);
                    }),
                ],
            );
            body.render();
        });

    // Reconcile ctx-data interactions with the ECS (the tree closure is
    // `'static`, so it persists selection / hidden-set to ctx-data here).
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    filter.0 = Pod::search_query(ctx, pid("tree", 1), 0);
    for (e, mut v) in visibility_q.iter_mut() {
        if let Ok((_, _, pref, _)) = prims.get(e) {
            let hidden = ctx.data_mut(|d| d.get_persisted::<bool>(tree_vis_key(&pref.path))).unwrap_or(false);
            let want = if hidden { Visibility::Hidden } else { Visibility::Inherited };
            if *v != want {
                *v = want;
            }
        }
    }
    let sel_path: String = ctx
        .data(|d| d.get_temp::<String>(egui::Id::new(TREE_SELECTED_KEY)))
        .unwrap_or_default();
    if !sel_path.is_empty()
        && let Some(&entity) = path_entity.get(&sel_path)
    {
        let already = selected.0 == Some(entity);
        selected.0 = Some(entity);
        if !already
            && let Ok(cam) = cameras.single()
        {
            let (target, target_dist) =
                fit_params_for_entity(entity, &gt_query, &extent_q, &children, cam.distance);
            fly.start_focus = cam.focus;
            fly.start_distance = cam.distance;
            fly.target_focus = target;
            fly.target_distance = target_dist;
            fly.duration = 0.4;
            fly.remaining = 0.4;
        }
    }
}

/// Render the prim hierarchy inside the (`'static`) `with_tree` closure
/// from the owned snapshot. Persists expansion / selection / hidden-set
/// to ctx-data so the system can reconcile with the ECS.
fn render_tree(
    tree: &mut mara_core::widget::TreeBody,
    nodes: &[TreeNodeSnap],
    filter: &str,
    accent: egui::Color32,
) {
    let flat = !filter.is_empty();
    let selected = tree
        .temp_string(egui::Id::new(TREE_SELECTED_KEY))
        .unwrap_or_default();

    let mut collapsed_at: Option<u32> = None;
    for node in nodes {
        if flat && !node.path.to_lowercase().contains(filter) {
            continue;
        }
        if !flat
            && let Some(d) = collapsed_at
        {
            if node.depth > d {
                continue;
            }
            collapsed_at = None;
        }
        let depth = if flat { 0 } else { node.depth };
        let expand_key = tree_expand_key(&node.path);
        let vis_key = tree_vis_key(&node.path);
        let mut expanded = tree.persisted_bool(expand_key).unwrap_or(true);
        let hidden = tree.persisted_bool(vis_key).unwrap_or(false);
        let mut visible = !hidden;

        let is_sel = selected == node.path;
        let id_salt = egui::Id::new(("usdview_treerow", &node.path));

        let resp = {
            let mut slots: Vec<TreeIconSlot<'_>> = Vec::with_capacity(2);
            slots.push(TreeIconSlot::new(TreeIconKind::Eye, &mut visible).with_tooltip("Toggle visibility"));
            let mut sentinel = false;
            if let Some(rgb) = node.swatch {
                slots.push(TreeIconSlot::new(
                    TreeIconKind::Color(style::srgb_to_egui(rgb)),
                    &mut sentinel,
                ));
            }
            let exp = if !flat && node.has_children {
                Some(&mut expanded)
            } else {
                None
            };
            tree.row(id_salt, depth, exp, None, &node.label, is_sel, accent, &mut slots)
        };

        let _ = hidden;
        tree.set_persisted_bool(vis_key, !visible);
        tree.set_persisted_bool(expand_key, expanded);
        if !flat && node.has_children && !expanded {
            collapsed_at = Some(node.depth);
        }
        if resp.body.clicked {
            tree.set_temp_string(egui::Id::new(TREE_SELECTED_KEY), node.path.clone());
        }
    }
}

/// Walk the subtree rooted at `root`, transforming each descendant's
/// authored local extent into world space, and fold into one AABB.
fn fit_params_for_entity(
    root: Entity,
    gt_q: &Query<&GlobalTransform>,
    extent_q: &Query<&usd_bevy::UsdLocalExtent>,
    children: &Query<&Children>,
    current_cam_dist: f32,
) -> (Vec3, f32) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;

    let mut stack: Vec<Entity> = vec![root];
    while let Some(e) = stack.pop() {
        if let (Ok(gt), Ok(le)) = (gt_q.get(e), extent_q.get(e)) {
            let m = gt.to_matrix();
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { le.min[0] } else { le.max[0] },
                    if i & 2 == 0 { le.min[1] } else { le.max[1] },
                    if i & 4 == 0 { le.min[2] } else { le.max[2] },
                );
                let w = m.transform_point3(c);
                min = min.min(w);
                max = max.max(w);
            }
            found = true;
        }
        if let Ok(cs) = children.get(e) {
            for c in cs.iter() {
                stack.push(c);
            }
        }
    }

    if found {
        let center = (min + max) * 0.5;
        let size = (max - min).abs();
        let max_dim = size.x.max(size.y).max(size.z).max(0.05);
        (center, (max_dim * 1.6).clamp(0.2, 200.0))
    } else if let Ok(gt) = gt_q.get(root) {
        (gt.translation(), (current_cam_dist * 0.25).clamp(0.2, 40.0))
    } else {
        (Vec3::ZERO, current_cam_dist)
    }
}

// ─── Stage-info panel ───────────────────────────────────────────────

fn draw_info_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
    mut reload: ResMut<ReloadRequest>,
    prims: Query<&UsdPrimRef>,
    meshes_q: Query<&Mesh3d, With<UsdPrimRef>>,
    spatial_audio_q: Query<&UsdSpatialAudio>,
    procedural_q: Query<&UsdProcedural>,
) {
    if !is_panel_open(&open, RIB_INFO) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let actions = cid("info_actions");
    let mut reload_clicked = false;
    Pane::new(RIB_INFO, "Stage info", pane_anchor_for(RIB_INFO), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                cid("info_stage"),
                "Stage",
                "document",
                vec![
                    Pod::new(pid("info_stage", 0)).with_readout("file", info.path.as_str()),
                    Pod::new(pid("info_stage", 1)).with_readout("defaultPrim", info.default_prim.as_deref().unwrap_or("—")),
                    Pod::new(pid("info_stage", 2)).with_readout("layers", info.layer_count.to_string()),
                    Pod::new(pid("info_stage", 3)).with_readout("prims", prims.iter().count().to_string()),
                    Pod::new(pid("info_stage", 4)).with_readout("meshes", meshes_q.iter().count().to_string()),
                    Pod::new(pid("info_stage", 5)).with_readout("variants", info.variant_count.to_string()),
                ],
            );
            body.add_normal(
                cid("info_lights"),
                "Lights & instances",
                "options",
                vec![
                    Pod::new(pid("info_lights", 0)).with_badge_row(
                        "lights",
                        vec![
                            format!("{} dir", info.lights_directional),
                            format!("{} pt", info.lights_point),
                            format!("{} spot", info.lights_spot),
                            format!("{} dome", info.lights_dome),
                        ],
                        accent_col,
                    ),
                    Pod::new(pid("info_lights", 1)).with_badge_row(
                        "instances",
                        vec![
                            format!("{} prim", info.instance_prim_count),
                            format!("{} reuse", info.instance_prototype_reuses),
                        ],
                        accent_col,
                    ),
                    Pod::new(pid("info_lights", 2)).with_readout("animated", format!("{} prim(s)", info.animated_prim_count)),
                ],
            );
            body.add_normal(
                cid("info_skel"),
                "Skel & render",
                "options",
                vec![
                    Pod::new(pid("info_skel", 0)).with_badge_row(
                        "skel",
                        vec![
                            format!("{} skel", info.skeleton_count),
                            format!("{} root", info.skel_root_count),
                            format!("{} bind", info.skel_binding_count),
                        ],
                        accent_col,
                    ),
                    Pod::new(pid("info_skel", 1)).with_badge_row(
                        "render",
                        vec![
                            format!("{} settings", info.render_settings_count),
                            format!("{} product", info.render_product_count),
                            format!("{} var", info.render_var_count),
                        ],
                        accent_col,
                    ),
                    Pod::new(pid("info_skel", 2)).with_badge_row(
                        "physics",
                        vec![
                            format!("{} scene", info.physics_scene_count),
                            format!("{} rigid", info.rigid_body_count),
                            format!("{} joint", info.joint_count),
                        ],
                        accent_col,
                    ),
                ],
            );
            body.add_normal(
                cid("info_authoring"),
                "Authoring detail",
                "document",
                vec![
                    Pod::new(pid("info_authoring", 0)).with_readout("custom", format!("{} prim · {} layer", info.custom_attr_prim_count, info.custom_layer_data_entries)),
                    Pod::new(pid("info_authoring", 1)).with_readout("subdiv", format!("{} mesh(es)", info.subdivision_prim_count)),
                    Pod::new(pid("info_authoring", 2)).with_readout("light-link", format!("{} light(s)", info.light_linked_count)),
                    Pod::new(pid("info_authoring", 3)).with_readout("clips", format!("{} prim(s)", info.clip_prim_count)),
                    Pod::new(pid("info_authoring", 4)).with_readout("spatial-audio", format!("{} source(s)", spatial_audio_q.iter().count())),
                    Pod::new(pid("info_authoring", 5)).with_readout("procedural", format!("{} prim(s)", procedural_q.iter().count())),
                ],
            );
            body.add_normal(
                actions,
                "Actions",
                "options",
                vec![Pod::new(pid("info_actions", 0)).with_button("Reload stage (R)", accent_col)],
            );
            let r = body.render();
            reload_clicked = pod(&r, actions, 0).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
        });
    if reload_clicked {
        reload.requested = true;
    }
}

// ─── Variants panel ─────────────────────────────────────────────────

fn draw_variants_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    stage: Option<Res<crate::StageHandle>>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut loader_tuning: ResMut<LoaderTuning>,
    mut pending_anim: ResMut<PendingAnimationClip>,
    mut pending_variant_switch: ResMut<usd_bevy::incremental::PendingVariantSwitch>,
) {
    if !is_panel_open(&open, RIB_VARIANTS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;

    // Owned snapshot of (prim, set_name, options, current_idx, is_anim).
    let mut sets: Vec<(String, String, Vec<String>, usize, bool)> = Vec::new();
    if let Some(asset) = stage.as_ref().and_then(|s| usd_assets.get(&s.0)) {
        for (prim_path, vsets) in &asset.variants {
            for set in vsets {
                if set.options.is_empty() {
                    continue;
                }
                let key = (prim_path.clone(), set.name.clone());
                let authored = set.selection.as_deref().unwrap_or("");
                let current = loader_tuning.variants.get(&key).cloned().unwrap_or_else(|| authored.to_string());
                let idx = set.options.iter().position(|o| o == &current).unwrap_or(0);
                sets.push((prim_path.clone(), set.name.clone(), set.options.clone(), idx, set.name == "anim"));
            }
        }
    }
    sets.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));

    let cont = cid("variants");
    let mut picks: Vec<(usize, usize)> = Vec::new();
    Pane::new(RIB_VARIANTS, "Variants", pane_anchor_for(RIB_VARIANTS), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            if sets.is_empty() {
                body.add_normal(
                    cont,
                    "Variants",
                    "options",
                    vec![Pod::new(pid("variants", 0)).with_readout("variants", "(none authored)")],
                );
                body.render();
                return;
            }
            let pods: Vec<Pod> = sets
                .iter()
                .enumerate()
                .map(|(i, (prim, name, opts, idx, _))| {
                    let label = format!("{}  ·  {name}", prim.rsplit('/').next().unwrap_or(prim));
                    Pod::new(pid("variants", i))
                        .with_readout("set", label)
                        .with_dropdown(opts.clone(), *idx, accent_col)
                })
                .collect();
            body.add_normal(cont, "Variant sets", "options", pods);
            let r = body.render();
            for (i, _) in sets.iter().enumerate() {
                if let Some(d) = pod(&r, cont, i).and_then(|p| p.dropdowns.first())
                    && d.changed
                {
                    picks.push((i, d.selected));
                }
            }
        });

    for (set_idx, opt_idx) in picks {
        let (prim, name, opts, _cur, is_anim) = &sets[set_idx];
        let Some(picked) = opts.get(opt_idx).cloned() else {
            continue;
        };
        loader_tuning.variants.insert((prim.clone(), name.clone()), picked.clone());
        if *is_anim {
            pending_anim.name = Some(picked);
        } else {
            pending_variant_switch.queue.push((prim.clone(), name.clone(), picked));
        }
    }
}

// ─── Cameras panel ──────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_cameras_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut camera_mount: ResMut<CameraMount>,
    mut bookmarks: ResMut<CameraBookmarks>,
    mut fly: ResMut<FlyTo>,
    cameras: Query<&ArcballCamera>,
) {
    if !is_panel_open(&open, RIB_CAMERAS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;

    let bm_names: Vec<String> = bookmarks.items.iter().map(|b| b.name.clone()).collect();
    let bm_trailing: Vec<String> = bookmarks.items.iter().map(|b| format!("d {:.1}", b.distance)).collect();

    let mut cam_labels: Vec<String> = vec!["Arcball (free)".to_string()];
    let mut cam_trailing: Vec<String> = vec!["free".to_string()];
    let mut cam_paths: Vec<Option<String>> = vec![None];
    if let Some(asset) = usd_assets.iter().next().map(|(_, a)| a) {
        for cam in &asset.cameras {
            let name = cam.path.rsplit('/').next().unwrap_or(&cam.path);
            let focal = cam.data.focal_length_mm.unwrap_or(50.0);
            let proj = match cam.data.projection {
                Some(usd_bevy::read::camera::Projection::Orthographic) => "ortho",
                _ => "persp",
            };
            cam_labels.push(name.to_string());
            cam_trailing.push(format!("{focal:.0}mm · {proj}"));
            cam_paths.push(Some(cam.path.clone()));
        }
    }

    let bm_cont = cid("cam_bm");
    let cam_cont = cid("cam_all");
    let mut save = false;
    let mut jump: Option<usize> = None;
    let mut delete: Option<usize> = None;
    let mut mount_idx: Option<usize> = None;

    Pane::new(RIB_CAMERAS, "Cameras", pane_anchor_for(RIB_CAMERAS), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            let mut bm_pods = vec![Pod::new(pid("cam_bm", 0)).with_button("Save current view", accent_col)];
            if bm_names.is_empty() {
                bm_pods.push(Pod::new(pid("cam_bm", 1)).with_readout("bookmarks", "(none yet)"));
            } else {
                bm_pods.push(
                    Pod::new(pid("cam_bm", 1)).with_hybrid_select_list(bm_names.clone(), Some(bm_trailing.clone()), accent_col),
                );
            }
            body.add_normal(bm_cont, "Bookmarks", "history", bm_pods);
            body.add_normal(
                cam_cont,
                "Cameras",
                "cube",
                vec![Pod::new(pid("cam_all", 0)).with_hybrid_select_list(cam_labels.clone(), Some(cam_trailing.clone()), accent_col)],
            );
            let r = body.render();
            save = pod(&r, bm_cont, 0).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
            if let Some(l) = pod(&r, bm_cont, 1).and_then(|p| p.hybrid_select_lists.first()) {
                jump = l.body_clicked;
                delete = l.radio_clicked;
            }
            if let Some(l) = pod(&r, cam_cont, 0).and_then(|p| p.hybrid_select_lists.first()) {
                mount_idx = l.body_clicked.or(l.radio_clicked);
            }
        });

    if save && let Ok(cam) = cameras.single() {
        let seq = bookmarks.next_seq + 1;
        bookmarks.next_seq = seq;
        bookmarks.items.push(CameraBookmark {
            name: format!("View {seq}"),
            focus: cam.focus,
            distance: cam.distance,
            yaw: cam.yaw,
            elevation: cam.elevation,
        });
    }
    if let Some(idx) = jump
        && let (Ok(cam), Some(bm)) = (cameras.single(), bookmarks.items.get(idx))
    {
        *camera_mount = CameraMount::Arcball;
        fly.start_focus = cam.focus;
        fly.start_distance = cam.distance;
        fly.start_yaw = Some(cam.yaw);
        fly.start_elevation = Some(cam.elevation);
        fly.target_focus = bm.focus;
        fly.target_distance = bm.distance;
        fly.target_yaw = Some(bm.yaw);
        fly.target_elevation = Some(bm.elevation);
        fly.duration = 0.5;
        fly.remaining = 0.5;
    }
    if let Some(idx) = delete
        && idx < bookmarks.items.len()
    {
        bookmarks.items.remove(idx);
    }
    if let Some(idx) = mount_idx {
        *camera_mount = match cam_paths.get(idx).and_then(|p| p.clone()) {
            Some(prim_path) => CameraMount::Mounted { prim_path },
            None => CameraMount::Arcball,
        };
    }
}

// ─── Materials panel ────────────────────────────────────────────────

fn draw_materials_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    materials: Res<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    usd_mesh_mats: Query<&MeshMaterial3d<StandardMaterial>, With<UsdPrimRef>>,
) {
    if !is_panel_open(&open, RIB_MATERIALS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;

    let mut bound: HashSet<AssetId<StandardMaterial>> = HashSet::new();
    for mm in usd_mesh_mats.iter() {
        bound.insert(mm.0.id());
    }
    let mut entries: Vec<(String, bool)> = materials
        .iter()
        .filter(|(id, _)| bound.contains(id))
        .map(|(id, mat)| {
            let label = asset_server.get_path(id).map(|p| p.to_string()).unwrap_or_else(|| format!("{id:?}"));
            let short = label.rsplit('/').next().unwrap_or(&label).chars().take(48).collect::<String>();
            (short, mat.base_color_texture.is_some())
        })
        .collect();
    entries.sort();

    Pane::new(RIB_MATERIALS, "Materials", pane_anchor_for(RIB_MATERIALS), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            let mut pods = vec![Pod::new(pid("mat", 0)).with_readout("materials", entries.len().to_string())];
            for (i, (name, textured)) in entries.iter().enumerate() {
                pods.push(
                    Pod::new(pid("mat", i + 1)).with_readout(name.as_str(), if *textured { "textured" } else { "constant color" }),
                );
            }
            body.add_normal(cid("mat"), "Bound materials", "color", pods);
            body.render();
        });
}

// ─── Overlays panel ─────────────────────────────────────────────────

fn draw_overlays_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    mut toggles: ResMut<DisplayToggles>,
    mut loader_tuning: ResMut<LoaderTuning>,
) {
    if !is_panel_open(&open, RIB_OVERLAYS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let world = cid("ov_world");
    let render = cid("ov_render");
    let curves = cid("ov_curves");

    let mut t = toggles.clone();
    let mut lt = loader_tuning.clone();
    Pane::new(RIB_OVERLAYS, "Overlays", pane_anchor_for(RIB_OVERLAYS), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                world,
                "World overlays",
                "square-multiple",
                vec![
                    Pod::new(pid("ov_world", 0)).with_toggle_initial("Ground grid (G)", accent_col, t.show_world_grid),
                    Pod::new(pid("ov_world", 1)).with_toggle_initial("World axes (X)", accent_col, t.show_world_axes),
                    Pod::new(pid("ov_world", 2)).with_toggle_initial("Prim markers (P)", accent_col, t.show_prim_markers),
                    Pod::new(pid("ov_world", 3)).with_slider("Prim marker bias", t.prim_marker_bias as f64, 0.0..=5.0, 2, "×", accent_col),
                    Pod::new(pid("ov_world", 4)).with_toggle_initial("Skeleton bones (B)", accent_col, t.show_skeleton),
                    Pod::new(pid("ov_world", 5)).with_toggle_initial("Physics gizmos (Y)", accent_col, t.show_physics),
                    Pod::new(pid("ov_world", 6)).with_toggle_initial("Collider wireframes (C)", accent_col, t.show_colliders),
                ],
            );
            body.add_normal(
                render,
                "Render",
                "color",
                vec![
                    Pod::new(pid("ov_render", 0)).with_toggle_initial("Wireframe", accent_col, t.wireframe),
                    Pod::new(pid("ov_render", 1)).with_slider("Light intensity", t.light_intensity_scale as f64, 0.0..=5.0, 2, "×", accent_col),
                ],
            );
            body.add_normal(
                curves,
                "Curves (tubes)",
                "options",
                vec![
                    Pod::new(pid("ov_curves", 0)).with_slider("Radius", lt.curves.default_radius as f64, 0.001..=0.2, 3, " m", accent_col),
                    Pod::new(pid("ov_curves", 1)).with_slider("Ring segments", lt.curves.ring_segments as f64, 3.0..=24.0, 0, "", accent_col),
                    Pod::new(pid("ov_curves", 2)).with_slider("Point scale", lt.curves.point_scale as f64, 0.05..=4.0, 2, "×", accent_col),
                ],
            );
            let r = body.render();
            let tog = |c, i: usize, cur: bool| -> bool {
                pod(&r, c, i).and_then(|p| p.toggles.first()).filter(|x| x.changed).map(|x| x.on).unwrap_or(cur)
            };
            let sld = |c, i: usize, cur: f64| -> f64 {
                pod(&r, c, i).and_then(|p| p.sliders.first()).filter(|x| x.changed).map(|x| x.value).unwrap_or(cur)
            };
            t.show_world_grid = tog(world, 0, t.show_world_grid);
            t.show_world_axes = tog(world, 1, t.show_world_axes);
            t.show_prim_markers = tog(world, 2, t.show_prim_markers);
            t.prim_marker_bias = sld(world, 3, t.prim_marker_bias as f64) as f32;
            t.show_skeleton = tog(world, 4, t.show_skeleton);
            t.show_physics = tog(world, 5, t.show_physics);
            t.show_colliders = tog(world, 6, t.show_colliders);
            t.wireframe = tog(render, 0, t.wireframe);
            t.light_intensity_scale = sld(render, 1, t.light_intensity_scale as f64) as f32;
            lt.curves.default_radius = sld(curves, 0, lt.curves.default_radius as f64) as f32;
            lt.curves.ring_segments = sld(curves, 1, lt.curves.ring_segments as f64).round() as u32;
            lt.curves.point_scale = sld(curves, 2, lt.curves.point_scale as f64) as f32;
        });
    *toggles = t;
    *loader_tuning = lt;
}

// ─── Timeline panel ─────────────────────────────────────────────────

fn draw_timeline_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    mut clock: ResMut<UsdStageTime>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    if !is_panel_open(&open, RIB_TIMELINE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let cont = cid("timeline");
    let animated_count = usd_assets.iter().next().map(|(_, a)| a.animated_prims.len()).unwrap_or(0);
    let dur = clock.duration_seconds().max(1e-3);
    let summary = format!(
        "{animated_count} animated · {:.1} fps · {:.1}s",
        clock.time_codes_per_second,
        clock.duration_seconds()
    );
    let play_label = if clock.playing { "Pause" } else { "Play" };

    let mut play = false;
    let mut rewind = false;
    let mut new_secs: Option<f64> = None;
    Pane::new(RIB_TIMELINE, "Timeline", pane_anchor_for(RIB_TIMELINE), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                cont,
                "Playback",
                "clock",
                vec![
                    Pod::new(pid("timeline", 0)).with_readout("clip", summary.clone()),
                    Pod::new(pid("timeline", 1)).with_button(play_label, accent_col),
                    Pod::new(pid("timeline", 2)).with_button("Rewind", accent_col),
                    Pod::new(pid("timeline", 3)).with_slider("Seconds", clock.seconds, 0.0..=dur, 3, " s", accent_col),
                    Pod::new(pid("timeline", 4)).with_readout("timeCode", format!("{:.3}", clock.current_time_code())),
                    Pod::new(pid("timeline", 5)).with_readout("fps", format!("{:.2}", clock.time_codes_per_second)),
                ],
            );
            let r = body.render();
            play = pod(&r, cont, 1).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
            rewind = pod(&r, cont, 2).and_then(|p| p.buttons.first()).is_some_and(|b| b.clicked);
            if let Some(s) = pod(&r, cont, 3).and_then(|p| p.sliders.first()).filter(|s| s.changed) {
                new_secs = Some(s.value);
            }
        });
    if play {
        clock.playing = !clock.playing;
    }
    if rewind {
        clock.seconds = 0.0;
    }
    if let Some(s) = new_secs {
        clock.seconds = s;
    }
}

// ─── Keys panel ─────────────────────────────────────────────────────

fn draw_keys_panel(mut contexts: EguiContexts, open: Res<RibbonOpen>, accent: Res<AccentColor>) {
    if !is_panel_open(&open, RIB_KEYS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    Pane::new(RIB_KEYS, "Controls", pane_anchor_for(RIB_KEYS), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            body.add_normal(
                cid("keys_cam"),
                "Camera",
                "keyboard",
                vec![Pod::new(pid("keys_cam", 0)).with_keybindings(vec![
                    ("L+R drag", "Orbit"),
                    ("Middle", "Pan"),
                    ("Scroll", "Zoom"),
                ])],
            );
            body.add_normal(
                cid("keys_panels"),
                "Panels",
                "keyboard",
                vec![Pod::new(pid("keys_panels", 0)).with_keybindings(vec![
                    ("T", "Toggle prim tree"),
                    ("I", "Toggle stage info"),
                    ("O", "Toggle overlays"),
                    ("?", "Toggle this panel"),
                ])],
            );
            body.add_normal(
                cid("keys_ov"),
                "Overlays",
                "keyboard",
                vec![Pod::new(pid("keys_ov", 0)).with_keybindings(vec![
                    ("G", "Ground grid"),
                    ("X", "World axes"),
                    ("P", "Prim markers"),
                    ("B", "Skeleton bones"),
                    ("R", "Reload stage"),
                ])],
            );
            body.render();
        });
}

// ─── Log panel ──────────────────────────────────────────────────────

fn draw_log_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    accent: Res<AccentColor>,
    log: Res<crate::log_panel::LoaderLog>,
) {
    if !is_panel_open(&open, RIB_LOG) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let cont = cid("log");
    let lines: Vec<crate::log_panel::LogLine> = log
        .buffer
        .lock()
        .map(|b| b.iter().rev().take(80).cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    Pane::new(RIB_LOG, "Log", pane_anchor_for(RIB_LOG), accent_col)
        .resize(PaneResize::SPAN)
        .show(ctx, |body| {
            let mut pods = vec![Pod::new(pid("log", 0)).with_readout("entries", lines.len().to_string())];
            if lines.is_empty() {
                pods.push(Pod::new(pid("log", 1)).with_readout("log", "(no events yet — load a stage)"));
            } else {
                for (i, line) in lines.iter().enumerate() {
                    pods.push(
                        Pod::new(pid("log", i + 1)).with_readout(short_target(&line.target), line.message.as_str()),
                    );
                }
            }
            body.add_normal(cont, "Loader log", "list", pods);
            body.render();
        });
}

fn short_target(target: &str) -> String {
    target.rsplit("::").next().unwrap_or(target).to_string()
}

// ─── Command palette (Ctrl+K) ───────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_palette_panel(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut palette: ResMut<ViewerCommandPalette>,
    mut ribbon: ResMut<RibbonOpen>,
    mut toggles: ResMut<DisplayToggles>,
    mut reload: ResMut<ReloadRequest>,
    mut load_req: ResMut<LoadRequest>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(id) = command_palette(ctx, &mut palette.0, PALETTE_ITEMS, accent.0) else {
        return;
    };
    match id {
        "open_selection" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_SELECTION); }
        "open_tree" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TREE); }
        "open_info" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_INFO); }
        "open_variants" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_VARIANTS); }
        "open_cameras" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_CAMERAS); }
        "open_overlays" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_OVERLAYS); }
        "open_timeline" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TIMELINE); }
        "open_keys" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_KEYS); }
        "open_log" => { ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_LOG); }
        "toggle_grid" => toggles.show_world_grid = !toggles.show_world_grid,
        "toggle_axes" => toggles.show_world_axes = !toggles.show_world_axes,
        "toggle_markers" => toggles.show_prim_markers = !toggles.show_prim_markers,
        "toggle_wireframe" => toggles.wireframe = !toggles.wireframe,
        "reload_stage" => reload.requested = true,
        "browse_usd" => {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                .pick_file()
            {
                load_req.path = Some(PathBuf::from(picked));
            }
        }
        _ => {}
    }
    palette.0.open = false;
}
