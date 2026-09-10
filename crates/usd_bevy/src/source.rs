//! Byte-backed root layers with filesystem-relative dependency resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use openusd::ar::{Asset, DefaultResolver, ResolvedPath, Resolver};
use openusd::usd::Stage;

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

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

    pub fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.identity
    }

    pub(crate) fn read_asset(&self, identifier: &str) -> io::Result<Vec<u8>> {
        SourceResolver {
            source: self.clone(),
            fallback: DefaultResolver::new(),
            requests: Arc::default(),
        }
        .open_asset(&ResolvedPath::new(identifier))?
        .read_all()
    }

    pub(crate) fn texture_requests(&self) -> Result<BTreeSet<(String, bool)>, String> {
        let stage = self.open_stage().map_err(|error| error.to_string())?;
        Self::stage_texture_requests(&stage)
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
            if let Some(material) = crate::read::shade::read_preview_material(stage, &path)
                .map_err(|error| error.to_string())?
            {
                for (path, srgb) in [
                    (&material.diffuse_texture, material.texture_srgb("diffuse")),
                    (&material.emissive_texture, material.texture_srgb("emissive")),
                    (&material.normal_texture, material.texture_srgb("normal")),
                    (&material.metallic_texture, material.texture_srgb("metallic")),
                    (&material.roughness_texture, material.texture_srgb("roughness")),
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

    pub(crate) fn insert_dependency(&mut self, identifier: String, bytes: Vec<u8>) {
        Arc::make_mut(&mut self.files).insert(identifier, bytes.into());
        self.identity = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn probe(&self) -> (Result<(), String>, BTreeSet<String>) {
        let requests = Arc::new(Mutex::new(BTreeSet::new()));
        let result = (|| -> anyhow::Result<()> {
            let stage = self.open_tracked(requests.clone())?;
            Self::validate_composition(&stage)
        })()
        .map_err(|error| error.to_string());
        let missing = std::mem::take(&mut *requests.lock().expect("dependency requests"));
        (result, missing)
    }

    /// Open an independent stage from this snapshot.
    pub fn open_stage(&self) -> openusd::Result<Stage> {
        self.open_tracked(Arc::default())
    }

    pub(crate) fn validate_composition(stage: &Stage) -> anyhow::Result<()> {
        let mut paths = Vec::new();
        stage.traverse(openusd::usd::PrimPredicate::DEFAULT_PROXIES, |path| paths.push(path.clone()))?;
        for path in paths {
            for attribute in stage.prim(path)?.attributes()? {
                attribute.get::<openusd::sdf::Value>()?;
                if attribute.type_name()?.is_some_and(|name| matches!(name.as_str(), "asset" | "asset[]")) {
                    for time in attribute.time_sample_times()? {
                        attribute.get_at::<openusd::sdf::Value>(Some(openusd::usd::TimeCode::new(time)))?;
                    }
                }
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
            })
            .open(&self.identifier)
    }
}

struct SourceResolver {
    source: UsdSource,
    fallback: DefaultResolver,
    requests: Arc<Mutex<BTreeSet<String>>>,
}

impl SourceResolver {
    fn bytes(&self, identifier: &str) -> Option<&[u8]> {
        if identifier == self.source.identifier {
            Some(&self.source.bytes)
        } else {
            self.source
                .files
                .get(identifier)
                .map(|bytes| bytes.as_ref())
        }
    }

    fn contains_packaged_path(&self, path: &str) -> bool {
        let Some((package, inner)) = openusd::ar::split_package_relative_path_outer(path) else {
            return false;
        };
        let Some(bytes) = self.bytes(&package) else {
            return false;
        };
        zip::ZipArchive::new(Cursor::new(bytes))
            .map(|mut archive| archive.by_name(&inner).is_ok())
            .unwrap_or(false)
    }

    fn packaged_bytes(&self, path: &str) -> io::Result<Option<Vec<u8>>> {
        let Some((package, inner)) = openusd::ar::split_package_relative_path_outer(path) else {
            return Ok(None);
        };
        let Some(bytes) = self.bytes(&package) else {
            return Ok(None);
        };
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(io::Error::other)?;
        let entry = archive.by_name(&inner).map_err(io::Error::other)?;
        const LIMIT: u64 = 256 * 1024 * 1024;
        if entry.size() > LIMIT {
            return Err(io::Error::other("USD package entry exceeds 256 MiB"));
        }
        let mut bytes = Vec::new();
        entry.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("USD package entry exceeds 256 MiB"));
        }
        Ok(Some(bytes))
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
        if let Some(bytes) = self.bytes(&path.to_string_lossy()) {
            Ok(Box::new(Cursor::new(bytes.to_vec())))
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
    use super::*;

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
