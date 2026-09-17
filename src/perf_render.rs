use std::{sync::Arc, time::Instant};
use bevy::{asset::{AssetId, UntypedAssetId, RenderAssetUsages, VisitAssetDependencies}, prelude::*};
use bevy::ecs::message::MessageCursor;
use bevy::render::{
    erased_render_asset::ErasedRenderAssets,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    mesh::RenderMesh, render_asset::RenderAssets,
    render_resource::{CachedPipelineState, PipelineCache},
    texture::GpuImage, Render, RenderApp, RenderSystems,
    renderer::RenderQueue,
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
    revisions: Arc<std::collections::HashMap<UntypedAssetId, u64>>,
    material_entities: Arc<std::collections::HashMap<Entity, AssetId<Mesh>>>,
    cpu_only_material_meshes: usize,
    empty_cpu_only_material_meshes: usize,
}

impl Default for UploadManifest {
    fn default() -> Self {
        Self { started: Instant::now(), document: None, generation: 0, collected_ms: 0.0,
            meshes: Arc::from([]), images: Arc::from([]), materials: Arc::from([]), flat_materials: Arc::from([]), flat_assets_present: false,
            revisions: Default::default(), material_entities: Default::default(),
            cpu_only_material_meshes: 0, empty_cpu_only_material_meshes: 0 }
    }
}

impl UploadManifest {
    fn required_ids(&self) -> impl Iterator<Item = UntypedAssetId> + '_ {
        self.meshes.iter().map(|id| id.untyped()).chain(self.images.iter().map(|id| id.untyped()))
            .chain(self.materials.iter().map(|id| id.untyped())).chain(self.flat_materials.iter().map(|id| id.untyped()))
    }
}

#[derive(Resource, Default)]
struct ExtractedRevisions(std::collections::HashMap<UntypedAssetId, u64>);

impl ExtractedRevisions {
    fn observe<A: Asset>(&mut self, manifest: &UploadManifest,
        removed: impl Iterator<Item = AssetId<A>>, extracted: impl Iterator<Item = AssetId<A>>) {
        for id in removed { self.0.remove(&id.untyped()); }
        for id in extracted {
            let id = id.untyped();
            if let Some(revision) = manifest.revisions.get(&id) { self.0.insert(id, *revision); }
            else { self.0.remove(&id); }
        }
    }

    fn pending(&self, manifest: &UploadManifest) -> usize {
        manifest.required_ids().filter(|id| manifest.revisions.get(id)
            .is_none_or(|revision| self.0.get(id) != Some(revision))).count()
    }
}

fn changed_asset<A: Asset>(event: &AssetEvent<A>) -> Option<UntypedAssetId> {
    match event {
        AssetEvent::Added { id } | AssetEvent::Modified { id } | AssetEvent::Removed { id } => Some(id.untyped()),
        _ => None,
    }
}

fn observe_extraction(
    manifest: Option<Res<UploadManifest>>, mut revisions: ResMut<ExtractedRevisions>,
    meshes: Option<Res<bevy::render::render_asset::ExtractedAssets<RenderMesh>>>,
    images: Option<Res<bevy::render::render_asset::ExtractedAssets<GpuImage>>>,
    materials: Option<Res<bevy::render::erased_render_asset::ExtractedAssets<MeshMaterial3d<StandardMaterial>>>>,
    flat: Option<Res<bevy::render::erased_render_asset::ExtractedAssets<MeshMaterial3d<FlatMaterial>>>>,
) {
    let Some(manifest) = manifest else { return };
    macro_rules! observe {
        ($assets:expr) => { if let Some(assets) = $assets {
            revisions.observe(&manifest, assets.removed.iter().copied(), assets.extracted.iter().map(|(id, _)| *id));
        }};
    }
    observe!(meshes); observe!(images); observe!(materials); observe!(flat);
    revisions.0.retain(|id, _| manifest.revisions.contains_key(id));
}

pub fn configure(app: &mut App) {
    if std::env::var_os("USD_PROFILE_RENDER").is_none() { return; }
    if app.get_sub_app(RenderApp).is_none() {
        eprintln!("render_asset_profile unavailable: no render sub-app");
        return;
    }
    app.init_resource::<UploadManifest>().add_plugins(ExtractResourcePlugin::<UploadManifest>::default())
        .add_systems(Last, collect_manifest);
    app.sub_app_mut(RenderApp).init_resource::<ExtractedRevisions>().init_resource::<ViewReadiness>()
        .add_systems(Render, observe_extraction.after(RenderSystems::ExtractCommands).before(RenderSystems::PrepareAssets))
        .add_systems(Render, (report_views, report_uploads).chain().after(RenderSystems::Render));
}

fn collect_manifest(
    session: Option<NonSend<usd_bevy::editor::EditorSession>>,
    meshes: Res<Assets<Mesh>>, images: Res<Assets<Image>>,
    materials: Res<Assets<StandardMaterial>>, flat_materials: Option<Res<Assets<FlatMaterial>>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut material_events: MessageReader<AssetEvent<StandardMaterial>>,
    flat_events: Option<Res<Messages<AssetEvent<FlatMaterial>>>>,
    mut flat_cursor: Local<MessageCursor<AssetEvent<FlatMaterial>>>,
    renderables: Query<(Entity, Option<Ref<Mesh3d>>, Option<Ref<MeshMaterial3d<StandardMaterial>>>, Option<Ref<MeshMaterial3d<FlatMaterial>>>),
        Or<(With<Mesh3d>, With<MeshMaterial3d<StandardMaterial>>, With<MeshMaterial3d<FlatMaterial>>)>>,
    mut removed: (RemovedComponents<Mesh3d>, RemovedComponents<MeshMaterial3d<StandardMaterial>>, RemovedComponents<MeshMaterial3d<FlatMaterial>>),
    mut manifest: ResMut<UploadManifest>,
) {
    let removed = removed.0.read().count() + removed.1.read().count() + removed.2.read().count() != 0;
    let mut revised: std::collections::HashSet<_> = mesh_events.read().filter_map(changed_asset)
        .chain(image_events.read().filter_map(changed_asset)).chain(material_events.read().filter_map(changed_asset)).collect();
    if let Some(events) = flat_events { revised.extend(flat_cursor.read(&events).filter_map(changed_asset)); }
    let changed = !revised.is_empty()
        || flat_materials.as_ref().is_some_and(|materials| materials.is_changed())
        || (flat_materials.is_none() && manifest.flat_assets_present) || removed
        || renderables.iter().any(|(_, mesh, material, flat)| mesh.is_some_and(|value| value.is_changed())
            || material.is_some_and(|value| value.is_changed()) || flat.is_some_and(|value| value.is_changed()));
    let document = session.as_ref().map(|session| session.document_id());
    if document == manifest.document && !changed { return; }
    manifest.document = document;
    manifest.flat_assets_present = flat_materials.is_some();
    manifest.generation = manifest.generation.checked_add(1).expect("upload generation exhausted");
    let generation = manifest.generation;
    for id in revised { Arc::make_mut(&mut manifest.revisions).insert(id, generation); }
    manifest.collected_ms = manifest.started.elapsed().as_secs_f64() * 1000.0;
    let mut required_meshes: std::collections::HashSet<_> = meshes.iter()
        .filter(|(_, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)).map(|(id, _)| id).collect();
    let mut required_materials: std::collections::HashSet<_> = materials.ids().collect();
    let mut required_flat: std::collections::HashSet<_> = flat_materials.as_ref()
        .map_or_else(Default::default, |materials| materials.ids().collect());
    let mut material_entities = std::collections::HashMap::new();
    manifest.cpu_only_material_meshes = 0;
    manifest.empty_cpu_only_material_meshes = 0;
    for (entity, mesh, material, flat) in &renderables {
        if let Some(mesh) = &mesh && (material.is_some() || flat.is_some()) {
            match meshes.get(mesh.id()) {
                Some(asset) if !asset.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) => {
                    manifest.cpu_only_material_meshes += 1;
                    manifest.empty_cpu_only_material_meshes += usize::from(asset.count_vertices() == 0);
                }
                _ => { material_entities.insert(entity, mesh.id()); }
            }
        }
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
    manifest.material_entities = Arc::new(material_entities);
    let required: std::collections::HashSet<_> = manifest.required_ids().collect();
    Arc::make_mut(&mut manifest.revisions).retain(|id, _| required.contains(id));
}

fn pending<T: Copy>(ids: &[T], mut prepared: impl FnMut(T) -> bool) -> usize {
    ids.iter().filter(|&&id| !prepared(id)).count()
}

#[derive(Resource, Default)]
struct ViewReadiness {
    ready: bool,
    previous: Option<(u64, Vec<serde_json::Value>)>,
}

fn expected_view_meshes(manifest: &UploadManifest, visible: Option<&bevy::render::view::visibility::RenderVisibleEntities>)
    -> std::collections::HashSet<bevy::render::sync_world::MainEntity> {
    let mut expected = std::collections::HashSet::new();
    if let Some(class) = visible.and_then(|visible| visible.get::<Mesh3d>()) {
        expected.extend(class.entities_cpu_culling.iter().map(|(_, entity)| *entity));
        expected.extend(class.entities_gpu_culling.keys().copied());
    }
    expected.retain(|entity| manifest.material_entities.contains_key(&entity.id()));
    expected
}

fn report_views(
    manifest: Option<Res<UploadManifest>>, mut readiness: ResMut<ViewReadiness>,
    views: Query<(Entity, &bevy::render::view::ExtractedView, Option<&bevy::render::view::visibility::RenderVisibleEntities>), With<Camera3d>>,
    specializations: Res<bevy::pbr::SpecializedMaterialPipelineCache>,
    pending_queues: Res<bevy::pbr::PendingMeshMaterialQueues>,
    instances: Res<bevy::pbr::RenderMeshInstances>,
    material_instances: Res<bevy::pbr::RenderMaterialInstances>,
    meshes: Res<RenderAssets<RenderMesh>>, materials: Res<ErasedRenderAssets<bevy::pbr::PreparedMaterial>>,
    allocator: Res<bevy::render::mesh::allocator::MeshAllocator>, pipelines: Res<PipelineCache>,
) {
    let Some(manifest) = manifest.filter(|manifest| manifest.document.is_some()) else {
        readiness.ready = false;
        return;
    };
    let mut rows = Vec::new();
    let mut ready = true;
    for (entity, view, visible) in &views {
        let expected = expected_view_meshes(&manifest, visible);
        let specialized = specializations.get(&view.retained_view_entity);
        let queued = pending_queues.get(&view.retained_view_entity);
        let mut missing_pipeline = 0;
        let mut missing_mesh = 0;
        let mut missing_material = 0;
        let mut missing_mesh_sample = Vec::new();
        let pending_queue = queued.map_or(expected.len(), |queue| queue.current_frame.iter()
            .filter(|(_, entity)| expected.contains(entity)).map(|(_, entity)| *entity)
            .collect::<std::collections::HashSet<_>>().len());
        let mut ordered: Vec<_> = expected.iter().copied().collect();
        ordered.sort_by_key(|entity| entity.id().to_bits());
        for entity in &ordered {
            if specialized.and_then(|cache| cache.get(entity))
                .is_none_or(|id| pipelines.get_render_pipeline(*id).is_none()) { missing_pipeline += 1; }
            if instances.render_mesh_queue_data(*entity).is_none_or(|instance| {
                let id = instance.mesh_asset_id();
                meshes.get(id).is_none() || allocator.mesh_slabs(&id).is_none()
            }) {
                missing_mesh += 1;
                if missing_mesh_sample.len() < 8 {
                    let id = manifest.material_entities[&entity.id()];
                    missing_mesh_sample.push(serde_json::json!({
                        "entity": format!("{entity:?}"), "mesh": format!("{id:?}"),
                        "gpu_mesh": meshes.get(id).map(|mesh| format!("{:?} vertices={}", mesh.buffer_info, mesh.vertex_count)),
                    }));
                }
            }
            if material_instances.instances.get(entity)
                .is_none_or(|instance| materials.get(instance.asset_id).is_none()) { missing_material += 1; }
        }
        let view_ready = visible.is_some() && missing_pipeline == 0 && missing_mesh == 0
            && missing_material == 0 && pending_queue == 0;
        ready &= view_ready;
        rows.push((entity.to_bits(), serde_json::json!({
            "view": format!("{:?}", view.retained_view_entity),
            "expected_visible_material_meshes": expected.len(), "missing_visibility_list": visible.is_none(),
            "missing_specialized_pipelines": missing_pipeline, "missing_mesh_data": missing_mesh,
            "missing_material_data": missing_material, "pending_queue_entities": pending_queue,
            "missing_mesh_sample": missing_mesh_sample,
            "queue_prerequisites_ready": view_ready,
        })));
    }
    readiness.ready = ready && !rows.is_empty();
    rows.sort_by_key(|(entity, _)| *entity);
    let rows: Vec<_> = rows.into_iter().map(|(_, row)| row).collect();
    if readiness.previous.as_ref().is_some_and(|(generation, previous)| *generation == manifest.generation && previous == &rows) { return; }
    eprintln!("render_view_profile {}", serde_json::json!({
        "document": manifest.document, "generation": manifest.generation, "complete_frame": false,
        "scope": "camera3d-visible-standard-flat-material-queue-prerequisites",
        "queue_prerequisites_ready": readiness.ready, "draw_submission_verified": false,
        "cpu_only_material_mesh_entities": manifest.cpu_only_material_meshes,
        "empty_cpu_only_material_mesh_entities": manifest.empty_cpu_only_material_meshes,
        "views": rows,
    }));
    readiness.previous = Some((manifest.generation, rows));
}

#[derive(Default)]
struct GpuCompletionProbe {
    pending: Arc<std::sync::atomic::AtomicBool>,
    requested: Option<(u64, u64)>,
}

impl GpuCompletionProbe {
    fn request(&mut self, document: u64, generation: u64) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        use std::sync::atomic::Ordering;
        let revision = (document, generation);
        if self.requested == Some(revision) || self.pending.swap(true, Ordering::AcqRel) { return None; }
        self.requested = Some(revision);
        Some(self.pending.clone())
    }
}

fn report_uploads(
    manifest: Option<Res<UploadManifest>>,
    meshes: Res<RenderAssets<RenderMesh>>, images: Res<RenderAssets<GpuImage>>,
    materials: Res<ErasedRenderAssets<bevy::pbr::PreparedMaterial>>,
    pipelines: Res<PipelineCache>, revisions: Res<ExtractedRevisions>,
    queue: Res<RenderQueue>, mut completion: Local<GpuCompletionProbe>,
    view_readiness: Res<ViewReadiness>,
    mut previous: Local<Option<(u64, usize, usize, usize, usize, usize, usize, bool)>>,
) {
    let Some(manifest) = manifest.filter(|manifest| manifest.document.is_some()) else { return; };
    let missing_meshes = pending(&manifest.meshes, |id| meshes.get(id).is_some());
    let missing_images = pending(&manifest.images, |id| images.get(id).is_some());
    let missing_materials = pending(&manifest.materials, |id| materials.get(id.untyped()).is_some())
        + pending(&manifest.flat_materials, |id| materials.get(id.untyped()).is_some());
    let pending_revisions = revisions.pending(&manifest);
    let mut waiting = 0;
    let mut errors = Vec::new();
    for pipeline in pipelines.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => (),
            CachedPipelineState::Err(error) => errors.push(error.to_string()),
            _ => waiting += 1,
        }
    }
    let ready = missing_meshes == 0 && missing_images == 0 && missing_materials == 0
        && pending_revisions == 0 && waiting == 0 && errors.is_empty();
    if ready && view_readiness.ready && let Some(pending) = completion.request(manifest.document.unwrap(), manifest.generation) {
        let document = manifest.document;
        let generation = manifest.generation;
        let started = manifest.started;
        let registered_ms = started.elapsed().as_secs_f64() * 1000.0;
        queue.on_submitted_work_done(move || {
            let completed_ms = started.elapsed().as_secs_f64() * 1000.0;
            eprintln!("render_gpu_completion_profile {}", serde_json::json!({
                "document": document, "generation": generation, "complete_frame": false,
                "scope": "queue-work-submitted-before-upload-ready-callback-registration",
                "clock_origin": "viewer-render-probe-configuration",
                "callback_registered_elapsed_ms": registered_ms,
                "completion_observed_elapsed_ms": completed_ms,
                "callback_delay_ms": completed_ms - registered_ms,
                "current_revision_verified": false,
            }));
            pending.store(false, std::sync::atomic::Ordering::Release);
        });
    }
    let state = (manifest.generation, missing_meshes, missing_images, missing_materials, waiting, errors.len(), pending_revisions, view_readiness.ready);
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
        "pending_revision_extractions": pending_revisions,
        "revision_scope": "mesh-image-standard-material-flat-material",
        "pending_pipelines": waiting, "pipeline_errors": errors,
        "observed_uploads_ready": ready,
        "view_queue_prerequisites_ready": view_readiness.ready,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_requirements_include_cpu_and_gpu_visibility_without_duplicates() {
        use bevy::render::{sync_world::MainEntity, view::visibility::{RenderVisibleEntities, RenderVisibleEntitiesClass}};
        let mut world = World::new();
        let cpu = world.spawn_empty().id();
        let gpu = world.spawn_empty().id();
        let hidden = world.spawn_empty().id();
        let unsupported = world.spawn_empty().id();
        let mut manifest = UploadManifest::default();
        let assets = Assets::<Mesh>::default();
        let mesh = assets.reserve_handle();
        manifest.material_entities = Arc::new([cpu, gpu, hidden].into_iter().map(|entity| (entity, mesh.id())).collect());
        let mut class = RenderVisibleEntitiesClass::default();
        class.entities_cpu_culling = vec![(cpu, cpu.into()), (unsupported, unsupported.into())];
        class.entities_gpu_culling.insert(cpu.into(), cpu);
        class.entities_gpu_culling.insert(gpu.into(), gpu);
        let mut visible = RenderVisibleEntities::default();
        visible.classes.insert(std::any::TypeId::of::<Mesh3d>(), class);
        assert_eq!(expected_view_meshes(&manifest, Some(&visible)),
            [MainEntity::from(cpu), MainEntity::from(gpu)].into_iter().collect());
        assert!(expected_view_meshes(&manifest, None).is_empty());
        assert!(expected_view_meshes(&manifest, Some(&RenderVisibleEntities::default())).is_empty());
        Arc::make_mut(&mut manifest.material_entities).remove(&gpu);
        assert_eq!(expected_view_meshes(&manifest, Some(&visible)), [cpu.into()].into_iter().collect());
    }

    #[test]
    fn gpu_completion_requests_are_bounded_and_retry_latest_revision() {
        use std::sync::atomic::Ordering;
        let mut probe = GpuCompletionProbe::default();
        let first = probe.request(1, 2).unwrap();
        assert!(probe.request(1, 2).is_none());
        assert!(probe.request(1, 3).is_none());
        assert!(probe.request(2, 1).is_none());
        first.store(false, Ordering::Release);
        assert!(probe.request(1, 2).is_none());
        let latest = probe.request(2, 1).unwrap();
        assert!(Arc::ptr_eq(&first, &latest));
        assert!(probe.request(2, 1).is_none());
        latest.store(false, Ordering::Release);
        assert!(probe.request(2, 2).is_some());
    }

    #[test]
    fn extraction_observer_runs_after_manifest_commands_before_asset_drain() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<ExtractedRevisions>();
        let mesh = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let id = mesh.id();
        app.add_systems(Update, (move |mut commands: Commands| {
            let mut manifest = UploadManifest::default();
            manifest.meshes = Arc::from([id]);
            Arc::make_mut(&mut manifest.revisions).insert(id.untyped(), 7);
            commands.insert_resource(manifest);
            let mut extracted = bevy::render::render_asset::ExtractedAssets::<RenderMesh>::default();
            extracted.extracted.push((id, Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList,
                RenderAssetUsages::default())));
            commands.insert_resource(extracted);
        }).in_set(RenderSystems::ExtractCommands));
        app.configure_sets(Update, (RenderSystems::ExtractCommands, RenderSystems::PrepareAssets).chain());
        app.add_systems(Update, observe_extraction.after(RenderSystems::ExtractCommands).before(RenderSystems::PrepareAssets));
        app.add_systems(Update, (|manifest: Res<UploadManifest>, witness: Res<ExtractedRevisions>,
            mut extracted: ResMut<bevy::render::render_asset::ExtractedAssets<RenderMesh>>| {
            assert_eq!(witness.pending(&manifest), 0);
            assert_eq!(extracted.extracted.len(), 1);
            extracted.extracted.clear();
        }).in_set(RenderSystems::PrepareAssets));
        app.update();
    }

    #[test]
    fn extraction_witness_tracks_replacements_without_invalidating_other_assets() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>().init_resource::<Assets<FlatMaterial>>()
            .init_resource::<UploadManifest>()
            .add_message::<AssetEvent<Mesh>>().add_message::<AssetEvent<Image>>()
            .add_message::<AssetEvent<StandardMaterial>>().add_message::<AssetEvent<FlatMaterial>>()
            .add_systems(Update, collect_manifest);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default()));
        app.world_mut().write_message(AssetEvent::<Mesh>::Added { id: mesh.id() });
        app.update();
        let mut witness = ExtractedRevisions::default();
        let manifest = app.world().resource::<UploadManifest>();
        assert_eq!(witness.pending(manifest), 1);
        witness.observe(manifest, std::iter::empty(), std::iter::once(mesh.id()));
        assert_eq!(witness.pending(manifest), 0);
        let mesh_revision = manifest.revisions[&mesh.id().untyped()];

        let flat = app.world_mut().resource_mut::<Assets<FlatMaterial>>().add(FlatMaterial::default());
        app.world_mut().write_message(AssetEvent::<FlatMaterial>::Added { id: flat.id() });
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert_eq!(manifest.revisions[&mesh.id().untyped()], mesh_revision);
        assert_eq!(witness.pending(manifest), 1);
        witness.observe(manifest, std::iter::empty(), std::iter::once(flat.id()));
        assert_eq!(witness.pending(manifest), 0);

        app.world_mut().write_message(AssetEvent::<Mesh>::Modified { id: mesh.id() });
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert!(manifest.revisions[&mesh.id().untyped()] > mesh_revision);
        assert_eq!(witness.pending(manifest), 1);
        witness.observe(manifest, std::iter::empty(), std::iter::once(mesh.id()));
        assert_eq!(witness.pending(manifest), 0);
        assert_eq!(pending(&manifest.meshes, |_| false), 1);

        app.world_mut().spawn(Mesh3d(mesh.clone()));
        app.world_mut().resource_mut::<Assets<Mesh>>().remove(mesh.id());
        app.world_mut().write_message(AssetEvent::<Mesh>::Removed { id: mesh.id() });
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        witness.observe(manifest, std::iter::once(mesh.id()), std::iter::empty());
        assert_eq!(witness.pending(manifest), 1);
        assert!(!witness.0.contains_key(&mesh.id().untyped()));

        let missing = app.world().resource::<Assets<Image>>().reserve_handle();
        let mut unknown = manifest.clone();
        unknown.images = Arc::from([missing.id()]);
        witness.observe(&unknown, std::iter::empty(), std::iter::once(missing.id()));
        assert_eq!(witness.pending(&unknown), 2);
        assert!(!witness.0.contains_key(&missing.id().untyped()));
    }

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
        let cpu_entity = app.world_mut().spawn((Mesh3d(cpu_only), MeshMaterial3d(material.clone()))).id();
        let entity = app.world_mut().spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), MeshMaterial3d(flat.clone()))).id();
        app.world_mut().remove_resource::<Assets<FlatMaterial>>();
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert_eq!(manifest.meshes.as_ref(), &[mesh.id()]);
        assert_eq!(manifest.material_entities.as_ref(), &[(entity, mesh.id())].into_iter().collect());
        assert_eq!(manifest.cpu_only_material_meshes, 1);
        assert_eq!(manifest.empty_cpu_only_material_meshes, 1);
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
        app.world_mut().despawn(cpu_entity);
        app.update();
        let manifest = app.world().resource::<UploadManifest>();
        assert!(manifest.meshes.is_empty() && manifest.materials.is_empty() && manifest.flat_materials.is_empty());
        assert!(manifest.material_entities.is_empty());
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
