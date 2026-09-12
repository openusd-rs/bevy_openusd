use bevy::{asset::AssetId, prelude::*};
use std::collections::VecDeque;

const MAX_ENTRIES: usize = 4096;

#[derive(Resource, Default)]
struct GeneratedImages(VecDeque<(blake3::Hash, AssetId<Image>)>);

pub(super) fn configure(app: &mut App) {
    app.add_systems(Last, prune);
}

fn prune(cache: Option<ResMut<GeneratedImages>>, images: Option<Res<Assets<Image>>>) {
    if let (Some(mut cache), Some(images)) = (cache, images) {
        cache.0.retain(|(_, id)| images.contains(*id));
    }
}

fn same_layout(a: &Image, b: &Image) -> bool {
    a.texture_descriptor == b.texture_descriptor
        && a.texture_view_descriptor == b.texture_view_descriptor
        && a.sampler == b.sampler && a.data_order == b.data_order
        && a.asset_usage == b.asset_usage && a.copy_on_resize == b.copy_on_resize
}

fn equivalent(a: &Image, b: &Image) -> bool { same_layout(a, b) && a.data == b.data }

pub(super) fn reuse_if(world: &mut World, id: AssetId<Image>, template: &Image,
    pixels_match: impl FnOnce(&[u8]) -> bool) -> Option<Handle<Image>>
{
    let matches = world.resource::<Assets<Image>>().get(id).is_some_and(|image|
        same_layout(image, template) && image.data.as_deref().is_some_and(pixels_match));
    if !matches { return None; }
    world.resource_mut::<Assets<Image>>().get_strong_handle(id)
}

pub(super) fn intern(world: &mut World, image: Image) -> Handle<Image> {
    let hash = blake3::hash(image.data.as_deref().unwrap_or_default());
    intern_hashed(world, image, hash)
}

fn intern_hashed(world: &mut World, image: Image, hash: blake3::Hash) -> Handle<Image> {
    let candidate = world.get_resource::<GeneratedImages>().and_then(|cache| {
        let images = world.resource::<Assets<Image>>();
        cache.0.iter().find_map(|(key, id)| (*key == hash
            && images.get(*id).is_some_and(|existing| equivalent(existing, &image))).then_some(*id))
    });
    if let Some(id) = candidate {
        if let Some(handle) = world.resource_mut::<Assets<Image>>().get_strong_handle(id) {
            return handle;
        }
    }
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    world.init_resource::<GeneratedImages>();
    let mut cache = world.resource_mut::<GeneratedImages>();
    if cache.0.len() == MAX_ENTRIES { cache.0.pop_front(); }
    cache.0.push_back((hash, handle.id()));
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    fn image(width: u32) -> Image {
        Image::new(Extent3d { width, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![255; width as usize * 4], TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default())
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world
    }

    #[test]
    fn live_generated_images_share_without_pixel_keys() {
        let mut world = world();
        let a = intern(&mut world, image(1024));
        let b = intern(&mut world, image(1024));
        assert_eq!(a.id(), b.id());
        assert_eq!(world.resource::<Assets<Image>>().len(), 1);
        assert_eq!(world.resource::<GeneratedImages>().0.len(), 1);
        assert!(std::mem::size_of::<(blake3::Hash, AssetId<Image>)>() < 128);
    }

    #[test]
    fn collisions_and_live_pixel_mutations_do_not_alias() {
        let mut world = world();
        let hash = blake3::hash(b"forced collision");
        let a = intern_hashed(&mut world, image(1), hash);
        let b = intern_hashed(&mut world, image(2), hash);
        assert_ne!(a.id(), b.id());
        world.resource_mut::<Assets<Image>>().get_mut(&a).unwrap().data.as_mut().unwrap()[0] = 0;
        let c = intern_hashed(&mut world, image(1), hash);
        assert_ne!(a.id(), c.id());
        assert_eq!(intern_hashed(&mut world, image(2), hash).id(), b.id());
    }

    #[test]
    fn incompatible_descriptors_and_removed_assets_do_not_alias() {
        let mut world = world();
        let a = intern(&mut world, image(1));
        let mut srgb = image(1);
        srgb.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
        assert_ne!(intern(&mut world, srgb).id(), a.id());
        let mut nearest = image(1);
        nearest.sampler = bevy::image::ImageSampler::nearest();
        assert_ne!(intern(&mut world, nearest).id(), a.id());
        world.resource_mut::<Assets<Image>>().remove(a.id());
        let b = intern(&mut world, image(1));
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn generated_index_does_not_keep_unowned_images_alive() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<Image>();
        configure(&mut app);
        let a = intern(app.world_mut(), image(1));
        let b = intern(app.world_mut(), image(1));
        let id = a.id();
        drop(a);
        for _ in 0..3 { app.update(); }
        assert!(app.world().resource::<Assets<Image>>().contains(id));
        drop(b);
        for _ in 0..3 { app.update(); }
        assert!(!app.world().resource::<Assets<Image>>().contains(id));
        assert!(app.world().resource::<GeneratedImages>().0.is_empty());
    }

    #[test]
    fn generated_index_is_bounded() {
        let mut world = world();
        let handles: Vec<_> = (0..MAX_ENTRIES + 10).map(|value| {
            let mut image = image(1);
            image.data = Some((value as u32).to_le_bytes().to_vec());
            intern(&mut world, image)
        }).collect();
        assert_eq!(world.resource::<GeneratedImages>().0.len(), MAX_ENTRIES);
        assert!(world.resource::<Assets<Image>>().contains(handles[0].id()));
    }
}
