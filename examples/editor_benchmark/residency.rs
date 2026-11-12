use super::*;
use std::{collections::BTreeMap, ffi::OsStr};
use usd_bevy::{
    live::PrimEntities,
    route::{gpu_skin, subset::UsdSubset},
};

type Failure = Box<dyn std::error::Error>;
type MeshKey = (String, Option<String>);

pub(super) fn reverse(value: Option<&OsStr>) -> Result<bool, Failure> {
    match value {
        None => Ok(false),
        Some(value) if value == "0" => Ok(false),
        Some(value) if value == "1" => Ok(true),
        _ => Err("USD_BENCH_PAIR_REVERSE must be 0 or 1".into()),
    }
}

fn hidden(world: &World, mut entity: Entity) -> Result<bool, Failure> {
    for _ in 0..128 {
        let current = world.get_entity(entity)?;
        match current.get::<Visibility>() {
            Some(Visibility::Hidden) => return Ok(true),
            Some(Visibility::Visible) | None => return Ok(false),
            Some(Visibility::Inherited) => (),
        }
        let Some(parent) = current.get::<ChildOf>() else {
            return Ok(false);
        };
        entity = parent.parent();
    }
    Err("visible comparison cannot resolve a cyclic or deep hierarchy".into())
}

fn render_meshes(world: &mut World) -> Result<BTreeMap<MeshKey, Entity>, Failure> {
    let entities: Vec<_> = world
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(world)
        .collect();
    let mut result = BTreeMap::new();
    for entity in entities {
        if hidden(world, entity)? {
            continue;
        }
        let key = if let Some(prim) = world.get::<usd_bevy::UsdPrimRef>(entity) {
            (prim.path.clone(), None)
        } else if let Some(subset) = world.get::<UsdSubset>(entity) {
            let parent = world
                .get::<ChildOf>(entity)
                .and_then(|parent| world.get::<usd_bevy::UsdPrimRef>(parent.parent()))
                .ok_or("subset lacks a USD parent")?;
            (parent.path.clone(), Some(subset.0.clone()))
        } else {
            return Err("render mesh lacks stable USD identity".into());
        };
        if result.insert(key, entity).is_some() {
            return Err("duplicate render mesh identity".into());
        }
    }
    Ok(result)
}

fn mesh(world: &World, entity: Entity) -> Result<&Mesh, Failure> {
    let handle = &world
        .get::<Mesh3d>(entity)
        .ok_or("missing mesh component")?
        .0;
    world
        .resource::<Assets<Mesh>>()
        .get(handle)
        .ok_or_else(|| "missing mesh asset".into())
}

fn morph_weights(world: &World, entity: Entity) -> Result<Option<&[f32]>, Failure> {
    use bevy::mesh::morph::MeshMorphWeights;
    match world.get::<MeshMorphWeights>(entity) {
        None => Ok(None),
        Some(MeshMorphWeights::Value { weights }) => Ok(Some(weights)),
        _ => Err("visible comparison requires explicit morph weights".into()),
    }
}

pub(super) fn compare_visible(a: &mut App, b: &mut App) -> Result<usize, Failure> {
    let prims = a.world().resource::<PrimEntities>();
    let other = b.world().resource::<PrimEntities>();
    if prims.len() != other.len() {
        return Err("logical prim counts differ".into());
    }
    for ((path, ae), (other, be)) in prims.iter().zip(other.iter()) {
        if path != other
            || a.world().get::<Transform>(ae) != b.world().get::<Transform>(be)
            || a.world().get::<Visibility>(ae) != b.world().get::<Visibility>(be)
        {
            return Err(format!("logical transform/visibility differs at {path}").into());
        }
    }
    let meshes = render_meshes(a.world_mut())?;
    let other = render_meshes(b.world_mut())?;
    if meshes.keys().ne(other.keys()) {
        return Err("visible mesh identities differ".into());
    }
    for (key, ae) in &meshes {
        let be = other[key];
        let (am, bm) = (mesh(a.world(), *ae)?, mesh(b.world(), be)?);
        if am.primitive_topology() != bm.primitive_topology()
            || am.get_index_buffer_bytes() != bm.get_index_buffer_bytes()
            || am.indices().map(|indices| indices.len())
                != bm.indices().map(|indices| indices.len())
            || am.attributes().count() != bm.attributes().count()
            || am.get_morph_targets().map(bytemuck::cast_slice::<_, u8>)
                != bm.get_morph_targets().map(bytemuck::cast_slice::<_, u8>)
            || am.morph_target_names() != bm.morph_target_names()
        {
            return Err(format!("visible topology/morph data differs at {key:?}").into());
        }
        for (attribute, values) in am.attributes() {
            if Some(values.get_bytes())
                != bm.attribute(attribute.id).map(|values| values.get_bytes())
            {
                return Err(format!("visible {} differs at {key:?}", attribute.name).into());
            }
        }
        if a.world()
            .get::<gpu_skin::UsdGpuSkin>(*ae)
            .map(|skin| &skin.matrices)
            != b.world()
                .get::<gpu_skin::UsdGpuSkin>(be)
                .map(|skin| &skin.matrices)
            || morph_weights(a.world(), *ae)? != morph_weights(b.world(), be)?
            || a.world()
                .get::<gpu_skin::UsdCpuSkinFallback>(*ae)
                .map(|fallback| &fallback.0)
                != b.world()
                    .get::<gpu_skin::UsdCpuSkinFallback>(be)
                    .map(|fallback| &fallback.0)
            || a.world()
                .get::<usd_bevy::route::skel::UsdDeformationError>(*ae)
                != b.world()
                    .get::<usd_bevy::route::skel::UsdDeformationError>(be)
        {
            return Err(format!("visible deformation state differs at {key:?}").into());
        }
    }
    Ok(meshes.len())
}

#[test]
fn residency_probe_keeps_visible_geometry_and_changes_only_initial_residency() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.usda");
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skel_morph_blended_animated.usda");
    std::fs::write(
        &path,
        format!(
            r#"#usda 1.0
(startTimeCode = 0; endTimeCode = 10)
def "Hidden" (references = @{}@</Test>) {{ token visibility = "invisible" }}
def "Visible" (references = @{}@</Test>) {{}}
"#,
            source.display(),
            source.display()
        ),
    )
    .unwrap();
    for gpu in [false, true] {
        for reverse in [false, true] {
            let [eager, deferred] = paired::run_residency(&path, gpu, 8, reverse).unwrap();
            assert_eq!(eager.mesh_entities, 4);
            assert_eq!(eager.subset_entities, 2);
            assert_eq!(deferred.mesh_entities, 2);
            assert_eq!(deferred.subset_entities, 1);
        }
    }
    let (mut eager, _, _) = open_editor_with_deferral(&path, true, None, false).unwrap();
    let (mut deferred, _, _) = open_editor_with_deferral(&path, true, None, true).unwrap();
    assert_eq!(compare_visible(&mut eager, &mut deferred).unwrap(), 2);
    let entity = deferred
        .world()
        .resource::<PrimEntities>()
        .entity("/Visible/Face")
        .unwrap();
    let handle = deferred
        .world_mut()
        .entity_mut(entity)
        .take::<Mesh3d>()
        .unwrap();
    assert!(compare_visible(&mut eager, &mut deferred).is_err());
    deferred
        .world_mut()
        .entity_mut(entity)
        .insert(handle.clone());
    {
        let mut meshes = deferred.world_mut().resource_mut::<Assets<Mesh>>();
        let mut mesh = meshes.get_mut(&handle.0).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(points) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!()
        };
        points[0][0] += 1.0;
    }
    assert!(compare_visible(&mut eager, &mut deferred).is_err());
    assert!(!reverse(None).unwrap());
    assert!(!reverse(Some(OsStr::new("0"))).unwrap());
    assert!(reverse(Some(OsStr::new("1"))).unwrap());
    assert!(reverse(Some(OsStr::new("true"))).is_err());
}
