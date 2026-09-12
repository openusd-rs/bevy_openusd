//! Media + volume coverage routes (PLAN Phase 7): `UsdMediaSpatialAudio` and
//! `UsdVolume` prims → typed marker components.
//!
//! Like [`super::physics`], Bevy has no built-in spatial-audio graph or volume
//! renderer that maps 1:1 to these schemas, so these routes project **data
//! markers** (read through openusd's `media`/`vol` schemas) that an app can
//! query to wire up its own backend (`bevy_audio`, a fog/VDB volume renderer).
//! This is the coverage hook the plan calls for; playback/rendering is out of
//! scope.

use bevy::prelude::*;
use openusd_schemas::media::SpatialAudioSchema;

use openusd::sdf::Value;
use openusd_schemas::media::SpatialAudio;
use openusd_schemas::vol::{Field3DAsset, FieldAsset, OpenVDBAsset, Volume};
use openusd::usd::Stage;

use super::{PrimRoute, RouteCtx};

/// A `UsdMediaSpatialAudio` prim: an audio source placed in the scene.
///
/// Carries the composed sound file (resolved as a plain path string — an app
/// turns it into a `Handle<AudioSource>`) plus the playback envelope, so a
/// backend can schedule it without re-reading the stage.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdSpatialAudio {
    /// The referenced audio file (`filePath`), as an unresolved path string.
    pub file: String,
    /// `auralMode`: `"spatial"` (positional) or `"nonSpatial"` (ambient).
    pub aural_mode: Option<String>,
    /// `playbackMode`: `"onceFromStart"`, `"loopFromStart"`, …
    pub playback_mode: Option<String>,
    /// Linear gain multiplier (`gain`), default `1.0`.
    pub gain: f64,
}

/// A `Volume` prim: a container binding named fields (density, temperature…)
/// to `FieldAsset` prims. Each binding is resolved through to the referenced
/// `OpenVDBAsset`/`Field3DAsset` (PLAN Phase F), so a volume renderer has the
/// actual `.vdb`/`.f3d` file + grid name without re-walking the stage.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdVolume {
    pub fields: Vec<UsdVolumeField>,
}

/// One resolved field of a [`UsdVolume`].
#[derive(Debug, Clone, PartialEq)]
pub struct UsdVolumeField {
    /// The volume's binding name for this field (e.g. `"density"`).
    pub binding: String,
    /// The bound `FieldAsset` prim path.
    pub prim: String,
    /// The on-disk voxel file (`filePath`), unresolved path string. Empty when
    /// the target isn't a `FieldAsset` (only the binding is known).
    pub file: String,
    /// The grid to read from the file (`fieldName`); one file holds several.
    pub grid: Option<String>,
    /// `fieldIndex` disambiguator when several grids share a name.
    pub index: Option<i32>,
}

fn path_string(v: Option<Value>) -> Option<String> {
    match v? {
        Value::AssetPath(a) => Some(a.as_str().to_string()),
        Value::String(s) => Some(s),
        Value::Token(t) => Some(t.as_str().to_string()),
        _ => None,
    }
}

fn token_string(v: Option<Value>) -> Option<String> {
    match v? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }
}

/// Projects `SpatialAudio` prims as [`UsdSpatialAudio`] markers.
pub struct SpatialAudioRoute;

impl PrimRoute for SpatialAudioRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<UsdSpatialAudio>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        // Gate on the composed typeName so the schema read only runs for the
        // handful of audio prims, not every prim on the stage.
        ctx.type_name.as_deref() == Some("SpatialAudio")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(audio)) = SpatialAudio::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdSpatialAudio {
            file: path_string(audio.file_path_attr().get::<Value>().ok().flatten())
                .unwrap_or_default(),
            aural_mode: token_string(audio.aural_mode_attr().get::<Value>().ok().flatten()),
            playback_mode: token_string(
                audio.playback_mode_attr().get::<Value>().ok().flatten(),
            ),
            gain: audio.gain_attr().get::<f64>().ok().flatten().unwrap_or(1.0),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

/// Projects `Volume` prims as [`UsdVolume`] markers.
pub struct VolumeRoute;

impl PrimRoute for VolumeRoute {
    fn remove(&self, _: &RouteCtx, world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<UsdVolume>();
    }
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Volume")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(vol)) = Volume::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let fields = vol
            .field_paths()
            .unwrap_or_default()
            .into_iter()
            .map(|(binding, path)| resolve_field(ctx.stage, binding, path))
            .collect();
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(UsdVolume { fields });
        }
    }
}

/// Resolve a `(binding, targetPrim)` volume field through to its `FieldAsset`
/// (OpenVDB or Field3D). If the target isn't a field asset, `file` is left
/// empty — the binding is still surfaced.
fn resolve_field(stage: &Stage, binding: String, path: openusd::sdf::Path) -> UsdVolumeField {
    let prim = path.as_str().to_string();
    // A `FieldAsset` (either concrete type) exposes file/grid/index via the trait.
    let (file, grid, index) = if let Ok(Some(f)) = OpenVDBAsset::get(stage, path.clone()) {
        field_asset_data(&f)
    } else if let Ok(Some(f)) = Field3DAsset::get(stage, path.clone()) {
        field_asset_data(&f)
    } else {
        (String::new(), None, None)
    };
    UsdVolumeField {
        binding,
        prim,
        file,
        grid,
        index,
    }
}

/// Pull `filePath` / `fieldName` / `fieldIndex` off any `FieldAsset`.
fn field_asset_data<F: FieldAsset>(f: &F) -> (String, Option<String>, Option<i32>) {
    let file = path_string(f.file_path_attr().get::<Value>().ok().flatten()).unwrap_or_default();
    let grid = token_string(f.field_name_attr().get::<Value>().ok().flatten());
    let index = f.field_index_attr().get::<i32>().ok().flatten();
    (file, grid, index)
}

#[cfg(test)]
mod tests {
    use openusd_schemas::vol::VolumeFieldAsset;
    use super::*;
    use crate::live::{LiveStage, PrimEntities, project_stage};
    use crate::route::SchemaRegistry;
    use openusd::sdf;
    use openusd::usd::Stage;

    #[test]
    fn spatial_audio_projects_marker() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("audio.usda").unwrap();
        stage.define_prim("/World").unwrap().set_type_name("Xform").unwrap();
        let audio = SpatialAudio::define(&stage, "/World/Ambient").unwrap();
        audio
            .create_file_path_attr()
            .unwrap()
            .set(sdf::Value::AssetPath("./ambient.wav".into()))
            .unwrap();
        audio.create_gain_attr().unwrap().set(0.5_f64).unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/World/Ambient").unwrap();
        let m = world.get::<UsdSpatialAudio>(e).expect("audio marker");
        assert_eq!(m.file, "./ambient.wav");
        assert_eq!(m.gain, 0.5);
    }

    #[test]
    fn volume_resolves_field_assets() {
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("vol.usda").unwrap();
        stage.define_prim("/World").unwrap().set_type_name("Xform").unwrap();
        // The field target is a real OpenVDBAsset with a file + grid name.
        let field = OpenVDBAsset::define(&stage, "/World/density").unwrap();
        field
            .create_file_path_attr()
            .unwrap()
            .set(sdf::Value::AssetPath("./smoke.vdb".into()))
            .unwrap();
        field
            .create_field_name_attr()
            .unwrap()
            .set(sdf::Value::Token("density".into()))
            .unwrap();
        let vol = Volume::define(&stage, "/World/Fog").unwrap();
        vol.create_field_relationship("density", "/World/density")
            .unwrap();

        let live = LiveStage::new(stage);
        let mut world = World::new();
        world.insert_resource(SchemaRegistry::builtin());
        let mut map = PrimEntities::default();
        project_stage(&mut world, &live, &mut map);

        let e = map.entity("/World/Fog").unwrap();
        let m = world.get::<UsdVolume>(e).expect("volume marker");
        assert_eq!(m.fields.len(), 1);
        let f = &m.fields[0];
        assert_eq!(f.binding, "density");
        assert_eq!(f.prim, "/World/density");
        assert_eq!(f.file, "./smoke.vdb", "resolved through to the .vdb file");
        assert_eq!(f.grid.as_deref(), Some("density"), "grid name from fieldName");
    }
}
