use super::*;

#[derive(Component)]
struct RuntimeOnly;

#[derive(Resource, Default)]
struct ParentDiscards(usize);

fn source() -> openusd::usd::Stage {
    crate::UsdSource::new(
        "parenting.usda",
        &br#"#usda 1.0
def PointInstancer "PI" {
    point3f[] positions = [(1,0,0), (2,0,0), (3,0,0)]
    int[] protoIndices = [0,0,0]
    int64[] ids = [10,20,30]
}
"#[..],
    )
    .unwrap()
    .open_stage()
    .unwrap()
}

fn project(stage: &openusd::usd::Stage, world: &mut World, parent: Entity) {
    PointInstancerRoute.project(
        &RouteCtx::at(stage, &openusd::sdf::path("/PI").unwrap(), Some(0.0)),
        world,
        parent,
    );
}

fn set_ids(stage: &openusd::usd::Stage, ids: &[i64]) {
    for (name, ty, value) in [
        ("ids", "int64[]", Value::Int64Vec(ids.to_vec())),
        ("protoIndices", "int[]", Value::IntVec(vec![0; ids.len()])),
        (
            "positions",
            "point3f[]",
            Value::Vec3fVec(
                (0..ids.len())
                    .map(|index| [index as f32, 0.0, 0.0].into())
                    .collect(),
            ),
        ),
    ] {
        crate::authoring::set_attribute(stage, "/PI", name, ty, value).unwrap();
    }
    stage.flush().unwrap();
}

fn children(world: &World, parent: Entity) -> Vec<Entity> {
    world
        .get::<Children>(parent)
        .into_iter()
        .flat_map(|children| children.iter())
        .collect()
}

#[test]
fn reconciliation_preserves_order_identity_and_only_discards_removed_links() {
    let stage = source();
    let mut world = World::new();
    world.init_resource::<ParentDiscards>();
    world.add_observer(
        |_: On<Discard<ChildOf>>, mut discards: ResMut<ParentDiscards>| {
            discards.0 += 1;
        },
    );
    let parent = world.spawn_empty().id();
    let prefix = world.spawn((RuntimeOnly, ChildOf(parent))).id();
    project(&stage, &mut world, parent);
    let initial = children(&world, parent);
    let ten = initial[1];
    world.entity_mut(ten).insert(RuntimeOnly);
    let tail = world.spawn((RuntimeOnly, ChildOf(parent))).id();
    let mut previous: HashMap<i64, Entity> = [10, 20, 30]
        .into_iter()
        .zip(initial.into_iter().skip(1))
        .collect();
    for ids in [
        &[10, 20, 30][..],
        &[20, 10, 30],
        &[40, 30, 20, 10],
        &[10, 20],
        &[20, 10],
        &[10, 20],
    ] {
        set_ids(&stage, ids);
        world.resource_mut::<ParentDiscards>().0 = 0;
        project(&stage, &mut world, parent);
        assert!(world.get::<UsdInstancerWarning>(parent).is_none());
        let actual = children(&world, parent);
        assert_eq!(&actual[..2], &[prefix, tail]);
        assert_eq!(actual.len(), ids.len() + 2);
        let mut next = HashMap::default();
        for (index, (&id, &child)) in ids.iter().zip(&actual[2..]).enumerate() {
            assert_eq!(world.get::<UsdInstanceId>(child), Some(&UsdInstanceId(id)));
            assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), parent);
            assert_eq!(
                world.get::<Transform>(child).unwrap().translation.x,
                index as f32
            );
            if let Some(&old) = previous.get(&id) {
                assert_eq!(child, old);
            }
            next.insert(id, child);
        }
        let removed: Vec<_> = previous
            .iter()
            .filter(|(id, _)| !next.contains_key(*id))
            .map(|(_, entity)| *entity)
            .collect();
        assert_eq!(world.resource::<ParentDiscards>().0, removed.len());
        assert!(
            removed
                .iter()
                .all(|entity| world.get_entity(*entity).is_err())
        );
        assert!(world.get::<RuntimeOnly>(ten).is_some());
        previous = next;
    }
    let before_invalid = children(&world, parent);
    set_ids(&stage, &[10, 10]);
    project(&stage, &mut world, parent);
    assert!(world.get::<UsdInstancerWarning>(parent).is_some());
    assert_eq!(children(&world, parent), before_invalid);
    PointInstancerRoute::clear_instances(&mut world, parent);
    assert_eq!(children(&world, parent), [prefix, tail]);
    assert!(
        previous
            .values()
            .all(|entity| world.get_entity(*entity).is_err())
    );
}

#[test]
fn clearing_large_sibling_sets_retains_unmanaged_children_and_cleans_descendants() {
    let mut world = World::new();
    let parent = world.spawn_empty().id();
    let unrelated = world.spawn((RuntimeOnly, ChildOf(parent))).id();
    let mut owned = Vec::new();
    for id in 0..2048 {
        let child = world
            .spawn((UsdInstance, UsdInstanceId(id), ChildOf(parent)))
            .id();
        let descendant = world.spawn((RuntimeOnly, ChildOf(child))).id();
        owned.extend([child, descendant]);
    }
    PointInstancerRoute::clear_instances(&mut world, parent);
    assert_eq!(children(&world, parent), [unrelated]);
    assert!(
        owned
            .iter()
            .all(|entity| world.get_entity(*entity).is_err())
    );
    assert!(world.get::<RuntimeOnly>(unrelated).is_some());
    PointInstancerRoute::clear_instances(&mut world, parent);
    world.despawn(unrelated);
    PointInstancerRoute::clear_instances(&mut world, parent);
    assert!(children(&world, parent).is_empty());
}
