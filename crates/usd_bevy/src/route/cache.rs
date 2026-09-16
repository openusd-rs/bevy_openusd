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
use crate::read::geom::{ReadMesh, MeshPrimvar};

/// Opt-in cumulative cache counters; byte counters measure CPU payload, not GPU memory.
#[derive(Resource, Default, Debug)]
pub struct MeshCacheMetrics(pub std::collections::BTreeMap<&'static str, u64>);

fn record_cache(world: &mut World, name: &'static str, value: u64) {
    if let Some(mut metrics) = world.get_resource_mut::<MeshCacheMetrics>() {
        let counter = metrics.0.entry(name).or_default();
        *counter = counter.saturating_add(value);
    }
}

/// Bounded geometry-input cache containing immutable assembled meshes.
#[derive(Resource)]
pub struct MeshAssemblyCache {
    entries: std::collections::VecDeque<(u64, ReadMesh, Mesh, usize, Option<AssetId<Mesh>>)>,
    payload_bytes: usize,
    byte_budget: usize,
}

impl Default for MeshAssemblyCache {
    fn default() -> Self { Self::with_byte_budget(128 * 1024 * 1024) }
}

impl MeshAssemblyCache {
    pub fn with_byte_budget(byte_budget: usize) -> Self {
        Self { entries: default(), payload_bytes: 0, byte_budget }
    }

    pub fn retained_payload_bytes(&self) -> usize { self.payload_bytes }
}

fn same_geometry(a: &ReadMesh, b: &ReadMesh) -> bool {
    a.points == b.points && a.triangulation_points == b.triangulation_points
        && a.face_vertex_counts == b.face_vertex_counts && a.face_vertex_indices == b.face_vertex_indices
        && a.hole_indices == b.hole_indices && a.orientation == b.orientation
        && a.normals == b.normals && a.uvs == b.uvs
        && a.display_color == b.display_color && a.display_opacity == b.display_opacity
        && a.subdivision_scheme == b.subdivision_scheme
}

pub(crate) fn read_mesh_bytes(read: &ReadMesh) -> usize {
    fn primvar<T>(value: &Option<MeshPrimvar<T>>) -> usize {
        value.as_ref().map_or(0, |value| std::mem::size_of_val(value.values.as_slice())
            + std::mem::size_of_val(value.indices.as_slice()))
    }
    std::mem::size_of_val(read.points.as_slice())
        + read.triangulation_points.as_ref().map_or(0, |points| std::mem::size_of_val(points.as_slice()))
        + std::mem::size_of_val(read.face_vertex_counts.as_slice())
        + std::mem::size_of_val(read.face_vertex_indices.as_slice())
        + std::mem::size_of_val(read.hole_indices.as_slice())
        + primvar(&read.normals) + primvar(&read.uvs) + primvar(&read.display_color) + primvar(&read.display_opacity)
}

fn geometry_signature(read: &ReadMesh) -> u64 {
    let mut hash = FixedHasher.build_hasher();
    bytemuck::cast_slice::<_, u8>(&read.points).hash(&mut hash);
    read.face_vertex_counts.hash(&mut hash);
    read.face_vertex_indices.hash(&mut hash);
    hash.finish()
}

pub(crate) fn intern_assembled_mesh(world: &mut World, read: &ReadMesh) -> Handle<Mesh> {
    let (signature, hit) = lookup_assembly(world, read);
    let budget = world.get_resource::<ProjectionCache>().map_or(0, |cache| cache.byte_budget);
    let cached = if budget == 0 { None } else {
        let assets = world.resource::<Assets<Mesh>>();
        hit.and_then(|index| {
            let (_, _, mesh, _, id) = &world.resource::<MeshAssemblyCache>().entries[index];
            if mesh_payload_bytes(mesh) > budget { return None; }
            id.filter(|id| assets.get(*id).is_some_and(|asset| meshes_equal(asset, mesh))).map(|id| (index, id))
        })
    };
    if let Some((index, id)) = cached
        && let Some(handle) = world.resource_mut::<Assets<Mesh>>().get_strong_handle(id) {
        let mut cache = world.resource_mut::<MeshAssemblyCache>();
        let entry = cache.entries.remove(index).unwrap();
        cache.entries.push_back(entry);
        record_cache(world, "assembly_handle_hits", 1);
        return handle;
    }
    let (mesh, retained) = assemble_after_lookup(world, read, signature, hit);
    let handle = intern_mesh(world, mesh);
    if retained {
        world.resource_mut::<MeshAssemblyCache>().entries.back_mut().unwrap().4 = Some(handle.id());
    }
    handle
}

pub(crate) fn assemble_cached_mesh(world: &mut World, read: &ReadMesh) -> Mesh {
    let (signature, hit) = lookup_assembly(world, read);
    assemble_after_lookup(world, read, signature, hit).0
}

fn lookup_assembly(world: &mut World, read: &ReadMesh) -> (u64, Option<usize>) {
    world.init_resource::<MeshAssemblyCache>();
    let signature = geometry_signature(read);
    let hit = world.resource::<MeshAssemblyCache>().entries.iter()
        .position(|(key, input, _, _, _)| *key == signature && same_geometry(input, read));
    record_cache(world, "assembly_lookups", 1);
    (signature, hit)
}

/// Returns the mesh and whether its immutable snapshot occupies the MRU slot.
fn assemble_after_lookup(world: &mut World, read: &ReadMesh, signature: u64, hit: Option<usize>) -> (Mesh, bool) {
    if let Some(index) = hit {
        let mut cache = world.resource_mut::<MeshAssemblyCache>();
        let entry = cache.entries.remove(index).unwrap();
        let mesh = entry.2.clone();
        cache.entries.push_back(entry);
        record_cache(world, "assembly_clone_hits", 1);
        return (mesh, true);
    }
    let started = world.contains_resource::<MeshCacheMetrics>().then(std::time::Instant::now);
    let mut mesh = crate::mesh::assemble_mesh(read, None, false);
    if read.uvs.is_some() { generate_cached_tangents(world, &mut mesh); }
    if let Some(started) = started {
        record_cache(world, "assembly_build_ns", started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
        record_cache(world, "assembly_builds", 1);
        record_cache(world, "assembly_build_input_bytes", read_mesh_bytes(read) as u64);
    }
    let bytes = read_mesh_bytes(read).saturating_add(mesh_payload_bytes(&mesh));
    let mut cache = world.resource_mut::<MeshAssemblyCache>();
    if cache.byte_budget == 0 || bytes > cache.byte_budget {
        record_cache(world, "assembly_uncached_builds", 1);
        return (mesh, false);
    }
    let mut evictions = 0;
    let mut evicted_bytes = 0;
    while cache.entries.len() >= 256 || bytes > cache.byte_budget.saturating_sub(cache.payload_bytes) {
        let Some((_, _, _, removed, _)) = cache.entries.pop_front() else { break; };
        cache.payload_bytes -= removed;
        evictions += 1;
        evicted_bytes += removed as u64;
    }
    let mut input = read.clone();
    input.subsets = Vec::new();
    cache.entries.push_back((signature, input, mesh.clone(), bytes, None));
    cache.payload_bytes += bytes;
    record_cache(world, "assembly_evictions", evictions);
    record_cache(world, "assembly_evicted_bytes", evicted_bytes);
    (mesh, true)
}

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
        record_cache(world, "intern_no_cache", 1);
        return world.resource_mut::<Assets<Mesh>>().add(mesh);
    }
    let payload_bytes = mesh_payload_bytes(&mesh);
    let budget = world.resource::<ProjectionCache>().byte_budget;
    if budget == 0 || payload_bytes > budget {
        record_cache(world, "intern_budget_bypasses", 1);
        record_cache(world, "intern_bypassed_bytes", payload_bytes as u64);
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
                let handle = existing.clone();
                record_cache(world, "intern_hits", 1);
                record_cache(world, "intern_reused_bytes", payload_bytes as u64);
                return handle;
            }
        }
    }
    let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
    record_cache(world, "intern_misses", 1);
    record_cache(world, "intern_inserted_bytes", payload_bytes as u64);
    let mut cache = world.resource_mut::<ProjectionCache>();
    let mut evicted = (0, 0);
    // Release cached handles when either retention limit would be exceeded.
    if cache.len() >= MAX_INTERNED || payload_bytes > cache.byte_budget.saturating_sub(cache.payload_bytes) {
        evicted = (cache.count as u64, cache.payload_bytes as u64);
        cache.meshes.clear();
        cache.count = 0;
        cache.payload_bytes = 0;
    }
    cache.meshes.entry(sig).or_default().push((handle.clone(),payload_bytes));
    cache.count += 1;
    cache.payload_bytes += payload_bytes;
    if evicted.0 != 0 {
        record_cache(world, "intern_flushes", 1);
        record_cache(world, "intern_evicted_entries", evicted.0);
        record_cache(world, "intern_evicted_bytes", evicted.1);
    }
    handle
}

/// Bounded CPU input/tangent cache; it retains no mesh asset handles.
#[derive(Resource)]
pub struct MeshTangentCache {
    entries: std::collections::VecDeque<(u64, Mesh, bevy::mesh::VertexAttributeValues, usize)>,
    payload_bytes: usize,
    byte_budget: usize,
}

impl Default for MeshTangentCache {
    fn default() -> Self { Self::with_byte_budget(16 * 1024 * 1024) }
}

impl MeshTangentCache {
    pub fn with_byte_budget(byte_budget: usize) -> Self {
        Self { entries: default(), payload_bytes: 0, byte_budget }
    }

    pub fn retained_payload_bytes(&self) -> usize { self.payload_bytes }
}

pub(crate) fn generate_cached_tangents(world: &mut World, mesh: &mut Mesh) {
    world.init_resource::<MeshTangentCache>();
    let signature = mesh_signature(mesh);
    if let Some((_, _, tangents, _)) = world.resource::<MeshTangentCache>().entries.iter()
        .find(|(key, input, _, _)| *key == signature && meshes_equal(input, mesh)) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents.clone());
        return;
    }
    if let Err(error) = mesh.generate_tangents() {
        bevy::log::debug!("mesh: generate_tangents failed: {error}");
        return;
    }
    let Some(tangents) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT) else { return; };
    let bytes = mesh_payload_bytes(mesh);
    let mut cache = world.resource_mut::<MeshTangentCache>();
    if bytes > cache.byte_budget { return; }
    while cache.entries.len() >= 32 || bytes > cache.byte_budget.saturating_sub(cache.payload_bytes) {
        let Some((_, _, _, removed)) = cache.entries.pop_front() else { break; };
        cache.payload_bytes -= removed;
    }
    let tangents = tangents.clone();
    let mut input = mesh.clone();
    input.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
    cache.entries.push_back((signature, input, tangents, bytes));
    cache.payload_bytes += bytes;
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
        && a.final_aabb == b.final_aabb
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
    fn metrics_distinguish_reuse_eviction_and_disabled_caches() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        let mesh = Mesh::from(Rectangle::default());
        let bytes = mesh_payload_bytes(&mesh);
        let _ = intern_mesh(&mut world, mesh.clone());
        assert!(!world.contains_resource::<MeshCacheMetrics>());
        world.init_resource::<MeshCacheMetrics>();
        let _ = intern_mesh(&mut world, mesh.clone());
        world.insert_resource(ProjectionCache::with_byte_budget(bytes));
        let first = intern_mesh(&mut world, mesh.clone());
        assert_eq!(intern_mesh(&mut world, mesh.clone()), first);
        let _ = intern_mesh(&mut world, Mesh::from(Rectangle::new(2.0, 1.0)));
        world.insert_resource(ProjectionCache::with_byte_budget(0));
        let _ = intern_mesh(&mut world, mesh);
        let metrics = &world.resource::<MeshCacheMetrics>().0;
        for (name, expected) in [("intern_no_cache", 1), ("intern_hits", 1),
            ("intern_misses", 2), ("intern_flushes", 1), ("intern_evicted_entries", 1),
            ("intern_evicted_bytes", bytes as u64), ("intern_budget_bypasses", 1),
            ("intern_bypassed_bytes", bytes as u64), ("intern_reused_bytes", bytes as u64)] {
            assert_eq!(metrics[name], expected, "{name}");
        }
        assert!(world.resource::<Assets<Mesh>>().contains(&first));
    }

    #[test]
    fn assembled_handles_recheck_mutations_removal_and_cache_budget() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/material_subsets.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let read = crate::read::geom::read_mesh_at(&stage, &openusd::sdf::path("/Panels").unwrap(), Some(0.0)).unwrap().unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<ProjectionCache>();
        world.init_resource::<MeshCacheMetrics>();
        let first = intern_assembled_mesh(&mut world, &read);
        assert_eq!(first.id(), intern_assembled_mesh(&mut world, &read).id());
        world.resource_mut::<Assets<Mesh>>().get_mut(&first).unwrap()
            .insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[9.0; 3]; read.points.len()]);
        let second = intern_assembled_mesh(&mut world, &read);
        assert_ne!(first.id(), second.id());
        assert!(meshes_equal(world.resource::<Assets<Mesh>>().get(&second).unwrap(), &crate::mesh::assemble_mesh(&read, None, true)));
        world.resource_mut::<Assets<Mesh>>().get_mut(&second).unwrap().final_aabb =
            Some(bevy::math::bounding::Aabb3d::new(Vec3::ZERO, Vec3::ONE));
        let third = intern_assembled_mesh(&mut world, &read);
        assert_ne!(second.id(), third.id());
        world.resource_mut::<Assets<Mesh>>().remove(third.id());
        let fourth = intern_assembled_mesh(&mut world, &read);
        assert_ne!(third.id(), fourth.id());
        world.insert_resource(ProjectionCache::with_byte_budget(0));
        assert_ne!(fourth.id(), intern_assembled_mesh(&mut world, &read).id());
        let metrics = &world.resource::<MeshCacheMetrics>().0;
        assert_eq!(metrics["assembly_builds"], 1);
        assert_eq!(metrics["assembly_handle_hits"], 1);
        assert_eq!(metrics["assembly_clone_hits"], 4);
        assert_eq!(metrics["assembly_lookups"], 6);
        assert_eq!(metrics["assembly_build_input_bytes"], read_mesh_bytes(&read) as u64);
        let previous = world.resource::<MeshAssemblyCache>().entries.back().unwrap().4;
        let bytes = world.resource::<MeshAssemblyCache>().payload_bytes;
        world.resource_mut::<MeshAssemblyCache>().byte_budget = bytes;
        let mut oversized = read.clone();
        oversized.points.push([10.0; 3]);
        let uncached = intern_assembled_mesh(&mut world, &oversized);
        let cache = world.resource::<MeshAssemblyCache>();
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries.back().unwrap().4, previous);
        assert_eq!(cache.payload_bytes, bytes);
        assert_ne!(Some(uncached.id()), previous);
        assert_eq!(world.resource::<MeshCacheMetrics>().0["assembly_uncached_builds"], 1);
    }

    #[test]
    fn assembly_cache_checks_geometry_and_isolates_mutations() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/material_subsets.usda");
        let stage = crate::UsdSource::new(file, std::fs::read(file).unwrap()).unwrap().open_stage().unwrap();
        let path = openusd::sdf::path("/Panels").unwrap();
        let mut read = crate::read::geom::read_mesh_at(&stage, &path, Some(0.0)).unwrap().unwrap();
        let mut world = World::new();
        let expected = crate::mesh::assemble_mesh(&read, None, true);
        let mut first = assemble_cached_mesh(&mut world, &read);
        assert!(meshes_equal(&first, &expected));
        first.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[9.0; 3]; first.count_vertices()]);
        read.subsets.clear();
        read.double_sided = !read.double_sided;
        assert!(meshes_equal(&assemble_cached_mesh(&mut world, &read), &expected));
        assert_eq!(world.resource::<MeshAssemblyCache>().entries.len(), 1);
        for change in 0..3 {
            match change {
                0 => read.normals.as_mut().unwrap().values[0] = [0.0, 1.0, 0.0],
                1 => read.hole_indices = vec![0],
                _ => read.orientation = crate::read::geom::Orientation::LeftHanded,
            }
            let reference = crate::mesh::assemble_mesh(&read, None, true);
            assert!(meshes_equal(&assemble_cached_mesh(&mut world, &read), &reference));
        }
        assert_eq!(world.resource::<MeshAssemblyCache>().entries.len(), 4);
        let budget = read_mesh_bytes(&read) + mesh_payload_bytes(&expected);
        world.insert_resource(MeshAssemblyCache::with_byte_budget(budget));
        for offset in [1.0, 2.0, 3.0] {
            read.points[0][0] = offset;
            assemble_cached_mesh(&mut world, &read);
            assert!(world.resource::<MeshAssemblyCache>().retained_payload_bytes() <= budget);
            assert_eq!(world.resource::<MeshAssemblyCache>().entries.len(), 1);
        }
        world.insert_resource(MeshAssemblyCache::with_byte_budget(0));
        assert!(meshes_equal(&assemble_cached_mesh(&mut world, &read), &crate::mesh::assemble_mesh(&read, None, true)));
        assert!(world.resource::<MeshAssemblyCache>().entries.is_empty());
    }

    #[test]
    fn tangent_cache_reuses_exact_inputs_and_isolates_output_mutation() {
        let mut world = World::new();
        let input = Mesh::from(Rectangle::default());
        let mut first = input.clone();
        generate_cached_tangents(&mut world, &mut first);
        let expected = first.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap().get_bytes().to_vec();
        first.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[0.0; 4]; first.count_vertices()]);
        let mut second = input.clone();
        generate_cached_tangents(&mut world, &mut second);
        assert_eq!(second.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap().get_bytes(), expected);
        assert_eq!(world.resource::<MeshTangentCache>().entries.len(), 1);
        let mut changed = input;
        if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) = changed.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
            for uv in uvs { uv[0] = 1.0 - uv[0]; }
        }
        let mut reference = changed.clone();
        reference.generate_tangents().unwrap();
        world.resource_mut::<MeshTangentCache>().entries[0].0 = mesh_signature(&changed);
        generate_cached_tangents(&mut world, &mut changed);
        assert!(meshes_equal(&changed, &reference));
        assert_ne!(changed.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap().get_bytes(), expected);
        assert_eq!(world.resource::<MeshTangentCache>().entries.len(), 2);
    }

    #[test]
    fn tangent_cache_budget_and_failure_do_not_change_mesh_results() {
        let mut world = World::new();
        world.insert_resource(MeshTangentCache::with_byte_budget(0));
        let mut mesh = Mesh::from(Rectangle::default());
        generate_cached_tangents(&mut world, &mut mesh);
        assert!(mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        assert!(world.resource::<MeshTangentCache>().entries.is_empty());
        let budget = mesh_payload_bytes(&mesh);
        world.insert_resource(MeshTangentCache::with_byte_budget(budget));
        for width in [1.0, 2.0, 3.0] {
            let mut mesh = Mesh::from(Rectangle::new(width, 1.0));
            generate_cached_tangents(&mut world, &mut mesh);
            let cache = world.resource::<MeshTangentCache>();
            assert_eq!(cache.entries.len(), 1);
            assert_eq!(cache.retained_payload_bytes(), budget);
        }
        let mut invalid = Mesh::from(Rectangle::default());
        invalid.remove_attribute(Mesh::ATTRIBUTE_UV_0);
        generate_cached_tangents(&mut world, &mut invalid);
        assert!(invalid.attribute(Mesh::ATTRIBUTE_TANGENT).is_none());
        assert_eq!(world.resource::<MeshTangentCache>().entries.len(), 1);
    }

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
