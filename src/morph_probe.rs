use bevy::{
    ecs::system::SystemParam,
    mesh::{
        morph::{MeshMorphWeights, MorphWeights},
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    pbr::{
        ExtractedDirectionalLight, MeshInputUniform, MeshUniform, MorphIndices, MorphUniforms,
        RenderMeshInstances, SkinUniforms, collect_meshes_for_gpu_building, prepare_lights,
        prepare_morph_descriptors,
    },
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        batching::gpu_preprocessing::BatchedInstanceBuffers,
        mesh::{RenderMesh, allocator::MeshAllocator},
        render_asset::RenderAssets,
        sync_world::{MainEntity, MainEntityHashMap},
    },
};
use std::{
    fmt::{self, Write},
    sync::{Arc, Mutex},
};

const TEXT_LIMIT: usize = 64 * 1024;
const HISTORY_LIMIT: usize = 2 * 1024 * 1024;
const PHASES: [&str; 4] = [
    "before-descriptors",
    "after-descriptors",
    "after-collection",
    "after-lights",
];

#[derive(Default)]
struct Text {
    value: String,
    truncated: bool,
}

impl Write for Text {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let mut len = value.len().min(TEXT_LIMIT - self.value.len());
        while !value.is_char_boundary(len) {
            len -= 1;
        }
        self.value.push_str(&value[..len]);
        self.truncated |= len != value.len();
        Ok(())
    }
}

#[derive(Default)]
struct Snapshot {
    latest: [String; 4],
    frames: u64,
    events: usize,
    omitted: usize,
    history: String,
}

impl Snapshot {
    fn record(&mut self, frame: u64, phase: usize, text: Text) {
        self.frames = frame;
        let value = format!(
            "{}morph_probe_text_truncated={}\n",
            text.value, text.truncated
        );
        if self.latest[phase] == value {
            return;
        }
        let event = format!(
            "morph_probe_event frame={frame} phase={}\n{value}",
            PHASES[phase]
        );
        if self.history.len() + event.len() <= HISTORY_LIMIT {
            self.history.push_str(&event);
            self.events += 1;
        } else {
            self.omitted += 1;
        }
        self.latest[phase] = value;
    }
}

#[derive(Clone, Resource, Default)]
pub struct MorphProbe(Arc<Mutex<Snapshot>>);

impl MorphProbe {
    pub fn report(&self) -> String {
        let snapshot = self.0.lock().unwrap();
        let mut text = format!(
            "morph_probe_scope=read-only-main-extraction-and-render-cpu-staging-not-GPU-readback\nmorph_probe_frames={}\nmorph_probe_events={}\nmorph_probe_omitted={}\n{}",
            snapshot.frames, snapshot.events, snapshot.omitted, snapshot.history
        );
        for (phase, latest) in PHASES.iter().zip(&snapshot.latest) {
            writeln!(text, "morph_probe_latest phase={phase}\n{latest}").unwrap();
        }
        text
    }
}

#[derive(Resource)]
struct Target(String);

#[derive(Resource, Default)]
struct Sources {
    frame: u64,
    text: String,
    entities: Vec<(MainEntity, Option<AssetId<Mesh>>)>,
    light_names: MainEntityHashMap<String>,
    joint_counts: MainEntityHashMap<usize>,
}

type MeshSource = (
    Entity,
    Option<&'static Mesh3d>,
    Option<&'static InheritedVisibility>,
    Option<&'static ViewVisibility>,
    Option<&'static MeshMorphWeights>,
    Option<&'static SkinnedMesh>,
    Option<&'static GlobalTransform>,
);

#[derive(SystemParam)]
struct MainInputs<'w, 's> {
    prims: Res<'w, usd_bevy::live::PrimEntities>,
    time: Res<'w, usd_bevy::route::StageTime>,
    bridge: Res<'w, usd_bevy::editor::EditorBridge>,
    sources: Query<'w, 's, MeshSource>,
    subsets: Query<
        'w,
        's,
        (
            Entity,
            &'static usd_bevy::route::subset::UsdSubset,
            &'static ChildOf,
        ),
    >,
    weights: Query<'w, 's, &'static MorphWeights>,
    joints: Query<'w, 's, &'static GlobalTransform>,
    meshes: Res<'w, Assets<Mesh>>,
    inverse_bindposes: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
    light_names: Query<'w, 's, (Entity, &'static Name), With<DirectionalLight>>,
}

pub fn configure(app: &mut App, path: &str) {
    let probe = MorphProbe::default();
    app.insert_resource(probe.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(probe)
        .insert_resource(Target(path.to_owned()))
        .init_resource::<Sources>()
        .add_systems(ExtractSchedule, extract_sources)
        .add_systems(
            Render,
            (
                before_descriptors.before(prepare_morph_descriptors),
                after_descriptors
                    .after(prepare_morph_descriptors)
                    .before(collect_meshes_for_gpu_building),
                after_collection.after(collect_meshes_for_gpu_building),
            )
                .in_set(RenderSystems::PrepareMeshes),
        )
        .add_systems(
            Render,
            observe_lights
                .after(prepare_lights)
                .in_set(RenderSystems::PrepareResources),
        );
}

fn mesh_report(text: &mut Text, mesh: &Mesh) {
    writeln!(
        text,
        "mesh_vertices={} mesh_indices={}",
        mesh.count_vertices(),
        mesh.indices().map_or(0, |indices| indices.len())
    )
    .unwrap();
    for (attribute, values) in mesh.attributes().take(16) {
        write!(text, "attribute={:?} len={} ", attribute.name, values.len()).unwrap();
        if values.len() <= 128 {
            writeln!(text, "values={values:?}").unwrap();
        } else {
            writeln!(text, "values_omitted=true").unwrap();
        }
    }
    if mesh.indices().is_some_and(|indices| indices.len() <= 384) {
        writeln!(text, "indices={:?}", mesh.indices()).unwrap();
    }
    if let Some(targets) = mesh.get_morph_targets() {
        writeln!(
            text,
            "morph_targets_count={} prefix={:?}",
            targets.len(),
            &targets[..targets.len().min(128)]
        )
        .unwrap();
    }
}

fn extract_sources(mut sources: ResMut<Sources>, target: Res<Target>, inputs: Extract<MainInputs>) {
    sources.frame += 1;
    sources.entities.clear();
    sources.joint_counts.clear();
    sources.light_names = inputs
        .light_names
        .iter()
        .take(32)
        .map(|(entity, name)| (entity.into(), name.as_str().to_owned()))
        .collect();
    let mut text = Text::default();
    let view = inputs.bridge.view().ok();
    writeln!(
        text,
        "source_clock={} revision={:?}",
        inputs.time.current,
        view.map(|v| v.document.revision)
    )
    .unwrap();
    if let Some(parent) = inputs.prims.entity(&target.0) {
        let mut selected = vec![(parent, target.0.clone())];
        for (entity, subset, child) in &inputs.subsets {
            if child.parent() == parent && selected.len() < 32 {
                selected.push((entity, format!("{} [subset:{}]", target.0, subset.0)));
            }
        }
        selected.sort_by(|a, b| a.1.cmp(&b.1));
        for (entity, label) in selected {
            let Ok((_, mesh, inherited, visible, weights, skin, transform)) =
                inputs.sources.get(entity)
            else {
                continue;
            };
            let asset = mesh.map(|mesh| mesh.0.id());
            sources.entities.push((entity.into(), asset));
            writeln!(text, "source_entity={entity:?} label={label:?} asset={asset:?} inherited={:?} view_visible={:?} world={:?}", inherited.map(|v| v.get()), visible.map(|v| v.get()), transform.map(|t| t.to_matrix())).unwrap();
            let resolved = match weights {
                Some(MeshMorphWeights::Value { weights }) => Some(weights.as_slice()),
                Some(MeshMorphWeights::Reference(entity)) => {
                    inputs.weights.get(*entity).ok().map(MorphWeights::weights)
                }
                None => None,
            };
            writeln!(
                text,
                "source_weights_count={:?} prefix={:?}",
                resolved.map(<[f32]>::len),
                resolved.map(|weights| &weights[..weights.len().min(128)])
            )
            .unwrap();
            if let Some(mesh) = asset.and_then(|asset| inputs.meshes.get(asset)) {
                mesh_report(&mut text, mesh);
            }
            if let Some(skin) = skin {
                sources
                    .joint_counts
                    .insert(entity.into(), skin.joints.len());
                writeln!(text, "source_joints_count={}", skin.joints.len()).unwrap();
                let inverse = inputs.inverse_bindposes.get(&skin.inverse_bindposes);
                for (index, joint) in skin.joints.iter().take(128).enumerate() {
                    writeln!(
                        text,
                        "joint={index}:{joint:?} world={:?} inverse={:?}",
                        inputs.joints.get(*joint).ok().map(|t| t.to_matrix()),
                        inverse.and_then(|v| v.get(index))
                    )
                    .unwrap();
                }
            }
        }
    }
    sources.text = format!("source_text_truncated={}\n{}", text.truncated, text.value);
}

#[derive(SystemParam)]
struct RenderInputs<'w> {
    sources: Res<'w, Sources>,
    indices: Res<'w, MorphIndices>,
    uniforms: Res<'w, MorphUniforms>,
    instances: Res<'w, RenderMeshInstances>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    allocator: Res<'w, MeshAllocator>,
    buffers: Option<Res<'w, BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
    skins: Res<'w, SkinUniforms>,
}

fn before_descriptors(probe: Res<MorphProbe>, inputs: RenderInputs) {
    observe(&probe, &inputs, 0);
}
fn after_descriptors(probe: Res<MorphProbe>, inputs: RenderInputs) {
    observe(&probe, &inputs, 1);
}
fn after_collection(probe: Res<MorphProbe>, inputs: RenderInputs) {
    observe(&probe, &inputs, 2);
}

fn observe(probe: &MorphProbe, inputs: &RenderInputs, phase: usize) {
    let mut text = Text::default();
    write!(text, "{}", inputs.sources.text).unwrap();
    for &(entity, asset) in &inputs.sources.entities {
        let extracted = match &*inputs.indices {
            MorphIndices::Storage {
                morph_weights_info, ..
            } => morph_weights_info.contains_key(&entity),
            MorphIndices::Uniform { current, .. } => current.contains_key(&entity),
        };
        let index = inputs.indices.morph_descriptor_index(entity);
        let descriptor =
            index.and_then(|index| inputs.uniforms.descriptors_buffer.as_ref()?.get(index.0));
        writeln!(text, "render_entity={entity:?} extracted_weights={extracted} instance_asset={:?} source_asset_ready={} source_target_range={:?} descriptor_index={index:?}", inputs.instances.mesh_asset_id(entity), asset.is_some_and(|asset| inputs.meshes.get(asset).is_some()), asset.and_then(|asset| inputs.allocator.mesh_morph_target_slice(&asset).map(|slice| slice.range))).unwrap();
        if let Some(descriptor) = descriptor {
            writeln!(
                text,
                "descriptor current={} previous={} targets={} vertices={} weights={}",
                descriptor.current_weights_offset,
                descriptor.prev_weights_offset,
                descriptor.targets_offset,
                descriptor.vertex_count,
                descriptor.weight_count
            )
            .unwrap();
            let weights: Vec<_> = (0..descriptor.weight_count.min(128))
                .map(|i| {
                    descriptor
                        .current_weights_offset
                        .checked_add(i)
                        .and_then(|offset| inputs.uniforms.current_buffer.get(offset))
                })
                .collect();
            writeln!(text, "render_weights={weights:?}").unwrap();
        }
        let skin_index = inputs.skins.skin_index(entity);
        let joint_count = inputs
            .sources
            .joint_counts
            .get(&entity)
            .copied()
            .unwrap_or(0);
        let palette = staging_palette(
            &inputs.skins.current_staging_buffer,
            skin_index,
            joint_count,
        );
        writeln!(
            text,
            "skin_staging index={skin_index:?} joints={joint_count} prefix={palette:?}"
        )
        .unwrap();
        match &*inputs.instances {
            RenderMeshInstances::CpuBuilding(_) => writeln!(text, "instance_builder=cpu").unwrap(),
            RenderMeshInstances::GpuBuilding(instances) => {
                let uniform = instances
                    .get(&entity)
                    .map(|instance| instance.gpu_specific.current_uniform_index());
                let input = uniform
                    .and_then(|index| inputs.buffers.as_ref()?.current_input_buffer.get(index));
                writeln!(
                    text,
                    "instance_builder=gpu input_index={uniform:?} input={input:?}"
                )
                .unwrap();
            }
        }
    }
    probe
        .0
        .lock()
        .unwrap()
        .record(inputs.sources.frame, phase, text);
}

fn staging_palette(buffer: &[Mat4], index: Option<u32>, joints: usize) -> Option<&[Mat4]> {
    let start = index? as usize;
    buffer.get(start..start.checked_add(joints.min(128))?)
}

fn observe_lights(
    probe: Res<MorphProbe>,
    sources: Res<Sources>,
    lights: Query<(
        Entity,
        &MainEntity,
        &ExtractedDirectionalLight,
        &bevy::camera::visibility::RenderLayers,
    )>,
) {
    let mut text = Text::default();
    let count = lights.iter().count();
    let mut ordered: Vec<_> = lights.iter().take(32).collect();
    ordered.sort_by_key(|(entity, _, light, _)| {
        (light.volumetric, light.shadow_maps_enabled, *entity)
    });
    writeln!(text, "directional_light_scope=extracted-after-prepare-not-GPU-readback\ndirectional_light_count={count}\ndirectional_light_omitted={}", count.saturating_sub(32)).unwrap();
    for (index, (entity, main, light, layers)) in ordered.iter().enumerate() {
        let name = sources
            .light_names
            .get(*main)
            .map_or("<unlabelled>", String::as_str);
        writeln!(text, "directional_light render_order={index} render={entity:?} main={main:?} name={name:?} volumetric={} shadows={} contact={} lightmapped={} color={:?} illuminance={:?} world={:?} layers={layers:?} depth_bias={:?} normal_bias={:?} soft_shadow={:?} cascade_config={:?} sun_size={:?} sun_intensity={:?}",
            light.volumetric, light.shadow_maps_enabled, light.contact_shadows_enabled,
            light.affects_lightmapped_mesh_diffuse, light.color, light.illuminance,
            light.transform.to_matrix(), light.shadow_depth_bias, light.shadow_normal_bias,
            light.soft_shadow_size, light.cascade_shadow_config, light.sun_disk_angular_size,
            light.sun_disk_intensity).unwrap();
    }
    ordered
        .sort_by_key(|(_, main, light, _)| (light.volumetric, light.shadow_maps_enabled, **main));
    let main_order: Vec<_> = ordered
        .iter()
        .map(|(_, main, _, _)| {
            sources
                .light_names
                .get(*main)
                .map_or("<unlabelled>", String::as_str)
        })
        .collect();
    writeln!(text, "directional_light_main_order={main_order:?}").unwrap();
    probe.0.lock().unwrap().record(sources.frame, 3, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_probe_distinguishes_render_and_main_entity_order() {
        use bevy::ecs::system::RunSystemOnce;
        let mut main = World::new();
        let fill: MainEntity = main.spawn_empty().id().into();
        let rim: MainEntity = main.spawn_empty().id().into();
        let mut world = World::new();
        let probe = MorphProbe::default();
        world.insert_resource(probe.clone());
        let mut sources = Sources::default();
        sources.light_names.insert(fill, "Fill".into());
        sources.light_names.insert(rim, "Rim".into());
        world.insert_resource(sources);
        for entity in [rim, fill] {
            world.spawn((
                entity,
                bevy::camera::visibility::RenderLayers::default(),
                ExtractedDirectionalLight {
                    color: LinearRgba::WHITE,
                    illuminance: 100.0,
                    transform: GlobalTransform::IDENTITY,
                    shadow_maps_enabled: false,
                    contact_shadows_enabled: false,
                    volumetric: false,
                    affects_lightmapped_mesh_diffuse: true,
                    shadow_depth_bias: 0.02,
                    shadow_normal_bias: 0.03,
                    cascade_shadow_config: default(),
                    cascades: default(),
                    frusta: default(),
                    soft_shadow_size: None,
                    occlusion_culling: false,
                    sun_disk_angular_size: 0.0,
                    sun_disk_intensity: 0.0,
                },
            ));
        }
        world.run_system_once(observe_lights).unwrap();
        let report = probe.report();
        let first = report
            .lines()
            .find(|line| line.starts_with("directional_light render_order=0 "))
            .unwrap();
        let main_order = if fill < rim {
            ["Fill", "Rim"]
        } else {
            ["Rim", "Fill"]
        };
        assert!(
            first.contains(&format!("name={:?}", main_order[1])),
            "{report}"
        );
        assert!(report.contains(&format!("directional_light_main_order={main_order:?}")));
        assert!(report.contains("directional_light_omitted=0"));
        assert!(report.contains("not-GPU-readback"));
    }

    #[test]
    fn skin_palette_report_bounds_offsets_and_keeps_exact_values() {
        let palette = [Mat4::IDENTITY, Mat4::from_scale(Vec3::new(1.0, 1.001, 3.0))];
        assert_eq!(staging_palette(&palette, Some(1), 1), Some(&palette[1..]));
        assert!(staging_palette(&palette, Some(1), 2).is_none());
        assert!(staging_palette(&palette, None, 1).is_none());
        assert!(staging_palette(&palette, Some(u32::MAX), usize::MAX).is_none());
        assert_eq!(
            staging_palette(&[Mat4::IDENTITY; 129], Some(0), 129)
                .unwrap()
                .len(),
            128
        );
    }

    #[test]
    fn text_is_utf8_safe_and_bounded() {
        let mut text = Text::default();
        text.write_str(&"x".repeat(TEXT_LIMIT - 1)).unwrap();
        text.write_str("é").unwrap();
        assert!(text.truncated);
        assert_eq!(text.value.len(), TEXT_LIMIT - 1);
        text.write_str("ab").unwrap();
        assert_eq!(text.value.len(), TEXT_LIMIT);
    }

    #[test]
    fn history_deduplicates_each_phase_and_reports_overflow() {
        let probe = MorphProbe::default();
        for frame in 0..100 {
            for phase in 0..PHASES.len() {
                let mut text = Text::default();
                write!(text, "{}", "x".repeat(TEXT_LIMIT)).unwrap();
                probe.0.lock().unwrap().record(frame, phase, text);
            }
        }
        assert_eq!(probe.0.lock().unwrap().events, PHASES.len());
        for frame in 100..200 {
            let mut text = Text::default();
            write!(text, "{frame}{}", "x".repeat(TEXT_LIMIT)).unwrap();
            probe.0.lock().unwrap().record(frame, 2, text);
        }
        let snapshot = probe.0.lock().unwrap();
        assert!(snapshot.history.len() <= HISTORY_LIMIT);
        assert!(snapshot.omitted > 0);
        assert!(snapshot.latest[2].starts_with("199"));
        drop(snapshot);
        let report = probe.report();
        assert!(report.contains("not-GPU-readback"));
        assert!(report.contains("morph_probe_latest phase=after-collection\n199"));
    }

    #[test]
    fn geometry_report_detects_same_size_deformation_changes() {
        let mut mesh = Mesh::from(Cuboid::default());
        let mut before = Text::default();
        mesh_report(&mut before, &mesh);
        mesh.scale_by(Vec3::new(1.0, 2.0, 1.0));
        let mut after = Text::default();
        mesh_report(&mut after, &mesh);
        assert_ne!(before.value, after.value);
        assert!(!before.truncated && !after.truncated);
    }
}
