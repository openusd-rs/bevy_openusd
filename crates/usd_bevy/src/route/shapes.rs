//! Shapes route (SCHEMA_INTEGRATION Phase B): USD geom shape prims
//! (`Cube`/`Sphere`/`Cylinder`/`Capsule`/`Cone`/`Plane`) → Bevy primitive
//! meshes. Reads through openusd's `geom` shape schemas.
//!
//! USD's `Cylinder`/`Cone`/`Capsule`/`Plane` carry an `axis` (default `Z`);
//! Bevy primitives are `Y`-aligned, so the generated mesh is rotated to match.
//! Registered before the material route, which then binds a real material.

use bevy::prelude::*;
use openusd_schemas::geom::CubeSchema;
use openusd_schemas::geom::SphereSchema;
use openusd_schemas::geom::CylinderSchema;
use openusd_schemas::geom::CapsuleSchema;
use openusd_schemas::geom::ConeSchema;
use openusd_schemas::geom::PlaneSchema;
use std::f32::consts::FRAC_PI_2;

use openusd_schemas::geom::{Capsule, Cone, Cube, Cylinder, Plane, Sphere};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// Maps USD geometric shapes to Bevy primitive meshes.
pub struct ShapesRoute;

/// Shape geometry could not be evaluated from the current sample.
#[derive(Component, Debug, Clone)]
pub struct UsdShapeError(pub String);

fn f32_attr(attr: openusd::usd::Attribute, default: f32, time: Option<f64>) -> Option<f32> {
    let value = match attr.get_at::<Value>(time.map(openusd::usd::TimeCode::new)).ok()? {
        Some(Value::Double(d)) => d as f32,
        Some(Value::Float(f)) => f,
        None => default,
        _ => return None,
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Rotation taking a `Y`-aligned Bevy primitive onto the USD `axis` token.
fn axis_rotation(attr: openusd::usd::Attribute) -> Quat {
    let axis = match attr.get::<Value>() {
        Ok(Some(Value::Token(t))) => t.as_str().to_string(),
        _ => "Z".to_string(), // USD default axis
    };
    match axis.as_str() {
        "X" => Quat::from_rotation_z(-FRAC_PI_2), // +Y → +X
        "Z" => Quat::from_rotation_x(FRAC_PI_2),  // +Y → +Z
        _ => Quat::IDENTITY,                       // "Y"
    }
}

pub(crate) struct ShapeMesh {
    pub mesh: Mesh,
    pub opacity: Option<crate::read::geom::MeshPrimvar<f32>>,
}

pub(crate) fn shape_mesh(ctx: &RouteCtx) -> Option<ShapeMesh> {
    let mut mesh = shape_geometry(ctx)?;
    for attribute in [Mesh::ATTRIBUTE_POSITION, Mesh::ATTRIBUTE_NORMAL] {
        let bevy::mesh::VertexAttributeValues::Float32x3(values) = mesh.attribute(attribute)? else { return None };
        if values.iter().flatten().any(|value| !value.is_finite()) { return None; }
    }
    let color = crate::read::geom::read_primvar_vec3f(ctx.stage, ctx.path, "primvars:displayColor", ctx.time)
        .ok().flatten().as_ref().and_then(constant_value);
    let opacity = crate::read::geom::read_primvar_float(ctx.stage, ctx.path, "primvars:displayOpacity", ctx.time)
        .ok().flatten();
    let alpha = opacity.as_ref().and_then(constant_value);
    if color.is_some() || alpha.is_some() {
        let color = color.unwrap_or([1.0;3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[color[0],color[1],color[2],alpha.unwrap_or(1.0)]; mesh.count_vertices()]);
    }
    Some(ShapeMesh { mesh, opacity })
}

fn constant_value<T: Copy>(value: &crate::read::geom::MeshPrimvar<T>) -> Option<T> {
    if value.interpolation != crate::read::geom::Interpolation::Constant
        && !(value.values.len() == 1 && value.indices.is_empty()) { return None; }
    let index = usize::try_from(value.indices.first().copied().unwrap_or(0)).ok()?;
    value.values.get(index).copied()
}

fn shape_geometry(ctx: &RouteCtx) -> Option<Mesh> {
    let stage = ctx.stage;
    let p = ctx.path.clone();
    match ctx.type_name.as_deref()? {
        "Cube" => {
            let cube = Cube::get(stage, p).ok()??;
            let size = f32_attr(cube.size_attr(), 2.0, ctx.time)?;
            Some(Mesh::from(Cuboid::from_length(size)))
        }
        "Sphere" => {
            let sphere = Sphere::get(stage, p).ok()??;
            let r = f32_attr(sphere.radius_attr(), 1.0, ctx.time)?;
            Some(Mesh::from(bevy::math::primitives::Sphere::new(r)))
        }
        "Cylinder" => {
            let cyl = Cylinder::get(stage, p).ok()??;
            let r = f32_attr(cyl.radius_attr(), 1.0, ctx.time)?;
            let h = f32_attr(cyl.height_attr(), 2.0, ctx.time)?;
            let mesh = Mesh::from(bevy::math::primitives::Cylinder::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cyl.axis_attr())))
        }
        "Capsule" => {
            let cap = Capsule::get(stage, p).ok()??;
            let r = f32_attr(cap.radius_attr(), 0.5, ctx.time)?;
            let h = f32_attr(cap.height_attr(), 1.0, ctx.time)?;
            let mesh = Mesh::from(Capsule3d::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cap.axis_attr())))
        }
        "Cone" => {
            let cone = Cone::get(stage, p).ok()??;
            let r = f32_attr(cone.radius_attr(), 1.0, ctx.time)?;
            let h = f32_attr(cone.height_attr(), 2.0, ctx.time)?;
            let mesh = Mesh::from(bevy::math::primitives::Cone::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cone.axis_attr())))
        }
        "Plane" => {
            let plane = Plane::get(stage, p).ok()??;
            let w = f32_attr(plane.width_attr(), 2.0, ctx.time)?;
            let l = f32_attr(plane.length_attr(), 2.0, ctx.time)?;
            let mesh = Mesh::from(Rectangle::new(w, l));
            let axis = plane.axis_attr().get::<Value>().ok().flatten();
            let rotation = match axis {
                Some(Value::Token(axis)) if axis.as_str() == "X" => Quat::from_rotation_y(FRAC_PI_2),
                Some(Value::Token(axis)) if axis.as_str() == "Y" => Quat::from_rotation_x(-FRAC_PI_2),
                _ => Quat::IDENTITY,
            };
            Some(mesh.rotated_by(rotation))
        }
        _ => None,
    }
}

impl PrimRoute for ShapesRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Shape);
        world.entity_mut(entity).remove::<UsdShapeError>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        matches!(
            ctx.type_name.as_deref(),
            Some("Cube" | "Sphere" | "Cylinder" | "Capsule" | "Cone" | "Plane")
        )
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            return;
        }
        let Some(shape) = shape_mesh(ctx) else {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Shape);
            world.entity_mut(entity).insert(UsdShapeError("shape dimensions must be finite and nonnegative, and generated geometry must be finite".into()));
            return;
        };
        let mesh_handle = super::cache::intern_mesh(world, shape.mesh);
        let material = super::cache::intern_material(world, super::material::default_material_with_opacity(ctx, shape.opacity.as_ref()));
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.remove::<(bevy::camera::primitives::Aabb, UsdShapeError)>();
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material), super::geom::GeometryOwner::Shape));
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
    fn invalid_dimensions_suppress_geometry_and_recover_without_stale_bounds() {
        use bevy::camera::primitives::Aabb;
        for (kind, dimensions) in [
            ("Cube", vec!["size"]), ("Sphere", vec!["radius"]),
            ("Cylinder", vec!["radius", "height"]), ("Capsule", vec!["radius", "height"]),
            ("Cone", vec!["radius", "height"]), ("Plane", vec!["width", "length"]),
        ] {
            let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("invalid-shape.usda").unwrap();
            stage.define_prim("/Shape").unwrap().set_type_name(kind).unwrap();
            let path = openusd::sdf::path("/Shape").unwrap();
            let mut world = world();
            let entity = world.spawn_empty().id();
            let child = world.spawn(ChildOf(entity)).id();
            for dimension in dimensions {
                let attribute = stage.create_attribute(format!("/Shape.{dimension}"), "double").unwrap();
                for invalid in [-1.0, f64::NAN, f64::INFINITY, 1e100] {
                    attribute.clone().set(Value::Double(2.0)).unwrap();
                    ShapesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
                    assert!(world.get::<Mesh3d>(entity).is_some(), "{kind}.{dimension}");
                    world.entity_mut(entity).insert(Aabb::from_min_max(Vec3::splat(-1.0), Vec3::ONE));
                    attribute.clone().set(Value::Double(4.0)).unwrap();
                    ShapesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
                    assert!(world.get::<Aabb>(entity).is_none());
                    attribute.clone().set(Value::Double(invalid)).unwrap();
                    ShapesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
                    assert!(world.get::<Mesh3d>(entity).is_none(), "{kind}.{dimension}={invalid}");
                    assert!(world.get::<UsdShapeError>(entity).is_some());
                    assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), entity);
                }
                attribute.set(Value::Double(1.0)).unwrap();
                ShapesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
                assert!(world.get::<UsdShapeError>(entity).is_none());
                assert!(world.get::<Mesh3d>(entity).is_some());
            }
            ShapesRoute.remove(&RouteCtx::new(&stage, &path), &mut world, entity);
            assert!(world.get::<UsdShapeError>(entity).is_none());
            assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), entity);
        }
    }

    #[test]
    fn constant_display_colors_sample_indices_for_all_shapes() {
        use bevy::mesh::VertexAttributeValues;
        for kind in ["Cube", "Sphere", "Cylinder", "Capsule", "Cone", "Plane"] {
            let text = format!(r#"#usda 1.0
def {kind} "Shape" {{
    color3f[] primvars:displayColor = [(1,0,0), (0,1,0)] (interpolation = "constant")
    int[] primvars:displayColor:indices.timeSamples = {{0: [0], 10: [1]}}
    float[] primvars:displayOpacity = [0.25, 1] (interpolation = "constant")
    int[] primvars:displayOpacity:indices.timeSamples = {{0: [0], 10: [1]}}
}}
"#);
            let source = crate::UsdSource::new("shape-color.usda", text.into_bytes()).unwrap();
            let stage = source.open_stage().unwrap();
            let path = openusd::sdf::path("/Shape").unwrap();
            for (time, expected) in [(0.0, [1.0,0.0,0.0,0.25]), (10.0, [0.0,1.0,0.0,1.0])] {
                let shape = shape_mesh(&RouteCtx::at(&stage, &path, Some(time))).unwrap();
                let ctx = RouteCtx::at(&stage, &path, Some(time));
                assert_eq!(super::super::material::default_material_with_opacity(&ctx, shape.opacity.as_ref()).alpha_mode,
                    if time == 0.0 { AlphaMode::Blend } else { AlphaMode::Opaque });
                let mesh = shape.mesh;
                let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("{kind} colors") };
                assert_eq!(colors.len(), mesh.count_vertices());
                assert!(colors.iter().all(|color| *color == expected), "{kind} time={time}");
                assert_eq!(super::super::material::default_material(&RouteCtx::at(&stage, &path, Some(time))).alpha_mode,
                    if time == 0.0 { AlphaMode::Blend } else { AlphaMode::Opaque });
            }
        }
    }

    #[test]
    fn all_shape_dimensions_sample_at_route_time() {
        let cases: [(&str, &[&str]); 6] = [
            ("Cube", &["size"]), ("Sphere", &["radius"]),
            ("Cylinder", &["radius", "height"]), ("Capsule", &["radius", "height"]),
            ("Cone", &["radius", "height"]), ("Plane", &["width", "length"]),
        ];
        for (kind, attributes) in cases {
            let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("sampled-shape.usda").unwrap();
            stage.define_prim("/Shape").unwrap().set_type_name(kind).unwrap();
            for attribute in attributes {
                stage.create_attribute(format!("/Shape.{attribute}"), "double").unwrap()
                    .set_at(Value::Double(1.0), openusd::usd::TimeCode::new(0.0)).unwrap()
                    .set_at(Value::Double(3.0), openusd::usd::TimeCode::new(10.0)).unwrap();
            }
            let path = openusd::sdf::path("/Shape").unwrap();
            let extent = |time| {
                let mesh = shape_mesh(&RouteCtx::at(&stage, &path, Some(time))).unwrap().mesh;
                let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
                points.iter().map(|point| Vec3::from_array(*point).abs()).fold(Vec3::ZERO, Vec3::max)
            };
            let start = extent(0.0);
            assert!(extent(5.0).abs_diff_eq(start * 2.0, 1e-5), "{kind}");
            assert!(extent(10.0).abs_diff_eq(start * 3.0, 1e-5), "{kind}");
        }
    }

    #[test]
    fn independent_shape_clocks_propagate_sampled_parent_visibility() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("shape-visibility.usda", &br#"#usda 1.0
def Xform "Group" {
    token visibility.timeSamples = { 0: "inherited", 10: "invisible", 20: "inherited" }
    def Cube "Box" { double size.timeSamples = { 0: 2, 10: 6, 20: 2 } }
}
"#[..]).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.add_plugins(bevy::camera::visibility::VisibilityPlugin);
        app.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let first = app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id();
        let second = app.world_mut().spawn((crate::UsdSceneRoot(handle), UsdInstanceTime { current: 10.0 })).id();
        app.update();
        let box_for = |world: &World, root| world.get_non_send::<UsdInstances>().unwrap().entity(root, "/Group/Box").unwrap();
        let a = box_for(app.world(), first);
        let b = box_for(app.world(), second);
        let half_extent = |world: &World, entity| {
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            points.iter().map(|point| point[0].abs()).fold(0.0_f32, f32::max)
        };
        assert_eq!(half_extent(app.world(), a), 1.0);
        assert_eq!(half_extent(app.world(), b), 3.0);
        assert_eq!(app.world().get::<bevy::camera::primitives::Aabb>(a).unwrap().half_extents.x, 1.0);
        assert_eq!(app.world().get::<bevy::camera::primitives::Aabb>(b).unwrap().half_extents.x, 3.0);
        assert!(app.world().get::<InheritedVisibility>(a).unwrap().get());
        assert!(!app.world().get::<InheritedVisibility>(b).unwrap().get());
        app.world_mut().get_mut::<UsdInstanceTime>(first).unwrap().current = 5.0;
        app.world_mut().get_mut::<UsdInstanceTime>(second).unwrap().current = 20.0;
        app.update();
        assert_eq!(box_for(app.world(), first), a);
        assert_eq!(box_for(app.world(), second), b);
        assert_eq!(half_extent(app.world(), a), 2.0);
        assert_eq!(half_extent(app.world(), b), 1.0);
        assert_eq!(app.world().get::<bevy::camera::primitives::Aabb>(a).unwrap().half_extents.x, 2.0);
        assert_eq!(app.world().get::<bevy::camera::primitives::Aabb>(b).unwrap().half_extents.x, 1.0);
        assert!(app.world().get::<InheritedVisibility>(a).unwrap().get());
        assert!(app.world().get::<InheritedVisibility>(b).unwrap().get());
    }

    #[test]
    fn planes_follow_usd_axis_dimensions_normals_and_sidedness() {
        for (axis, normal, extent) in [("X", Vec3::X, Vec3::new(0.0, 3.0, 2.0)),
            ("Y", Vec3::Y, Vec3::new(2.0, 0.0, 3.0)), ("Z", Vec3::Z, Vec3::new(2.0, 3.0, 0.0))] {
            let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("plane.usda").unwrap();
            stage.define_prim("/Plane").unwrap().set_type_name("Plane").unwrap();
            crate::authoring::set_attribute(&stage, "/Plane", "axis", "token", Value::Token(axis.into())).unwrap();
            crate::authoring::set_attribute(&stage, "/Plane", "width", "double", Value::Double(4.0)).unwrap();
            crate::authoring::set_attribute(&stage, "/Plane", "length", "double", Value::Double(6.0)).unwrap();
            let live = LiveStage::new(stage);
            let mut world = world();
            let mut map = PrimEntities::default();
            project_stage(&mut world, &live, &mut map);
            let entity = map.entity("/Plane").unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&world.get::<Mesh3d>(entity).unwrap().0).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            let actual = positions.iter().map(|p| Vec3::from(*p).abs()).fold(Vec3::ZERO, Vec3::max);
            assert!(actual.abs_diff_eq(extent, 0.00001), "{axis}: {actual:?}");
            let bevy::mesh::VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else { panic!("normals") };
            assert!(normals.iter().all(|n| Vec3::from(*n).abs_diff_eq(normal, 0.00001)));
            let material = world.resource::<Assets<StandardMaterial>>().get(&world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert!(material.double_sided);
            assert!(material.cull_mode.is_none());
            crate::authoring::set_attribute(&live.stage, "/Plane", "doubleSided", "bool", Value::Bool(false)).unwrap();
            crate::live::apply_changes(&mut world, &live, &mut map);
            let material = world.resource::<Assets<StandardMaterial>>().get(&world.get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert!(!material.double_sided);
            assert_eq!(material.cull_mode, Some(bevy::render::render_resource::Face::Back));
        }
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(Assets::<Mesh>::default());
        w.insert_resource(Assets::<StandardMaterial>::default());
        w.insert_resource(SchemaRegistry::builtin());
        w
    }

    #[test]
    fn shapes_project_meshes() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("shapes.usda").unwrap();
        for ty in ["Cube", "Sphere", "Cylinder", "Capsule", "Cone", "Plane"] {
            stage
                .define_prim(format!("/{ty}").as_str())
                .unwrap()
                .set_type_name(ty)
                .unwrap();
        }
        let live = LiveStage::new(stage);
        let mut world = world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        for ty in ["Cube", "Sphere", "Cylinder", "Capsule", "Cone", "Plane"] {
            let e = map.entity(&format!("/{ty}")).unwrap();
            assert!(
                world.get::<Mesh3d>(e).is_some(),
                "{ty} projected a primitive mesh"
            );
        }
    }

    #[test]
    fn cube_size_respected() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cube.usda").unwrap();
        stage
            .define_prim("/C")
            .unwrap()
            .set_type_name("Cube")
            .unwrap();
        stage
            .create_attribute("/C.size", "double")
            .unwrap()
            .set(Value::Double(4.0))
            .unwrap();
        let live = LiveStage::new(stage);
        let mut world = world();
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        let e = map.entity("/C").unwrap();
        let handle = world.get::<Mesh3d>(e).unwrap().0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        // A size-4 cube spans [-2, 2] on each axis.
        let pos = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(v) = pos else {
            panic!("positions");
        };
        let max_x = v.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        assert!((max_x - 2.0).abs() < 1e-4, "cube half-extent from size, got {max_x}");
    }
}
