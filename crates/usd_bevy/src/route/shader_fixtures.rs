use super::{
    flat_material::FlatMaterial,
    medium_material::MediumMaterial,
    sheen_material::SheenMaterial,
    strand_material::{ATTRIBUTE_STRAND_NORMAL, StrandMaterial},
};
use bevy::prelude::*;

#[test]
fn render_fixture_selects_custom_materials_and_gpu_instancing() {
    let stage =
        crate::snippet::UsdSnippet::new(include_str!("../../../../assets/wesl_materials.usda"))
            .open_stage()
            .unwrap();
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin);
    app.insert_resource(super::gpu_instancing::UsdGpuInstancing {
        threshold: 0,
        min_pixels: 0.0,
        shadows: true,
    });
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<FlatMaterial>>()
        .init_resource::<Assets<MediumMaterial>>()
        .init_resource::<Assets<SheenMaterial>>()
        .init_resource::<Assets<StrandMaterial>>();
    let world = app.world_mut();
    let live = crate::live::LiveStage::new(stage);
    let mut entities = crate::live::PrimEntities::default();
    crate::live::project_stage(world, &live, &mut entities);
    assert_eq!(
        world
            .query::<&super::gpu_instancing::UsdInstancedDraw>()
            .iter(world)
            .count(),
        1
    );
    let flat = entities.entity("/Flat").unwrap();
    assert!(world.get::<MeshMaterial3d<FlatMaterial>>(flat).is_some());
    let sheen = entities.entity("/Sheen").unwrap();
    assert!(world.get::<MeshMaterial3d<SheenMaterial>>(sheen).is_some());
    let medium = entities.entity("/Medium").unwrap();
    let material = &world
        .get::<MeshMaterial3d<MediumMaterial>>(medium)
        .unwrap()
        .0;
    let material = world
        .resource::<Assets<MediumMaterial>>()
        .get(material)
        .unwrap();
    assert!(material.base.specular_transmission > 0.0);
    assert!(material.base.attenuation_distance.is_finite());
    for (path, oriented) in [("/RoundStrands", false), ("/OrientedStrands", true)] {
        let entity = entities.entity(path).unwrap();
        assert!(
            world
                .get::<MeshMaterial3d<StrandMaterial>>(entity)
                .is_some()
        );
        let mesh = &world.get::<Mesh3d>(entity).unwrap().0;
        let mesh = world.resource::<Assets<Mesh>>().get(mesh).unwrap();
        assert_eq!(mesh.attribute(ATTRIBUTE_STRAND_NORMAL).is_some(), oriented);
    }
}
