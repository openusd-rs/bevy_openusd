//! UsdLux readers — typed light descriptions decoded from the composed
//! stage via openusd. The Bevy-side mapping lives in [`crate::light`].

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::*;

/// Inputs shared across every UsdLux light. `None` = inherit the UsdLux default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LightCommon {
    pub intensity: Option<f32>,
    pub exposure: Option<f32>,
    pub color: Option<[f32; 3]>,
    pub diffuse: Option<f32>,
    pub specular: Option<f32>,
    pub enable_color_temperature: Option<bool>,
    pub color_temperature: Option<f32>,
    pub normalize: Option<bool>,
    pub light_link_targets: Vec<String>,
    pub shadow_link_targets: Vec<String>,
    pub light_filters: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadDistantLight {
    pub common: LightCommon,
    pub angle_deg: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadSphereLight {
    pub common: LightCommon,
    pub radius: Option<f32>,
    pub cone_angle_deg: Option<f32>,
    pub cone_softness: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadRectLight {
    pub common: LightCommon,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadDiskLight {
    pub common: LightCommon,
    pub radius: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadCylinderLight {
    pub common: LightCommon,
    pub length: Option<f32>,
    pub radius: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadDomeLight {
    pub common: LightCommon,
    pub texture_file: Option<String>,
    pub texture_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadLight {
    Distant(ReadDistantLight),
    Sphere(ReadSphereLight),
    Rect(ReadRectLight),
    Disk(ReadDiskLight),
    Cylinder(ReadCylinderLight),
    Dome(ReadDomeLight),
}

pub fn is_light_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "DistantLight"
            | "SphereLight"
            | "RectLight"
            | "DiskLight"
            | "CylinderLight"
            | "DomeLight"
            | "GeometryLight"
            | "PortalLight"
    )
}

pub fn read_light(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadLight>> {
    let Some(type_name) = stage.prim_at(prim.clone()).type_name()? else {
        return Ok(None);
    };
    Ok(match type_name.as_str() {
        "DistantLight" => Some(ReadLight::Distant(read_distant_light(stage, prim)?)),
        "SphereLight" => Some(ReadLight::Sphere(read_sphere_light(stage, prim)?)),
        "RectLight" => Some(ReadLight::Rect(read_rect_light(stage, prim)?)),
        "DiskLight" => Some(ReadLight::Disk(read_disk_light(stage, prim)?)),
        "CylinderLight" => Some(ReadLight::Cylinder(read_cylinder_light(stage, prim)?)),
        "DomeLight" => Some(ReadLight::Dome(read_dome_light(stage, prim)?)),
        _ => None,
    })
}

fn read_common(stage: &Stage, prim: &Path) -> anyhow::Result<LightCommon> {
    Ok(LightCommon {
        intensity: read_f32(stage, prim, "inputs:intensity")?,
        exposure: read_f32(stage, prim, "inputs:exposure")?,
        color: read_vec3f(stage, prim, "inputs:color")?,
        diffuse: read_f32(stage, prim, "inputs:diffuse")?,
        specular: read_f32(stage, prim, "inputs:specular")?,
        enable_color_temperature: read_bool(stage, prim, "inputs:enableColorTemperature")?,
        color_temperature: read_f32(stage, prim, "inputs:colorTemperature")?,
        normalize: read_bool(stage, prim, "inputs:normalize")?,
        light_link_targets: read_rel_targets(stage, prim, "light:link")?,
        shadow_link_targets: read_rel_targets(stage, prim, "shadow:link")?,
        light_filters: read_rel_targets(stage, prim, "light:filters")?,
    })
}

fn read_distant_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadDistantLight> {
    Ok(ReadDistantLight {
        common: read_common(stage, prim)?,
        angle_deg: read_f32(stage, prim, "inputs:angle")?,
    })
}

fn read_sphere_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadSphereLight> {
    Ok(ReadSphereLight {
        common: read_common(stage, prim)?,
        radius: read_f32(stage, prim, "inputs:radius")?,
        cone_angle_deg: read_f32(stage, prim, "inputs:shaping:cone:angle")?,
        cone_softness: read_f32(stage, prim, "inputs:shaping:cone:softness")?,
    })
}

fn read_rect_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadRectLight> {
    Ok(ReadRectLight {
        common: read_common(stage, prim)?,
        width: read_f32(stage, prim, "inputs:width")?,
        height: read_f32(stage, prim, "inputs:height")?,
    })
}

fn read_disk_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadDiskLight> {
    Ok(ReadDiskLight {
        common: read_common(stage, prim)?,
        radius: read_f32(stage, prim, "inputs:radius")?,
    })
}

fn read_cylinder_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadCylinderLight> {
    Ok(ReadCylinderLight {
        common: read_common(stage, prim)?,
        length: read_f32(stage, prim, "inputs:length")?,
        radius: read_f32(stage, prim, "inputs:radius")?,
    })
}

fn read_dome_light(stage: &Stage, prim: &Path) -> anyhow::Result<ReadDomeLight> {
    Ok(ReadDomeLight {
        common: read_common(stage, prim)?,
        texture_file: read_asset_path(stage, prim, "inputs:texture:file")?,
        texture_format: read_token_or_string(stage, prim, "inputs:texture:format")?,
    })
}
