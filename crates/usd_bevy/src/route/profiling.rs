use bevy::prelude::*;

fn trace_range(value: &str) -> Option<std::ops::RangeInclusive<usize>> {
    let (start, end) = value.split_once(':')?;
    let (start, end) = (start.parse::<usize>().ok()?, end.parse::<usize>().ok()?);
    (end.checked_sub(start)? < 4096).then_some(start..=end)
}

pub(crate) fn memory_trace_range() -> Option<std::ops::RangeInclusive<usize>> {
    let value = std::env::var("USD_PROFILE_MEMORY_RANGE").ok()?;
    let range = trace_range(&value);
    if range.is_none() { eprintln!("USD_PROFILE_MEMORY_RANGE must be start:end with at most 4096 prims"); }
    range
}

fn resident_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut fields = status.lines().find(|line| line.starts_with("VmRSS:"))?.split_whitespace();
    fields.next()?;
    let kib = fields.next()?.parse::<usize>().ok()?;
    (fields.next()? == "kB").then(|| kib.checked_mul(1024)).flatten()
}

pub(crate) fn memory_event(phase: &str, path: &openusd::sdf::Path, route: &str, decoded_bytes: Option<usize>) {
    eprintln!("projection_memory_event phase={phase} path={path:?} route={route} rss_bytes={:?} decoded_payload_bytes={decoded_bytes:?}", resident_bytes());
}

fn component_label(info: &bevy::ecs::component::ComponentInfo) -> String {
    macro_rules! known {
        ($($ty:ty),*) => { $(if info.type_id() == Some(std::any::TypeId::of::<$ty>()) {
            return std::any::type_name::<$ty>().into();
        })* };
    }
    known!(Transform, GlobalTransform, Visibility, InheritedVisibility, ViewVisibility,
        ChildOf, Children, Mesh3d, MeshMaterial3d<StandardMaterial>,
        super::instancer::UsdInstance, super::instancer::UsdInstanceId, super::instancer::UsdPrototypePart);
    format!("{:?}:{}", info.id(), info.name())
}

pub(crate) fn ecs_snapshot(world: &World, phase: &str) {
    let components: Vec<_> = world.components().iter_registered().collect();
    let mut rows = Vec::new();
    for (index, table) in world.storages().tables.iter().enumerate() {
        let stride = components.iter().filter(|info| table.has_column(info.id()))
            .map(|info| info.layout().size()).sum::<usize>();
        rows.push((stride.saturating_mul(table.capacity()), index, table.entity_count(), table.capacity(), stride));
    }
    let live = rows.iter().map(|row| u64::from(row.2)).sum::<u64>();
    let inline = rows.iter().map(|row| row.0).sum::<usize>();
    eprintln!("projection_ecs_snapshot phase={phase} live_entities={live} allocated_entity_indices={} table_component_capacity_bytes={inline} excludes=component-heaps,ticks,entity-metadata,archetypes,sparse-sets,allocator,gpu", world.entities().len());
    rows.sort_unstable_by_key(|row| std::cmp::Reverse(row.0));
    for (bytes, index, count, capacity, stride) in rows.into_iter().take(5) {
        let table = world.storages().tables.iter().nth(index).unwrap();
        let names: Vec<_> = components.iter().filter(|info| table.has_column(info.id()))
            .map(|info| format!("{}:{}", component_label(info), info.layout().size())).collect();
        eprintln!("projection_ecs_table phase={phase} table={index} live={count} capacity={capacity} row_bytes={stride} component_capacity_bytes={bytes} components={names:?}");
    }
}

pub(crate) fn memory_snapshot(world: &World, phase: &str, prims: usize) {
    ecs_snapshot(world, phase);
    let mut unavailable = 0usize;
    let mut mesh_bytes = 0usize;
    let meshes = world.get_resource::<Assets<Mesh>>().map_or(0, |assets| {
        for (_, mesh) in assets.iter() {
            if mesh.try_attributes().is_err() || mesh.try_indices_option().is_err() { unavailable += 1; continue; }
            mesh_bytes = mesh_bytes.saturating_add(super::cache::mesh_payload_bytes(mesh));
        }
        assets.len()
    });
    let mut image_bytes = 0usize;
    let images = world.get_resource::<Assets<Image>>().map_or(0, |assets| {
        for (_, image) in assets.iter() { image_bytes = image_bytes.saturating_add(image.data.as_ref().map_or(0, Vec::len)); }
        assets.len()
    });
    let interned = world.get_resource::<super::cache::ProjectionCache>().map_or(0, |cache| cache.retained_payload_bytes());
    let assembly = world.get_resource::<super::cache::MeshAssemblyCache>().map_or(0, |cache| cache.retained_payload_bytes());
    let tangent = world.get_resource::<super::cache::MeshTangentCache>().map_or(0, |cache| cache.retained_payload_bytes());
    eprintln!("projection_memory_snapshot phase={phase} prims={prims} rss_bytes={:?} allocated_entity_indices={} meshes={meshes} mesh_payload_bytes={mesh_bytes} unavailable_meshes={unavailable} images={images} image_payload_bytes={image_bytes} interned_payload_bytes={interned} assembly_payload_bytes={assembly} tangent_payload_bytes={tangent} cache_totals=non-additive excludes=source,ecs,allocator,capacity,gpu",
        resident_bytes(), world.entities().len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_windows_are_bounded_and_inclusive() {
        assert_eq!(trace_range("333800:334850"), Some(333800..=334850));
        assert_eq!(trace_range("0:4095"), Some(0..=4095));
        for invalid in ["0:4096", "5:4", "-1:2", "1", "a:b"] { assert!(trace_range(invalid).is_none()); }
    }
}
