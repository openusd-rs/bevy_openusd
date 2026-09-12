//! Light route (PLAN P4 + Phase C): UsdLux prims → Bevy lights.
//!
//! * `DistantLight` → [`DirectionalLight`]
//! * `SphereLight` / `DiskLight` → [`PointLight`], or [`SpotLight`] when a
//!   `shaping:cone:angle` is authored
//! * `RectLight` / `CylinderLight` → [`PointLight`] **approximation** (Bevy has
//!   no true area light) plus a [`UsdAreaLight`] marker carrying the authored
//!   dimensions, so an app with an area-light backend can upgrade it.
//!
//! Photometric units differ between USD (author-defined intensity, exposure
//! stops) and Bevy (lux / lumens), so intensity is an **approximate** mapping
//! through the named scale constants below — colour and light *kind* project
//! faithfully; absolute brightness is a best-effort default the app can tune.

use bevy::prelude::*;
use openusd_schemas::lux::SphereLightSchema;
use openusd_schemas::lux::DiskLightSchema;
use openusd_schemas::lux::RectLightSchema;
use openusd_schemas::lux::CylinderLightSchema;
use std::f32::consts::PI;

use openusd_schemas::lux::{
    CylinderLight, DiskLight, DistantLight, LightAPI, RectLight, ShapingAPI, SphereLight,
};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// A UsdLux area light that Bevy can't represent natively. Carries the authored
/// shape so an app with a real area-light backend can build the exact light;
/// the route itself only approximates it with a [`PointLight`].
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum UsdAreaLight {
    /// `RectLight` — a `width` × `height` rectangle.
    Rect { width: f32, height: f32 },
    /// `CylinderLight` — a rod of the given `length` and `radius`.
    Cylinder { length: f32, radius: f32 },
}

/// USD `DistantLight` intensity → Bevy illuminance (lux). USD's default distant
/// intensity is 1; daylight in Bevy is ~10⁴ lux.
const DISTANT_LUX_SCALE: f32 = 10_000.0;
/// USD point/sphere intensity → Bevy luminous intensity (lumens), rough.
const POINT_LUMEN_SCALE: f32 = 1_000.0;

/// Decoded UsdLux common inputs.
struct LuxInputs {
    color: Color,
    intensity: f32,
    radius: Option<f32>,
    cone_angle: Option<f32>,
    /// Present for `RectLight`/`CylinderLight` — the area-light shape marker.
    area: Option<UsdAreaLight>,
}

fn sampled_scalar(attribute: openusd::usd::Attribute, time: Option<f64>) -> Option<f32> {
    attribute.get_at::<f32>(time.map(openusd::usd::TimeCode::new)).ok().flatten()
}

/// Sample common UsdLux color, intensity and exposure inputs.
fn common_inputs<L: openusd::usd::SchemaBase>(light: &L, time: Option<f64>) -> (Color, f32) {
    let light = LightAPI::new(light.prim().clone());
    let color = match light.color_attr().get_at::<Value>(time.map(openusd::usd::TimeCode::new)) {
        Ok(Some(Value::Vec3f(c))) => Color::linear_rgb(c.x, c.y, c.z),
        Ok(Some(Value::Vec3d(c))) => Color::linear_rgb(c.x as f32, c.y as f32, c.z as f32),
        _ => Color::WHITE,
    };
    let base = sampled_scalar(light.intensity_attr(), time).unwrap_or(1.0);
    let exposure = sampled_scalar(light.exposure_attr(), time).unwrap_or(0.0);
    (color, base * 2f32.powf(exposure))
}

fn cone_angle(ctx: &RouteCtx) -> Option<f32> {
    let shaping = ShapingAPI::get(ctx.stage, ctx.path.clone()).ok().flatten()?;
    sampled_scalar(shaping.shaping_cone_angle_attr(), ctx.time)
}

/// Maps UsdLux prims to Bevy light components.
pub struct LightRoute;

#[derive(Component)]
enum LightOwner { Directional, Point, Spot }

fn clear_light(world: &mut World, entity: Entity) {
    let Some(owner) = world.entity_mut(entity).take::<LightOwner>() else { return };
    let mut entity = world.entity_mut(entity);
    match owner {
        LightOwner::Directional => { entity.remove::<DirectionalLight>(); }
        LightOwner::Point => { entity.remove::<PointLight>(); }
        LightOwner::Spot => { entity.remove::<SpotLight>(); }
    }
    entity.remove::<UsdAreaLight>();
}

impl LightRoute {
    fn kind(ctx: &RouteCtx) -> Option<&'static str> {
        match ctx.type_name.as_deref()? {
            "DistantLight" => Some("distant"),
            "SphereLight" | "DiskLight" | "RectLight" | "CylinderLight" => Some("point"),
            _ => None,
        }
    }
}

impl PrimRoute for LightRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        clear_light(world, entity);
    }

    fn matches(&self, ctx: &RouteCtx) -> bool {
        Self::kind(ctx).is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Some(kind) = Self::kind(ctx) else { return };
        // Read through the typed UsdLux schema for the prim's kind.
        let lux = match kind {
            "distant" => {
                let Ok(Some(light)) = DistantLight::get(ctx.stage, ctx.path.clone()) else {
                    return;
                };
                let (color, intensity) = common_inputs(&light, ctx.time);
                LuxInputs {
                    color,
                    intensity,
                    radius: None,
                    cone_angle: None,
                    area: None,
                }
            }
            _ => {
                // Sphere/Disk carry a radius; Rect/Cylinder are area lights we
                // approximate with a point at their center + a shape marker.
                let (color, intensity, radius, area) =
                    if let Ok(Some(l)) = SphereLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l, ctx.time);
                        (c, i, sampled_scalar(l.radius_attr(), ctx.time), None)
                    } else if let Ok(Some(l)) = DiskLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l, ctx.time);
                        (c, i, sampled_scalar(l.radius_attr(), ctx.time), None)
                    } else if let Ok(Some(l)) = RectLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l, ctx.time);
                        let w = sampled_scalar(l.width_attr(), ctx.time).unwrap_or(1.0);
                        let h = sampled_scalar(l.height_attr(), ctx.time).unwrap_or(1.0);
                        // Point radius ≈ the rectangle's half-extent.
                        (
                            c,
                            i,
                            Some(w.max(h) * 0.5),
                            Some(UsdAreaLight::Rect { width: w, height: h }),
                        )
                    } else if let Ok(Some(l)) = CylinderLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l, ctx.time);
                        let length = sampled_scalar(l.length_attr(), ctx.time).unwrap_or(1.0);
                        let r = sampled_scalar(l.radius_attr(), ctx.time).unwrap_or(0.5);
                        (
                            c,
                            i,
                            Some(r),
                            Some(UsdAreaLight::Cylinder { length, radius: r }),
                        )
                    } else {
                        return;
                    };
                LuxInputs {
                    color,
                    intensity,
                    radius,
                    cone_angle: cone_angle(ctx),
                    area,
                }
            }
        };
        clear_light(world, entity);
        let Ok(mut e) = world.get_entity_mut(entity) else {
            return;
        };
        if let Some(area) = lux.area {
            e.insert(area);
        }

        match kind {
            "distant" => {
                e.insert(LightOwner::Directional);
                e.insert(DirectionalLight {
                    color: lux.color,
                    illuminance: lux.intensity * DISTANT_LUX_SCALE,
                    ..default()
                });
            }
            _ => {
                let intensity = lux.intensity * POINT_LUMEN_SCALE;
                let radius = lux.radius.unwrap_or(0.0);
                if let Some(cone_deg) = lux.cone_angle {
                    e.insert(LightOwner::Spot);
                    let outer = (cone_deg * PI / 180.0).clamp(0.0, PI / 2.0);
                    e.insert(SpotLight {
                        color: lux.color,
                        intensity,
                        radius,
                        outer_angle: outer,
                        inner_angle: outer * 0.9,
                        ..default()
                    });
                } else {
                    e.insert(LightOwner::Point);
                    e.insert(PointLight {
                        color: lux.color,
                        intensity,
                        radius,
                        ..default()
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::sdf::Value;
    use openusd::usd::Stage;

    fn light_world() -> World {
        let mut w = World::new();
        w.insert_resource(SchemaRegistry::builtin());
        w
    }

    #[test]
    fn sampled_lights_update_inputs_dimensions_and_cones() {
        use openusd::usd::TimeCode;
        for kind in ["DistantLight", "SphereLight", "DiskLight", "RectLight", "CylinderLight"] {
            let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("sampled-light.usda").unwrap();
            stage.define_prim("/Light").unwrap().set_type_name(kind).unwrap();
            for name in ["inputs:intensity", "inputs:radius", "inputs:width", "inputs:height", "inputs:length"] {
                stage.create_attribute(format!("/Light.{name}"), "float").unwrap()
                    .set_at(Value::Float(1.0), TimeCode::new(0.0)).unwrap()
                    .set_at(Value::Float(3.0), TimeCode::new(10.0)).unwrap();
            }
            stage.create_attribute("/Light.inputs:exposure", "float").unwrap()
                .set_at(Value::Float(0.0), TimeCode::new(0.0)).unwrap()
                .set_at(Value::Float(2.0), TimeCode::new(10.0)).unwrap();
            stage.create_attribute("/Light.inputs:color", "color3f").unwrap()
                .set_at(Value::Vec3f([1.0,0.0,0.0].into()), TimeCode::new(0.0)).unwrap()
                .set_at(Value::Vec3f([0.0,0.0,1.0].into()), TimeCode::new(10.0)).unwrap();
            let path = openusd::sdf::path("/Light").unwrap();
            let mut world = light_world();
            let entity = world.spawn_empty().id();
            LightRoute.project(&RouteCtx::at(&stage, &path, Some(5.0)), &mut world, entity);
            let color = Color::linear_rgb(0.5, 0.0, 0.5);
            if kind == "DistantLight" {
                let light = world.get::<DirectionalLight>(entity).unwrap();
                assert_eq!(light.color, color);
                assert_eq!(light.illuminance, 4.0 * DISTANT_LUX_SCALE);
            } else {
                let light = world.get::<PointLight>(entity).unwrap();
                assert_eq!(light.color, color);
                assert_eq!(light.intensity, 4.0 * POINT_LUMEN_SCALE);
                assert_eq!(light.radius, if kind == "RectLight" { 1.0 } else { 2.0 });
                if kind == "RectLight" { assert_eq!(world.get::<UsdAreaLight>(entity), Some(&UsdAreaLight::Rect { width: 2.0, height: 2.0 })); }
                if kind == "CylinderLight" { assert_eq!(world.get::<UsdAreaLight>(entity), Some(&UsdAreaLight::Cylinder { length: 2.0, radius: 2.0 })); }
                stage.prim(path.clone()).unwrap().add_applied_schema("ShapingAPI").unwrap();
                stage.create_attribute("/Light.inputs:shaping:cone:angle", "float").unwrap()
                    .set_at(Value::Float(20.0), TimeCode::new(0.0)).unwrap()
                    .set_at(Value::Float(60.0), TimeCode::new(10.0)).unwrap();
                LightRoute.project(&RouteCtx::at(&stage, &path, Some(5.0)), &mut world, entity);
                assert!(world.get::<PointLight>(entity).is_none());
                let light = world.get::<SpotLight>(entity).unwrap();
                assert!((light.outer_angle - 40.0_f32.to_radians()).abs() < 1e-6);
                assert_eq!(light.intensity, 4.0 * POINT_LUMEN_SCALE);
            }
        }
    }

    #[test]
    fn independent_clocks_sample_light_intensity() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("light-clocks.usda", &br#"#usda 1.0
def DistantLight "Sun" { float inputs:intensity.timeSamples = { 0: 1, 10: 3 } }
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let light_for = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/Sun").unwrap();
        let a = light_for(app.world(), first);
        let b = light_for(app.world(), second);
        assert_eq!(app.world().get::<DirectionalLight>(a).unwrap().illuminance, DISTANT_LUX_SCALE);
        assert_eq!(app.world().get::<DirectionalLight>(b).unwrap().illuminance, 3.0 * DISTANT_LUX_SCALE);
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.update();
        assert_eq!(light_for(app.world(), first), a);
        assert_eq!(light_for(app.world(), second), b);
        assert_eq!(app.world().get::<DirectionalLight>(a).unwrap().illuminance, 2.0 * DISTANT_LUX_SCALE);
        assert_eq!(app.world().get::<DirectionalLight>(b).unwrap().illuminance, 3.0 * DISTANT_LUX_SCALE);
    }

    #[test]
    fn light_and_camera_type_changes_clean_only_owned_components() {
        #[derive(Component)]
        struct RuntimeOnly;
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("light-lifecycle.usda").unwrap();
        stage.define_prim("/Prim").unwrap().set_type_name("RectLight").unwrap();
        let live = LiveStage::new(stage);
        let mut world = light_world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Prim").unwrap();
        world.entity_mut(entity).insert(RuntimeOnly);
        let child = world.spawn((RuntimeOnly, ChildOf(entity))).id();
        assert!(world.get::<PointLight>(entity).is_some());
        assert!(world.get::<UsdAreaLight>(entity).is_some());
        for kind in ["DistantLight", "Camera", "Xform", "SphereLight", "Xform"] {
            live.stage.prim("/Prim").unwrap().set_type_name(kind).unwrap();
            crate::live::apply_changes(&mut world, &live, &mut map);
            assert_eq!(map.entity("/Prim"), Some(entity));
            assert_eq!(world.get::<DirectionalLight>(entity).is_some(), kind == "DistantLight");
            assert_eq!(world.get::<PointLight>(entity).is_some(), kind == "SphereLight");
            assert_eq!(world.get::<Projection>(entity).is_some(), kind == "Camera");
            assert_eq!(world.get::<crate::route::camera::UsdCamera>(entity).is_some(), kind == "Camera");
            assert!(world.get::<SpotLight>(entity).is_none());
            assert!(world.get::<UsdAreaLight>(entity).is_none());
            assert!(world.get::<RuntimeOnly>(entity).is_some());
            assert!(world.get::<RuntimeOnly>(child).is_some());
        }
        live.stage.prim("/Prim").unwrap().set_type_name("SphereLight").unwrap()
            .add_applied_schema("ShapingAPI").unwrap();
        crate::authoring::set_attribute(&live.stage, "/Prim", "inputs:shaping:cone:angle", "float", Value::Float(30.0)).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<SpotLight>(entity).is_some());
        assert!(world.get::<PointLight>(entity).is_none());
        live.stage.prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<SpotLight>(entity).is_none());
        world.entity_mut(entity).insert((PointLight::default(), Projection::default()));
        let path = openusd::sdf::path("/Prim").unwrap();
        let registry = SchemaRegistry::builtin();
        registry.project_prim(&live.stage, &path, &mut world, entity);
        assert!(world.get::<PointLight>(entity).is_some());
        assert!(world.get::<Projection>(entity).is_some());
        live.stage.prim("/Prim").unwrap().set_type_name("DistantLight").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<DirectionalLight>(entity).is_some());
        assert!(world.get::<PointLight>(entity).is_some());
        live.stage.prim("/Prim").unwrap().set_type_name("Xform").unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<DirectionalLight>(entity).is_none());
        assert!(world.get::<PointLight>(entity).is_some());
        assert!(world.get::<Projection>(entity).is_some());
    }

    #[test]
    fn distant_light_projects_directional() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("lux.usda").unwrap();
        stage
            .define_prim("/Sun")
            .unwrap()
            .set_type_name("DistantLight")
            .unwrap();
        stage
            .create_attribute("/Sun.inputs:color", "color3f")
            .unwrap()
            .set(Value::Vec3f([1.0, 0.9, 0.8].into()))
            .unwrap();
        stage
            .create_attribute("/Sun.inputs:intensity", "float")
            .unwrap()
            .set(Value::Float(2.0))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = light_world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/Sun").unwrap();
        let d = world.get::<DirectionalLight>(e).expect("directional light");
        assert!((d.illuminance - 2.0 * DISTANT_LUX_SCALE).abs() < 1.0);
        assert_eq!(d.color, Color::linear_rgb(1.0, 0.9, 0.8));
    }

    #[test]
    fn sphere_light_projects_point_and_spot() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("lux2.usda").unwrap();
        stage
            .define_prim("/Bulb")
            .unwrap()
            .set_type_name("SphereLight")
            .unwrap();
        stage
            .create_attribute("/Bulb.inputs:intensity", "float")
            .unwrap()
            .set(Value::Float(50.0))
            .unwrap();
        // A separate spot: a SphereLight with the ShapingAPI applied + a cone.
        stage
            .define_prim("/Spot")
            .unwrap()
            .set_type_name("SphereLight")
            .unwrap()
            .add_applied_schema("ShapingAPI")
            .unwrap();
        stage
            .create_attribute("/Spot.inputs:shaping:cone:angle", "float")
            .unwrap()
            .set(Value::Float(30.0))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = light_world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let bulb = map.entity("/Bulb").unwrap();
        assert!(
            world.get::<PointLight>(bulb).is_some(),
            "sphere light with no cone → point light"
        );
        let spot = map.entity("/Spot").unwrap();
        assert!(
            world.get::<SpotLight>(spot).is_some(),
            "sphere light with a cone → spot light"
        );
    }

    #[test]
    fn rect_and_cylinder_lights_project_point_plus_area_marker() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("area.usda").unwrap();
        stage.define_prim("/Rect").unwrap().set_type_name("RectLight").unwrap();
        stage
            .create_attribute("/Rect.inputs:width", "float")
            .unwrap()
            .set(Value::Float(4.0))
            .unwrap();
        stage
            .create_attribute("/Rect.inputs:height", "float")
            .unwrap()
            .set(Value::Float(2.0))
            .unwrap();
        stage.define_prim("/Rod").unwrap().set_type_name("CylinderLight").unwrap();
        stage
            .create_attribute("/Rod.inputs:length", "float")
            .unwrap()
            .set(Value::Float(6.0))
            .unwrap();
        stage
            .create_attribute("/Rod.inputs:radius", "float")
            .unwrap()
            .set(Value::Float(0.25))
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = light_world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let rect = map.entity("/Rect").unwrap();
        assert!(world.get::<PointLight>(rect).is_some(), "rect → point approx");
        assert_eq!(
            world.get::<UsdAreaLight>(rect).copied(),
            Some(UsdAreaLight::Rect { width: 4.0, height: 2.0 }),
            "rect dimensions carried on the marker"
        );

        let rod = map.entity("/Rod").unwrap();
        assert!(world.get::<PointLight>(rod).is_some(), "cylinder → point approx");
        assert_eq!(
            world.get::<UsdAreaLight>(rod).copied(),
            Some(UsdAreaLight::Cylinder { length: 6.0, radius: 0.25 }),
            "cylinder dimensions carried on the marker"
        );
    }
}
