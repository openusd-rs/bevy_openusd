//! Incremental, prim-keyed scene updates.
//!
//! Instead of rebuilding the whole scene when something changes, apply the
//! change to the *live entities* keyed by prim path (`UsdPrimRef`). The first
//! path implemented here is material/texture **variant switching**: recompose
//! the stage with the new selection, rebuild only the affected meshes'
//! materials, and reassign `MeshMaterial3d` in place — no re-tessellation, no
//! respawn, no metadata passes. Results are cached per `(prim, set, option)`
//! so re-selecting an option is instant.
//!
//! This is the seed of a general retained-mode projection: the same
//! "find affected entities by path, update in place" shape extends to
//! transforms / meshes, and pairs with a future change-notification layer
//! (which would supply the affected-path set directly).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use crate::asset::{VariantSelection, author_variant_session_layer};
use crate::prim_ref::UsdPrimRef;
use crate::read::{geom as ugeom, shade as ushade};
use crate::texture::AssetServerTextures;

/// The loaded stage's source layer + current variant selections, so the
/// incremental updater can recompose with a new selection. The viewer sets
/// this on every (re)load.
#[derive(Resource, Default, Clone)]
pub struct LoadedStageSource {
    /// Absolute path to the root layer.
    pub source: PathBuf,
    /// Search root (the source layer's directory).
    pub root: PathBuf,
    /// All currently-applied variant selections.
    pub base_variants: Vec<VariantSelection>,
}

/// Variant switches to apply incrementally — `(prim_path, set_name, option)`.
/// The viewer pushes here instead of triggering a full reload.
#[derive(Resource, Default)]
pub struct PendingVariantSwitch {
    pub queue: Vec<(String, String, String)>,
}

/// Set when an incremental switch isn't possible (geometry changed, or the
/// recompose/read failed). The viewer watches this and falls back to a full
/// reload, then resets it.
#[derive(Resource, Default)]
pub struct VariantReloadFallback(pub bool);

#[derive(Resource, Default)]
struct MaterialVariantCache(HashMap<(String, String, String), Vec<(String, Handle<StandardMaterial>)>>);

/// Per-mesh USD point counts of the currently-loaded geometry, cached per
/// source. Used to detect a variant that *changes geometry* (point count
/// differs) so it falls back to a full reload instead of swapping a material
/// onto a stale mesh.
#[derive(Resource, Default)]
struct BaseMeshPoints {
    points: HashMap<String, usize>,
    source: Option<PathBuf>,
}

/// Registers the incremental-update resources + system.
/// Every `(prim, set, option)` to precompute in the background after a load,
/// so the user's first click on any option is already cached → instant. The
/// viewer fills this when a stage finishes loading.
#[derive(Resource, Default)]
pub struct WarmVariantsQueue {
    pub queue: Vec<(String, String, String)>,
}

pub struct IncrementalPlugin;

impl Plugin for IncrementalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedStageSource>()
            .init_resource::<PendingVariantSwitch>()
            .init_resource::<VariantReloadFallback>()
            .init_resource::<WarmVariantsQueue>()
            .init_resource::<MaterialVariantCache>()
            .init_resource::<BaseMeshPoints>()
            .add_systems(Update, (apply_variant_switch, warm_variant_cache));
    }
}

type MeshMatQuery<'w, 's> =
    Query<'w, 's, (&'static UsdPrimRef, &'static Mesh3d, &'static mut MeshMaterial3d<StandardMaterial>)>;

/// Apply queued variant switches to the live scene — cached options swap
/// instantly; uncached ones compute once (recompose + build) then cache.
fn apply_variant_switch(
    mut pending: ResMut<PendingVariantSwitch>,
    source: Res<LoadedStageSource>,
    mut fallback: ResMut<VariantReloadFallback>,
    mut cache: ResMut<MaterialVariantCache>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut base: ResMut<BaseMeshPoints>,
    mut q: MeshMatQuery,
) {
    if pending.queue.is_empty() {
        return;
    }
    let live = live_meshes(&q);
    ensure_base(&source, &mut base, &live);
    for (prim, set, option) in std::mem::take(&mut pending.queue) {
        let key = (prim.clone(), set.clone(), option.clone());
        if let Some(per_mesh) = cache.0.get(&key).cloned() {
            reassign(&mut q, &per_mesh);
            info!("variant: instant swap {prim} {set}={option}");
            continue;
        }
        match compute_option(&source, &prim, &set, &option, &live, &base.points, &mut materials, &asset_server) {
            Some(per_mesh) => {
                reassign(&mut q, &per_mesh);
                info!("variant: computed + swapped {prim} {set}={option} ({} meshes)", per_mesh.len());
                cache.0.insert(key, per_mesh);
            }
            None => {
                warn!("variant: cannot live-swap {set}={option} (geometry change or read fail); reloading");
                fallback.0 = true;
            }
        }
    }
}

/// Background warm-up: precompute one queued option per frame so switching is
/// instant by the time the user clicks. Skips already-cached options.
fn warm_variant_cache(
    mut warm: ResMut<WarmVariantsQueue>,
    source: Res<LoadedStageSource>,
    mut cache: ResMut<MaterialVariantCache>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut base: ResMut<BaseMeshPoints>,
    q: Query<&UsdPrimRef>,
) {
    // One recompose per frame keeps the warm-up off the visible thread budget.
    let Some((prim, set, option)) = warm.queue.pop() else {
        return;
    };
    let key = (prim.clone(), set.clone(), option.clone());
    if cache.0.contains_key(&key) {
        return;
    }
    let live: Vec<String> = q.iter().map(|pref| pref.path.clone()).collect();
    ensure_base(&source, &mut base, &live);
    if let Some(per_mesh) = compute_option(&source, &prim, &set, &option, &live, &base.points, &mut materials, &asset_server) {
        cache.0.insert(key, per_mesh);
    }
}

/// Recompute the base geometry point-counts when the source changes.
fn ensure_base(source: &LoadedStageSource, base: &mut BaseMeshPoints, mesh_paths: &[String]) {
    if base.source.as_ref() == Some(&source.source) {
        return;
    }
    base.points.clear();
    base.source = Some(source.source.clone());
    let Some(stage) = open_stage(source, &source.base_variants) else {
        return;
    };
    for path in mesh_paths {
        if let Ok(ppath) = openusd::sdf::path(path)
            && let Ok(Some(rm)) = ugeom::read_mesh(&stage, &ppath)
        {
            base.points.insert(path.clone(), rm.points.len());
        }
    }
}

fn live_meshes(q: &MeshMatQuery) -> Vec<String> {
    q.iter().map(|(pref, _, _)| pref.path.clone()).collect()
}

/// Recompose with `option` selected for `(prim, set)` (other sets keep their
/// current selection), then build each mesh's bound material. Returns `None`
/// if the recompose fails or the variant changes geometry (caller reloads).
fn compute_option(
    source: &LoadedStageSource,
    prim: &str,
    set: &str,
    option: &str,
    mesh_paths: &[String],
    base_points: &HashMap<String, usize>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) -> Option<Vec<(String, Handle<StandardMaterial>)>> {
    let stage = open_with_variant(source, prim, set, option)?;
    let search = [source.root.clone()];
    let mut tex = AssetServerTextures {
        asset_server,
        search_paths: &search,
    };
    let mut per_mesh = Vec::new();
    // NOTE: no geometry-change guard yet — a variant that also changes topology
    // would leave stale meshes. Material/texture variants (the common case) are
    // correct; proper USD-to-USD geometry detection is a follow-up.
    for path in mesh_paths {
        let Ok(ppath) = openusd::sdf::path(path) else {
            continue;
        };
        // Geometry guard: if this option's mesh has a different point count
        // than the loaded geometry, the variant changes topology — bail to a
        // full reload rather than swap a material onto a stale mesh.
        if let Some(&base_n) = base_points.get(path)
            && let Ok(Some(rm)) = ugeom::read_mesh(&stage, &ppath)
            && rm.points.len() != base_n
        {
            return None;
        }
        let Ok(Some(mat_prim)) = ushade::read_material_binding(&stage, &ppath) else {
            continue;
        };
        // Full builder (name-guessed textures, glass, MDL fix) so the live
        // material matches what the loader bakes — not a stripped-down grey.
        let mat = crate::build::build_material_inner(&stage, &mat_prim, false, &mut tex, None);
        let handle = materials.add(mat);
        per_mesh.push((path.clone(), handle));
    }
    Some(per_mesh)
}

fn reassign(q: &mut MeshMatQuery, per_mesh: &[(String, Handle<StandardMaterial>)]) {
    for (path, handle) in per_mesh {
        for (pref, _, mut mat) in q.iter_mut() {
            if &pref.path == path {
                mat.0 = handle.clone();
            }
        }
    }
}

/// Recompose the source layer with `option` selected for `(prim, set)` via a
/// session layer (openusd directly — no Bevy AssetServer needed for the stage).
fn open_with_variant(source: &LoadedStageSource, prim: &str, set: &str, option: &str) -> Option<openusd::usd::Stage> {
    let mut sels: Vec<VariantSelection> = source
        .base_variants
        .iter()
        .filter(|v| !(v.prim_path.as_str() == prim && v.set_name.as_str() == set))
        .cloned()
        .collect();
    sels.push(VariantSelection {
        prim_path: prim.to_string(),
        set_name: set.to_string(),
        option: option.to_string(),
    });
    open_stage(source, &sels)
}

/// Recompose the source layer with `sels` applied via a session layer.
fn open_stage(source: &LoadedStageSource, sels: &[VariantSelection]) -> Option<openusd::usd::Stage> {
    let text = author_variant_session_layer(sels);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    source.source.hash(&mut hasher);
    let tmp = std::env::temp_dir().join(format!("usd_variant_session_{:016x}.usda", hasher.finish()));
    std::fs::write(&tmp, &text).ok()?;
    openusd::usd::Stage::builder()
        .resolver(openusd::ar::DefaultResolver::with_search_paths(vec![source.root.clone()]))
        .session_layer(tmp.to_str()?.to_string())
        .open(source.source.to_str()?)
        .ok()
}
