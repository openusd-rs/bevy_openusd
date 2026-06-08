//! Texture resolution — USD asset paths → `Handle<Image>`.
//!
//! Three strategies, tried in order:
//!
//! 1. **USDZ-embedded**: the archive bytes have been pre-extracted; decode
//!    inline via `Image::from_buffer` and register as a labeled sub-asset.
//! 2. **Filesystem search**: probe each configured search path both
//!    directly (`<search>/<raw>`) and via a lazily-built basename index
//!    (`cover_square_seamless.jpg` found anywhere under the search root).
//!    Pixel payload is read + decoded + added as a labeled sub-asset. This
//!    is what makes real Isaac / Omniverse stages work — their textures
//!    live next to the *authoring* `.usd`, not next to the root stage.
//! 3. **AssetServer fallback**: hand the raw path to
//!    `lc.loader().load::<Image>(...)` so asset-root-relative references
//!    still work for hand-authored scenes.
//!
//! Colour-space handling: Bevy's `is_srgb` flag gates gamma decoding. USD
//! conventions we assume:
//!
//! | channel                                            | is_srgb |
//! |----------------------------------------------------|---------|
//! | diffuse / base colour / emissive                   | true    |
//! | normal / roughness / metallic / occlusion / opacity| false   |

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::{Handle, RenderAssetUsages};
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageType};

use crate::build::BuildCtx;

/// The colour-space semantics of a texture channel, as authored in USD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureChannel {
    Srgb,
    Linear,
}

impl TextureChannel {
    fn is_srgb(self) -> bool {
        matches!(self, TextureChannel::Srgb)
    }
}

/// Where a material builder gets its texture handles. The asset loader
/// implements this over its [`LoadContext`](bevy::asset::LoadContext) (so
/// textures register against the load); the viewer implements it over the
/// live [`AssetServer`](bevy::asset::AssetServer) so it can rebuild a
/// material for an incremental update (e.g. a variant switch) without going
/// through the loader. This is what lets one material builder serve both the
/// bake path and the live-update path.
pub trait TextureSource {
    fn load(&mut self, path: &str, channel: TextureChannel) -> Option<Handle<Image>>;
    /// Roughness (G) + metallic (B) packed into one image, per the glTF
    /// convention Bevy expects. Implementations may fall back to a single
    /// channel when packing isn't available.
    fn load_packed(&mut self, roughness: &str, metallic: &str) -> Option<Handle<Image>>;
    /// Whether `path` resolves to a real texture (used by name-guessing to
    /// probe candidates without emitting "unresolved" warnings).
    fn can_resolve(&mut self, path: &str) -> bool;
}

impl TextureSource for BuildCtx<'_, '_> {
    fn load(&mut self, path: &str, channel: TextureChannel) -> Option<Handle<Image>> {
        load_texture(self, path, channel)
    }
    fn load_packed(&mut self, roughness: &str, metallic: &str) -> Option<Handle<Image>> {
        load_metallic_roughness_packed(self, roughness, metallic)
    }
    fn can_resolve(&mut self, path: &str) -> bool {
        can_resolve_texture(self, path)
    }
}

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp", "hdr", "exr", "ktx2", "dds",
];

/// Resolve a USD asset path into a `Handle<Image>`. Returns `None` when
/// the texture can't be located via USDZ-embedded bytes, the configured
/// filesystem search roots, or by walking up to two parent dirs of each
/// search root for a `textures/<basename>` sibling. Returning `None`
/// instead of a broken AssetServer handle matters: a `StandardMaterial`
/// whose `normal_map_texture` is bound to a never-loading handle picks
/// up the NORMAL_MAP shader variant and samples zero-vectors, which on
/// some Bevy paths makes the mesh disappear entirely. Letting callers
/// drop the slot keeps the mesh on screen with its base colour.
pub fn load_texture(
    ctx: &mut BuildCtx<'_, '_>,
    raw_path: &str,
    channel: TextureChannel,
) -> Option<Handle<Image>> {
    let clean = raw_path.strip_prefix("./").unwrap_or(raw_path).to_string();
    let is_srgb = channel.is_srgb();

    // 1. USDZ-embedded bytes.
    if let Some(bytes) = lookup_embedded(ctx.embedded, &clean) {
        bevy::log::info!(
            "texture: usdz-embedded hit for {clean:?} ({} bytes)",
            bytes.len()
        );
        if let Some(handle) = decode_and_register(ctx, &clean, bytes, is_srgb, "usdz") {
            return Some(handle);
        }
        bevy::log::warn!("texture: usdz-embedded {clean:?} found but decode failed");
    }

    // 2. Filesystem search via configured roots. Load through the AssetServer
    //    *by path* (not as a UsdAsset sub-asset): the decoded Image is then
    //    owned by the asset server and cached across UsdAsset reloads, so a
    //    variant switch reuses already-decoded textures instead of re-decoding
    //    every one (the old sub-asset path re-decoded the lot each reload —
    //    seconds per switch). `AssetPlugin.unapproved_path_mode = Allow` lets
    //    the absolute texture path resolve outside the startup asset root.
    if let Some(path) = locate_on_filesystem(ctx, &clean) {
        bevy::log::info!("texture: fs hit for {clean:?} → {path:?}");
        let sampler = usd_texture_sampler();
        let handle = ctx
            .lc
            .loader()
            .with_settings(move |s: &mut bevy::image::ImageLoaderSettings| {
                s.is_srgb = is_srgb;
                s.sampler = sampler.clone();
            })
            .load::<Image>(path.to_string_lossy().into_owned());
        return Some(handle);
    }

    bevy::log::warn!(
        "texture: unresolved {clean:?} (search roots: {:?}) — material slot dropped",
        ctx.search_paths
    );
    None
}

/// Cheap, quiet texture probe for heuristic callers.
///
/// `load_texture` deliberately warns on a miss because an authored USD texture
/// path failing to resolve is usually actionable. Name-based recovery code,
/// however, tries optional convention guesses like
/// `Material_AO.png`/`Material_Roughness.png`; those misses are expected and
/// should not look like broken authored materials in the log.
pub fn can_resolve_texture(ctx: &mut BuildCtx<'_, '_>, raw_path: &str) -> bool {
    let clean = raw_path.strip_prefix("./").unwrap_or(raw_path).to_string();
    lookup_embedded(ctx.embedded, &clean).is_some() || locate_on_filesystem(ctx, &clean).is_some()
}

/// Composite separately-authored roughness + metallic textures into
/// the single packed RGBA image Bevy's `StandardMaterial` consumes
/// (glTF convention: R unused, G = roughness, B = metallic, A = 1.0).
///
/// USD's `UsdPreviewSurface` lets each channel be an independent
/// asset, so production exports often author two separate
/// single-channel PNGs. We resolve both via the regular
/// `load_texture` byte path (USDZ embedded → fs search → asset
/// server), decode in linear space, sample R from each, pack into
/// fresh RGBA8 bytes, and hand Bevy a synthesised Image.
///
/// Falls back to the metallic texture alone if the byte sources for
/// either input can't be found — that's the cheapest non-broken
/// path; Bevy will sample its B channel for metallic.
pub fn load_metallic_roughness_packed(
    ctx: &mut BuildCtx<'_, '_>,
    rough_path: &str,
    metal_path: &str,
) -> Option<Handle<Image>> {
    let rough_clean = rough_path
        .strip_prefix("./")
        .unwrap_or(rough_path)
        .to_string();
    let metal_clean = metal_path
        .strip_prefix("./")
        .unwrap_or(metal_path)
        .to_string();

    let cache_key = format!("MetalRoughPacked:r={rough_clean}|m={metal_clean}");
    if let Some(h) = ctx.embedded_textures.get(&cache_key) {
        return Some(h.clone());
    }

    // Pull raw bytes for both. Linear (non-sRGB) — these are data
    // textures, not colour.
    let rough_bytes = fetch_texture_bytes(ctx, &rough_clean);
    let metal_bytes = fetch_texture_bytes(ctx, &metal_clean);

    if let (Some(rough), Some(metal)) = (rough_bytes, metal_bytes) {
        if let Some(packed) = pack_metal_rough(&rough, &metal) {
            let label = format!("Image:{cache_key}");
            let handle = ctx.lc.add_labeled_asset(label, packed);
            ctx.embedded_textures.insert(cache_key, handle.clone());
            return Some(handle);
        }
        bevy::log::warn!(
            "texture: failed to pack metal/rough textures (m={metal_clean}, r={rough_clean}); using metallic only"
        );
    }

    // Fallback: just use the metallic texture verbatim. Bevy will
    // sample its B channel; the roughness side is lost but we don't
    // crash.
    load_texture(ctx, metal_path, TextureChannel::Linear)
}

/// Resolve a texture path to its raw bytes via the same priority
/// chain as `load_texture` — USDZ-embedded → filesystem search.
/// Returns `None` if neither produces bytes (we don't fall through
/// to the AssetServer here because we need the bytes synchronously
/// for compositing).
fn fetch_texture_bytes(ctx: &BuildCtx<'_, '_>, clean: &str) -> Option<Vec<u8>> {
    if let Some(b) = lookup_embedded(ctx.embedded, clean) {
        return Some(b.clone());
    }
    if let Some(path) = locate_on_filesystem_const(ctx, clean) {
        if let Ok(b) = std::fs::read(&path) {
            return Some(b);
        }
    }
    None
}

/// Read-only twin of `locate_on_filesystem` — takes `&BuildCtx` so
/// it composes with `&self` consumers. Skips the lazy texture-index
/// rebuild that the mut-version triggers; we just probe the
/// search-paths list directly. Adequate for the metal/rough packer
/// since the per-texture index is already warm by the time we hit
/// this codepath.
fn locate_on_filesystem_const(ctx: &BuildCtx<'_, '_>, clean: &str) -> Option<std::path::PathBuf> {
    for root in ctx.search_paths {
        let candidate = root.join(clean);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Decode two byte streams (PNG/JPEG/etc.) and produce a single
/// RGBA8 Bevy `Image` with G = roughness's R, B = metallic's R.
/// Resizes the smaller one to match if dimensions differ.
fn pack_metal_rough(rough_bytes: &[u8], metal_bytes: &[u8]) -> Option<Image> {
    use image::GenericImageView;
    let rough = image::load_from_memory(rough_bytes).ok()?;
    let metal = image::load_from_memory(metal_bytes).ok()?;

    // Take the larger dimension as canvas; resize the other to match.
    // Production textures are usually authored at the same size, so
    // this branch only fires when an exporter mismatched LOD-pair.
    let (w, h) = {
        let (rw, rh) = rough.dimensions();
        let (mw, mh) = metal.dimensions();
        (rw.max(mw), rh.max(mh))
    };
    let rough_rgba = if rough.dimensions() != (w, h) {
        rough
            .resize_exact(w, h, image::imageops::FilterType::Triangle)
            .to_rgba8()
    } else {
        rough.to_rgba8()
    };
    let metal_rgba = if metal.dimensions() != (w, h) {
        metal
            .resize_exact(w, h, image::imageops::FilterType::Triangle)
            .to_rgba8()
    } else {
        metal.to_rgba8()
    };

    // Pack RGBA: R=0, G=rough.R, B=metal.R, A=255.
    let mut packed = vec![0u8; (w * h * 4) as usize];
    for ((dst, r), m) in packed
        .chunks_exact_mut(4)
        .zip(rough_rgba.pixels())
        .zip(metal_rgba.pixels())
    {
        dst[0] = 0;
        dst[1] = r[0];
        dst[2] = m[0];
        dst[3] = 255;
    }

    // Hand to Bevy as a fresh non-sRGB RGBA8 image.
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        packed,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    img.sampler = usd_texture_sampler();
    Some(img)
}

/// Decode `bytes` to `Image`, register as a labeled sub-asset, cache the
/// handle under `cache_key`. Returns `None` on decode failure.
fn decode_and_register(
    ctx: &mut BuildCtx<'_, '_>,
    cache_key: &str,
    bytes: &[u8],
    is_srgb: bool,
    source: &str,
) -> Option<Handle<Image>> {
    let full_key = format!("{cache_key}?srgb={is_srgb}");
    if let Some(h) = ctx.embedded_textures.get(&full_key) {
        return Some(h.clone());
    }
    let ext = Path::new(cache_key)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    match Image::from_buffer(
        bytes,
        ImageType::Extension(ext),
        default_compressed_formats(),
        is_srgb,
        usd_texture_sampler(),
        RenderAssetUsages::default(),
    ) {
        Ok(img) => {
            let label = format!("Image:{full_key}");
            let handle = ctx.lc.add_labeled_asset(label, img);
            ctx.embedded_textures.insert(full_key, handle.clone());
            Some(handle)
        }
        Err(err) => {
            bevy::log::warn!("texture({source}): decode {cache_key} failed: {err}");
            None
        }
    }
}

fn usd_texture_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..Default::default()
    })
}

/// Locate a texture on disk from a set of search roots — the loader-context-
/// free core of [`locate_on_filesystem`] (direct join + parent-walk; skips the
/// cached basename index). Used by the live-update [`AssetServerTextures`].
pub(crate) fn resolve_texture_fs(search_paths: &[PathBuf], clean: &str) -> Option<PathBuf> {
    let p = Path::new(clean);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    for root in search_paths {
        let candidate = root.join(clean);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let basename = Path::new(clean).file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
    for root in search_paths {
        let mut cur = root.clone();
        for _ in 0..3 {
            let Some(parent) = cur.parent() else { break };
            cur = parent.to_path_buf();
            let direct = cur.join(clean);
            if direct.is_file() {
                return Some(direct);
            }
            if let Some(ref bn) = basename {
                let sibling = cur.join("textures").join(bn);
                if sibling.is_file() {
                    return Some(sibling);
                }
            }
        }
    }
    None
}

/// [`TextureSource`] backed by the live [`AssetServer`] — used for incremental
/// updates (variant switches) where there's no `LoadContext`. Resolves the
/// texture against the stage's search roots and loads it by absolute path
/// (cached by the asset server, shared with the bake path).
pub(crate) struct AssetServerTextures<'a> {
    pub asset_server: &'a bevy::asset::AssetServer,
    pub search_paths: &'a [PathBuf],
}

impl TextureSource for AssetServerTextures<'_> {
    fn load(&mut self, path: &str, channel: TextureChannel) -> Option<Handle<Image>> {
        let clean = path.strip_prefix("./").unwrap_or(path);
        let resolved = resolve_texture_fs(self.search_paths, clean)?;
        let is_srgb = channel.is_srgb();
        let sampler = usd_texture_sampler();
        Some(self.asset_server.load_with_settings::<Image, bevy::image::ImageLoaderSettings>(
            resolved.to_string_lossy().into_owned(),
            move |s: &mut bevy::image::ImageLoaderSettings| {
                s.is_srgb = is_srgb;
                s.sampler = sampler.clone();
            },
        ))
    }
    fn load_packed(&mut self, roughness: &str, _metallic: &str) -> Option<Handle<Image>> {
        // Packing two source images needs CPU decode (no LoadContext here);
        // for the live path fall back to the roughness map — metallic still
        // applies via the scalar factor.
        self.load(roughness, TextureChannel::Linear)
    }
    fn can_resolve(&mut self, path: &str) -> bool {
        let clean = path.strip_prefix("./").unwrap_or(path);
        resolve_texture_fs(self.search_paths, clean).is_some()
    }
}

/// Find a texture on disk. Tries the obvious direct join first; on miss,
/// walks the search roots once (lazy + cached) and looks the texture up by
/// basename.
fn locate_on_filesystem(ctx: &mut BuildCtx<'_, '_>, clean: &str) -> Option<PathBuf> {
    // Absolute → direct check.
    let p = Path::new(clean);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }

    // Direct joins: <search>/clean.
    for root in ctx.search_paths {
        let candidate = root.join(clean);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Walk up to 3 parents of each search root and try <parent>/<clean>
    // and <parent>/textures/<basename>. Real-world Blender-export layouts
    // put the .usdc under e.g. `project/Exports/` while the texture lives
    // at `project/textures/foo.png` — one level up. Limited depth so we
    // don't accidentally probe `/`.
    let basename = Path::new(clean)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    for root in ctx.search_paths {
        let mut cur = root.clone();
        for _ in 0..3 {
            let Some(parent) = cur.parent() else { break };
            cur = parent.to_path_buf();
            let direct = cur.join(clean);
            if direct.is_file() {
                return Some(direct);
            }
            if let Some(ref bn) = basename {
                let sibling = cur.join("textures").join(bn);
                if sibling.is_file() {
                    return Some(sibling);
                }
            }
        }
    }

    // Basename index: covers the common Isaac case where a material in
    // `<root>/greenhouse/materials/foo.usd` authors a texture as
    // `./textures/bar.jpg` meaning `<root>/greenhouse/textures/bar.jpg`.
    let basename = Path::new(clean)
        .file_name()
        .and_then(|n| n.to_str())?
        .to_string();
    let index = ctx
        .texture_index
        .get_or_insert_with(|| build_texture_index(ctx.search_paths));
    // Prefer case-sensitive, fall back to case-insensitive (Isaac scenes
    // mix `.PNG` and `.png` in the same tree).
    if let Some(p) = index.get(&basename) {
        return Some(p.clone());
    }
    let lower = basename.to_ascii_lowercase();
    index
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == lower)
        .map(|(_, v)| v.clone())
}

/// Walk every search path recursively and collect `basename → absolute path`
/// for every file with a known image extension. Multiple matches: last wins
/// (rare; Isaac scenes typically have unique basenames).
fn build_texture_index(roots: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut idx = HashMap::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .map(|e| IMAGE_EXTENSIONS.iter().any(|allowed| *allowed == e))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                idx.insert(name.to_string(), path);
            }
        }
    }
    bevy::log::debug!(
        "texture index built: {} entries from {:?}",
        idx.len(),
        roots
    );
    idx
}

/// Find a USDZ entry by asset path. USD pipelines emit several flavours:
/// `./tex/foo.png`, `tex/foo.png`, sometimes percent-encoded. Try the
/// obvious normalizations before giving up.
fn lookup_embedded<'a>(
    embedded: &'a HashMap<String, Vec<u8>>,
    asset_path: &str,
) -> Option<&'a Vec<u8>> {
    if let Some(v) = embedded.get(asset_path) {
        return Some(v);
    }
    let trimmed = asset_path.trim_start_matches("./");
    if trimmed != asset_path
        && let Some(v) = embedded.get(trimmed)
    {
        return Some(v);
    }
    // `foo.png` should match `some/path/foo.png` when there's exactly one.
    let mut matches = embedded
        .iter()
        .filter(|(k, _)| k.ends_with(asset_path) || k.ends_with(trimmed));
    match (matches.next(), matches.next()) {
        (Some((_, v)), None) => Some(v),
        _ => None,
    }
}

fn default_compressed_formats() -> bevy::image::CompressedImageFormats {
    bevy::image::CompressedImageFormats::NONE
}
