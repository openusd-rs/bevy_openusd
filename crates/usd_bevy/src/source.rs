//! Byte-backed root layers with filesystem-relative dependency resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use openusd::ar::{Asset, DefaultResolver, ResolvedPath, Resolver};
use openusd::usd::Stage;

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

pub(crate) type DiskBaselines = Arc<EditorDisk>;

#[derive(Default)]
pub(crate) struct EditorDisk {
    hashes: Mutex<BTreeMap<PathBuf, blake3::Hash>>,
    pub replacements: Mutex<BTreeMap<String, Arc<[u8]>>>,
    pub snapshots: Mutex<BTreeMap<String, Arc<[u8]>>>,
}

impl std::ops::Deref for EditorDisk {
    type Target = Mutex<BTreeMap<PathBuf, blake3::Hash>>;
    fn deref(&self) -> &Self::Target { &self.hashes }
}

/// An immutable root-layer snapshot anchored at its source filename.
#[derive(Clone, Debug)]
pub struct UsdSource {
    identifier: String,
    bytes: Arc<[u8]>,
    identity: u64,
    files: Arc<BTreeMap<String, Arc<[u8]>>>,
    filesystem: bool,
}

impl UsdSource {
    /// Builds a snapshot-only USDA root using canonical typed schema APIs.
    /// The callback authors one root layer; dependencies are composed afterward
    /// with `with_dependency` and the reference helpers. No source file is written.
    pub fn build(path: impl AsRef<Path>, author: impl FnOnce(&Stage) -> anyhow::Result<()>) -> anyhow::Result<Self> {
        anyhow::ensure!(path.as_ref().extension().is_some_and(|extension| extension == "usda"),
            "typed source construction requires a .usda identifier");
        let mut source = Self::snapshot(path, b"#usda 1.0\n".as_slice())?;
        let stage = source.open_stage()?;
        author(&stage)?;
        anyhow::ensure!(stage.layer_identifiers().len() == 1,
            "typed source construction accepts one root layer; compose dependencies afterward");
        Self::validate_composition(&stage)?;
        source.bytes = stage.root_layer().export_to_string()?.into_bytes().into();
        Self::validate_composition(&source.open_stage()?)?;
        Ok(source)
    }

    /// Anchor bytes at a filename without creating that file.
    pub fn new(path: impl AsRef<Path>, bytes: impl Into<Arc<[u8]>>) -> io::Result<Self> {
        let path = path.as_ref();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let raw = absolute.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "USD source path is not UTF-8")
        })?;
        let identifier = DefaultResolver::new().create_identifier(raw, None);
        Ok(Self {
            identifier,
            bytes: bytes.into(),
            identity: NEXT_SOURCE.fetch_add(1, Ordering::Relaxed),
            files: Arc::default(),
            filesystem: true,
        })
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub(crate) fn filesystem_backed(&self) -> bool { self.filesystem }

    pub fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.identity
    }

    pub(crate) fn read_asset(&self, identifier: &str) -> io::Result<Vec<u8>> {
        if !Path::new(identifier).is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                format!("unresolved asset identifier: {identifier}")));
        }
        SourceResolver {
            source: self.clone(),
            fallback: DefaultResolver::new(),
            requests: Arc::default(),
            disk_baselines: None,
        }
        .open_asset(&ResolvedPath::new(identifier))?
        .read_all()
    }

    pub(crate) fn stage_texture_requests(stage: &Stage) -> Result<BTreeSet<(String, bool)>, String> {
        let mut paths = Vec::new();
        stage
            .traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |path| {
                paths.push(path.clone());
            })
            .map_err(|error| error.to_string())?;
        let mut requests = BTreeSet::new();
        for path in paths {
            let prim = stage.prim(&path).map_err(|error| error.to_string())?;
            if matches!(prim.type_name().map_err(|error| error.to_string())?.as_deref(), Some("DomeLight" | "DomeLight_1")) {
                let attr = prim.attribute("inputs:texture:file");
                let mut times = vec![None];
                times.extend(attr.time_sample_times().map_err(|error| error.to_string())?.into_iter()
                    .map(|time| Some(openusd::usd::TimeCode::new(time))));
                for time in times {
                    let path = crate::route::dome::asset_string(attr.get_at::<openusd::sdf::Value>(time).map_err(|error| error.to_string())?);
                    if !path.is_empty() { requests.insert((path, false)); }
                }
                continue;
            }
            if stage
                .prim(&path)
                .map_err(|error| error.to_string())?
                .type_name()
                .map_err(|error| error.to_string())?
                .as_deref()
                != Some("Material")
            {
                continue;
            }
            let texture_times = crate::read::shade::material_texture_sample_times(stage, &path)
                .map_err(|error| error.to_string())?;
            for time in std::iter::once(None).chain(texture_times.into_iter().map(Some)) {
                let Some(material) = crate::read::shade::read_preview_material_at(stage, &path, time)
                    .map_err(|error| error.to_string())? else { continue; };
                for (path, srgb) in [
                    (&material.diffuse_texture, material.texture_srgb("diffuse")),
                    (&material.emissive_texture, material.texture_srgb("emissive")),
                    (&material.normal_texture, material.texture_srgb("normal")),
                    (&material.metallic_texture, material.texture_srgb("metallic")),
                    (&material.roughness_texture, material.texture_srgb("roughness")),
                    (&material.clearcoat_texture, material.texture_srgb("clearcoat")),
                    (&material.clearcoat_roughness_texture, material.texture_srgb("clearcoat_roughness")),
                    (&material.occlusion_texture, material.texture_srgb("occlusion")),
                    (&material.opacity_texture, material.texture_srgb("opacity")),
                ] {
                    if let Some(path) = path {
                        requests.insert((path.clone(), srgb));
                    }
                }
            }
        }
        Ok(requests)
    }

    /// Anchor bytes without allowing filesystem fallback for missing dependencies.
    /// Bytes may represent a USD layer, package, or opaque dependency asset.
    pub fn snapshot(path: impl AsRef<Path>, bytes: impl Into<Arc<[u8]>>) -> io::Result<Self> {
        let path = path.as_ref();
        let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        let mut source = Self::new(normalize(&absolute), bytes)?;
        source.filesystem = false;
        Ok(source)
    }

    /// Return a snapshot containing another source and its captured dependencies.
    /// Identical identifiers must have identical bytes. The receiver's filesystem
    /// fallback policy is retained; no dependency files are read or created.
    pub fn with_dependency(&self, dependency: &Self) -> io::Result<Self> {
        let mut combined = self.clone();
        let mut changed = false;
        for (identifier, bytes) in std::iter::once((&dependency.identifier, &dependency.bytes)).chain(dependency.files.iter()) {
            let existing = if identifier == &combined.identifier { Some(&combined.bytes) } else { combined.files.get(identifier) };
            if let Some(existing) = existing {
                if existing.as_ref() != bytes.as_ref() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("conflicting USD source bytes: {identifier}")));
                }
            } else {
                Arc::make_mut(&mut combined.files).insert(identifier.clone(), bytes.clone());
                changed = true;
            }
        }
        if changed { combined.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed); }
        Ok(combined)
    }

    /// Applies typed editor commands atomically to a USDA root snapshot.
    /// Retains captured dependencies and filesystem policy; never writes input files.
    /// New asset dependencies must be supplied with `with_dependency` or resolve
    /// through the receiver's filesystem policy. Empty batches preserve identity.
    pub fn with_edits(&self, edits: impl IntoIterator<Item = crate::editor::EditorEdit>) -> anyhow::Result<Self> {
        let edits: Vec<_> = edits.into_iter().collect();
        if edits.is_empty() { return Ok(self.clone()); }
        anyhow::ensure!(self.identifier.ends_with(".usda"), "snapshot editing requires a .usda root identifier");
        let mut editor = crate::editor::EditorSession::new(self.open_stage()?);
        editor.edit(crate::editor::EditorEdit::Batch(edits))?;
        Self::validate_composition(editor.stage())?;
        let mut edited = self.clone();
        edited.bytes = editor.stage().root_layer().export_to_string()?.into_bytes().into();
        edited.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        Ok(edited)
    }

    /// Mounts a source prim at a new absolute prim path in a USDA root snapshot.
    /// Captured dependencies and the receiver's filesystem policy are retained.
    /// Existing destinations, missing targets and conflicting source bytes fail.
    /// An empty typed target path references the dependency's defaultPrim.
    pub fn with_reference(
        &self,
        destination: impl openusd::sdf::IntoPath,
        dependency: &Self,
        target: impl openusd::sdf::IntoPath,
    ) -> anyhow::Result<Self> {
        self.with_references([(destination, dependency, target)])
    }

    /// Atomically mounts (destination, source, target) entries in iterator order.
    /// Opens one assembly stage and one validation stage per distinct source snapshot.
    /// An empty batch returns an unchanged snapshot; inputs are never mutated.
    pub fn with_references<'a, D: openusd::sdf::IntoPath, T: openusd::sdf::IntoPath>(
        &self,
        references: impl IntoIterator<Item = (D, &'a Self, T)>,
    ) -> anyhow::Result<Self> {
        self.with_offset_references(references.into_iter().map(|(destination, source, target)|
            (destination, source, target, openusd::sdf::LayerOffset::IDENTITY)))
    }

    /// Atomically mounts (destination, source, target, time offset) entries.
    /// Offsets must be finite and scales finite and positive.
    /// Empty typed target paths select defaultPrim; input snapshots stay unchanged.
    pub fn with_offset_references<'a, D: openusd::sdf::IntoPath, T: openusd::sdf::IntoPath>(
        &self,
        references: impl IntoIterator<Item = (D, &'a Self, T, openusd::sdf::LayerOffset)>,
    ) -> anyhow::Result<Self> {
        self.assemble_references(references, false)
    }

    /// Mounts retimed references with instanceable metadata on every destination.
    /// Entries use (destination, source, target, offset); descendants are USD proxies.
    /// Composition determines prototype sharing; offsets follow with_offset_references.
    pub fn with_instanceable_references<'a, D: openusd::sdf::IntoPath, T: openusd::sdf::IntoPath>(
        &self,
        references: impl IntoIterator<Item = (D, &'a Self, T, openusd::sdf::LayerOffset)>,
    ) -> anyhow::Result<Self> {
        self.assemble_references(references, true)
    }

    fn assemble_references<'a, D: openusd::sdf::IntoPath, T: openusd::sdf::IntoPath>(
        &self,
        references: impl IntoIterator<Item = (D, &'a Self, T, openusd::sdf::LayerOffset)>,
        instanceable: bool,
    ) -> anyhow::Result<Self> {
        let references = references.into_iter().map(|(destination, source, target, offset)| {
            Ok((openusd::sdf::try_into_path(destination)?, source, openusd::sdf::try_into_path(target)?, offset))
        }).collect::<Result<Vec<_>, openusd::sdf::PathParseError>>()?;
        if references.is_empty() { return Ok(self.clone()); }
        anyhow::ensure!(self.identifier.ends_with(".usda"), "reference assembly requires a .usda root identifier");
        let mut combined = self.clone();
        let mut sources = BTreeMap::new();
        for (destination, dependency, target, offset) in &references {
            anyhow::ensure!(offset.is_valid() && offset.scale > 0.0,
                "reference assembly requires a finite offset and positive finite scale");
            for path in std::iter::once(destination).chain((!target.is_empty()).then_some(target)) {
                anyhow::ensure!(path.as_str().starts_with('/') && path.as_str() != "/"
                    && path.is_prim_path() && !path.contains_prim_variant_selection(),
                    "reference assembly requires absolute non-root prim paths");
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(dependency.revision()) {
                combined = combined.with_dependency(dependency)?;
                entry.insert(dependency.open_stage()?);
            }
            let dependency_stage = &sources[&dependency.revision()];
            if target.is_empty() {
                anyhow::ensure!(dependency_stage.default_prim().is_some(), "reference source has no defaultPrim");
            } else {
                anyhow::ensure!(dependency_stage.prim(target)?.is_valid()?, "reference target does not exist: {target}");
            }
        }
        let stage = combined.open_stage()?;
        for (destination, dependency, target, layer_offset) in references {
            anyhow::ensure!(!stage.prim(&destination)?.is_valid()?, "reference destination already exists: {destination}");
            stage.define_prim(&destination)?;
            crate::authoring::set_references(&stage, destination.as_str(), &[openusd::sdf::Reference {
                asset_path: dependency.identifier.clone(), prim_path: target, layer_offset, ..Default::default()
            }])?;
            if instanceable { stage.prim(&destination)?.set_instanceable(true)?; }
            anyhow::ensure!(stage.prim(&destination)?.is_valid()?, "reference destination did not compose");
        }
        Self::validate_composition(&stage)?;
        combined.bytes = stage.root_layer().export_to_string()?.into_bytes().into();
        combined.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        Ok(combined)
    }

    pub(crate) fn insert_dependency(&mut self, identifier: String, bytes: Vec<u8>) {
        Arc::make_mut(&mut self.files).insert(identifier, bytes.into());
        self.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn replace_file_bytes(&mut self, identifier: String, bytes: Vec<u8>) {
        if identifier == self.identifier {
            self.bytes = bytes.into();
            self.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        } else {
            self.insert_dependency(identifier, bytes);
        }
    }

    #[cfg(test)]
    pub(crate) fn probe(&self) -> (Result<(), String>, BTreeSet<String>) {
        let (result, missing) = self.probe_stage();
        (result.map(|_| ()), missing)
    }

    /// Like `probe`, keeping the validated stage so a caller that needs it
    /// next (texture requests, say) does not parse the source again.
    pub(crate) fn probe_stage(&self) -> (Result<Stage, String>, BTreeSet<String>) {
        let requests = Arc::new(Mutex::new(BTreeSet::new()));
        let result = (|| -> anyhow::Result<Stage> {
            let stage = self.open_tracked(requests.clone())?;
            Self::validate_composition(&stage)?;
            Ok(stage)
        })()
        .map_err(|error| error.to_string());
        let missing = std::mem::take(&mut *requests.lock().expect("dependency requests"));
        (result, missing)
    }

    /// Open an independent stage from this snapshot.
    pub fn open_stage(&self) -> openusd::Result<Stage> {
        self.open_tracked(Arc::default())
    }

    pub(crate) fn open_stage_for_editor(&self) -> openusd::Result<(Stage, DiskBaselines)> {
        let baselines = DiskBaselines::default();
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .resolver(SourceResolver {
                source: self.clone(), fallback: DefaultResolver::new(),
                requests: Arc::default(), disk_baselines: self.filesystem.then_some(baselines.clone()),
            })
            .open(&self.identifier)?;
        Ok((stage, baselines))
    }

    pub(crate) fn validate_composition(stage: &Stage) -> anyhow::Result<()> {
        let mut paths = Vec::new();
        stage.traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |path| paths.push(path.clone()))?;
        for path in paths {
            let prim = stage.prim(&path)?;
            if let Some(openusd::sdf::Value::ReferenceListOp(references)) = prim.get_metadata("references")? {
                for reference in references.explicit_items.iter().chain(&references.prepended_items)
                    .chain(&references.appended_items).chain(&references.added_items) {
                    anyhow::ensure!(reference.layer_offset.is_valid_composition(),
                        "unsupported reference time offset at {path}: {:?}", reference.layer_offset);
                }
            }
            if let Some(openusd::sdf::Value::PayloadListOp(payloads)) = prim.get_metadata("payload")? {
                for payload in payloads.explicit_items.iter().chain(&payloads.prepended_items)
                    .chain(&payloads.appended_items).chain(&payloads.added_items) {
                    anyhow::ensure!(payload.layer_offset.as_ref().is_none_or(openusd::sdf::LayerOffset::is_valid_composition),
                        "unsupported payload time offset at {path}: {:?}", payload.layer_offset);
                }
            }
            for attribute in prim.attributes()? {
                attribute.get::<openusd::sdf::Value>()?;
                let times = attribute.time_sample_times()?;
                if attribute.type_name()?.is_some_and(|name| matches!(name.as_str(), "asset" | "asset[]")) {
                    for time in times {
                        attribute.get_at::<openusd::sdf::Value>(Some(openusd::usd::TimeCode::new(time)))?;
                    }
                }
            }
        }
        for identifier in stage.layer_identifiers() {
            if stage.is_layer_muted(&identifier) { continue; }
            let Some(layer) = stage.layer(&identifier) else { continue; };
            let Some(root) = layer.pseudo_root() else { continue; };
            let sublayers = root.sublayers().unwrap_or_default();
            let offsets = root.get::<Vec<openusd::sdf::LayerOffset>>(openusd::sdf::FieldKey::SubLayerOffsets).unwrap_or_default();
            for (index, offset) in offsets.iter().take(sublayers.len()).enumerate() {
                anyhow::ensure!(offset.is_valid_composition(),
                    "unsupported sublayer time offset in {identifier} at index {index}: {offset:?}");
            }
        }
        let errors = stage.composition_errors();
        anyhow::ensure!(errors.is_empty(), "USD composition failed: {}",
            errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
        Ok(())
    }

    fn open_tracked(&self, requests: Arc<Mutex<BTreeSet<String>>>) -> openusd::Result<Stage> {
        Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .resolver(SourceResolver {
                source: self.clone(),
                fallback: DefaultResolver::new(),
                requests,
                disk_baselines: None,
            })
            .open(&self.identifier)
    }
}

struct SourceResolver {
    source: UsdSource,
    fallback: DefaultResolver,
    requests: Arc<Mutex<BTreeSet<String>>>,
    disk_baselines: Option<DiskBaselines>,
}

struct SharedAsset(Cursor<Arc<[u8]>>);

impl Read for SharedAsset {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> { self.0.read(buffer) }
}

impl Seek for SharedAsset {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> { self.0.seek(position) }
}

impl Asset for SharedAsset {
    fn size(&self) -> io::Result<u64> { Ok(self.0.get_ref().len() as u64) }
}

impl SourceResolver {
    fn bytes(&self, identifier: &str) -> Option<Arc<[u8]>> {
        if let Some(bytes) = self.disk_baselines.as_ref().and_then(|disk|
            disk.replacements.lock().expect("editor source replacements").get(identifier).cloned()) {
            return Some(bytes);
        }
        if identifier == self.source.identifier {
            Some(self.source.bytes.clone())
        } else {
            self.source.files.get(identifier).cloned()
        }
    }

    fn contains_packaged_path(&self, path: &str) -> bool {
        let Some((package, inner)) = openusd::ar::split_package_relative_path_outer(path) else {
            return false;
        };
        let Some(bytes) = self.bytes(&package) else {
            return false;
        };
        zip::ZipArchive::new(Cursor::new(bytes.as_ref()))
            .map(|mut archive| {
                let entry = openusd::ar::split_package_relative_path_outer(&inner).map(|(package, _)| package).unwrap_or(inner);
                archive.by_name(&entry).is_ok()
            })
            .unwrap_or(false)
    }

    fn packaged_bytes(&self, path: &str) -> io::Result<Option<Vec<u8>>> {
        let Some((package, inner)) = openusd::ar::split_package_relative_path_outer(path) else {
            return Ok(None);
        };
        let Some(bytes) = self.bytes(&package) else {
            return Ok(None);
        };
        openusd::ar::read_package_entry(Box::new(SharedAsset(Cursor::new(bytes))), &inner).map(Some)
    }
}

impl Resolver for SourceResolver {
    fn create_identifier(&self, path: &str, anchor: Option<&ResolvedPath>) -> String {
        if !self.source.filesystem
            && !openusd::ar::is_package_relative_path(path)
            && !Path::new(path).is_absolute()
            && let Some(anchor) = anchor
            && !openusd::ar::is_package_relative_path(&anchor.to_string_lossy())
            && !anchor.to_string_lossy().ends_with(".usdz")
        {
            return normalize(&anchor.parent().unwrap_or(Path::new("/")).join(path))
                .to_string_lossy()
                .into_owned();
        }
        self.fallback.create_identifier(path, anchor)
    }

    fn resolve(&self, path: &str) -> Option<ResolvedPath> {
        if self.bytes(path).is_some() {
            Some(ResolvedPath::new(PathBuf::from(path)))
        } else if self.contains_packaged_path(path) {
            Some(ResolvedPath::new(PathBuf::from(path)))
        } else if self.source.filesystem {
            self.fallback.resolve(path)
        } else {
            let dependency = openusd::ar::split_package_relative_path_outer(path)
                .map(|(package, _)| package)
                .unwrap_or_else(|| path.to_string());
            let request = if self.bytes(&dependency).is_some() {
                path.to_string()
            } else {
                dependency
            };
            self.requests
                .lock()
                .expect("dependency requests")
                .insert(request);
            None
        }
    }

    fn resolve_for_new_asset(&self, path: &str) -> Option<ResolvedPath> {
        if self.source.filesystem {
            self.resolve(path)
                .or_else(|| self.fallback.resolve_for_new_asset(path))
        } else {
            Some(ResolvedPath::new(PathBuf::from(path)))
        }
    }

    fn open_asset(&self, path: &ResolvedPath) -> io::Result<Box<dyn Asset>> {
        if let Some(baselines) = &self.disk_baselines {
            let identifier = path.to_string_lossy();
            let packaged = openusd::ar::split_package_relative_path_outer(&identifier);
            let outer = packaged.as_ref().map_or(identifier.as_ref(), |(outer, _)| outer.as_str());
            let bytes: Arc<[u8]> = if let Some(bytes) = self.bytes(outer) {
                bytes.clone()
            } else if self.source.filesystem {
                self.fallback.open_asset(&ResolvedPath::new(outer))?.read_all()?.into()
            } else {
                return Err(io::Error::new(io::ErrorKind::NotFound, outer.to_owned()));
            };
            let key = crate::persistence::destination_identity(Path::new(outer))?;
            baselines.lock().expect("disk baselines").entry(key).or_insert_with(|| blake3::hash(&bytes));
            baselines.snapshots.lock().expect("editor read snapshots").entry(outer.to_owned()).or_insert_with(|| bytes.clone());
            return if let Some((_, inner)) = packaged {
                openusd::ar::read_package_entry(Box::new(SharedAsset(Cursor::new(bytes))), &inner)
                    .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Asset>)
            } else {
                Ok(Box::new(SharedAsset(Cursor::new(bytes))))
            };
        }
        if let Some(bytes) = self.bytes(&path.to_string_lossy()) {
            Ok(Box::new(SharedAsset(Cursor::new(bytes))))
        } else if let Some(bytes) = self.packaged_bytes(&path.to_string_lossy())? {
            Ok(Box::new(Cursor::new(bytes)))
        } else if self.source.filesystem {
            self.fallback.open_asset(path)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                path.to_string_lossy().into_owned(),
            ))
        }
    }

    fn identity(&self) -> String {
        format!(
            "usd-bevy-source:{}:{}",
            self.source.identity, self.source.identifier
        )
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_builder_publishes_independent_snapshot_without_files() {
        use openusd_schemas::geom::{Sphere, SphereSchema};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("typed.usda");
        let source = super::UsdSource::build(&path, |stage| {
            Sphere::define(stage, "/Model")?.create_radius_attr()?.set(2.5_f64)?;
            Ok(())
        }).unwrap();
        assert!(!source.filesystem);
        assert!(!path.exists());
        let first = source.open_stage().unwrap();
        Sphere::define(&first, "/Model").unwrap().create_radius_attr().unwrap().set(4.0_f64).unwrap();
        assert_eq!(source.open_stage().unwrap().prim("/Model").unwrap().attribute("radius").get::<f64>().unwrap(), Some(2.5));
        let assembly = super::UsdSource::build(directory.path().join("assembly.usda"), |_| Ok(())).unwrap()
            .with_reference("/Copy", &source, openusd::sdf::path("/Model").unwrap()).unwrap();
        assert_eq!(assembly.open_stage().unwrap().prim("/Copy").unwrap().attribute("radius").get::<f64>().unwrap(), Some(2.5));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn typed_builder_rejects_failed_authoring_and_missing_composition() {
        let directory = tempfile::tempdir().unwrap();
        assert!(super::UsdSource::build(directory.path().join("wrong.usdc"), |_| panic!("must reject before callback")).is_err());
        let path = directory.path().join("failed.usda");
        assert!(super::UsdSource::build(&path, |_| anyhow::bail!("authoring failed")).is_err());
        assert!(super::UsdSource::build(&path, |stage| {
            crate::authoring::define_prim(stage, "/Missing", "Xform")?;
            crate::authoring::set_references(stage, "/Missing", &[openusd::sdf::Reference {
                asset_path: "missing.usda".into(), ..Default::default()
            }])?;
            Ok(())
        }).is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn nested_packages_preserve_relative_layers_and_asset_bytes() {
        use super::*;
        fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
            let mut writer = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
            for (name, bytes) in entries { writer.add_layer(name, bytes).unwrap(); }
            writer.finish().unwrap().into_inner()
        }
        let inner = archive(&[
            ("scenes/model.usda", b"#usda 1.0\n(defaultPrim = \"Model\")\ndef Xform \"Model\" (references = @part.usda@</Part>) {}\n"),
            ("scenes/part.usda", b"#usda 1.0\ndef Xform \"Part\" {\n    asset paint = @pixel.png@\n    def Cube \"Box\" {}\n}\n"),
            ("scenes/pixel.png", b"captured nested pixels"),
        ]);
        let directory = tempfile::tempdir().unwrap();
        for (name, reference) in [("implicit", "inner.usdz"), ("explicit", "inner.usdz[scenes/model.usda]")] {
            let root = format!("#usda 1.0\ndef Xform \"Root\" (references = @{reference}@</Model>) {{}}\n");
            let bytes = archive(&[("root.usda", root.as_bytes()), ("inner.usdz", &inner)]);
            let virtual_path = directory.path().join(format!("virtual-{name}.usdz"));
            let source = UsdSource::snapshot(&virtual_path, bytes.clone()).unwrap();
            let stage = source.open_stage().unwrap();
            UsdSource::validate_composition(&stage).unwrap();
            assert!(stage.prim("/Root/Box").unwrap().is_valid().unwrap());
            let asset = stage.prim("/Root").unwrap().attribute("paint").get::<openusd::sdf::AssetPath>().unwrap().unwrap();
            assert_eq!(asset.authored_path, "pixel.png");
            assert_eq!(source.read_asset(asset.resolved_path().unwrap()).unwrap(), b"captured nested pixels");
            assert!(!virtual_path.exists());
            assert!(source.read_asset(&format!("{}[inner.usdz[missing.png]]", source.identifier())).is_err());
            let disk = directory.path().join(format!("{name}.usdz"));
            std::fs::write(&disk, &bytes).unwrap();
            let stage = Stage::open(disk.to_str().unwrap()).unwrap();
            UsdSource::validate_composition(&stage).unwrap();
            assert!(stage.prim("/Root/Box").unwrap().is_valid().unwrap());
            let asset = stage.prim("/Root").unwrap().attribute("paint").get::<openusd::sdf::AssetPath>().unwrap().unwrap();
            assert_eq!(DefaultResolver::new().open_asset(&ResolvedPath::new(asset.resolved_path().unwrap())).unwrap().read_all().unwrap(), b"captured nested pixels");
            assert_eq!(std::fs::read(&disk).unwrap(), bytes);
        }
    }

    #[test]
    fn clip_switch_interpolates_to_the_next_activation_sample() {
        let directory = tempfile::tempdir().unwrap();
        let source = super::UsdSource::snapshot(directory.path().join("root.usda"), br#"#usda 1.0
def Sphere "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@a.usda@, @b.usda@]
            double2[] active = [(0, 0), (10, 1)]
            string primPath = "/Model"
        }
    }
) {
    double radius
}
"#.as_slice()).unwrap();
        let a = super::UsdSource::snapshot(directory.path().join("a.usda"), br#"#usda 1.0
def Sphere "Model" {
    double radius.timeSamples = {0: 1, 20: 3}
}
"#.as_slice()).unwrap();
        let b = super::UsdSource::snapshot(directory.path().join("b.usda"), br#"#usda 1.0
def Sphere "Model" {
    double radius.timeSamples = {0: 5, 20: 7}
}
"#.as_slice()).unwrap();
        let stage = source.with_dependency(&a).unwrap().with_dependency(&b).unwrap().open_stage().unwrap();
        let radius = stage.attribute("/Model.radius").unwrap();
        assert_eq!(radius.time_sample_times().unwrap(), [0.0, 10.0, 20.0]);
        for (time, expected) in [(0.0, 1.0), (5.0, 3.5), (9.0, 5.5), (10.0, 6.0), (15.0, 6.5), (20.0, 7.0)] {
            assert_eq!(radius.get_at::<f64>(Some(openusd::usd::TimeCode::new(time))).unwrap(), Some(expected), "time {time}");
        }
    }

    #[test]
    fn probe_discovers_numeric_value_clip_layers() {
        let directory = tempfile::tempdir().unwrap();
        let mut source = super::UsdSource::snapshot(directory.path().join("root.usda"),
            br#"#usda 1.0
def Sphere "Model" (
    clips = {
        dictionary default = {
            asset[] assetPaths = [@clipped_sphere_values.usda@]
            double2[] active = [(0, 0)]
            double2[] times = [(0, 0), (20, 10)]
            string primPath = "/Model"
        }
    }
) { double radius }
"#.as_slice()).unwrap();
        let clip = directory.path().join("clipped_sphere_values.usda").to_string_lossy().into_owned();
        let (_, missing) = source.probe();
        assert!(missing.contains(&clip), "numeric clip not discovered: {missing:?}");
        source.insert_dependency(clip, br#"#usda 1.0
def Sphere "Model" { double radius.timeSamples = {0: 1, 10: 3} }
"#.to_vec());
        let (result, missing) = source.probe();
        result.unwrap();
        assert!(missing.is_empty());
        let stage = source.open_stage().unwrap();
        assert_eq!(stage.attribute("/Model.radius").unwrap().get_at::<f64>(Some(openusd::usd::TimeCode::new(10.0))).unwrap(), Some(2.0));
    }

    use super::*;

    #[test]
    fn reference_assembly_preserves_sources_dependencies_and_reuse() {
        use openusd_schemas::geom::{Sphere, SphereSchema};
        let directory = tempfile::tempdir().unwrap();
        let model_stage = Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .in_memory("model.usda").unwrap();
        Sphere::define(&model_stage, "/Model").unwrap().create_radius_attr().unwrap().set(1.5_f64).unwrap();
        let image = UsdSource::snapshot(directory.path().join("texture.bin"), &b"texture bytes"[..]).unwrap();
        let model = UsdSource::snapshot(directory.path().join("model.usda"),
            model_stage.root_layer().export_to_string().unwrap().into_bytes()).unwrap()
            .with_dependency(&image).unwrap();
        let root = UsdSource::snapshot(directory.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap();
        let before = root.bytes.clone();
        let first = root.with_reference("/Assembly/First", &model, "/Model").unwrap();
        let second = first.with_reference("/Assembly/Second", &model, "/Model").unwrap();
        let stage = second.open_stage().unwrap();
        for path in ["/Assembly/First", "/Assembly/Second"] {
            assert_eq!(stage.prim(path).unwrap().type_name().unwrap().as_deref(), Some("Sphere"));
            assert_eq!(stage.prim(path).unwrap().attribute("radius").get::<f64>().unwrap(), Some(1.5));
        }
        assert_eq!(second.dependencies().count(), 2);
        assert_eq!(second.read_asset(image.identifier()).unwrap(), b"texture bytes");
        assert_eq!(root.bytes, before);
        assert!(!first.open_stage().unwrap().prim("/Assembly/Second").unwrap().is_valid().unwrap());
        assert_eq!(model.open_stage().unwrap().root_layer().export_to_string().unwrap(),
            model_stage.root_layer().export_to_string().unwrap());
        assert!(!root.filesystem && !first.filesystem && !second.filesystem);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn typed_snapshot_edits_preserve_dependencies_and_inputs() {
        use crate::editor::EditorEdit;
        use openusd::sdf::Value;
        let directory = tempfile::tempdir().unwrap();
        let model = UsdSource::snapshot(directory.path().join("model.usda"), &b"#usda 1.0\ndef Cube \"Model\" {}\n"[..]).unwrap();
        let source = UsdSource::snapshot(directory.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap()
            .with_reference("/Object", &model, "/Model").unwrap();
        let original = source.bytes.clone();
        let edited = source.with_edits([
            EditorEdit::Attribute { prim: "/Object".into(), name: "size".into(), type_name: "double".into(), value: Value::Double(3.) },
            EditorEdit::Define { path: "/Other".into(), type_name: "Sphere".into() },
            EditorEdit::RelationshipTargets { prim: "/Other".into(), name: "peer".into(), targets: vec![openusd::sdf::path("/Object").unwrap()] },
        ]).unwrap();
        let stage = edited.open_stage().unwrap();
        assert_eq!(stage.prim("/Object").unwrap().attribute("size").get::<f64>().unwrap(), Some(3.));
        assert!(stage.prim("/Other").unwrap().is_valid().unwrap());
        assert_eq!(edited.files, source.files);
        assert!(!edited.filesystem);
        assert_eq!(edited.identifier, source.identifier);
        assert_ne!(edited.revision(), source.revision());
        assert_eq!(source.bytes, original);
        assert!(!source.open_stage().unwrap().prim("/Other").unwrap().is_valid().unwrap());
        assert_eq!(model.open_stage().unwrap().prim("/Model").unwrap().attribute("size").get::<f64>().unwrap(), Some(2.));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        assert_eq!(source.with_edits([]).unwrap().revision(), source.revision());
        assert!(source.with_edits([
            EditorEdit::Define { path: "/Partial".into(), type_name: "Cube".into() },
            EditorEdit::Attribute { prim: "/Partial".into(), name: "size".into(), type_name: "double".into(), value: Value::String("invalid".into()) },
        ]).is_err());
        assert_eq!(source.bytes, original);
        assert!(!source.open_stage().unwrap().prim("/Partial").unwrap().is_valid().unwrap());
        assert!(source.with_edits([
            EditorEdit::Define { path: "/Broken".into(), type_name: String::new() },
            EditorEdit::References { prim: "/Broken".into(), references: vec![openusd::sdf::Reference {
                asset_path: "missing.usda".into(), prim_path: openusd::sdf::path("/Model").unwrap(), ..Default::default()
            }] },
        ]).is_err());
        assert_eq!(source.bytes, original);
        let binary_name = UsdSource::snapshot("root.usdc", &b"#usda 1.0\n"[..]).unwrap();
        assert!(binary_name.with_edits([EditorEdit::Define { path: "/New".into(), type_name: "Cube".into() }]).is_err());
    }

    #[test]
    fn reference_batches_match_sequential_mounts_and_reject_late_conflicts() {
        let root = UsdSource::snapshot("batch/root.usda", &b"#usda 1.0\n"[..]).unwrap();
        let sphere = UsdSource::snapshot("batch/sphere.usda", &b"#usda 1.0\ndef Sphere \"Model\" {}\n"[..]).unwrap();
        let cube = UsdSource::snapshot("batch/cube.usda", &b"#usda 1.0\ndef Cube \"Model\" {}\n"[..]).unwrap();
        let batch = root.with_references([
            ("/First", &sphere, "/Model"), ("/Second", &cube, "/Model"), ("/Third", &sphere, "/Model"),
        ]).unwrap();
        let sequential = root.with_reference("/First", &sphere, "/Model").unwrap()
            .with_reference("/Second", &cube, "/Model").unwrap()
            .with_reference("/Third", &sphere, "/Model").unwrap();
        assert_eq!(batch.bytes, sequential.bytes);
        assert_eq!(batch.dependencies().collect::<Vec<_>>(), sequential.dependencies().collect::<Vec<_>>());
        let stage = batch.open_stage().unwrap();
        for (path, name) in [("/First", "Sphere"), ("/Second", "Cube"), ("/Third", "Sphere")] {
            assert_eq!(stage.prim(path).unwrap().type_name().unwrap().as_deref(), Some(name));
        }
        let empty = root.with_references(std::iter::empty::<(&str, &UsdSource, &str)>()).unwrap();
        assert_eq!(empty.revision(), root.revision());
        assert!(Arc::ptr_eq(&empty.bytes, &root.bytes));
        assert!(root.with_references([
            ("/First", &sphere, "/Model"), ("/First", &cube, "/Model"),
        ]).is_err());
        let conflict = UsdSource::snapshot(sphere.identifier(), cube.bytes.clone()).unwrap();
        assert!(root.with_references([
            ("/First", &sphere, "/Model"), ("/Second", &conflict, "/Model"),
        ]).is_err());
        assert_eq!(root.bytes.as_ref(), b"#usda 1.0\n");
        assert_eq!(root.dependencies().count(), 0);
    }

    #[test]
    fn instanceable_reference_batches_share_native_prototypes() {
        use openusd::sdf::{LayerOffset, Path};
        let root = UsdSource::snapshot("instanced/root.usda", &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot("instanced/model.usda", &b"#usda 1.0\n(defaultPrim = \"Model\")\ndef Xform \"Model\" { def Cube \"Geometry\" {} }\n"[..]).unwrap();
        let assembly = root.with_instanceable_references([
            ("/First", &model, Path::default(), LayerOffset::IDENTITY),
            ("/Second", &model, Path::default(), LayerOffset::IDENTITY),
        ]).unwrap();
        let stage = assembly.open_stage().unwrap();
        let first = stage.prim("/First").unwrap();
        let second = stage.prim("/Second").unwrap();
        assert!(first.is_instance().unwrap() && second.is_instance().unwrap());
        assert!(first.prototype().unwrap().is_some());
        assert_eq!(first.prototype().unwrap(), second.prototype().unwrap());
        for path in ["/First/Geometry", "/Second/Geometry"] {
            let prim = stage.prim(path).unwrap();
            assert!(prim.is_instance_proxy().unwrap());
            assert_eq!(prim.type_name().unwrap().as_deref(), Some("Cube"));
        }
        assert_eq!(assembly.dependencies().count(), 1);
        assert_eq!(&*root.bytes, b"#usda 1.0\n");
        assert!(!model.open_stage().unwrap().prim("/Model").unwrap().is_instance().unwrap());
        let ordinary = root.with_reference("/First", &model, Path::default()).unwrap().open_stage().unwrap();
        assert!(!ordinary.prim("/First").unwrap().is_instance().unwrap());
        assert!(root.with_instanceable_references([
            ("/First", &model, Path::default(), LayerOffset::IDENTITY),
            ("/First/Geometry", &model, Path::default(), LayerOffset::IDENTITY),
        ]).is_err());
        assert!(root.dependencies().next().is_none());
    }

    #[test]
    fn offset_reference_batches_preserve_arcs_and_retime_samples() {
        use openusd::sdf::{LayerOffset, Path, Value};
        let root = UsdSource::snapshot("retimed/root.usda", &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot("retimed/model.usda", &b"#usda 1.0\n(defaultPrim = \"Model\")\ndef Sphere \"Model\" { double radius.timeSamples = {0: 1, 10: 3} }\n"[..]).unwrap();
        let offset = LayerOffset::new(10.0, 2.0);
        let assembly = root.with_offset_references([
            ("/Original", &model, Path::new("/Model").unwrap(), LayerOffset::IDENTITY),
            ("/Retimed", &model, Path::default(), offset),
        ]).unwrap();
        let stage = assembly.open_stage().unwrap();
        for (path, time, expected) in [("/Original", 10.0, 3.0), ("/Retimed", 10.0, 1.0),
            ("/Retimed", 20.0, 2.0), ("/Retimed", 30.0, 3.0)] {
            assert_eq!(stage.prim(path).unwrap().attribute("radius")
                .get_at::<f64>(Some(openusd::usd::TimeCode::new(time))).unwrap(), Some(expected));
        }
        let Value::ReferenceListOp(arcs) = stage.prim("/Retimed").unwrap()
            .get_metadata("references").unwrap().unwrap() else { panic!() };
        assert!(arcs.explicit_items[0].prim_path.is_empty());
        assert_eq!(arcs.explicit_items[0].layer_offset, offset);
        assert_eq!(assembly.dependencies().count(), 1);
        for invalid in [LayerOffset::new(f64::NAN, 1.0), LayerOffset::new(0.0, f64::INFINITY),
            LayerOffset::new(0.0, 0.0), LayerOffset::new(0.0, -1.0)] {
            assert!(root.with_offset_references([
                ("/Valid", &model, "/Model", offset), ("/Invalid", &model, "/Model", invalid),
            ]).is_err());
        }
        assert_eq!(&*root.bytes, b"#usda 1.0\n");
        assert!(root.dependencies().next().is_none());
    }

    #[test]
    fn reference_assembly_uses_and_preserves_default_prim_arcs() {
        let root = UsdSource::snapshot("defaults/root.usda", &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot("defaults/model.usda", &b"#usda 1.0\n(defaultPrim = \"Model\")\ndef Sphere \"Model\" { double radius = 2 }\n"[..]).unwrap();
        let assembly = root.with_reference("/Instance", &model, openusd::sdf::Path::default()).unwrap();
        let stage = assembly.open_stage().unwrap();
        assert_eq!(stage.prim("/Instance").unwrap().attribute("radius").get::<f64>().unwrap(), Some(2.0));
        let openusd::sdf::Value::ReferenceListOp(references) = stage.prim("/Instance").unwrap().get_metadata("references").unwrap().unwrap() else { panic!() };
        assert_eq!(references.explicit_items.len(), 1);
        assert!(references.explicit_items[0].prim_path.is_empty());
        let changed = UsdSource::snapshot("defaults/model.usda", &b"#usda 1.0\n(defaultPrim = \"Other\")\ndef Cube \"Other\" {}\n"[..]).unwrap();
        let replacement = root.with_reference("/Instance", &changed, openusd::sdf::Path::default()).unwrap();
        assert_eq!(replacement.open_stage().unwrap().prim("/Instance").unwrap().type_name().unwrap().as_deref(), Some("Cube"));
        for contents in [
            "#usda 1.0\ndef Sphere \"Model\" {}\n",
            "#usda 1.0\n(defaultPrim = \"Missing\")\ndef Sphere \"Model\" {}\n",
        ] {
            let invalid = UsdSource::snapshot("defaults/invalid.usda", contents.as_bytes()).unwrap();
            assert!(root.with_reference("/Instance", &invalid, openusd::sdf::Path::default()).is_err());
        }
        assert!(root.with_reference(openusd::sdf::Path::default(), &model, "/Model").is_err());
        assert!(root.dependencies().next().is_none());
    }

    #[test]
    fn reference_assembly_exports_respect_composition_modes() {
        let input = tempfile::tempdir().unwrap();
        let root = UsdSource::snapshot(input.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot(input.path().join("model.usda"),
            &b"#usda 1.0\ndef Sphere \"Model\" { double radius = 1.5 }\n"[..]).unwrap();
        let assembly = root.with_reference("/First", &model, "/Model").unwrap()
            .with_reference("/Second", &model, "/Model").unwrap();
        let editor = crate::editor::EditorSession::new(assembly.open_stage().unwrap());
        let output = tempfile::tempdir().unwrap();
        for (extension, mode, retains_references) in [
            ("usda", crate::editor::SaveMode::Flattened, false),
            ("usdc", crate::editor::SaveMode::Flattened, false),
            ("usdz", crate::editor::SaveMode::RootLayer, true),
        ] {
            let original = output.path().join(format!("original-{extension}"));
            std::fs::create_dir(&original).unwrap();
            let filename = format!("assembly.{extension}");
            editor.save(original.join(&filename).to_str().unwrap(), mode).unwrap();
            let moved = output.path().join(format!("moved-{extension}"));
            std::fs::rename(&original, &moved).unwrap();
            let file = moved.join(&filename);
            let reopened = UsdSource::new(&file, std::fs::read(&file).unwrap()).unwrap().open_stage().unwrap();
            UsdSource::validate_composition(&reopened).unwrap();
            for path in ["/First", "/Second"] {
                let prim = reopened.prim(path).unwrap();
                assert_eq!(prim.type_name().unwrap().as_deref(), Some("Sphere"));
                assert_eq!(prim.attribute("radius").get::<f64>().unwrap(), Some(1.5));
                assert_eq!(matches!(prim.get_metadata("references").unwrap(), Some(openusd::sdf::Value::ReferenceListOp(_))), retains_references);
            }
        }
        for extension in ["usda", "usdc"] {
            let path = output.path().join(format!("unbundled.{extension}"));
            editor.save(path.to_str().unwrap(), crate::editor::SaveMode::RootLayer).unwrap();
            let reopened = UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
            assert!(UsdSource::validate_composition(&reopened).unwrap_err().to_string().contains("model.usda"));
            assert!(matches!(reopened.prim("/First").unwrap().get_metadata("references").unwrap(),
                Some(openusd::sdf::Value::ReferenceListOp(_))));
        }
        assert_eq!(std::fs::read_dir(input.path()).unwrap().count(), 0);
        assert_eq!(root.dependencies().count(), 0);
    }

    #[test]
    fn reference_assembly_rejects_invalid_inputs_without_changing_sources() {
        let root = UsdSource::snapshot("assembly/root.usda", &b"#usda 1.0\ndef Scope \"Existing\" {}\n"[..]).unwrap();
        let model = UsdSource::snapshot("assembly/model.usda", &b"#usda 1.0\ndef Sphere \"Model\" {}\n"[..]).unwrap();
        let before = root.bytes.clone();
        for path in ["/", "relative", "/Prim.attr", "/Prim{choice=a}"] {
            assert!(root.with_reference(path, &model, "/Model").is_err());
            assert!(root.with_reference("/New", &model, path).is_err());
        }
        assert!(root.with_reference("/Existing", &model, "/Model").is_err());
        assert!(root.with_reference("/New", &model, "/Missing").is_err());
        let incomplete = UsdSource::snapshot("assembly/incomplete.usda",
            &b"#usda 1.0\ndef Sphere \"Model\" (prepend references = @missing.usda@</Missing>) {}\n"[..]).unwrap();
        assert!(root.with_reference("/New", &incomplete, "/Model").is_err());
        let conflict = UsdSource::snapshot(root.identifier(), &b"#usda 1.0\ndef Scope \"Other\" {}\n"[..]).unwrap();
        assert!(root.with_reference("/New", &conflict, "/Other").is_err());
        let package = UsdSource::snapshot("assembly/root.usdz", &b"not a package"[..]).unwrap();
        assert!(package.with_reference("/New", &model, "/Model").is_err());
        assert_eq!(root.bytes, before);
        assert_eq!(root.dependencies().count(), 0);
    }

    #[test]
    fn asset_reads_reject_unresolved_working_directory_paths() {
        let directory = tempfile::tempdir().unwrap();
        let source = UsdSource::new(directory.path().join("scene.usda"), &b"root bytes"[..]).unwrap();
        let cwd_file = std::env::current_dir().unwrap().join("Cargo.toml");
        assert!(cwd_file.is_file());
        let error = source.read_asset("Cargo.toml").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("unresolved asset identifier"));
        assert_eq!(source.read_asset(cwd_file.to_str().unwrap()).unwrap(), std::fs::read(cwd_file).unwrap());
        assert_eq!(source.read_asset(source.identifier()).unwrap(), b"root bytes");
    }

    #[test]
    fn captured_asset_handles_share_bytes_with_independent_cursors() {
        let root: Arc<[u8]> = Arc::from(b"root contents".as_slice());
        let dependency: Arc<[u8]> = Arc::from(b"dependency contents".as_slice());
        let source = UsdSource::snapshot("shared/root.usda", root.clone()).unwrap()
            .with_dependency(&UsdSource::snapshot("shared/data.bin", dependency.clone()).unwrap()).unwrap();
        let resolver = SourceResolver { source, fallback: DefaultResolver::new(), requests: Arc::default(), disk_baselines: None };
        for (identifier, bytes) in [(resolver.source.identifier(), &root), (resolver.source.dependencies().next().unwrap(), &dependency)] {
            let before = Arc::strong_count(bytes);
            let mut first = resolver.open_asset(&ResolvedPath::new(identifier)).unwrap();
            let mut second = resolver.open_asset(&ResolvedPath::new(identifier)).unwrap();
            assert_eq!(Arc::strong_count(bytes), before + 2);
            assert_eq!(first.size().unwrap(), bytes.len() as u64);
            let mut prefix = [0; 4];
            first.read_exact(&mut prefix).unwrap();
            assert_eq!(&prefix, &bytes[..4]);
            assert_eq!(second.read_all().unwrap(), bytes.as_ref());
            assert_eq!(first.read_all().unwrap(), &bytes[4..]);
            first.seek(SeekFrom::End(-3)).unwrap();
            assert_eq!(first.read_all().unwrap(), &bytes[bytes.len() - 3..]);
            first.rewind().unwrap();
            assert_eq!(first.read_all().unwrap(), bytes.as_ref());
            assert!(first.seek(SeekFrom::Current(-1000)).is_err());
            drop((first, second));
            assert_eq!(Arc::strong_count(bytes), before);
        }
        let identifier = resolver.source.identifier().to_string();
        let mut asset = resolver.open_asset(&ResolvedPath::new(identifier)).unwrap();
        drop(resolver);
        assert_eq!(std::thread::spawn(move || asset.read_all().unwrap()).join().unwrap(), root.as_ref());
    }

    #[test]
    fn composition_validation_follows_active_variant_selection() {
        let directory = tempfile::tempdir().unwrap();
        let source = UsdSource::snapshot(directory.path().join("variants.usda"), &br#"#usda 1.0
def Scope "Model" (
    prepend variantSets = ["choice"]
    variants = { string choice = "good" }
) {
    variantSet "choice" = {
        "good" {
            def Scope "Content" {}
        }
        "bad" {
            def Scope "Content" (prepend references = @missing.usda@</Model>) {}
        }
    }
}
"#[..]).unwrap();
        let (result, missing) = source.probe();
        result.unwrap();
        assert!(missing.is_empty());
        let stage = source.open_stage().unwrap();
        crate::authoring::set_variant(&stage, "/Model", "choice", "bad").unwrap();
        let error = UsdSource::validate_composition(&stage).unwrap_err();
        assert!(error.to_string().contains("missing.usda"));
        crate::authoring::set_variant(&stage, "/Model", "choice", "good").unwrap();
        UsdSource::validate_composition(&stage).unwrap();
    }

    #[test]
    fn snapshot_anchors_parent_relative_paths_before_normalizing() {
        let cwd = std::env::current_dir().unwrap();
        let expected = normalize(&cwd.join("../virtual-source.usda"));
        let source = UsdSource::snapshot("../virtual-source.usda", &b"root"[..]).unwrap();
        assert_eq!(source.identifier(), expected.to_str().unwrap());
    }

    #[test]
    fn merged_sources_preserve_transitive_layer_and_asset_anchors() {
        let directory = tempfile::tempdir().unwrap();
        let source = |name: &str, bytes: &[u8]| UsdSource::snapshot(&directory.path().join(name), bytes.to_vec()).unwrap();
        let weak = source("parts/weak.usda", b"#usda 1.0\ndef Scope \"Model\" {\n double score = 17\n}\n");
        let paint = source("paint.bin", b"captured paint");
        let model = source("parts/model.usda", b"#usda 1.0\n( subLayers = [@weak.usda@] )\ndef Scope \"Model\" {\n asset paint = @../paint.bin@\n}\n")
            .with_dependency(&weak).unwrap().with_dependency(&paint).unwrap();
        let root = source("root.usda", b"#usda 1.0\ndef Scope \"First\" (prepend references = @parts/model.usda@</Model>) {}\ndef Scope \"Second\" (prepend references = @parts/model.usda@</Model>) {}\n");
        let assembled = root.with_dependency(&model).unwrap();
        assert_eq!(root.dependencies().count(), 0);
        assert_eq!(assembled.dependencies().count(), 3);
        assert_ne!(assembled.revision(), root.revision());
        assert_eq!(assembled.with_dependency(&model).unwrap().revision(), assembled.revision());
        let stage = assembled.open_stage().unwrap();
        for path in ["/First", "/Second"] {
            let prim = stage.prim(path).unwrap();
            assert_eq!(prim.attribute("score").get::<f64>().unwrap(), Some(17.0));
            let value = prim.attribute("paint").get::<openusd::sdf::Value>().unwrap().unwrap();
            let openusd::sdf::Value::AssetPath(asset) = value else { panic!("expected asset") };
            assert_eq!(assembled.read_asset(asset.resolved_path().unwrap()).unwrap(), b"captured paint");
        }
        let output = tempfile::tempdir().unwrap();
        let package = output.path().join("assembled.usdz");
        crate::authoring::save_stage_as(&stage, package.to_str().unwrap()).unwrap();
        let bytes = std::fs::read(&package).unwrap();
        std::fs::remove_file(&package).unwrap();
        let reopened = UsdSource::snapshot(&package, bytes).unwrap().open_stage().unwrap();
        assert_eq!(reopened.prim("/First").unwrap().attribute("score").get::<f64>().unwrap(), Some(17.0));
        assert_eq!(reopened.prim("/Second").unwrap().attribute("score").get::<f64>().unwrap(), Some(17.0));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn merging_conflicting_identifiers_never_changes_either_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = |name: &str, bytes: &[u8]| UsdSource::snapshot(&directory.path().join(name), bytes.to_vec()).unwrap();
        let root = source("root.usda", b"root");
        let original = source("asset.bin", b"original");
        let root = root.with_dependency(&original).unwrap();
        let conflict = source("asset.bin", b"replacement");
        let incoming = source("extra.usda", b"extra").with_dependency(&conflict).unwrap();
        assert_eq!(root.with_dependency(&incoming).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(root.read_asset(original.identifier()).unwrap(), b"original");
        assert_eq!(root.dependencies().count(), 1);
        assert_eq!(incoming.dependencies().count(), 1);
        assert!(root.with_dependency(&source("root.usda", b"changed root")).is_err());
        assert_eq!(root.with_dependency(&root).unwrap().revision(), root.revision());
        let alias = source("parts/../asset.bin", b"replacement");
        assert!(root.with_dependency(&alias).is_err());
    }

    #[test]
    fn merged_sources_retain_the_receivers_filesystem_policy() {
        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().join("root.usda");
        let asset_path = directory.path().join("disk.bin");
        std::fs::write(&asset_path, b"on disk").unwrap();
        let closed = UsdSource::snapshot(&root_path, b"root".to_vec()).unwrap();
        let open = UsdSource::new(&root_path, &b"root"[..]).unwrap();
        let dependency = UsdSource::new(directory.path().join("dependency.bin"), &b"captured"[..]).unwrap();
        assert!(closed.with_dependency(&dependency).unwrap().read_asset(asset_path.to_str().unwrap()).is_err());
        assert_eq!(open.with_dependency(&dependency).unwrap().read_asset(asset_path.to_str().unwrap()).unwrap(), b"on disk");
    }

    #[test]
    fn bytes_are_the_root_layer_and_stages_are_independent() {
        let source = UsdSource::new(
            "virtual-root.usda",
            &b"#usda 1.0\ndef Xform \"Model\" {}\n"[..],
        )
        .unwrap();
        let first = source.open_stage().unwrap();
        let second = source.open_stage().unwrap();
        first.define_prim("/OnlyFirst").unwrap();
        assert!(first.prim("/OnlyFirst").unwrap().is_valid().unwrap());
        assert!(!second.prim("/OnlyFirst").unwrap().is_valid().unwrap());
        let text = second.root_layer().export_to_string().unwrap();
        assert!(text.contains("Model"));
        assert!(!text.contains("subLayers"));
        assert_eq!(second.root_layer().identifier(), source.identifier());
    }

    #[test]
    fn malformed_bytes_fail_without_materialization() {
        let source = UsdSource::new("invalid-root.usda", &b"not USD"[..]).unwrap();
        assert!(source.open_stage().is_err());
    }

    #[test]
    fn source_anchor_resolves_siblings_without_writing_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("virtual-root.usda");
        std::fs::write(
            directory.path().join("part.usda"),
            b"#usda 1.0\ndef Xform \"Part\" { def Cube \"Box\" {} }\n",
        )
        .unwrap();
        let source = UsdSource::new(
            &root,
            &b"#usda 1.0\ndef Xform \"Model\" (references = @part.usda@</Part>) {}\n"[..],
        )
        .unwrap();
        let stage = source.open_stage().unwrap();
        assert!(stage.prim("/Model/Box").unwrap().is_valid().unwrap());
        assert!(!root.exists());
    }

    #[test]
    fn packaged_relative_references_resolve_from_memory() {
        let mut archive = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
        archive
            .add_layer(
                "scenes/root.usda",
                b"#usda 1.0\ndef Xform \"Model\" (references = @part.usda@</Part>) {}\n",
            )
            .unwrap();
        archive
            .add_layer(
                "scenes/part.usda",
                b"#usda 1.0\ndef Xform \"Part\" { def Cube \"Box\" {} }\n",
            )
            .unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let source = UsdSource::new("virtual-package.usdz", bytes).unwrap();
        let stage = source.open_stage().unwrap();
        assert!(stage.prim("/Model/Box").unwrap().is_valid().unwrap());
    }
}
