//! `UsdRender` read side: `RenderSettings`, `RenderProduct`, `RenderVar` —
//! render-config metadata surfaced from the stage via openusd.

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{read_asset_path, read_f32, read_rel_targets, read_token_or_string, read_token_vec, read_vec2i};

#[derive(Debug, Clone)]
pub struct ReadRenderSettings {
    pub path: String,
    pub resolution: Option<[i32; 2]>,
    pub pixel_aspect_ratio: Option<f32>,
    pub aspect_ratio_conform_policy: Option<String>,
    pub products: Vec<String>,
    pub included_purposes: Vec<String>,
    pub material_binding_purposes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReadRenderProduct {
    pub path: String,
    pub product_type: Option<String>,
    pub product_name: Option<String>,
    pub camera: Option<String>,
    pub ordered_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReadRenderVar {
    pub path: String,
    pub data_type: Option<String>,
    pub source_name: Option<String>,
    pub source_type: Option<String>,
}

fn type_name(stage: &Stage, prim: &Path) -> anyhow::Result<String> {
    Ok(stage.prim_at(prim.clone()).type_name()?.unwrap_or_default())
}

pub fn read_render_settings(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadRenderSettings>> {
    if type_name(stage, prim)? != "RenderSettings" {
        return Ok(None);
    }
    Ok(Some(ReadRenderSettings {
        path: prim.as_str().to_string(),
        resolution: read_vec2i(stage, prim, "resolution")?,
        pixel_aspect_ratio: read_f32(stage, prim, "pixelAspectRatio")?,
        aspect_ratio_conform_policy: read_token_or_string(stage, prim, "aspectRatioConformPolicy")?,
        products: read_rel_targets(stage, prim, "products")?,
        included_purposes: read_token_vec(stage, prim, "includedPurposes")?,
        material_binding_purposes: read_token_vec(stage, prim, "materialBindingPurposes")?,
    }))
}

pub fn read_render_product(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadRenderProduct>> {
    if type_name(stage, prim)? != "RenderProduct" {
        return Ok(None);
    }
    Ok(Some(ReadRenderProduct {
        path: prim.as_str().to_string(),
        product_type: read_token_or_string(stage, prim, "productType")?,
        product_name: read_asset_path(stage, prim, "productName")?,
        camera: read_rel_targets(stage, prim, "camera")?.into_iter().next(),
        ordered_vars: read_rel_targets(stage, prim, "orderedVars")?,
    }))
}

pub fn read_render_var(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadRenderVar>> {
    if type_name(stage, prim)? != "RenderVar" {
        return Ok(None);
    }
    Ok(Some(ReadRenderVar {
        path: prim.as_str().to_string(),
        data_type: read_token_or_string(stage, prim, "dataType")?,
        source_name: read_asset_path(stage, prim, "sourceName")?,
        source_type: read_token_or_string(stage, prim, "sourceType")?,
    }))
}
