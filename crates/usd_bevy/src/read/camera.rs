//! UsdGeom.Camera reader — raw post-composition camera values (mm + scene
//! units), decoded from the stage via openusd.

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{read_f32, read_token_or_string, read_vec2f};

/// Decoded `UsdGeom.Camera`. `None` fields use USD defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadCamera {
    pub focal_length_mm: Option<f32>,
    pub h_aperture_mm: Option<f32>,
    pub v_aperture_mm: Option<f32>,
    pub clip_near: Option<f32>,
    pub clip_far: Option<f32>,
    pub projection: Option<Projection>,
    pub focus_distance: Option<f32>,
    pub f_stop: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

impl ReadCamera {
    pub fn vertical_fov_rad(&self) -> f32 {
        let v_aperture = self.v_aperture_mm.unwrap_or(15.2908);
        let focal = self.focal_length_mm.unwrap_or(50.0).max(0.001);
        2.0 * (v_aperture / (2.0 * focal)).atan()
    }

    pub fn horizontal_fov_rad(&self) -> f32 {
        let h_aperture = self.h_aperture_mm.unwrap_or(20.955);
        let focal = self.focal_length_mm.unwrap_or(50.0).max(0.001);
        2.0 * (h_aperture / (2.0 * focal)).atan()
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.h_aperture_mm.unwrap_or(20.955) / self.v_aperture_mm.unwrap_or(15.2908).max(0.001)
    }
}

pub fn is_camera_type(type_name: &str) -> bool {
    type_name == "Camera"
}

pub fn read_camera(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadCamera>> {
    if stage.prim_at(prim.clone()).type_name()?.as_deref() != Some("Camera") {
        return Ok(None);
    }

    let clip = read_vec2f(stage, prim, "clippingRange")?;
    Ok(Some(ReadCamera {
        focal_length_mm: read_f32(stage, prim, "focalLength")?,
        h_aperture_mm: read_f32(stage, prim, "horizontalAperture")?,
        v_aperture_mm: read_f32(stage, prim, "verticalAperture")?,
        clip_near: clip.map(|c| c[0]),
        clip_far: clip.map(|c| c[1]),
        projection: read_projection(stage, prim)?,
        focus_distance: read_f32(stage, prim, "focusDistance")?,
        f_stop: read_f32(stage, prim, "fStop")?,
    }))
}

fn read_projection(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Projection>> {
    Ok(match read_token_or_string(stage, prim, "projection")?.as_deref() {
        Some("perspective") => Some(Projection::Perspective),
        Some("orthographic") => Some(Projection::Orthographic),
        _ => None,
    })
}
