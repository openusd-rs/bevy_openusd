use bevy::{asset::AssetId, prelude::*};
use std::collections::{HashMap, VecDeque};

const MAX_ENTRIES: usize = 4096;

#[derive(Resource, Default)]
pub(super) struct GeneratedImages(
    VecDeque<(blake3::Hash, AssetId<Image>)>,
    HashMap<AssetId<Image>, FinalizedImage>,
);

struct FinalizedImage {
    source: Image,
    layout: Image,
    base_len: usize,
    hash: blake3::Hash,
}

pub(super) fn layout(image: &Image) -> Image {
    Image {
        data: None,
        data_order: image.data_order,
        texture_descriptor: image.texture_descriptor.clone(),
        sampler: image.sampler.clone(),
        texture_view_descriptor: image.texture_view_descriptor.clone(),
        asset_usage: image.asset_usage,
        copy_on_resize: image.copy_on_resize,
    }
}

impl GeneratedImages {
    pub(super) fn record_finalized(
        &mut self,
        id: AssetId<Image>,
        source: Image,
        base_len: usize,
        image: &Image,
    ) {
        let Some(data) = image.data.as_deref() else {
            return;
        };
        let Some(base) = data.get(..base_len) else {
            return;
        };
        if let Some(previous) = self.1.get_mut(&id)
            && same_layout(&previous.layout, &source)
            && previous.hash == blake3::hash(base)
        {
            previous.layout = layout(image);
            previous.hash = blake3::hash(data);
            return;
        }
        if !self
            .0
            .iter()
            .any(|(hash, known)| *known == id && *hash == blake3::hash(base))
        {
            return;
        }
        self.1.insert(
            id,
            FinalizedImage {
                source,
                layout: layout(image),
                base_len,
                hash: blake3::hash(data),
            },
        );
    }

    fn pixels<'a>(
        &self,
        id: AssetId<Image>,
        image: &'a Image,
        template: &Image,
    ) -> Option<&'a [u8]> {
        let data = image.data.as_deref()?;
        if same_layout(image, template) {
            return Some(data);
        }
        let finalized = self.1.get(&id)?;
        (same_layout(&finalized.source, template)
            && same_layout(&finalized.layout, image)
            && finalized.hash == blake3::hash(data))
        .then(|| data.get(..finalized.base_len))
        .flatten()
    }
}

pub(super) fn configure(app: &mut App) {
    app.add_systems(Last, prune);
}

fn prune(cache: Option<ResMut<GeneratedImages>>, images: Option<Res<Assets<Image>>>) {
    if let (Some(mut cache), Some(images)) = (cache, images) {
        cache.0.retain(|(_, id)| images.contains(*id));
        cache.1.retain(|id, _| images.contains(*id));
    }
}

fn same_layout(a: &Image, b: &Image) -> bool {
    a.texture_descriptor == b.texture_descriptor
        && a.texture_view_descriptor == b.texture_view_descriptor
        && a.sampler == b.sampler
        && a.data_order == b.data_order
        && a.asset_usage == b.asset_usage
        && a.copy_on_resize == b.copy_on_resize
}

fn equivalent(a: &Image, b: &Image) -> bool {
    same_layout(a, b) && a.data == b.data
}

pub(super) fn reuse_if(
    world: &mut World,
    id: AssetId<Image>,
    template: &Image,
    pixels_match: impl FnOnce(&[u8]) -> bool,
) -> Option<Handle<Image>> {
    let matches = world
        .resource::<Assets<Image>>()
        .get(id)
        .is_some_and(|image| {
            let pixels = if same_layout(image, template) {
                image.data.as_deref()
            } else {
                world
                    .get_resource::<GeneratedImages>()
                    .and_then(|cache| cache.pixels(id, image, template))
            };
            pixels.is_some_and(pixels_match)
        });
    if !matches {
        return None;
    }
    world.resource_mut::<Assets<Image>>().get_strong_handle(id)
}

pub(super) fn intern(world: &mut World, image: Image) -> Handle<Image> {
    let hash = blake3::hash(image.data.as_deref().unwrap_or_default());
    intern_hashed(world, image, hash)
}

fn intern_hashed(world: &mut World, image: Image, hash: blake3::Hash) -> Handle<Image> {
    let candidate = world.get_resource::<GeneratedImages>().and_then(|cache| {
        let images = world.resource::<Assets<Image>>();
        cache.0.iter().find_map(|(key, id)| {
            (*key == hash
                && images.get(*id).is_some_and(|existing| {
                    equivalent(existing, &image)
                        || image
                            .data
                            .as_deref()
                            .is_some_and(|data| cache.pixels(*id, existing, &image) == Some(data))
                }))
            .then_some(*id)
        })
    });
    if let Some(id) = candidate
        && let Some(handle) = world.resource_mut::<Assets<Image>>().get_strong_handle(id)
    {
        return handle;
    }
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    world.init_resource::<GeneratedImages>();
    let mut cache = world.resource_mut::<GeneratedImages>();
    if cache.0.len() == MAX_ENTRIES
        && let Some((_, id)) = cache.0.pop_front()
    {
        cache.1.remove(&id);
    }
    cache.0.push_back((hash, handle.id()));
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    fn image(width: u32) -> Image {
        Image::new(
            Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![255; width as usize * 4],
            TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default(),
        )
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world
    }

    #[test]
    fn finalized_generated_images_reuse_without_rebuilding_mips() {
        let (mut app, first) = finalized();
        let second = intern(app.world_mut(), image(4));
        assert_eq!(first.id(), second.id());
        let source = image(4);
        assert_eq!(
            reuse_if(app.world_mut(), first.id(), &source, |data| Some(data)
                == source.data.as_deref())
            .unwrap()
            .id(),
            first.id()
        );
        let assets = app.world().resource::<Assets<Image>>();
        assert_eq!(assets.len(), 1);
        assert_eq!(
            assets
                .get(&first)
                .unwrap()
                .texture_descriptor
                .mip_level_count,
            3
        );
        assert!(
            app.world().resource::<GeneratedImages>().1[&first.id()]
                .source
                .data
                .is_none()
        );
        drop((first, second));
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 0);
        assert!(app.world().resource::<GeneratedImages>().1.is_empty());
    }

    #[test]
    fn finalized_images_follow_later_material_wrap_changes() {
        use crate::read::shade::{ReadPreviewMaterial, TextureWrap};
        let (mut app, first) = finalized();
        let read = ReadPreviewMaterial {
            diffuse_texture: Some("albedo.png".into()),
            texture_wraps: [("albedo.png".into(), [TextureWrap::Clamp; 2])].into(),
            ..default()
        };
        super::super::sampling::note(
            app.world_mut(),
            &StandardMaterial {
                base_color_texture: Some(first.clone()),
                ..default()
            },
            &read,
        );
        app.update();
        assert_eq!(intern(app.world_mut(), image(4)).id(), first.id());
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&first)
                .unwrap()
                .sampler,
            super::super::sampling::sampler([TextureWrap::Clamp; 2])
        );
    }

    #[test]
    fn finalized_images_reject_changed_pixels_mips_and_layouts() {
        for change in 0..4 {
            let (mut app, first) = finalized();
            {
                let mut images = app.world_mut().resource_mut::<Assets<Image>>();
                let mut target = images.get_mut(&first).unwrap();
                match change {
                    0 => target.data.as_mut().unwrap()[0] = 0,
                    1 => *target.data.as_mut().unwrap().last_mut().unwrap() = 0,
                    2 => target.sampler = bevy::image::ImageSampler::nearest(),
                    _ => target.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb,
                }
            }
            assert!(reuse_if(app.world_mut(), first.id(), &image(4), |_| true).is_none());
            assert_ne!(intern(app.world_mut(), image(4)).id(), first.id());
        }
    }

    fn finalized() -> (App, Handle<Image>) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>();
        configure(&mut app);
        super::super::sampling::configure(&mut app);
        let handle = intern(app.world_mut(), image(4));
        super::super::sampling::note(
            app.world_mut(),
            &StandardMaterial {
                base_color_texture: Some(handle.clone()),
                ..default()
            },
            &default(),
        );
        app.update();
        (app, handle)
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
        world
            .resource_mut::<Assets<Image>>()
            .get_mut(&a)
            .unwrap()
            .data
            .as_mut()
            .unwrap()[0] = 0;
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
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>();
        configure(&mut app);
        let a = intern(app.world_mut(), image(1));
        let b = intern(app.world_mut(), image(1));
        let id = a.id();
        drop(a);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().resource::<Assets<Image>>().contains(id));
        drop(b);
        for _ in 0..3 {
            app.update();
        }
        assert!(!app.world().resource::<Assets<Image>>().contains(id));
        assert!(app.world().resource::<GeneratedImages>().0.is_empty());
    }

    #[test]
    fn generated_index_is_bounded() {
        let mut world = world();
        let handles: Vec<_> = (0..MAX_ENTRIES + 10)
            .map(|value| {
                let mut image = image(1);
                image.data = Some((value as u32).to_le_bytes().to_vec());
                intern(&mut world, image)
            })
            .collect();
        assert_eq!(world.resource::<GeneratedImages>().0.len(), MAX_ENTRIES);
        assert!(world.resource::<Assets<Image>>().contains(handles[0].id()));
    }
}
