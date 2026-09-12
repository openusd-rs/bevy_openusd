//! Points route (SCHEMA_INTEGRATION Phase C): `UsdGeomPoints` → a Bevy
//! `PointList` mesh (one vertex per point). Read through the geom `Points` /
//! `PointBased` schema.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use openusd_schemas::geom::{PointBased, Points};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// Maps `UsdGeomPoints` to a point-cloud mesh.
pub struct PointsRoute;

fn positions(ctx: &RouteCtx) -> Option<Vec<[f32; 3]>> {
    let points = Points::get(ctx.stage, ctx.path.clone()).ok()??;
    match points.points_attr().get_at::<Value>(ctx.time.map(openusd::usd::TimeCode::new)) {
        Ok(Some(Value::Vec3fVec(v))) => Some(v.iter().map(|p| [p.x, p.y, p.z]).collect()),
        Ok(Some(Value::Vec3dVec(v))) => {
            Some(v.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect())
        }
        _ => None,
    }
}

impl PrimRoute for PointsRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Points);
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Points")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            return;
        }
        let Some(pos) = positions(ctx) else {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Points);
            return;
        };
        if pos.is_empty() {
            super::geom::clear_geometry(world, entity, super::geom::GeometryOwner::Points);
            return;
        }
        let mut mesh = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
        let color = crate::read::geom::read_primvar_vec3f(ctx.stage, ctx.path, "primvars:displayColor", ctx.time).ok().flatten();
        let opacity = crate::read::geom::read_primvar_float(ctx.stage, ctx.path, "primvars:displayOpacity", ctx.time).ok().flatten();
        if let Some(colors) = crate::mesh::build_vertex_colors(color.as_ref(), opacity.as_ref(), pos.len()) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        let mesh_handle = super::cache::intern_mesh(world, mesh);
        let mut material = super::material::default_material(ctx);
        material.unlit = true;
        let material = super::cache::intern_material(world, material);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material), super::geom::GeometryOwner::Points));
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
    fn point_display_primvars_follow_clocks_and_live_overrides() {
        use crate::instance::{UsdInstances, UsdInstanceTime};
        let source = crate::UsdSource::new("point-colors.usda", include_bytes!("../../../../assets/point_colors.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>().add(crate::UsdScene { source, textures: default() });
        let roots = [0,1].map(|_| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 })).id());
        let check = |app: &App, root, path: &str, expected: Vec<[f32;4]>| {
            let entity = app.world().get_non_send::<UsdInstances>().unwrap().entity(root, path).unwrap();
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(entity).unwrap().0).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
            assert_eq!(colors, &expected);
            let material = app.world().resource::<Assets<StandardMaterial>>().get(&app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0).unwrap();
            assert!(material.unlit);
            assert_eq!(material.base_color.alpha(), 1.0);
            assert_eq!(material.alpha_mode, if expected.iter().any(|color| color[3] < 1.0) { AlphaMode::Blend } else { AlphaMode::Opaque });
            entity
        };
        for times in [[0.0,10.0], [10.0,0.0]] {
            for (root,time) in roots.into_iter().zip(times) { app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = time; }
            app.update();
            for (root,time) in roots.into_iter().zip(times) {
                check(&app, root, "/Root/Inherited", vec![if time == 0.0 { [1.,0.,0.,0.25] } else { [0.,1.,0.,1.] };2]);
                check(&app, root, "/Root/Local", if time == 0.0 { vec![[1.,1.,0.,0.5], [0.,0.,1.,1.]] } else { vec![[0.,0.,1.,0.5], [1.,1.,0.,1.]] });
            }
        }
        let entity = check(&app, roots[0], "/Root/Inherited", vec![[0.,1.,0.,1.];2]);
        let child = app.world_mut().spawn(ChildOf(entity)).id();
        let stage = app.world().get_non_send::<UsdInstances>().unwrap().stage(roots[0]).unwrap().clone();
        stage.attribute("/Root.primvars:displayColor:indices").unwrap().set_at(Value::IntVec(vec![0]), openusd::usd::TimeCode::new(10.0)).unwrap();
        app.update();
        assert_eq!(check(&app, roots[0], "/Root/Inherited", vec![[1.,0.,0.,1.];2]), entity);
        let local = stage.create_attribute("/Root/Inherited.primvars:displayColor", "color3f[]").unwrap();
        local.clone().set(Value::Vec3fVec(vec![[0.,0.,1.].into()])).unwrap();
        app.update();
        check(&app, roots[0], "/Root/Inherited", vec![[0.,0.,1.,1.];2]);
        local.clear().unwrap();
        app.update();
        check(&app, roots[0], "/Root/Inherited", vec![[1.,0.,0.,1.];2]);
        check(&app, roots[1], "/Root/Inherited", vec![[1.,0.,0.,0.25];2]);
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), entity);
    }

    #[test]
    fn points_project_pointlist_mesh() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("pts.usda").unwrap();
        stage
            .define_prim("/Pts")
            .unwrap()
            .set_type_name("Points")
            .unwrap();
        stage
            .create_attribute("/Pts.points", "point3f[]")
            .unwrap()
            .set(Value::Vec3fVec(vec![
                [0.0, 0.0, 0.0].into(),
                [1.0, 1.0, 1.0].into(),
            ]))
            .unwrap();

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/Pts").unwrap();
        let handle = world.get::<Mesh3d>(e).expect("points mesh").0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle).unwrap();
        assert_eq!(mesh.primitive_topology(), PrimitiveTopology::PointList);
        assert_eq!(mesh.count_vertices(), 2);
    }
}
