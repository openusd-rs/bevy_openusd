//! Dome-light markers and an opt-in per-camera ambient approximation.
//!
//! Bevy 0.19 filters cubemaps through `GeneratedEnvironmentMapLight`.
//! [`super::environment_map::latlong_cubemap`] converts Y-pole latitude-longitude
//! textures to that input format. Loaded snapshots supply [`UsdDomeTexture`];
//! this route does not attach an environment light to cameras.

use bevy::prelude::*;

use openusd_schemas::lux::{DomeLight, DomeLightSchema, DomeLight_1, DomeLight_1Schema, NonboundableLightBase};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// USD dome intensity to approximate Bevy ambient brightness.
const AMBIENT_SCALE: f32 = 100.0;

/// A `UsdLuxDomeLight`: an image-based environment light. Bevy can't consume the
/// lat-long HDR directly (see the module docs), so this carries the authored
/// data for an app to build real IBL / a skybox from.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct UsdDomeLight {
    /// The environment texture (`inputs:texture:file`), resolved when available.
    pub texture: String,
    /// `inputs:texture:format` (`latlong`, `mirroredBall`, `angular`,
    /// `cubeMapVerticalCross`, or `automatic`).
    pub format: String,
    /// Linear light color, as authored.
    pub color: [f32; 3],
    /// The exposure-scaled `inputs:intensity`.
    pub intensity: f32,
}

pub(crate) fn asset_string(v: Option<Value>) -> String {
    match v {
        Some(Value::AssetPath(a)) => a.resolved_path().unwrap_or(a.as_str()).to_string(),
        Some(Value::String(s)) => s,
        Some(Value::Token(t)) => t.as_str().to_string(),
        _ => String::new(),
    }
}

/// Loaded linear-radiance texture for a projected dome, before cubemap conversion.
#[derive(Component, Clone, Debug)]
pub struct UsdDomeTexture(pub Handle<Image>);

/// Dome-only pole alignment, before the prim's world transform.
#[derive(Component, Clone, Copy, Debug)]
pub struct UsdDomePoleRotation(pub Quat);

fn token_string(v: Option<Value>) -> String {
    match v {
        Some(Value::Token(t)) => t.as_str().to_string(),
        Some(Value::String(s)) => s,
        _ => "automatic".to_string(),
    }
}

/// Selects a projected dome as the ambient approximation for this camera.
#[derive(Component, Clone, Copy)]
pub struct UsdDomeAmbientSource(pub Entity);

#[derive(Component, Clone)]
struct AmbientRestore { previous: Option<AmbientLight>, applied: AmbientLight }

/// Updates explicitly selected cameras without modifying global ambient lighting.
pub struct UsdDomeAmbientPlugin;

impl Plugin for UsdDomeAmbientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update_camera_ambient.after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate));
    }
}

fn same_ambient(a: &AmbientLight, b: &AmbientLight) -> bool {
    a.color == b.color && a.brightness == b.brightness && a.affects_lightmapped_meshes == b.affects_lightmapped_meshes
}

fn update_camera_ambient(world: &mut World) {
    let cameras: Vec<_> = world.query::<(Entity, Option<&UsdDomeAmbientSource>, Option<&AmbientRestore>)>()
        .iter(world).filter(|(_, source, restore)| source.is_some() || restore.is_some())
        .map(|(entity, source, restore)| (entity, source.copied(), restore.cloned())).collect();
    for (camera, source, restore) in cameras {
        let dome = source.filter(|_| world.get::<Camera>(camera).is_some()).and_then(|source| {
            let visible = world.get::<InheritedVisibility>(source.0).map_or_else(
                || world.get::<Visibility>(source.0) != Some(&Visibility::Hidden), |visibility| visibility.get());
            visible.then(|| world.get::<UsdDomeLight>(source.0)).flatten()
        });
        let ambient = dome.filter(|dome| dome.intensity.is_finite() && dome.intensity >= 0.0
            && dome.color.iter().all(|value| value.is_finite()) && (dome.intensity * AMBIENT_SCALE).is_finite())
            .map(|dome| AmbientLight { color: Color::linear_rgb(dome.color[0], dome.color[1], dome.color[2]),
                brightness: dome.intensity * AMBIENT_SCALE, ..default() });
        if let Some(ambient) = ambient {
            let previous = restore.map(|state| state.previous).unwrap_or_else(|| world.get::<AmbientLight>(camera).cloned());
            world.entity_mut(camera).insert((AmbientRestore { previous, applied: ambient.clone() }, ambient));
        } else if let Some(restore) = restore {
            let unchanged = world.get::<AmbientLight>(camera).is_some_and(|current| same_ambient(current, &restore.applied));
            let mut camera = world.entity_mut(camera);
            camera.remove::<AmbientRestore>();
            if unchanged {
                if let Some(previous) = restore.previous { camera.insert(previous); }
                else { camera.remove::<AmbientLight>(); }
            }
        }
    }
}

/// Projects `DomeLight` prims as [`UsdDomeLight`] markers.
pub struct DomeLightRoute;

impl PrimRoute for DomeLightRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<(UsdDomeLight, UsdDomeTexture, UsdDomePoleRotation)>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        matches!(ctx.type_name.as_deref(), Some("DomeLight") | Some("DomeLight_1"))
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let (light, texture_file, texture_format) = if ctx.type_name.as_deref() == Some("DomeLight_1") {
            let Ok(Some(dome)) = DomeLight_1::get(ctx.stage, ctx.path.clone()) else { return };
            (dome.light_api(), dome.texture_file_attr(), dome.texture_format_attr())
        } else {
            let Ok(Some(dome)) = DomeLight::get(ctx.stage, ctx.path.clone()) else { return };
            (dome.light_api(), dome.texture_file_attr(), dome.texture_format_attr())
        };
        let time = ctx.time.map(openusd::usd::TimeCode::new);
        let color = match light.color_attr().get_at::<Value>(time) {
            Ok(Some(Value::Vec3f(c))) => [c.x, c.y, c.z],
            Ok(Some(Value::Vec3d(c))) => [c.x as f32, c.y as f32, c.z as f32],
            _ => [1.0, 1.0, 1.0],
        };
        let base = light.intensity_attr().get_at::<f32>(time).ok().flatten().unwrap_or(1.0);
        let exposure = light.exposure_attr().get_at::<f32>(time).ok().flatten().unwrap_or(0.0);
        let intensity = base * 2f32.powf(exposure);

        let marker = UsdDomeLight {
            texture: asset_string(texture_file.get_at::<Value>(time).ok().flatten()),
            format: token_string(texture_format.get_at::<Value>(time).ok().flatten()),
            color,
            intensity,
        };

        let texture = world.get_resource::<crate::asset::SnapshotTextures>()
            .and_then(|textures| textures.0.get(&(marker.texture.clone(), false))).cloned();
        let pole = if ctx.type_name.as_deref() == Some("DomeLight_1") {
            let axis = ctx.stage.prim(ctx.path).ok().and_then(|prim|
                prim.attribute("poleAxis").get_at::<Value>(time).ok().flatten()).map(|value| token_string(Some(value))).unwrap_or_else(|| "scene".into());
            match axis.as_str() {
                "Y" => Quat::IDENTITY,
                "Z" => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                _ => crate::live::stage_up_axis(ctx.stage).inverse(),
            }
        } else { Quat::IDENTITY };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            if let Some(texture) = texture { e.insert(UsdDomeTexture(texture)); }
            else { e.remove::<UsdDomeTexture>(); }
            e.insert((marker, UsdDomePoleRotation(pole)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::usd::Stage;

    #[test]
    fn versioned_pole_alignment_is_dome_local() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("pole.usda").unwrap();
        DomeLight::define(&stage, "/Env").unwrap();
        let prim = stage.prim("/Env").unwrap();
        let prim = prim.set_type_name("DomeLight_1").unwrap();
        let path = openusd::sdf::path("/Env").unwrap();
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        for (axis, expected) in [("Y", Vec3::Y), ("Z", Vec3::Z), ("scene", Vec3::Y)] {
            prim.attribute("poleAxis").set(Value::Token(axis.into())).unwrap();
            DomeLightRoute.project(&RouteCtx::at(&stage, &path, None), &mut world, entity);
            assert!((world.get::<UsdDomePoleRotation>(entity).unwrap().0 * Vec3::Y).abs_diff_eq(expected, 1e-6));
            assert!(world.get::<Transform>(entity).is_none());
        }
    }

    #[test]
    fn dome_light_projects_marker_without_global_side_effects() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("dome.usda").unwrap();
        let dome = DomeLight::define(&stage, "/Env").unwrap();
        dome.create_texture_file_attr()
            .unwrap()
            .set(Value::AssetPath("./studio.hdr".into()))
            .unwrap();
        dome.create_texture_format_attr()
            .unwrap()
            .set(Value::Token("latlong".into()))
            .unwrap();
        dome.create_intensity_attr().unwrap().set(Value::Float(2.0)).unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Env").unwrap();
        let m = world.get::<UsdDomeLight>(e).expect("dome marker");
        assert_eq!(m.texture, "./studio.hdr");
        assert_eq!(m.format, "latlong");
        assert_eq!(m.intensity, 2.0);

        assert!(!world.contains_resource::<GlobalAmbientLight>());
        live.stage.prim("/Env").unwrap().attribute("inputs:intensity")
            .set_at(Value::Float(2.0), openusd::usd::TimeCode::new(0.0)).unwrap()
            .set_at(Value::Float(4.0), openusd::usd::TimeCode::new(10.0)).unwrap();
        let path = openusd::sdf::path("/Env").unwrap();
        DomeLightRoute.project(&RouteCtx::at(&live.stage, &path, Some(5.0)), &mut world, e);
        assert_eq!(world.get::<UsdDomeLight>(e).unwrap().intensity, 3.0);
        assert!(!world.contains_resource::<GlobalAmbientLight>());
        live.stage.prim("/Env").unwrap().set_type_name("Xform").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<UsdDomeLight>(e).is_none());
    }

    #[test]
    fn camera_adapters_isolate_domes_and_restore_owned_ambient() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::camera::visibility::VisibilityPlugin, UsdDomeAmbientPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        app.insert_resource(GlobalAmbientLight { brightness: 17.0, ..default() });
        let first = app.world_mut().spawn((Visibility::default(), UsdDomeLight { intensity: 2.0, color: [1.0,0.0,0.0], ..default() })).id();
        let second = app.world_mut().spawn((Visibility::default(), UsdDomeLight { intensity: 3.0, color: [0.0,0.0,1.0], ..default() })).id();
        let a = app.world_mut().spawn((Camera::default(), AmbientLight { brightness: 12.0, ..default() }, UsdDomeAmbientSource(first))).id();
        let b = app.world_mut().spawn((Camera::default(), UsdDomeAmbientSource(second))).id();
        app.update();
        assert_eq!(app.world().get::<AmbientLight>(a).unwrap().brightness, 200.0);
        assert_eq!(app.world().get::<AmbientLight>(b).unwrap().brightness, 300.0);
        assert_eq!(app.world().resource::<GlobalAmbientLight>().brightness, 17.0);
        app.world_mut().get_mut::<UsdDomeLight>(first).unwrap().intensity = 4.0;
        app.update();
        assert_eq!(app.world().get::<AmbientLight>(a).unwrap().brightness, 400.0);
        assert_eq!(app.world().get::<AmbientLight>(b).unwrap().brightness, 300.0);
        app.world_mut().entity_mut(first).insert(Visibility::Hidden);
        app.update();
        assert_eq!(app.world().get::<AmbientLight>(a).unwrap().brightness, 12.0);
        app.world_mut().entity_mut(first).insert(Visibility::Inherited);
        app.update();
        assert_eq!(app.world().get::<AmbientLight>(a).unwrap().brightness, 400.0);
        app.world_mut().despawn(second);
        app.update();
        assert!(app.world().get::<AmbientLight>(b).is_none());
        app.world_mut().get_mut::<AmbientLight>(a).unwrap().brightness = 42.0;
        app.world_mut().entity_mut(a).remove::<UsdDomeAmbientSource>();
        app.update();
        assert_eq!(app.world().get::<AmbientLight>(a).unwrap().brightness, 42.0);
    }
}
