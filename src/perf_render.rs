use std::{sync::Arc, time::Instant};
use bevy::{asset::{AssetId, RenderAssetUsages}, prelude::*};
use bevy::render::{
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    mesh::RenderMesh, render_asset::RenderAssets,
    render_resource::{CachedPipelineState, PipelineCache},
    texture::GpuImage, Render, RenderApp, RenderSystems,
};

#[derive(Resource, Clone, ExtractResource)]
struct UploadManifest {
    started: Instant,
    document: Option<u64>,
    generation: u64,
    collected_ms: f64,
    meshes: Arc<[AssetId<Mesh>]>,
    images: Arc<[AssetId<Image>]>,
}

pub fn configure(app: &mut App) {
    if std::env::var_os("USD_PROFILE_RENDER").is_none() { return; }
    if app.get_sub_app(RenderApp).is_none() {
        eprintln!("render_asset_profile unavailable: no render sub-app");
        return;
    }
    app.insert_resource(UploadManifest {
        started: Instant::now(), document: None, generation: 0, collected_ms: 0.0,
        meshes: Arc::from([]), images: Arc::from([]),
    }).add_plugins(ExtractResourcePlugin::<UploadManifest>::default())
        .add_systems(Last, collect_manifest);
    app.sub_app_mut(RenderApp).add_systems(Render, report_uploads.after(RenderSystems::Render));
}

fn collect_manifest(
    session: Option<NonSend<usd_bevy::editor::EditorSession>>,
    meshes: Res<Assets<Mesh>>, images: Res<Assets<Image>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut manifest: ResMut<UploadManifest>,
) {
    let changed = mesh_events.read().count() + image_events.read().count() != 0;
    let document = session.as_ref().map(|session| session.document_id());
    if document == manifest.document && !changed { return; }
    manifest.document = document;
    manifest.generation = manifest.generation.checked_add(1).expect("upload generation exhausted");
    manifest.collected_ms = manifest.started.elapsed().as_secs_f64() * 1000.0;
    manifest.meshes = meshes.iter().filter(|(_, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .map(|(id, _)| id).collect::<Vec<_>>().into();
    manifest.images = images.iter().filter(|(_, image)| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .map(|(id, _)| id).collect::<Vec<_>>().into();
}

fn pending<T: Copy>(ids: &[T], mut prepared: impl FnMut(T) -> bool) -> usize {
    ids.iter().filter(|&&id| !prepared(id)).count()
}

fn report_uploads(
    manifest: Option<Res<UploadManifest>>,
    meshes: Res<RenderAssets<RenderMesh>>, images: Res<RenderAssets<GpuImage>>,
    pipelines: Res<PipelineCache>, mut previous: Local<Option<(u64, usize, usize, usize, usize)>>,
) {
    let Some(manifest) = manifest.filter(|manifest| manifest.document.is_some()) else { return; };
    let missing_meshes = pending(&manifest.meshes, |id| meshes.get(id).is_some());
    let missing_images = pending(&manifest.images, |id| images.get(id).is_some());
    let mut waiting = 0;
    let mut errors = Vec::new();
    for pipeline in pipelines.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => (),
            CachedPipelineState::Err(error) => errors.push(error.to_string()),
            _ => waiting += 1,
        }
    }
    let state = (manifest.generation, missing_meshes, missing_images, waiting, errors.len());
    if previous.as_ref() == Some(&state) { return; }
    *previous = Some(state);
    eprintln!("render_asset_profile {}", serde_json::json!({
        "scope": "all-render-world-meshes-images-and-global-pipelines",
        "clock_origin": "viewer-render-probe-configuration",
        "complete_frame": false,
        "document": manifest.document,
        "generation": manifest.generation,
        "cpu_manifest_elapsed_ms": manifest.collected_ms,
        "post_render_poll_elapsed_ms": manifest.started.elapsed().as_secs_f64()*1000.0,
        "meshes": manifest.meshes.len(), "images": manifest.images.len(),
        "pending_meshes": missing_meshes, "pending_images": missing_images,
        "pending_pipelines": waiting, "pipeline_errors": errors,
        "observed_uploads_ready": missing_meshes == 0 && missing_images == 0 && waiting == 0 && errors.is_empty(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

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
