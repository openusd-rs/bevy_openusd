//! Projected-mesh cache (PLAN Phase 6d): BSN's copy-on-write analog for USD.
//!
//! Every mesh-producing route builds a fresh [`Mesh`] and would otherwise
//! `Assets::add` it on each projection — so N identical prototype prims (a
//! kitbashed scene) allocate N identical GPU meshes, and re-projecting the same
//! prim mints a new handle each time. This resource interns meshes by a hash of
//! their geometry: identical content resolves to one shared [`Handle<Mesh>`].
//!
//! The cache is **opt-in**: when the resource is absent (a bare test `World`)
//! [`intern_mesh`] falls back to a plain `add`, so routes work either way.
//! [`crate::UsdPlugin`] inserts it.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use bevy::platform::hash::FixedHasher;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct MaterialCache {
    materials: HashMap<u64, Vec<Handle<StandardMaterial>>>,
    count: usize,
}

pub(crate) fn externally_owned<A: Asset>(handle: &Handle<A>) -> bool {
    match handle {
        Handle::Strong(strong) => std::sync::Arc::strong_count(strong) > 1,
        Handle::Uuid(..) => true,
    }
}

pub(crate) fn prune_material_cache(cache: Option<ResMut<MaterialCache>>, assets: Option<Res<Assets<StandardMaterial>>>) {
    let (Some(mut cache), Some(assets)) = (cache, assets) else { return };
    cache.materials.retain(|_, candidates| {
        candidates.retain(|handle| assets.contains(handle) && externally_owned(handle));
        !candidates.is_empty()
    });
    cache.count = cache.materials.values().map(Vec::len).sum();
}

/// Shares fully equal materials while retaining at most 1024 cached handles.
pub fn intern_material(world: &mut World, material: StandardMaterial) -> Handle<StandardMaterial> {
    if !world.contains_resource::<MaterialCache>() {
        return world.resource_mut::<Assets<StandardMaterial>>().add(material);
    }
    let mut hash = FixedHasher.build_hasher();
    format!("{material:?}").hash(&mut hash);
    let signature = hash.finish();
    if let Some(candidates) = world.resource::<MaterialCache>().materials.get(&signature) {
        let assets = world.resource::<Assets<StandardMaterial>>();
        for handle in candidates {
            if assets.get(handle).is_some_and(|existing| existing.cull_mode == material.cull_mode
                && existing.reflect_partial_eq(&material) == Some(true)) {
                return handle.clone();
            }
        }
    }
    let handle = world.resource_mut::<Assets<StandardMaterial>>().add(material);
    let mut cache = world.resource_mut::<MaterialCache>();
    if cache.count >= 1024 { cache.materials.clear(); cache.count = 0; }
    cache.materials.entry(signature).or_default().push(handle.clone());
    cache.count += 1;
    handle
}

/// Maximum retained entries, independently of the configured payload budget.
const MAX_INTERNED: usize = 8192;

/// Interns projected meshes by geometry signature so identical prims share one
/// [`Handle<Mesh>`]. Insert via [`crate::UsdPlugin`]; absent ⇒ no interning.
///
/// `UsdPlugin` releases cache-only handles in `Last`. Meshes owned by entities
/// or external strong handles remain eligible for sharing, subject to budgets.
#[derive(Resource)]
pub struct ProjectionCache {
    meshes: HashMap<u64, Vec<(Handle<Mesh>, usize)>>,
    count: usize,
    payload_bytes: usize,
    byte_budget: usize,
}

impl Default for ProjectionCache {
    fn default() -> Self { Self::with_byte_budget(256 * 1024 * 1024) }
}

impl ProjectionCache {
    /// Limits retained mesh payload bytes; zero disables new cache entries.
    pub fn with_byte_budget(byte_budget: usize) -> Self {
        Self { meshes: HashMap::new(), count: 0, payload_bytes: 0, byte_budget }
    }

    /// Payload bytes measured at insertion, excluding allocator and GPU overhead.
    pub fn retained_payload_bytes(&self) -> usize { self.payload_bytes }

    /// Number of distinct meshes currently interned.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the cache holds no interned meshes.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    fn retain_live(&mut self, assets: &Assets<Mesh>) {
        self.count = 0;
        self.payload_bytes = 0;
        self.meshes.retain(|_, candidates| {
            candidates.retain(|(handle, _)| assets.get(handle).is_some() && externally_owned(handle));
            self.count += candidates.len();
            self.payload_bytes += candidates.iter().map(|(_, bytes)| bytes).sum::<usize>();
            !candidates.is_empty()
        });
    }
}

pub(crate) fn prune_mesh_cache(cache: Option<ResMut<ProjectionCache>>, assets: Option<Res<Assets<Mesh>>>) {
    if let (Some(mut cache), Some(assets)) = (cache,assets) { cache.retain_live(&assets); }
}

/// Add `mesh` to `Assets<Mesh>`, reusing an existing handle when a mesh with
/// identical geometry remains interned. Falls back to a plain
/// `add` when there is no [`ProjectionCache`] resource.
pub fn intern_mesh(world: &mut World, mesh: Mesh) -> Handle<Mesh> {
    // No cache resource → behave exactly like `Assets::add`.
    if world.get_resource::<ProjectionCache>().is_none() {
        return world.resource_mut::<Assets<Mesh>>().add(mesh);
    }
    let payload_bytes = mesh_payload_bytes(&mesh);
    let budget = world.resource::<ProjectionCache>().byte_budget;
    if budget == 0 || payload_bytes > budget {
        return world.resource_mut::<Assets<Mesh>>().add(mesh);
    }
    let sig = mesh_signature(&mesh);
    if let Some(candidates) = world
        .resource::<ProjectionCache>()
        .meshes
        .get(&sig)
    {
        let assets = world.resource::<Assets<Mesh>>();
        for (existing, _) in candidates {
            if assets.get(existing).is_some_and(|cached| meshes_equal(cached, &mesh)) {
                return existing.clone();
            }
        }
    }
    let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let mut cache = world.resource_mut::<ProjectionCache>();
    // Release cached handles when either retention limit would be exceeded.
    if cache.len() >= MAX_INTERNED || payload_bytes > cache.byte_budget.saturating_sub(cache.payload_bytes) {
        cache.meshes.clear();
        cache.count = 0;
        cache.payload_bytes = 0;
    }
    cache.meshes.entry(sig).or_default().push((handle.clone(),payload_bytes));
    cache.count += 1;
    cache.payload_bytes += payload_bytes;
    handle
}

fn mesh_payload_bytes(mesh: &Mesh) -> usize {
    let attributes = mesh.attributes().fold(0usize, |total, (_, values)| total.saturating_add(values.get_bytes().len()));
    let indices = mesh.get_index_buffer_bytes().map_or(0, |bytes| bytes.len());
    let morph = mesh.get_morph_targets().map_or(0, std::mem::size_of_val);
    let names = mesh.morph_target_names().map_or(0, |names| names.iter().fold(0usize, |total, name| total.saturating_add(name.len())));
    attributes.saturating_add(indices).saturating_add(morph).saturating_add(names)
}

/// Hashes topology, index representation and every vertex attribute's bytes.
fn mesh_signature(mesh: &Mesh) -> u64 {
    let mut h = FixedHasher.build_hasher();
    std::mem::discriminant(&mesh.primitive_topology()).hash(&mut h);
    match mesh.indices() {
        Some(bevy::mesh::Indices::U16(v)) => {
            0u8.hash(&mut h);
            v.hash(&mut h);
        }
        Some(bevy::mesh::Indices::U32(v)) => {
            1u8.hash(&mut h);
            v.hash(&mut h);
        }
        None => 2u8.hash(&mut h),
    }
    mesh.asset_usage.hash(&mut h);
    mesh.enable_raytracing.hash(&mut h);
    for (attribute, values) in mesh.attributes() {
        attribute.id.hash(&mut h);
        attribute.format.hash(&mut h);
        values.get_bytes().hash(&mut h);
    }
    mesh.morph_target_names().hash(&mut h);
    mesh.get_morph_targets().map(|targets| targets.len()).hash(&mut h);
    if let Some(targets) = mesh.get_morph_targets() {
        for target in targets {
            for vector in [target.position, target.normal, target.tangent] {
                vector.to_array().map(f32::to_bits).hash(&mut h);
            }
        }
    }
    h.finish()
}

fn meshes_equal(a: &Mesh, b: &Mesh) -> bool {
    if a.try_attributes().is_err() || a.try_indices_option().is_err() {
        return false;
    }
    a.primitive_topology() == b.primitive_topology()
        && a.asset_usage == b.asset_usage
        && a.enable_raytracing == b.enable_raytracing
        && a.morph_target_names() == b.morph_target_names()
        && a.get_morph_targets() == b.get_morph_targets()
        && a.indices().map(std::mem::discriminant) == b.indices().map(std::mem::discriminant)
        && a.get_index_buffer_bytes() == b.get_index_buffer_bytes()
        && a.attributes().count() == b.attributes().count()
        && a.attributes().zip(b.attributes()).all(|((a, av), (b, bv))|
            a.id == b.id && a.format == b.format && av.get_bytes() == bv.get_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_materials_preserves_sharing_and_external_ownership() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<MaterialCache>();
        let first = intern_material(&mut world, StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(first.clone())).id();
        drop(intern_material(&mut world, StandardMaterial { perceptual_roughness: 0.17, ..default() }));
        world.run_system_once(prune_material_cache).unwrap();
        assert_eq!(world.resource::<MaterialCache>().count, 1);
        assert_eq!(intern_material(&mut world, StandardMaterial::default()), first);
        world.despawn(entity);
        world.run_system_once(prune_material_cache).unwrap();
        assert_eq!(world.resource::<MaterialCache>().count, 1);
        drop(first);
        world.run_system_once(prune_material_cache).unwrap();
        assert_eq!(world.resource::<MaterialCache>().count, 0);
        assert!(world.resource::<MaterialCache>().materials.is_empty());
        let stale = intern_material(&mut world, StandardMaterial::default());
        world.resource_mut::<Assets<StandardMaterial>>().remove(stale.id());
        world.run_system_once(prune_material_cache).unwrap();
        assert_eq!(world.resource::<MaterialCache>().count, 0);
    }

    #[test]
    fn pruning_releases_only_cache_owned_meshes_and_preserves_sharing() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<ProjectionCache>();
        let mesh = Mesh::from(Cuboid::from_length(1.0));
        let bytes = mesh_payload_bytes(&mesh);
        let first = intern_mesh(&mut world,mesh.clone());
        let entity = world.spawn(Mesh3d(first.clone())).id();
        drop(intern_mesh(&mut world,Mesh::from(Cuboid::from_length(2.0))));
        let prune = |world: &mut World| world.resource_scope(|world,mut cache: Mut<ProjectionCache>| cache.retain_live(world.resource::<Assets<Mesh>>()));
        prune(&mut world);
        assert_eq!(world.resource::<ProjectionCache>().len(),1);
        assert_eq!(world.resource::<ProjectionCache>().retained_payload_bytes(),bytes);
        assert_eq!(intern_mesh(&mut world,mesh.clone()),first);
        world.despawn(entity);
        prune(&mut world);
        assert_eq!(world.resource::<ProjectionCache>().len(),1);
        drop(first);
        prune(&mut world);
        assert!(world.resource::<ProjectionCache>().is_empty());
        assert_eq!(world.resource::<ProjectionCache>().retained_payload_bytes(),0);
        let stale = intern_mesh(&mut world,mesh);
        world.resource_mut::<Assets<Mesh>>().remove(stale.id());
        prune(&mut world);
        assert!(world.resource::<ProjectionCache>().is_empty());
    }

    #[test]
    fn mesh_byte_budget_bounds_retention_without_removing_live_assets() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        let mesh = Mesh::from(Cuboid::from_length(1.0));
        let bytes = mesh_payload_bytes(&mesh);
        assert!(bytes > 0);
        world.insert_resource(ProjectionCache::with_byte_budget(bytes));
        let first = intern_mesh(&mut world, mesh.clone());
        assert_eq!(intern_mesh(&mut world, mesh.clone()), first);
        assert_eq!(world.resource::<ProjectionCache>().retained_payload_bytes(), bytes);
        let changed = Mesh::from(Cuboid::from_length(2.0));
        let second = intern_mesh(&mut world, changed);
        assert_ne!(first, second);
        assert_eq!(world.resource::<ProjectionCache>().len(), 1);
        assert_eq!(world.resource::<ProjectionCache>().retained_payload_bytes(), bytes);
        assert!(world.resource::<Assets<Mesh>>().get(&first).is_some());
        world.insert_resource(ProjectionCache::with_byte_budget(bytes - 1));
        assert_ne!(intern_mesh(&mut world, mesh.clone()), intern_mesh(&mut world, mesh.clone()));
        assert!(world.resource::<ProjectionCache>().is_empty());
        assert_eq!(world.resource::<ProjectionCache>().retained_payload_bytes(), 0);
        world.insert_resource(ProjectionCache::with_byte_budget(0));
        assert_ne!(intern_mesh(&mut world, mesh.clone()), intern_mesh(&mut world, mesh));
        assert!(world.resource::<ProjectionCache>().is_empty());
    }

    #[test]
    fn mesh_payload_accounts_for_indices_attributes_and_morph_data() {
        let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, default());
        assert_eq!(mesh_payload_bytes(&mesh), 0);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0_f32;3];3]);
        mesh.insert_indices(bevy::mesh::Indices::U16(vec![0,1,2]));
        assert_eq!(mesh_payload_bytes(&mesh), 36 + 6);
        mesh.set_morph_targets(vec![bevy::mesh::morph::MorphAttributes::default();3]);
        mesh.set_morph_target_names(vec!["shape".into()]);
        assert_eq!(mesh_payload_bytes(&mesh), 42 + 3 * std::mem::size_of::<bevy::mesh::morph::MorphAttributes>() + 5);
    }

    #[test]
    fn morph_targets_and_names_participate_in_cache_identity() {
        use bevy::mesh::morph::MorphAttributes;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<ProjectionCache>();
        let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, default());
        let plain = intern_mesh(&mut world, mesh.clone());
        let mut morph = mesh.clone();
        morph.set_morph_targets(vec![MorphAttributes::default()]);
        let zero = intern_mesh(&mut world, morph.clone());
        assert_ne!(plain, zero);
        assert_eq!(intern_mesh(&mut world, morph.clone()), zero);
        for target in [MorphAttributes::new(Vec3::X, Vec3::ZERO, Vec3::ZERO),
            MorphAttributes::new(Vec3::ZERO, Vec3::X, Vec3::ZERO),
            MorphAttributes::new(Vec3::ZERO, Vec3::ZERO, Vec3::X)] {
            let mut changed = mesh.clone();
            changed.set_morph_targets(vec![target]);
            assert_ne!(intern_mesh(&mut world, changed), zero);
        }
        morph.set_morph_target_names(vec!["shape".into()]);
        assert_ne!(intern_mesh(&mut world, morph), zero);
    }

    #[test]
    fn mutated_cull_mode_does_not_contaminate_cached_materials() {
        let mut world = World::new();
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.init_resource::<MaterialCache>();
        let original = intern_material(&mut world, StandardMaterial::default());
        world.resource_mut::<Assets<StandardMaterial>>().get_mut(&original).unwrap().cull_mode = None;
        let clean = intern_material(&mut world, StandardMaterial::default());
        assert_ne!(original, clean);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(&original).unwrap().cull_mode, None);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(&clean).unwrap().cull_mode, StandardMaterial::default().cull_mode);
    }

    #[test]
    fn materials_share_only_when_all_reflected_fields_match() {
        let mut world = World::new();
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.init_resource::<MaterialCache>();
        let first = intern_material(&mut world, StandardMaterial::default());
        assert_eq!(intern_material(&mut world, StandardMaterial::default()), first);
        world.resource_mut::<Assets<StandardMaterial>>().get_mut(&first).unwrap().perceptual_roughness = 0.1;
        let second = intern_material(&mut world, StandardMaterial::default());
        assert_ne!(first, second);
        let textured = StandardMaterial { base_color_texture: Some(Handle::default()), ..default() };
        let third = intern_material(&mut world, textured.clone());
        assert_ne!(second, third);
        assert_eq!(intern_material(&mut world, textured), third);
    }
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    use openusd::usd::Stage;

    #[test]
    fn native_instance_children_project_and_share_geometry() {
        let mut text = String::from("#usda 1.0\ndef Xform \"Template\" { def Cube \"Shape\" {} }\n");
        for i in 0..128 {
            text.push_str(&format!("def Xform \"Instance{i}\" (instanceable = true\n prepend references = </Template>) {{}}\n"));
        }
        let source = crate::UsdSource::new("native.usda", text.into_bytes()).unwrap();
        let stage = source.open_stage().unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(ProjectionCache::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        assert_eq!(live.stage.prototypes().unwrap().len(), 1);
        let template = world.get::<Mesh3d>(map.entity("/Template/Shape").unwrap()).unwrap().0.clone();
        for i in 0..128 {
            let parent = map.entity(&format!("/Instance{i}")).unwrap();
            let shape = map.entity(&format!("/Instance{i}/Shape")).expect("instance proxy geometry projected");
            assert!(world.get::<super::super::native::UsdNativeInstance>(parent).is_some());
            assert!(world.get::<super::super::native::UsdInstanceProxy>(shape).is_some());
            assert_eq!(world.get::<Mesh3d>(shape).unwrap().0, template);
            assert_eq!(world.get::<ChildOf>(shape).unwrap().parent(), parent);
        }
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
        assert_eq!(world.resource::<ProjectionCache>().len(), 1);
        let proxy_entity = map.entity("/Instance0/Shape").unwrap();
        crate::authoring::set_attribute(&live.stage, "/Template/Shape", "size", "double", openusd::sdf::Value::Double(4.0)).unwrap();
        crate::live::apply_changes(&mut world, &live, &mut map);
        assert_eq!(map.entity("/Instance0/Shape"), Some(proxy_entity));
        let updated = world.get::<Mesh3d>(proxy_entity).unwrap().0.clone();
        assert_ne!(updated, template);
        for i in 0..128 {
            let proxy = map.entity(&format!("/Instance{i}/Shape")).unwrap();
            assert_eq!(world.get::<Mesh3d>(proxy).unwrap().0, updated);
        }
    }

    #[test]
    fn cache_checks_full_attributes_and_hash_collisions() {
        let mut a = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        a.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0_f32; 3]; 3]);
        a.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0_f32; 2]; 3]);
        let mut b = a.clone();
        b.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0_f32; 2]; 3]);
        assert_ne!(mesh_signature(&a), mesh_signature(&b));
        assert!(!meshes_equal(&a, &b));
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(ProjectionCache::default());
        let old = world.resource_mut::<Assets<Mesh>>().add(a);
        world.resource_mut::<ProjectionCache>().meshes.insert(mesh_signature(&b), vec![(old.clone(),0)]);
        let new = intern_mesh(&mut world, b.clone());
        assert_ne!(old, new);
        assert_eq!(intern_mesh(&mut world, b), new);
    }

    /// The signature must fold in vertex color and be deterministic: same
    /// content → same hash (guards against attribute-iteration-order flakiness),
    /// different color → different hash (so recoloured geometry isn't aliased).
    #[test]
    fn signature_folds_color_and_is_deterministic() {
        let mk = |color: [f32; 4]| {
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            m.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            );
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 3]);
            m
        };
        let red = mesh_signature(&mk([1.0, 0.0, 0.0, 1.0]));
        let blue = mesh_signature(&mk([0.0, 0.0, 1.0, 1.0]));
        assert_ne!(red, blue, "vertex color must affect the signature");
        assert_eq!(
            red,
            mesh_signature(&mk([1.0, 0.0, 0.0, 1.0])),
            "identical meshes must hash identically"
        );
    }

    #[test]
    fn identical_prims_share_one_mesh() {
        // Two Cube prims of the same size → one interned mesh handle.
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cache.usda").unwrap();
        for name in ["/A", "/B"] {
            stage.define_prim(name).unwrap().set_type_name("Cube").unwrap();
        }

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(ProjectionCache::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let a = world.get::<Mesh3d>(map.entity("/A").unwrap()).unwrap().0.clone();
        let b = world.get::<Mesh3d>(map.entity("/B").unwrap()).unwrap().0.clone();
        assert_eq!(a, b, "identical cubes share one interned mesh handle");
        assert_eq!(world.resource::<ProjectionCache>().len(), 1, "one distinct mesh");
    }

    #[test]
    fn differing_geometry_is_not_aliased() {
        // Two cubes of *different* size must get distinct handles — the
        // signature must not collide across genuinely different geometry.
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cache2.usda").unwrap();
        for (name, size) in [("/Small", 1.0f32), ("/Big", 4.0f32)] {
            stage.define_prim(name).unwrap().set_type_name("Cube").unwrap();
            stage
                .create_attribute(format!("{name}.size").as_str(), "double")
                .unwrap()
                .set(openusd::sdf::Value::Double(size as f64))
                .unwrap();
        }

        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(ProjectionCache::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let s = world.get::<Mesh3d>(map.entity("/Small").unwrap()).unwrap().0.clone();
        let b = world.get::<Mesh3d>(map.entity("/Big").unwrap()).unwrap().0.clone();
        assert_ne!(s, b, "different-sized cubes must not share a mesh");
        assert_eq!(world.resource::<ProjectionCache>().len(), 2, "two distinct meshes");
    }

    #[test]
    fn no_cache_resource_falls_back_to_plain_add() {
        // Without the resource, interning must still produce working meshes.
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("cache3.usda").unwrap();
        stage.define_prim("/A").unwrap().set_type_name("Cube").unwrap();
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(SchemaRegistry::builtin());
        let live = LiveStage::new(stage);
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);
        assert!(
            world.get::<Mesh3d>(map.entity("/A").unwrap()).is_some(),
            "mesh still attaches with no ProjectionCache present"
        );
    }
}
