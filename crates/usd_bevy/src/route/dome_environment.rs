//! Explicit camera selection and ownership for runtime-filtered dome lighting.

use bevy::prelude::*;
use bevy::light::{EnvironmentMapLight, GeneratedEnvironmentMapLight};
use super::dome::{UsdDomeLight, UsdDomeTexture};
use bevy::pbr::generate::{GeneratorBindGroups, GeneratorPipelines, IntermediateTextures,
    RenderEnvironmentMap, downsampling_system, filtering_system, extract_generated_environment_map_entities};
use bevy::render::{Render, RenderApp, RenderSystems, ExtractSchedule,
    extract_component::{ExtractComponent, ExtractComponentPlugin}, render_resource::PipelineCache, sync_world::MainEntity};

/// Selects a projected dome for this camera. Does not change ambient or skybox.
#[derive(Component, Clone, Copy)]
pub struct UsdDomeEnvironmentSource {
    pub dome: Entity,
    pub face_size: u32,
}

impl UsdDomeEnvironmentSource {
    pub fn new(dome: Entity) -> Self { Self { dome, face_size: 128 } }
}

/// Attachment state, not confirmation that GPU convolution has completed.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub enum UsdDomeEnvironmentState {
    WaitingForMaps,
    Attached,
    Unavailable(String),
}

#[derive(Clone, PartialEq)]
struct Key { image: bevy::asset::AssetId<Image>, tint: [f32; 3], size: u32 }

#[derive(Component, Clone)]
struct OwnedEnvironment {
    key: Key,
    baker: Entity,
    previous: Option<EnvironmentMapLight>,
    applied: Option<EnvironmentMapLight>,
}

#[derive(Resource, Default)]
struct ImageChanges(std::collections::HashSet<bevy::asset::AssetId<Image>>);

#[derive(Resource)]
struct GenerationUnavailable(String);

fn generation_limit_error(storage_textures: u32, workgroup_storage: u32, workgroup_x: u32, compute: bool) -> Option<String> {
    (storage_textures < 6 || workgroup_storage == 0 || workgroup_x == 0 || !compute).then(||
        format!("GPU device cannot filter dome maps: requires 6 storage textures per shader stage and compute support; device provides {storage_textures} storage textures, workgroup storage {workgroup_storage}, workgroup X {workgroup_x}, compute={compute}"))
}

#[derive(Component, Clone, Copy, ExtractComponent)]
struct OwnedGenerator;

#[derive(Component)]
struct FilteringRecorded;

#[derive(Resource, Clone, Default)]
struct FilterFeedback(std::sync::Arc<std::sync::Mutex<Vec<Entity>>>);

/// Number of owned generations whose filtering commands were recorded.
/// This is not GPU timing or a GPU completion fence.
#[derive(Resource, Default, Debug)]
pub struct UsdDomeEnvironmentDiagnostics {
    pub recorded_generations: u64,
}

/// Attaches selected dome maps after transforms and visibility are propagated.
/// Requires Bevy's PBR runtime environment-map generation support.
pub struct UsdDomeEnvironmentPlugin;

impl Plugin for UsdDomeEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        let feedback = FilterFeedback::default();
        app.insert_resource(feedback.clone()).init_resource::<UsdDomeEnvironmentDiagnostics>()
            .add_plugins(ExtractComponentPlugin::<OwnedGenerator>::default());
        app.init_resource::<ImageChanges>().add_systems(PostUpdate, (image_changes, receive_filtering, update).chain()
            .after(bevy::transform::TransformSystems::Propagate)
            .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate));
    }

    fn finish(&self, app: &mut App) {
        let feedback = app.world().resource::<FilterFeedback>().clone();
        let unavailable = app.get_sub_app(RenderApp).and_then(|render_app| {
            let device = render_app.world().get_resource::<bevy::render::renderer::RenderDevice>()?;
            let adapter = render_app.world().get_resource::<bevy::render::renderer::RenderAdapter>()?;
            let limits = device.limits();
            generation_limit_error(limits.max_storage_textures_per_shader_stage,
                limits.max_compute_workgroup_storage_size, limits.max_compute_workgroup_size_x,
                adapter.get_downlevel_capabilities().flags.contains(bevy::render::render_resource::DownlevelFlags::COMPUTE_SHADERS))
        });
        if let Some(error) = unavailable { app.insert_resource(GenerationUnavailable(error)); }
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(feedback)
                .add_systems(ExtractSchedule, suppress_recorded.after(extract_generated_environment_map_entities))
                .add_systems(Render, record_filtering.after(downsampling_system).after(filtering_system)
                    .before(RenderSystems::Render));
        }
    }
}

fn suppress_recorded(mut commands: Commands, recorded: Query<Entity, With<FilteringRecorded>>) {
    for entity in &recorded { commands.entity(entity).remove::<RenderEnvironmentMap>(); }
}

fn record_filtering(mut commands: Commands, cache: Res<PipelineCache>, pipelines: Option<Res<GeneratorPipelines>>,
    generators: Query<(Entity, MainEntity), (With<OwnedGenerator>, With<GeneratorBindGroups>, With<RenderEnvironmentMap>, Without<FilteringRecorded>)>,
    feedback: Res<FilterFeedback>) {
    let Some(pipelines) = pipelines else { return };
    if [pipelines.downsample_first, pipelines.downsample_second, pipelines.copy, pipelines.radiance, pipelines.irradiance]
        .into_iter().any(|pipeline| cache.get_compute_pipeline(pipeline).is_none()) { return; }
    let mut feedback = feedback.0.lock().unwrap();
    for (entity, main) in &generators {
        commands.entity(entity).insert(FilteringRecorded)
            .remove::<(RenderEnvironmentMap, GeneratorBindGroups, IntermediateTextures)>();
        feedback.push(main);
    }
}

fn receive_filtering(world: &mut World) {
    let completed = std::mem::take(&mut *world.resource::<FilterFeedback>().0.lock().unwrap());
    for entity in completed {
        if world.get::<OwnedGenerator>(entity).is_some() && world.get::<GeneratedEnvironmentMapLight>(entity).is_some() {
            world.entity_mut(entity).remove::<GeneratedEnvironmentMapLight>();
            world.resource_mut::<UsdDomeEnvironmentDiagnostics>().recorded_generations += 1;
        }
    }
}

fn image_changes(mut events: MessageReader<bevy::asset::AssetEvent<Image>>, mut changes: ResMut<ImageChanges>) {
    changes.0.clear();
    for event in events.read() {
        if let bevy::asset::AssetEvent::Modified { id } | bevy::asset::AssetEvent::Removed { id } = event {
            changes.0.insert(*id);
        }
    }
}

fn same(a: &EnvironmentMapLight, b: &EnvironmentMapLight) -> bool {
    a.diffuse_map == b.diffuse_map && a.specular_map == b.specular_map && a.intensity == b.intensity
        && a.rotation == b.rotation && a.affects_lightmapped_mesh_diffuse == b.affects_lightmapped_mesh_diffuse
}

fn clear(world: &mut World, camera: Entity, owned: OwnedEnvironment) {
    world.despawn(owned.baker);
    let unchanged = owned.applied.as_ref().is_some_and(|applied|
        world.get::<EnvironmentMapLight>(camera).is_some_and(|current| same(current, applied)));
    let mut entity = world.entity_mut(camera);
    entity.remove::<OwnedEnvironment>();
    if unchanged {
        if let Some(previous) = owned.previous { entity.insert(previous); }
        else { entity.remove::<EnvironmentMapLight>(); }
    }
}

fn selected(world: &World, camera: Entity, source: UsdDomeEnvironmentSource) -> anyhow::Result<(Key, f32, Quat)> {
    if let Some(error) = world.get_resource::<GenerationUnavailable>() { anyhow::bail!("{}", error.0); }
    anyhow::ensure!(world.get::<Camera>(camera).is_some(), "dome environment selection requires a camera");
    anyhow::ensure!(world.get::<GeneratedEnvironmentMapLight>(camera).is_none(), "camera already has an application-owned environment generator");
    let dome = world.get::<UsdDomeLight>(source.dome).ok_or_else(|| anyhow::anyhow!("selected dome is missing"))?;
    let visible = world.get::<InheritedVisibility>(source.dome).map_or_else(
        || world.get::<Visibility>(source.dome) != Some(&Visibility::Hidden), |v| v.get());
    anyhow::ensure!(visible, "selected dome is hidden");
    anyhow::ensure!(matches!(dome.format.as_str(), "latlong" | "automatic"), "unsupported dome texture format: {}", dome.format);
    anyhow::ensure!(dome.intensity.is_finite() && dome.intensity >= 0.0, "invalid dome intensity");
    let texture = world.get::<UsdDomeTexture>(source.dome).ok_or_else(|| anyhow::anyhow!("dome texture is not loaded"))?;
    let rotation = world.get::<GlobalTransform>(source.dome).map_or(Quat::IDENTITY, |t| t.to_scale_rotation_translation().1)
        * world.get::<super::dome::UsdDomePoleRotation>(source.dome).map_or(Quat::IDENTITY, |pole| pole.0);
    anyhow::ensure!(rotation.is_finite() && rotation.length_squared() > 1e-8, "invalid dome transform");
    Ok((Key { image: texture.0.id(), tint: dome.color, size: source.face_size }, dome.intensity, rotation.normalize()))
}

fn update(world: &mut World) {
    let cameras: Vec<_> = world.query::<(Entity, Option<&UsdDomeEnvironmentSource>, Option<&OwnedEnvironment>)>()
        .iter(world).filter(|(_, source, owned)| source.is_some() || owned.is_some())
        .map(|(e, s, o)| (e, s.copied(), o.cloned())).collect();
    for (camera, source, mut owned) in cameras {
        let Some(source) = source else {
            if let Some(owned) = owned { clear(world, camera, owned); }
            world.entity_mut(camera).remove::<UsdDomeEnvironmentState>();
            continue;
        };
        let result = selected(world, camera, source).and_then(|(key, intensity, rotation)| {
            let rebuild = owned.as_ref().is_none_or(|state| state.key != key || world.get_entity(state.baker).is_err())
                || world.resource::<ImageChanges>().0.contains(&key.image);
            if rebuild {
                let image = world.resource::<Assets<Image>>().get(key.image)
                    .ok_or_else(|| anyhow::anyhow!("dome image is unavailable"))?;
                let cube = super::environment_map::latlong_cubemap(image, key.size, key.tint)?;
                if let Some(old) = owned.take() { clear(world, camera, old); }
                let previous = world.get::<EnvironmentMapLight>(camera).cloned();
                let cube = world.resource_mut::<Assets<Image>>().add(cube);
                let baker = world.spawn((ChildOf(camera), OwnedGenerator, GeneratedEnvironmentMapLight {
                    environment_map: cube, intensity, rotation, ..default()
                })).id();
                owned = Some(OwnedEnvironment { key, baker, previous, applied: None });
            }
            let state = owned.as_mut().unwrap();
            let maps = world.get::<EnvironmentMapLight>(state.baker).cloned();
            if let Some(mut maps) = maps {
                maps.intensity = intensity;
                maps.rotation = rotation;
                state.applied = Some(maps.clone());
                world.entity_mut(camera).insert(maps);
                Ok(UsdDomeEnvironmentState::Attached)
            } else { Ok(UsdDomeEnvironmentState::WaitingForMaps) }
        });
        match result {
            Ok(status) => { world.entity_mut(camera).insert((owned.unwrap(), status)); }
            Err(error) => {
                if let Some(owned) = owned { clear(world, camera, owned); }
                world.entity_mut(camera).insert(UsdDomeEnvironmentState::Unavailable(error.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    #[test]
    fn unsupported_device_limits_report_unavailable_instead_of_waiting() {
        assert!(generation_limit_error(6, 16384, 256, true).is_none());
        for (textures, storage, x, compute) in [(4, 16384, 256, true), (6, 0, 256, true),
            (6, 16384, 0, true), (6, 16384, 256, false)] {
            let error = generation_limit_error(textures, storage, x, compute).unwrap();
            let (mut world, camera, _) = fixture();
            world.insert_resource(GenerationUnavailable(error.clone()));
            update(&mut world);
            assert_eq!(world.get::<UsdDomeEnvironmentState>(camera), Some(&UsdDomeEnvironmentState::Unavailable(error)));
            assert_eq!(world.query::<&GeneratedEnvironmentMapLight>().iter(&world).count(), 0);
        }
    }

    fn fixture() -> (World, Entity, Entity) {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<ImageChanges>();
        let image = Image::new_fill(Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, &[255; 4], TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::all());
        let image = world.resource_mut::<Assets<Image>>().add(image);
        let dome = world.spawn((UsdDomeLight { format: "latlong".into(), color: [1.0; 3], intensity: 4.0, ..default() },
            UsdDomeTexture(image), GlobalTransform::from_rotation(Quat::from_rotation_y(0.5)))).id();
        let camera = world.spawn((Camera::default(), UsdDomeEnvironmentSource { dome, face_size: 16 })).id();
        (world, camera, dome)
    }

    fn finish_maps(world: &mut World, camera: Entity) -> Entity {
        let baker = world.get::<OwnedEnvironment>(camera).unwrap().baker;
        world.entity_mut(baker).insert(EnvironmentMapLight::default());
        update(world);
        assert_eq!(world.get::<UsdDomeEnvironmentState>(camera), Some(&UsdDomeEnvironmentState::Attached));
        baker
    }

    #[test]
    fn feedback_retires_only_owned_generators_and_preserves_maps() {
        let (mut world, camera, dome) = fixture();
        world.init_resource::<FilterFeedback>();
        world.init_resource::<UsdDomeEnvironmentDiagnostics>();
        update(&mut world);
        let baker = finish_maps(&mut world, camera);
        let maps = world.get::<EnvironmentMapLight>(camera).unwrap().clone();
        let external = world.spawn(GeneratedEnvironmentMapLight::default()).id();
        world.resource::<FilterFeedback>().0.lock().unwrap().extend([baker, baker, external]);
        receive_filtering(&mut world);
        assert!(world.get::<GeneratedEnvironmentMapLight>(baker).is_none());
        assert!(world.get::<GeneratedEnvironmentMapLight>(external).is_some());
        assert_eq!(world.resource::<UsdDomeEnvironmentDiagnostics>().recorded_generations, 1);
        update(&mut world);
        assert!(same(world.get::<EnvironmentMapLight>(camera).unwrap(), &maps));
        assert_eq!(world.get::<OwnedEnvironment>(camera).unwrap().baker, baker);
        let source = world.get::<UsdDomeTexture>(dome).unwrap().0.id();
        world.resource_mut::<ImageChanges>().0.insert(source);
        update(&mut world);
        let replacement = world.get::<OwnedEnvironment>(camera).unwrap().baker;
        assert_ne!(replacement, baker);
        assert!(world.get::<GeneratedEnvironmentMapLight>(replacement).is_some());
        world.resource::<FilterFeedback>().0.lock().unwrap().extend([baker, replacement]);
        receive_filtering(&mut world);
        assert_eq!(world.resource::<UsdDomeEnvironmentDiagnostics>().recorded_generations, 2);
    }

    #[test]
    fn camera_rotations_and_intensities_are_independent_without_rebaking() {
        let (mut world, first, dome) = fixture();
        let texture = world.get::<UsdDomeTexture>(dome).unwrap().clone();
        let other_dome = world.spawn((texture, UsdDomeLight {
            format: "latlong".into(), color: [1.0; 3], intensity: 12.0, ..default()
        }, GlobalTransform::from_rotation(Quat::from_rotation_y(-0.5)))).id();
        let second = world.spawn((Camera::default(), UsdDomeEnvironmentSource { dome: other_dome, face_size: 16 })).id();
        update(&mut world);
        let first_baker = finish_maps(&mut world, first);
        let second_baker = finish_maps(&mut world, second);
        let previous = world.get::<EnvironmentMapLight>(second).unwrap().clone();
        world.entity_mut(dome).insert(GlobalTransform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)));
        world.get_mut::<UsdDomeLight>(dome).unwrap().intensity = 0.0;
        update(&mut world);
        let current = world.get::<EnvironmentMapLight>(first).unwrap();
        assert!((current.rotation * Vec3::X).abs_diff_eq(-Vec3::X, 1e-6));
        assert_eq!(current.intensity, 0.0);
        assert!(same(world.get::<EnvironmentMapLight>(second).unwrap(), &previous));
        assert_eq!(world.get::<OwnedEnvironment>(first).unwrap().baker, first_baker);
        assert_eq!(world.get::<OwnedEnvironment>(second).unwrap().baker, second_baker);
    }

    #[test]
    fn camera_selection_updates_without_rebaking_and_restores() {
        let (mut world, camera, dome) = fixture();
        let previous = EnvironmentMapLight { intensity: 17.0, ..default() };
        world.entity_mut(camera).insert(previous.clone());
        update(&mut world);
        assert_eq!(world.get::<UsdDomeEnvironmentState>(camera), Some(&UsdDomeEnvironmentState::WaitingForMaps));
        assert!(same(world.get::<EnvironmentMapLight>(camera).unwrap(), &previous));
        let baker = finish_maps(&mut world, camera);
        let env = world.get::<EnvironmentMapLight>(camera).unwrap();
        assert_eq!(env.intensity, 4.0);
        assert!(env.rotation.abs_diff_eq(Quat::from_rotation_y(0.5), 1e-6));
        world.get_mut::<UsdDomeLight>(dome).unwrap().intensity = 8.0;
        update(&mut world);
        assert_eq!(world.get::<OwnedEnvironment>(camera).unwrap().baker, baker);
        assert_eq!(world.get::<EnvironmentMapLight>(camera).unwrap().intensity, 8.0);
        world.entity_mut(camera).remove::<UsdDomeEnvironmentSource>();
        update(&mut world);
        assert!(world.get_entity(baker).is_err());
        assert!(same(world.get::<EnvironmentMapLight>(camera).unwrap(), &previous));
        assert!(world.get::<UsdDomeEnvironmentState>(camera).is_none());
    }

    #[test]
    fn hidden_invalid_and_external_replacements_are_safe() {
        let (mut world, camera, dome) = fixture();
        update(&mut world);
        let baker = finish_maps(&mut world, camera);
        world.entity_mut(camera).insert(EnvironmentMapLight { intensity: 91.0, ..default() });
        world.entity_mut(dome).insert(Visibility::Hidden);
        update(&mut world);
        assert!(world.get_entity(baker).is_err());
        assert_eq!(world.get::<EnvironmentMapLight>(camera).unwrap().intensity, 91.0);
        world.entity_mut(dome).insert((Visibility::Visible, InheritedVisibility::VISIBLE));
        world.get_mut::<UsdDomeLight>(dome).unwrap().format = "angular".into();
        update(&mut world);
        assert!(matches!(world.get::<UsdDomeEnvironmentState>(camera), Some(UsdDomeEnvironmentState::Unavailable(_))));
        assert!(world.get::<OwnedEnvironment>(camera).is_none());
        world.get_mut::<UsdDomeLight>(dome).unwrap().format = "latlong".into();
        update(&mut world);
        finish_maps(&mut world, camera);
        world.despawn(dome);
        update(&mut world);
        assert_eq!(world.get::<EnvironmentMapLight>(camera).unwrap().intensity, 91.0);
    }

    #[test]
    fn image_changes_rebuild_and_camera_despawn_owns_baker() {
        let (mut world, camera, dome) = fixture();
        update(&mut world);
        let baker = finish_maps(&mut world, camera);
        let image = world.get::<UsdDomeTexture>(dome).unwrap().0.id();
        world.resource_mut::<ImageChanges>().0.insert(image);
        update(&mut world);
        let replacement = world.get::<OwnedEnvironment>(camera).unwrap().baker;
        assert_ne!(baker, replacement);
        assert!(world.get_entity(baker).is_err());
        world.despawn(camera);
        assert!(world.get_entity(replacement).is_err());
    }
}
