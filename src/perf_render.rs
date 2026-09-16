use std::{sync::Arc, time::Instant};
use bevy::{asset::{AssetId, RenderAssetUsages}, prelude::*};
use bevy::render::{
    erased_render_asset::ErasedRenderAssets,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    mesh::RenderMesh, render_asset::RenderAssets,
    render_resource::{CachedPipelineState, PipelineCache},
    texture::GpuImage, Render, RenderApp, RenderSystems,
};
use usd_bevy::route::flat_material::FlatMaterial;

#[derive(Resource, Clone, ExtractResource)]
struct UploadManifest {
    started: Instant,
    document: Option<u64>,
    generation: u64,
    collected_ms: f64,
    meshes: Arc<[AssetId<Mesh>]>,
    images: Arc<[AssetId<Image>]>,
    materials: Arc<[AssetId<StandardMaterial>]>,
    flat_materials: Arc<[AssetId<FlatMaterial>]>,
}

impl Default for UploadManifest {
    fn default() -> Self {
        Self { started: Instant::now(), document: None, generation: 0, collected_ms: 0.0,
            meshes: Arc::from([]), images: Arc::from([]), materials: Arc::from([]), flat_materials: Arc::from([]) }
    }
}

pub fn configure(app: &mut App) {
    if std::env::var_os("USD_PROFILE_RENDER").is_none() { return; }
    if app.get_sub_app(RenderApp).is_none() {
        eprintln!("render_asset_profile unavailable: no render sub-app");
        return;
    }
    app.init_resource::<UploadManifest>().add_plugins(ExtractResourcePlugin::<UploadManifest>::default())
        .add_systems(Last, collect_manifest);
    app.sub_app_mut(RenderApp).add_systems(Render, report_uploads.after(RenderSystems::Render));
}

fn collect_manifest(
    session: Option<NonSend<usd_bevy::editor::EditorSession>>,
    meshes: Res<Assets<Mesh>>, images: Res<Assets<Image>>,
    materials: Res<Assets<StandardMaterial>>, flat_materials: Option<Res<Assets<FlatMaterial>>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut material_events: MessageReader<AssetEvent<StandardMaterial>>,
    mut manifest: ResMut<UploadManifest>,
) {
    let changed = mesh_events.read().count() + image_events.read().count() + material_events.read().count() != 0
        || flat_materials.as_ref().is_some_and(|materials| materials.is_changed())
        || (flat_materials.is_none() && !manifest.flat_materials.is_empty());
    let document = session.as_ref().map(|session| session.document_id());
    if document == manifest.document && !changed { return; }
    manifest.document = document;
    manifest.generation = manifest.generation.checked_add(1).expect("upload generation exhausted");
    manifest.collected_ms = manifest.started.elapsed().as_secs_f64() * 1000.0;
    manifest.meshes = meshes.iter().filter(|(_, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .map(|(id, _)| id).collect::<Vec<_>>().into();
    manifest.images = images.iter().filter(|(_, image)| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .map(|(id, _)| id).collect::<Vec<_>>().into();
    manifest.materials = materials.ids().collect::<Vec<_>>().into();
    manifest.flat_materials = flat_materials.map_or_else(Vec::new, |materials| materials.ids().collect::<Vec<_>>()).into();
}

fn pending<T: Copy>(ids: &[T], mut prepared: impl FnMut(T) -> bool) -> usize {
    ids.iter().filter(|&&id| !prepared(id)).count()
}

fn report_uploads(
    manifest: Option<Res<UploadManifest>>,
    meshes: Res<RenderAssets<RenderMesh>>, images: Res<RenderAssets<GpuImage>>,
    materials: Res<ErasedRenderAssets<bevy::pbr::PreparedMaterial>>,
    pipelines: Res<PipelineCache>, mut previous: Local<Option<(u64, usize, usize, usize, usize, usize)>>,
) {
    let Some(manifest) = manifest.filter(|manifest| manifest.document.is_some()) else { return; };
    let missing_meshes = pending(&manifest.meshes, |id| meshes.get(id).is_some());
    let missing_images = pending(&manifest.images, |id| images.get(id).is_some());
    let missing_materials = pending(&manifest.materials, |id| materials.get(id.untyped()).is_some())
        + pending(&manifest.flat_materials, |id| materials.get(id.untyped()).is_some());
    let mut waiting = 0;
    let mut errors = Vec::new();
    for pipeline in pipelines.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => (),
            CachedPipelineState::Err(error) => errors.push(error.to_string()),
            _ => waiting += 1,
        }
    }
    let state = (manifest.generation, missing_meshes, missing_images, missing_materials, waiting, errors.len());
    if previous.as_ref() == Some(&state) { return; }
    *previous = Some(state);
    eprintln!("render_asset_profile {}", serde_json::json!({
        "scope": "all-render-world-meshes-images-retained-standard-flat-materials-and-global-pipelines",
        "clock_origin": "viewer-render-probe-configuration",
        "complete_frame": false,
        "document": manifest.document,
        "generation": manifest.generation,
        "cpu_manifest_elapsed_ms": manifest.collected_ms,
        "post_render_poll_elapsed_ms": manifest.started.elapsed().as_secs_f64()*1000.0,
        "meshes": manifest.meshes.len(), "images": manifest.images.len(),
        "standard_materials": manifest.materials.len(), "flat_materials": manifest.flat_materials.len(),
        "pending_meshes": missing_meshes, "pending_images": missing_images,
        "pending_prepared_materials": missing_materials,
        "pending_pipelines": waiting, "pipeline_errors": errors,
        "observed_uploads_ready": missing_meshes == 0 && missing_images == 0 && missing_materials == 0 && waiting == 0 && errors.is_empty(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_only_changes_refresh_manifest_without_flat_plugin() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>().init_resource::<UploadManifest>()
            .add_message::<AssetEvent<Mesh>>().add_message::<AssetEvent<Image>>()
            .add_message::<AssetEvent<StandardMaterial>>().add_systems(Update, collect_manifest);
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        app.world_mut().write_message(AssetEvent::<StandardMaterial>::Added { id: material.id() });
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().materials.as_ref(), &[material.id()]);
        let generation = app.world().resource::<UploadManifest>().generation;
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation);
        app.init_resource::<Assets<FlatMaterial>>();
        let flat = app.world_mut().resource_mut::<Assets<FlatMaterial>>().add(FlatMaterial::default());
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().flat_materials.as_ref(), &[flat.id()]);
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation + 1);
        app.world_mut().remove_resource::<Assets<FlatMaterial>>();
        app.update();
        assert!(app.world().resource::<UploadManifest>().flat_materials.is_empty());
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation + 2);
        app.world_mut().resource_mut::<Assets<StandardMaterial>>().remove(material.id());
        app.world_mut().write_message(AssetEvent::<StandardMaterial>::Removed { id: material.id() });
        app.update();
        assert!(app.world().resource::<UploadManifest>().materials.is_empty());
    }

    #[test]
    fn delayed_or_removed_assets_remain_pending() {
        let manifest = [1, 2, 3];
        assert_eq!(pending(&manifest, |_| false), 3);
        assert_eq!(pending(&manifest, |id| id != 2), 1);
        assert_eq!(pending(&manifest, |_| true), 0);
        assert_eq!(pending(&manifest, |id| id != 3), 1);
        assert_eq!(pending::<u32>(&[], |_| false), 0);
    }
}
