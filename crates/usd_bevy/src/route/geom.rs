//! Geometry routes: `visibility` → Bevy [`Visibility`], and mesh prims →
//! [`Mesh3d`] + a placeholder [`MeshMaterial3d`]. Real material binding (from
//! `read::shade`) layers on as its own route later (PLAN P4).

use bevy::prelude::*;

use super::{DisplayPurposes, PrimRoute, RouteCtx};
use crate::read::geom::{
    VisibilityState, read_authored_purpose, read_effective_purpose, read_visibility_at,
};

/// The prim's effective (inherited) USD `purpose`: `"default"`, `"render"`,
/// `"proxy"`, or `"guide"`. Carried so gameplay/UI can query or re-filter it.
#[derive(Component, Debug, Clone)]
pub struct UsdPurpose(pub String);

/// Maps `visibility` (+ `purpose`) → [`Visibility`]. Applies to every prim
/// (imageable or not); an unauthored `visibility` reads as inherited/visible.
///
/// A prim is hidden if it is authored `invisible` **or** its effective purpose
/// isn't in the world's [`DisplayPurposes`] (PLAN Phase A) — so `guide`
/// annotations and, by default, the `render` twin of a `proxy`/`render` pair
/// don't draw. `purpose` is inherited down namespace, so this resolves the
/// effective purpose from the nearest ancestor with an authored opinion.
pub struct VisibilityRoute;

/// Combined visibility + effective purpose for `entity`'s prim, honoring the
/// world's [`DisplayPurposes`] (defaults when the resource is absent).
fn resolve(ctx: &RouteCtx, world: &World, entity: Entity) -> (Visibility, String) {
    let purpose = effective_purpose(ctx, world, entity).unwrap_or_else(|_| "default".to_string());
    let purposes = world
        .get_resource::<DisplayPurposes>()
        .copied()
        .unwrap_or_default();
    let invisible = matches!(
        read_visibility_at(ctx.stage, ctx.path, ctx.time),
        Ok(VisibilityState::Invisible)
    );
    let hidden = invisible
        || !purposes.shows(&purpose)
        || super::instancer::is_instancer_prototype(world, entity, ctx.prim_str());
    let vis = if hidden {
        Visibility::Hidden
    } else {
        Visibility::default()
    };
    (vis, purpose)
}

/// The prim's own purpose, else the one its parent prim's entity resolved,
/// else the nearest authored ancestor's.
fn effective_purpose(ctx: &RouteCtx, world: &World, entity: Entity) -> anyhow::Result<String> {
    if let Some(purpose) = read_authored_purpose(ctx.stage, ctx.path)? {
        return Ok(purpose);
    }
    let parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
    let inherited = parent.zip(ctx.path.parent()).and_then(|(parent, path)| {
        let prim = world.get::<crate::prim_ref::UsdPrimRef>(parent)?;
        (prim.path == path.as_str())
            .then(|| world.get::<UsdPurpose>(parent))
            .flatten()
    });
    match inherited {
        Some(purpose) => Ok(purpose.0.clone()),
        None => read_effective_purpose(ctx.stage, ctx.path),
    }
}

fn apply(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let (vis, purpose) = resolve(ctx, world, entity);
    if let Ok(mut e) = world.get_entity_mut(entity) {
        e.insert((vis, UsdPurpose(purpose)));
    }
}

impl PrimRoute for VisibilityRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        apply(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        let touches =
            changed.is_empty() || changed.contains(&"visibility") || changed.contains(&"purpose");
        if !touches {
            return;
        }
        apply(ctx, world, entity);
    }
}

/// Bakes a UsdGeomMesh's points/topology into a Bevy [`Mesh`] and attaches
/// [`Mesh3d`] + a default [`StandardMaterial`]. No-op (with a warning) when the
/// render `Assets` are absent (headless) — the prim still projects, it just
/// carries no renderable geometry.
pub struct MeshRoute;

#[derive(Component, PartialEq, Eq)]
pub(crate) enum GeometryOwner {
    Mesh,
    Shape,
    Points,
    Curves,
}

pub(crate) fn clear_geometry(world: &mut World, entity: Entity, owner: GeometryOwner) {
    if world.get::<GeometryOwner>(entity) != Some(&owner) {
        return;
    }
    super::gpu_skin::clear(world, entity);
    super::flat_material::clear(world, entity);
    super::strand_material::clear(world, entity);
    super::medium_material::clear(world, entity);
    world.entity_mut(entity).remove::<(
        Mesh3d,
        MeshMaterial3d<StandardMaterial>,
        GeometryOwner,
        bevy::camera::primitives::Aabb,
        super::material::UsdMaterialWarning,
    )>();
}

impl MeshRoute {
    /// Bake + attach; returns whether a `Mesh3d` was inserted.
    fn attach(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) -> bool {
        let Ok(Some(read)) = ctx.read_mesh() else {
            clear_geometry(world, entity, GeometryOwner::Mesh);
            return false;
        };
        let limited = super::subdivision::limit_cage(ctx, world, read);
        let read = limited.as_ref().unwrap_or(read);
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            bevy::log::warn!(
                target: "usd_bevy::route::geom",
                "{}: has a mesh but render Assets are absent — not attached",
                ctx.prim_str()
            );
            return false;
        }
        bevy::log::trace!(
            target: "usd_bevy::route::geom",
            "{}: mesh {} points -> Mesh3d",
            ctx.prim_str(),
            read.points.len()
        );
        let assembly =
            if crate::mesh::uses_flat_normals(read) && super::flat_material::enabled(world) {
                super::cache::Assembly::FlatShared
            } else {
                super::cache::Assembly::Standard
            };
        // A bound material that colors the mesh is applied before the mesh is
        // stored, so the uncolored version never reaches the GPU.
        let color = super::material::resolve_material(ctx, world)
            .ok()
            .flatten()
            .map(|(_, _, color)| color)
            .filter(|color| !matches!(color, super::material::MaterialColor::Open));
        let mesh_handle = match color {
            Some(color) => {
                let mut mesh = assembly.build(world, read);
                color.apply(world, &mut mesh, read);
                super::cache::intern_mesh(world, mesh)
            }
            None => super::cache::intern_assembled_mesh(world, read, assembly),
        };
        let material = super::cache::intern_material(world, super::material::default_material(ctx));
        if world.get_entity(entity).is_err() {
            return false;
        }
        super::flat_material::clear(world, entity);
        world.entity_mut(entity).insert((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            GeometryOwner::Mesh,
        ));
        super::flat_material::attach_if_normalless(world, entity);
        true
    }
}

#[cfg(test)]
mod mesh_matching_tests {
    use super::*;
    use crate::read::geom::read_mesh_at;

    #[test]
    fn catmull_clark_cages_draw_at_their_limit_when_asked() {
        let text = r#"#usda 1.0
def Mesh "Cube" {
    point3f[] points = [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]
    int[] faceVertexCounts = [4,4,4,4,4,4]
    int[] faceVertexIndices = [0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7]
}
"#;
        let corner = |limit: bool| {
            let stage = crate::snippet::UsdSnippet::new(text).open_stage().unwrap();
            let mut world = World::new();
            world.init_resource::<Assets<Mesh>>();
            world.init_resource::<Assets<StandardMaterial>>();
            if limit {
                world.init_resource::<super::super::subdivision::UsdSubdivisionLimit>();
            }
            let live = crate::live::LiveStage::new(stage);
            let mut map = crate::live::PrimEntities::default();
            crate::live::project_stage(&mut world, &live, &mut map);
            let mesh = &world.get::<Mesh3d>(map.entity("/Cube").unwrap()).unwrap().0;
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = world
                .resource::<Assets<Mesh>>()
                .get(mesh)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("no positions")
            };
            positions.iter().map(|p| p[0].abs()).fold(0.0f32, f32::max)
        };
        assert_eq!(corner(false), 1.0, "the cage draws as authored");
        assert!(
            (corner(true) - 0.5).abs() < 1e-5,
            "the limit surface is inside"
        );
    }

    #[test]
    fn material_colors_apply_before_the_mesh_is_stored() {
        let stage = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0
def Mesh "Leaf" (prepend apiSchemas = ["MaterialBindingAPI"]) {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
    color3f[] primvars:displayColor = [(1,0,0), (0,1,0), (0,0,1)] (interpolation = "vertex")
    rel material:binding = </Green>
}
def Material "Green" {
    token outputs:surface.connect = </Green/Surface.outputs:surface>
    def Shader "Surface" {
        uniform token info:id = "UsdPreviewSurface"
        color3f inputs:diffuseColor = (0.2, 0.5, 0.1)
        token outputs:surface
    }
}
"#,
        )
        .open_stage()
        .unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let live = crate::live::LiveStage::new(stage);
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let meshes = world.resource::<Assets<Mesh>>();
        assert_eq!(meshes.len(), 1, "only the colored mesh is stored");
        let mesh = &world.get::<Mesh3d>(map.entity("/Leaf").unwrap()).unwrap().0;
        assert!(
            meshes
                .get(mesh)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_COLOR)
                .is_none()
        );
    }

    #[test]
    fn sampled_mesh_geometry_and_primvars_follow_independent_roots() {
        use crate::instance::{UsdInstanceTime, UsdInstances};
        let source = crate::UsdSource::new("mesh-samples.usda", &br#"#usda 1.0
def Mesh "Mesh" {
    point3f[] points.timeSamples = { 0: [(0,0,0), (1,0,0), (0,1,0)], 10: [(0,0,0), (3,0,0), (0,3,0)] }
    int[] faceVertexCounts.timeSamples = { 0: [3], 10: [3,3] }
    int[] faceVertexIndices.timeSamples = { 0: [0,1,2], 10: [0,1,2,2,1,0] }
    int[] holeIndices.timeSamples = { 0: [], 20: [0] }
    normal3f[] normals.timeSamples = { 0: [(0,0,1), (0,0,1), (0,0,1)], 10: [(0,1,0), (0,1,0), (0,1,0)] }
    texCoord2f[] primvars:st ( interpolation = "vertex" )
    texCoord2f[] primvars:st.timeSamples = { 0: [(0,0), (1,0), (0,1)], 10: [(0,0), (0.5,0), (0,0.5)] }
    int[] primvars:st:indices.timeSamples = { 0: [0,1,2], 10: [2,1,0] }
    color3f[] primvars:displayColor ( interpolation = "constant" )
    color3f[] primvars:displayColor.timeSamples = { 0: [(1,0,0)], 10: [(0,0,1)] }
    float[] primvars:displayOpacity ( interpolation = "constant" )
    float[] primvars:displayOpacity.timeSamples = { 0: [1], 10: [0.5] }
    float3[] extent.timeSamples = { 0: [(0,0,0), (1,1,0)], 10: [(0,0,0), (3,3,0)] }
}
def PointInstancer "PI" {
    point3f[] positions = [(0,0,0)]
    int[] protoIndices = [0]
    rel prototypes = [</Mesh>]
}
"#[..]).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Mesh").unwrap();
        assert!(
            crate::read::geom::read_mesh(&stage, &path)
                .unwrap()
                .is_none()
        );
        let middle = read_mesh_at(&stage, &path, Some(5.0)).unwrap().unwrap();
        assert_eq!(middle.points[1], [2.0, 0.0, 0.0]);
        assert_eq!(middle.face_vertex_counts, vec![3]);
        assert_eq!(middle.normals.unwrap().values[0], [0.0, 0.5, 0.5]);
        let uv = middle.uvs.unwrap();
        assert_eq!(uv.values[1], [0.75, 0.0]);
        assert_eq!(uv.indices, vec![0, 1, 2]);
        assert_eq!(middle.display_color.unwrap().values[0], [0.5, 0.0, 0.5]);
        assert_eq!(middle.display_opacity.unwrap().values, vec![0.75]);
        assert_eq!(middle.extent.unwrap()[1], [2.0, 2.0, 0.0]);
        let end = read_mesh_at(&stage, &path, Some(10.0)).unwrap().unwrap();
        assert_eq!(end.face_vertex_indices, vec![0, 1, 2, 2, 1, 0]);
        assert_eq!(end.uvs.unwrap().indices, vec![2, 1, 0]);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::UsdPlugin,
            crate::UsdAssetPlugin,
        ));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene {
                source,
                textures: default(),
            });
        let first = app
            .world_mut()
            .spawn((
                crate::UsdSceneRoot(handle.clone()),
                UsdInstanceTime { current: 0.0 },
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                crate::UsdSceneRoot(handle),
                UsdInstanceTime { current: 10.0 },
            ))
            .id();
        app.update();
        let entities = |world: &World, root| {
            let instances = world.get_non_send::<UsdInstances>().unwrap();
            let mesh = instances.entity(root, "/Mesh").unwrap();
            let parent = instances.entity(root, "/PI").unwrap();
            let child = world
                .get::<Children>(parent)
                .unwrap()
                .iter()
                .find(|child| {
                    world
                        .get::<crate::route::instancer::UsdInstance>(*child)
                        .is_some()
                })
                .unwrap();
            [mesh, child]
        };
        let first_entities = entities(app.world(), first);
        let second_entities = entities(app.world(), second);
        let extent = |world: &World, entity| {
            let mesh = &world.get::<Mesh3d>(entity).unwrap().0;
            let mesh = world.resource::<Assets<Mesh>>().get(mesh).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(points) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!("positions")
            };
            (
                points.iter().map(|point| point[0]).fold(0.0_f32, f32::max),
                mesh.indices()
                    .map_or(mesh.count_vertices(), |indices| indices.len()),
            )
        };
        for entity in first_entities {
            assert_eq!(extent(app.world(), entity), (1.0, 3));
        }
        for entity in second_entities {
            assert_eq!(extent(app.world(), entity), (3.0, 6));
        }
        app.world_mut()
            .get_mut::<UsdInstanceTime>(first)
            .unwrap()
            .current = 5.0;
        app.update();
        assert_eq!(entities(app.world(), first), first_entities);
        assert_eq!(entities(app.world(), second), second_entities);
        for entity in first_entities {
            assert_eq!(extent(app.world(), entity), (2.0, 3));
        }
        for entity in second_entities {
            assert_eq!(extent(app.world(), entity), (3.0, 6));
        }
        app.world_mut()
            .get_mut::<UsdInstanceTime>(first)
            .unwrap()
            .current = 20.0;
        app.update();
        assert_eq!(entities(app.world(), first), first_entities);
        for entity in first_entities {
            assert_eq!(extent(app.world(), entity), (3.0, 3));
        }
        for entity in second_entities {
            assert_eq!(extent(app.world(), entity), (3.0, 6));
        }
    }

    #[test]
    fn changed_geometry_types_remove_only_owned_render_components() {
        #[derive(Component)]
        struct RuntimeOnly;
        let stage =
            crate::UsdSource::new("cleanup.usda", &b"#usda 1.0\ndef Cube \"Shape\" {}\n"[..])
                .unwrap()
                .open_stage()
                .unwrap();
        let live = crate::live::LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Shape").unwrap();
        world.entity_mut(entity).insert(RuntimeOnly);
        let child = world.spawn((RuntimeOnly, ChildOf(entity))).id();
        assert!(world.get::<Mesh3d>(entity).is_some());
        live.stage
            .prim("/Shape")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(map.entity("/Shape"), Some(entity));
        assert!(world.get::<Mesh3d>(entity).is_none());
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_none()
        );
        assert!(world.get::<RuntimeOnly>(entity).is_some());
        assert!(world.get::<RuntimeOnly>(child).is_some());
        live.stage
            .prim("/Shape")
            .unwrap()
            .set_type_name("Sphere")
            .unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert!(world.get::<Mesh3d>(entity).is_some());
        let unrelated = world.spawn(Mesh3d::default()).id();
        let path = openusd::sdf::path("/Shape").unwrap();
        MeshRoute.remove(&RouteCtx::new(&live.stage, &path), &mut world, unrelated);
        assert!(world.get::<Mesh3d>(unrelated).is_some());
    }

    #[test]
    fn metadata_matching_preserves_untyped_and_custom_geometry() {
        let source = crate::UsdSource::new(
            "matching.usda",
            &br#"#usda 1.0
def "Untyped" {
    point3f[] points = [(0,0,0), (1,0,0), (0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
}
def CustomGeometry "Custom" (
    prepend references = </Untyped>
) {}
def Xform "Empty" {}
def Xform "Relationship" {
    rel points = </Untyped>
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
}
"#[..],
        )
        .unwrap();
        let stage = source.open_stage().unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        for name in ["Untyped", "Custom"] {
            let path = openusd::sdf::path(format!("/{name}")).unwrap();
            let ctx = RouteCtx::new(&stage, &path);
            assert!(MeshRoute.matches(&ctx));
            let entity = world.spawn_empty().id();
            MeshRoute.project(&ctx, &mut world, entity);
            let mesh = world
                .resource::<Assets<Mesh>>()
                .get(&world.get::<Mesh3d>(entity).unwrap().0)
                .unwrap();
            assert_eq!(mesh.count_vertices(), 3);
        }
        for name in ["Empty", "Relationship"] {
            let path = openusd::sdf::path(format!("/{name}")).unwrap();
            assert!(!MeshRoute.matches(&RouteCtx::new(&stage, &path)));
        }
    }
}

impl PrimRoute for MeshRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        clear_geometry(world, entity, GeometryOwner::Mesh);
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        if matches!(ctx.type_name.as_deref(), Some("Mesh")) {
            return true;
        }
        let Ok(prim) = ctx.stage.prim(ctx.path.clone()) else {
            return false;
        };
        ["points", "faceVertexCounts", "faceVertexIndices"]
            .into_iter()
            .all(|name| prim.attribute(name).is_defined().unwrap_or(false))
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        self.attach(ctx, world, entity);
    }

    // patch falls back to project (rebuild the mesh). Mesh topology changes
    // arrive via `resynced` in practice, so this is rarely hit on changed_info.
}

#[cfg(test)]
mod purpose_tests {
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::{DisplayPurposes, SchemaRegistry};
    use openusd::usd::Stage;

    fn purpose_stage() -> Stage {
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .in_memory("purpose.usda")
            .unwrap();
        let def = |path: &str, purpose: Option<&str>| {
            stage
                .define_prim(path)
                .unwrap()
                .set_type_name("Xform")
                .unwrap();
            if let Some(p) = purpose {
                stage
                    .create_attribute(format!("{path}.purpose").as_str(), "token")
                    .unwrap()
                    .set(openusd::sdf::Value::Token(p.into()))
                    .unwrap();
            }
        };
        def("/Plain", None);
        def("/Proxy", Some("proxy"));
        def("/Render", Some("render"));
        def("/Guide", Some("guide"));
        // A Scope authored `proxy` — its child inherits proxy (pruning).
        stage
            .define_prim("/Grp")
            .unwrap()
            .set_type_name("Scope")
            .unwrap();
        stage
            .create_attribute("/Grp.purpose", "token")
            .unwrap()
            .set(openusd::sdf::Value::Token("proxy".into()))
            .unwrap();
        stage
            .define_prim("/Grp/Child")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        stage
    }

    fn hidden(world: &World, map: &PrimEntities, path: &str) -> bool {
        let e = map.entity(path).unwrap();
        matches!(world.get::<Visibility>(e), Some(Visibility::Hidden))
    }

    #[test]
    fn default_shows_proxy_hides_render_and_guide() {
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        world.insert_resource(DisplayPurposes::default());
        let live = LiveStage::new(purpose_stage());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        assert!(!hidden(&world, &map, "/Plain"), "default purpose shown");
        assert!(!hidden(&world, &map, "/Proxy"), "proxy shown by default");
        assert!(hidden(&world, &map, "/Render"), "render hidden by default");
        assert!(hidden(&world, &map, "/Guide"), "guide hidden by default");
        // Inherited: child of a proxy Scope resolves to proxy → shown.
        assert!(
            !hidden(&world, &map, "/Grp/Child"),
            "inherits proxy → shown"
        );
        // The effective purpose is carried on the entity.
        let child = map.entity("/Grp/Child").unwrap();
        assert_eq!(world.get::<UsdPurpose>(child).unwrap().0, "proxy");
    }

    #[test]
    fn render_toggle_swaps_proxy_and_render() {
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        // Render-quality viewport: show render, hide proxy.
        world.insert_resource(DisplayPurposes {
            render: true,
            proxy: false,
            guide: false,
        });
        let live = LiveStage::new(purpose_stage());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        assert!(
            !hidden(&world, &map, "/Render"),
            "render shown when toggled on"
        );
        assert!(
            hidden(&world, &map, "/Proxy"),
            "proxy hidden when toggled off"
        );
        assert!(!hidden(&world, &map, "/Plain"), "default always shown");
    }
}
