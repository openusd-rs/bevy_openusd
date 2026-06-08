//! `UsdMedia.SpatialAudio` — sound-source metadata, read from the stage via openusd.

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{read_asset_path, read_double_or_timecode, read_token_or_string};

#[derive(Debug, Clone, Default)]
pub struct ReadSpatialAudio {
    pub file_path: Option<String>,
    pub aural_mode: Option<String>,
    pub playback_mode: Option<String>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub media_offset: Option<f64>,
    pub gain: Option<f64>,
}

pub fn read_spatial_audio(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadSpatialAudio>> {
    let read = ReadSpatialAudio {
        file_path: read_asset_path(stage, prim, "filePath")?,
        aural_mode: read_token_or_string(stage, prim, "auralMode")?,
        playback_mode: read_token_or_string(stage, prim, "playbackMode")?,
        start_time: read_double_or_timecode(stage, prim, "startTime")?,
        end_time: read_double_or_timecode(stage, prim, "endTime")?,
        media_offset: read_double_or_timecode(stage, prim, "mediaOffset")?,
        gain: read_double_or_timecode(stage, prim, "gain")?,
    };
    if read.file_path.is_none()
        && read.aural_mode.is_none()
        && read.playback_mode.is_none()
        && read.start_time.is_none()
        && read.end_time.is_none()
        && read.gain.is_none()
    {
        return Ok(None);
    }
    Ok(Some(read))
}
