use std::{sync::Arc, time::Instant};
use bevy::{asset::{AssetId, RenderAssetUsages, VisitAssetDependencies}, prelude::*};
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
    flat_assets_present: bool,
}

impl Default for UploadManifest {
    fn default() -> Self {
        Self { started: Instant::now(), document: None, generation: 0, collected_ms: 0.0,
            meshes: Arc::from([]), images: Arc::from([]), materials: Arc::from([]), flat_materials: Arc::from([]), flat_assets_present: false }
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
    renderables: Query<(Option<Ref<Mesh3d>>, Option<Ref<MeshMaterial3d<StandardMaterial>>>, Option<Ref<MeshMaterial3d<FlatMaterial>>>),
        Or<(With<Mesh3d>, With<MeshMaterial3d<StandardMaterial>>, With<MeshMaterial3d<FlatMaterial>>)>>,
    mut removed: (RemovedComponents<Mesh3d>, RemovedComponents<MeshMaterial3d<StandardMaterial>>, RemovedComponents<MeshMaterial3d<FlatMaterial>>),
    mut manifest: ResMut<UploadManifest>,
) {
    let removed = removed.0.read().count() + removed.1.read().count() + removed.2.read().count() != 0;
    let changed = mesh_events.read().count() + image_events.read().count() + material_events.read().count() != 0
        || flat_materials.as_ref().is_some_and(|materials| materials.is_changed())
        || (flat_materials.is_none() && manifest.flat_assets_present) || removed
        || renderables.iter().any(|(mesh, material, flat)| mesh.is_some_and(|value| value.is_changed())
            || material.is_some_and(|value| value.is_changed()) || flat.is_some_and(|value| value.is_changed()));
    let document = session.as_ref().map(|session| session.document_id());
    if document == manifest.document && !changed { return; }
    manifest.document = document;
    manifest.flat_assets_present = flat_materials.is_some();
    manifest.generation = manifest.generation.checked_add(1).expect("upload generation exhausted");
    manifest.collected_ms = manifest.started.elapsed().as_secs_f64() * 1000.0;
    let mut required_meshes: std::collections::HashSet<_> = meshes.iter()
        .filter(|(_, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)).map(|(id, _)| id).collect();
    let mut required_materials: std::collections::HashSet<_> = materials.ids().collect();
    let mut required_flat: std::collections::HashSet<_> = flat_materials.as_ref()
        .map_or_else(Default::default, |materials| materials.ids().collect());
    for (mesh, material, flat) in &renderables {
        if let Some(mesh) = mesh && meshes.get(mesh.id()).is_none_or(|asset| asset.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)) {
            required_meshes.insert(mesh.id());
        }
        if let Some(material) = material { required_materials.insert(material.id()); }
        if let Some(flat) = flat { required_flat.insert(flat.id()); }
    }
    manifest.meshes = required_meshes.into_iter().collect::<Vec<_>>().into();
    let mut required_images: std::collections::HashSet<_> = images.iter()
        .filter(|(_, image)| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)).map(|(id, _)| id).collect();
    let mut visit_image = |id| {
        if let Ok(id) = AssetId::<Image>::try_from(id) { required_images.insert(id); }
    };
    for id in &required_materials {
        if let Some(material) = materials.get(*id) { material.visit_dependencies(&mut visit_image); }
    }
    for id in &required_flat {
        if let Some(material) = flat_materials.as_ref().and_then(|materials| materials.get(*id)) {
            material.base.visit_dependencies(&mut visit_image);
        }
    }
    manifest.images = required_images.into_iter().collect::<Vec<_>>().into();
    manifest.materials = required_materials.into_iter().collect::<Vec<_>>().into();
    manifest.flat_materials = required_flat.into_iter().collect::<Vec<_>>().into();
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
        "scope": "all-render-world-meshes-images-retained-and-entity-referenced-standard-flat-materials-meshes-and-global-pipelines",
        "clock_origin": "viewer-render-probe-configuration",
        "complete_frame": false,
        "document": manifest.document,
        "generation": manifest.generation,
        "cpu_manifest_elapsed_ms": manifest.collected_ms,
        "post_render_poll_elapsed_ms": manifest.started.elapsed().as_secs_f64()*1000.0,
        "meshes": manifest.meshes.len(), "images": manifest.images.len(),
        "standard_materials": manifest.materials.len(), "flat_materials": manifest.flat_materials.len(),
        "pending_meshes": missing_meshes, "pending_images": missing_images,
        "pending_mesh_id_sample": manifest.meshes.iter().filter(|id| meshes.get(**id).is_none()).take(8)
            .map(|id| format!("{id:?}")).collect::<Vec<_>>(),
        "pending_prepared_materials": missing_materials,
        "pending_pipelines": waiting, "pipeline_errors": errors,
        "observed_uploads_ready": missing_meshes == 0 && missing_images == 0 && missing_materials == 0 && waiting == 0 && errors.is_empty(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_dependencies_track_missing_replaced_and_flat_textures() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>().init_resource::<Assets<FlatMaterial>>()
            .init_resource::<UploadManifest>()
            .add_message::<AssetEvent<Mesh>>().add_message::<AssetEvent<Image>>()
            .add_message::<AssetEvent<StandardMaterial>>().add_systems(Update, collect_manifest);
        let first = app.world().resource::<Assets<Image>>().reserve_handle();
        let second = app.world().resource::<Assets<Image>>().reserve_handle();
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color_texture: Some(first.clone()), normal_map_texture: Some(first.clone()), ..default()
        });
        app.world_mut().write_message(AssetEvent::<StandardMaterial>::Added { id: material.id() });
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().images.as_ref(), &[first.id()]);
        assert_eq!(pending(&app.world().resource::<UploadManifest>().images, |_| false), 1);
        let generation = app.world().resource::<UploadManifest>().generation;
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation);
        {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            let mut material = materials.get_mut(&material).unwrap();
            material.base_color_texture = Some(second.clone());
            material.normal_map_texture = None;
        }
        app.world_mut().write_message(AssetEvent::<StandardMaterial>::Modified { id: material.id() });
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().images.as_ref(), &[second.id()]);
        let flat = app.world_mut().resource_mut::<Assets<FlatMaterial>>().add(FlatMaterial {
            base: StandardMaterial { emissive_texture: Some(first.clone()), ..default() }, ..default()
        });
        app.update();
        let required = &app.world().resource::<UploadManifest>().images;
        assert_eq!(required.len(), 2);
        assert!(required.contains(&first.id()) && required.contains(&second.id()));
        app.world_mut().resource_mut::<Assets<FlatMaterial>>().remove(flat.id());
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().images.as_ref(), &[second.id()]);
    }

    #[test]
    fn referenced_unloaded_assets_remain_required_until_handles_change() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>().init_resource::<Assets<FlatMaterial>>()
            .init_resource::<UploadManifest>()
            .add_message::<AssetEvent<Mesh>>().add_message::<AssetEvent<Image>>()
            .add_message::<AssetEvent<StandardMaterial>>().add_systems(Update, collect_manifest);
        let mesh = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let next_mesh = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let material = app.world().resource::<Assets<StandardMaterial>>().reserve_handle();
        let flat = app.world().resource::<Assets<FlatMaterial>>().reserve_handle();
        let cpu_only = app.world_mut().resource_mut::<Assets<Mesh>>().add(Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD));
        app.world_mut().spawn(Mesh3d(cpu_only));
        let entity = app.world_mut().spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), MeshMaterial3d(flat.clone()))).id();
        app.world_mut().remove_resource::<Assets<FlatMaterial>>();
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert_eq!(manifest.meshes.as_ref(), &[mesh.id()]);
        assert_eq!(manifest.materials.as_ref(), &[material.id()]);
        assert_eq!(manifest.flat_materials.as_ref(), &[flat.id()]);
        assert_eq!(pending(&manifest.meshes, |_| false), 1);
        let generation = manifest.generation;
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation);
        app.world_mut().entity_mut(entity).insert(Mesh3d(next_mesh.clone()));
        app.update();
        assert_eq!(app.world().resource::<UploadManifest>().meshes.as_ref(), &[next_mesh.id()]);
        assert_eq!(app.world().resource::<UploadManifest>().generation, generation + 1);
        app.world_mut().despawn(entity);
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert!(manifest.meshes.is_empty() && manifest.materials.is_empty() && manifest.flat_materials.is_empty());
        assert_eq!(manifest.generation, generation + 2);
    }

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
