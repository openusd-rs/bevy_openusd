//! UsdPreviewSurface → `bevy::pbr::StandardMaterial`.
//!
//! `crate::read::shade::read_preview_material` extracts the authored inputs
//! from the Material prim + its surface Shader; this module turns those
//! inputs into a fully-textured Bevy material. Textures are resolved via
//! [`crate::texture::load_texture`] so colour space is correct per channel.
//!
//! Bevy and USD disagree on some conventions:
//!
//! - `diffuseColor` in USD is linear RGB; `StandardMaterial::base_color`
//!   takes `Color`. We go through `LinearRgba::rgb` so no gamma is reapplied.
//! - `normal` textures in UsdPreviewSurface sample `[0, 1]` and are remapped
//!   to `[-1, 1]` with `inputs:scale = (2,2,2,1)` + `inputs:bias = (-1,-1,-1,0)`.
//!   Bevy expects that remap *inside* the texture (tangent-space normal map),
//!   so we just hand the raw linear image through — the authored scale/bias
//!   is the renderer's responsibility under UsdPreviewSurface semantics.
//! - `opacity` < 1.0 or textured → `AlphaMode::Blend`; `opacityThreshold` > 0
//!   switches to `AlphaMode::Mask(threshold)`.

use bevy::asset::{Handle, LoadContext};
use bevy::color::{Color, LinearRgba};
use bevy::pbr::StandardMaterial;
use crate::read::shade::ReadPreviewMaterial;

use crate::build::BuildCtx;
use crate::texture::{TextureChannel, load_texture};

/// Build a Bevy `StandardMaterial` from a decoded UsdPreviewSurface.
///
/// Calls `lc.loader()` to register every referenced texture as a dependent
/// asset; the returned `StandardMaterial` is then registered separately by
/// the caller via `lc.add_labeled_asset`.
pub fn standard_material_from_usd(
    ctx: &mut BuildCtx<'_, '_>,
    read: &ReadPreviewMaterial,
) -> StandardMaterial {
    // Pixar's UsdPreviewSurface defaults (metallic=0, roughness=0.5) translate
    // into Bevy's PBR as a semi-glossy plastic surface — too shiny / reflective
    // for unlit-style or DCC-imported assets where the author expects a
    // flat-shaded read. Default to max metallic + max roughness so the base
    // colour shows through cleanly; authored opinions still override below.
    let mut mat = StandardMaterial {
        base_color: Color::linear_rgb(0.8, 0.8, 0.8),
        perceptual_roughness: 1.0,
        metallic: 1.0,
        ..Default::default()
    };

    if let Some([r, g, b]) = read.diffuse_color {
        mat.base_color = Color::LinearRgba(LinearRgba::rgb(r, g, b));
    }
    if let Some(r) = read.roughness {
        mat.perceptual_roughness = r;
    }
    if let Some(m) = read.metallic {
        mat.metallic = m;
    }
    if let Some([r, g, b]) = read.emissive_color {
        mat.emissive = LinearRgba::rgb(r, g, b);
    }
    if let Some(ior) = read.ior {
        mat.ior = ior;
    }

    // Texture maps. Colour space flows from the channel kind, not from the
    // USD-authored `sourceColorSpace` token — we trust the M3 convention
    // that diffuse/emissive are sRGB and the rest are linear. (M3.1 can
    // read `inputs:sourceColorSpace` directly if authoring gets sloppy.)
    //
    // Bevy's StandardMaterial multiplies texture samples by the scalar
    // `base_color` / `metallic` / `perceptual_roughness` values. Leaving
    // those at the USD-side defaults (grey 0.8, metallic 0, roughness 0.5)
    // would darken / zero out textured materials. When a texture is bound
    // and the matching scalar wasn't authored, reset the factor to unity
    // (1.0 / WHITE) so the texture passes through unchanged.
    if let Some(path) = read.diffuse_texture.as_deref() {
        mat.base_color_texture = load_texture(ctx, path, TextureChannel::Srgb);
        if read.diffuse_color.is_none() {
            // base_color multiplies the sampled texture — use WHITE so
            // texture colours pass through unchanged.
            mat.base_color = Color::WHITE;
        }
    }
    if let Some(path) = read.normal_texture.as_deref() {
        mat.normal_map_texture = load_texture(ctx, path, TextureChannel::Linear);
    }
    if let Some(path) = read.occlusion_texture.as_deref() {
        mat.occlusion_texture = load_texture(ctx, path, TextureChannel::Linear);
    }
    if let Some(path) = read.emissive_texture.as_deref() {
        mat.emissive_texture = load_texture(ctx, path, TextureChannel::Srgb);
    }
    // Roughness + metallic bind to the same combined texture slot.
    // Bevy's `metallic_roughness_texture` expects a SINGLE RGBA texture
    // with G = roughness, B = metallic (glTF spec). USD authors them as
    // TWO independent texture assets. When both are authored, composite
    // into one packed image so neither side gets dropped. When only one
    // is authored, hand the single image to the slot — Bevy will sample
    // the right channel at shade time.
    //
    // Both factors already start at 1.0 (the unity multiplier Bevy
    // applies to sampled texture values), so we just bind the textures
    // and let the data pass through unmodified.
    match (
        read.metallic_texture.as_deref(),
        read.roughness_texture.as_deref(),
    ) {
        (Some(m_path), Some(r_path)) if m_path != r_path => {
            mat.metallic_roughness_texture =
                crate::texture::load_metallic_roughness_packed(ctx, r_path, m_path);
        }
        (Some(path), _) | (_, Some(path)) => {
            mat.metallic_roughness_texture = load_texture(ctx, path, TextureChannel::Linear);
        }
        (None, None) => {}
    }

    // Alpha. `opacityThreshold` wins over `opacity` (UsdPreviewSurface says
    // the threshold kicks the shader into opaque-mask mode).
    use bevy::render::alpha::AlphaMode;
    if let Some(thr) = read.opacity_threshold.filter(|t| *t > 0.0) {
        mat.alpha_mode = AlphaMode::Mask(thr);
    } else if read.opacity.map(|o| o < 1.0).unwrap_or(false) || read.opacity_texture.is_some() {
        mat.alpha_mode = AlphaMode::Blend;
        if let Some(o) = read.opacity {
            let LinearRgba {
                red, green, blue, ..
            } = mat.base_color.into();
            mat.base_color = Color::LinearRgba(LinearRgba {
                red,
                green,
                blue,
                alpha: o,
            });
        }
    }

    mat
}

/// Small helper to let callers construct a default "Material:Default"
/// StandardMaterial without going through UsdShade at all.
pub fn default_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgb(0.72, 0.72, 0.75),
        perceptual_roughness: 0.8,
        metallic: 0.0,
        ..Default::default()
    }
}

/// Register a `StandardMaterial` as a labeled sub-asset under
/// `"Material:<prim_path>"`. Returns the handle.
pub fn add_material_labeled(
    lc: &mut LoadContext<'_>,
    prim_path: &str,
    mat: StandardMaterial,
) -> Handle<StandardMaterial> {
    lc.add_labeled_asset(format!("Material:{prim_path}"), mat)
}
