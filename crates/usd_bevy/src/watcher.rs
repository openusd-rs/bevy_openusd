//! Filesystem asset sources with dependency invalidation on file removal and rename.

use std::{collections::BTreeSet, path::{Path, PathBuf}, sync::{Arc, Mutex}, thread::JoinHandle, time::Duration};
use bevy::asset::io::{AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceEvent, AssetWatcher, PathStream, Reader};
use bevy::asset::io::file::{FileAssetReader, FileWatcher};

#[derive(Default)]
struct RequestedPaths(Mutex<BTreeSet<PathBuf>>);

impl RequestedPaths {
    fn record(&self, path: &Path) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).insert(path.to_owned());
    }

    fn invalidations(&self, event: &AssetSourceEvent) -> BTreeSet<PathBuf> {
        let mut paths: BTreeSet<_> = invalidated_paths(event).into_iter().flatten().map(Path::to_owned).collect();
        let folders = match event {
            AssetSourceEvent::AddedFolder(path) | AssetSourceEvent::RemovedFolder(path)
            | AssetSourceEvent::RemovedUnknown { path, is_meta: false } => [Some(path), None],
            AssetSourceEvent::RenamedFolder { old, new } => [Some(old), Some(new)],
            _ => [None, None],
        };
        if folders.iter().any(Option::is_some) {
            let requested = self.0.lock().unwrap_or_else(|error| error.into_inner());
            paths.extend(requested.iter().filter(|path|
                folders.iter().flatten().any(|folder| path.starts_with(folder))).cloned());
        }
        paths
    }
}

struct TrackingReader {
    reader: FileAssetReader,
    requested: Arc<RequestedPaths>,
}

impl AssetReader for TrackingReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.requested.record(path);
        self.reader.read(path).await
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.reader.read_meta(path).await
    }

    async fn read_directory<'a>(&'a self, path: &'a Path) -> Result<Box<PathStream>, AssetReaderError> {
        self.reader.read_directory(path).await
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        self.reader.is_directory(path).await
    }
}

struct RemovalWatcher {
    receiver: async_channel::Receiver<AssetSourceEvent>,
    worker: Option<JoinHandle<()>>,
}

fn forward_event(output: &async_channel::Sender<AssetSourceEvent>, requested: &RequestedPaths, event: AssetSourceEvent) -> bool {
    for path in requested.invalidations(&event) {
        if output.try_send(AssetSourceEvent::ModifiedAsset(path)).is_err() { return false; }
    }
    output.try_send(event).is_ok()
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    metadata.is_dir().then(|| (metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
fn rearming_events(root: PathBuf, input: async_channel::Sender<AssetSourceEvent>, events: async_channel::Receiver<AssetSourceEvent>,
    output: async_channel::Sender<AssetSourceEvent>, requested: Arc<RequestedPaths>, mut watcher: Option<FileWatcher>, mut identity: Option<(u64, u64)>) {
    let mut next_check = std::time::Instant::now();
    let invalidate_all = || {
        let paths = requested.0.lock().unwrap_or_else(|error| error.into_inner()).clone();
        paths.into_iter().all(|path| output.try_send(AssetSourceEvent::ModifiedAsset(path)).is_ok())
    };
    while !events.is_closed() && !output.is_closed() {
        let now = std::time::Instant::now();
        if now >= next_check {
            next_check = now + Duration::from_secs(1);
            if watcher.is_some() && directory_identity(&root) != identity {
                watcher.take();
                identity = None;
                while events.try_recv().is_ok() {}
                if !invalidate_all() { break; }
            }
            if watcher.is_none() && let Some(before) = directory_identity(&root) {
                match FileWatcher::new(root.clone(), input.clone(), Duration::from_millis(300)) {
                    Ok(new) if directory_identity(&root) == Some(before) => {
                        watcher = Some(new);
                        identity = Some(before);
                        if !invalidate_all() { break; }
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("USD file watcher could not re-arm {}: {error}", root.display()),
                }
            }
        }
        match events.try_recv() {
            Ok(event) => if !forward_event(&output, &requested, event) { break; },
            Err(async_channel::TryRecvError::Closed) => break,
            Err(async_channel::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

impl AssetWatcher for RemovalWatcher {}

impl Drop for RemovalWatcher {
    fn drop(&mut self) {
        self.receiver.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn invalidated_paths(event: &AssetSourceEvent) -> [Option<&Path>; 2] {
    match event {
        AssetSourceEvent::RemovedAsset(path)
        | AssetSourceEvent::RemovedUnknown { path, is_meta: false } => [Some(path), None],
        AssetSourceEvent::RenamedAsset { old, new } => [Some(old), Some(new)],
        _ => [None, None],
    }
}

/// Read-only native file source that reloads dependents on file removal or rename.
/// Register before `AssetPlugin`; watching follows its runtime watch setting.
/// Folder events invalidate requested descendants, retained for the source lifetime.
/// Unix roots are checked once per second and re-armed after replacement.
/// Failed initial watcher setup retries on Unix when the root is a directory.
/// Processed sources and non-Unix root replacement are unsupported.
pub fn file_source(root: impl AsRef<Path>) -> AssetSourceBuilder {
    let root = FileAssetReader::new(root).root_path().clone();
    let reader_root = root.clone();
    let requested = Arc::new(RequestedPaths::default());
    let reader_paths = requested.clone();
    AssetSourceBuilder::new(move || Box::new(TrackingReader {
        reader: FileAssetReader::new(&reader_root), requested: reader_paths.clone(),
    }))
        .with_watcher(move |output| {
            let (input, receiver) = async_channel::unbounded();
            #[cfg(unix)]
            let identity = directory_identity(&root);
            let watcher = match FileWatcher::new(root.clone(), input.clone(), Duration::from_millis(300)) {
                Ok(watcher) => Some(watcher),
                Err(error) => {
                    log::error!("USD file watcher could not watch {}: {error}", root.display());
                    #[cfg(not(unix))]
                    return None;
                    #[cfg(unix)]
                    { None }
                }
            };
            let events = receiver.clone();
            let requested = requested.clone();
            #[cfg(unix)]
            let root = root.clone();
            let worker = std::thread::Builder::new().name("usd-asset-events".into())
                .spawn(move || {
                    #[cfg(unix)]
                    rearming_events(root, input, events, output, requested, watcher, identity);
                    #[cfg(not(unix))]
                    {
                        let _watcher = watcher;
                        while let Ok(event) = events.recv_blocking() {
                            if !forward_event(&output, &requested, event) { break; }
                        }
                    }
                });
            match worker {
                Ok(worker) => Some(Box::new(RemovalWatcher {
                    receiver, worker: Some(worker),
                })),
                Err(error) => {
                    log::error!("USD file watcher event worker failed: {error}");
                    None
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn directory_identity_tracks_replacement_and_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        let backup = directory.path().join("backup");
        let link = directory.path().join("link");
        assert_eq!(directory_identity(&root), None);
        std::fs::write(&root, b"not a directory").unwrap();
        assert_eq!(directory_identity(&root), None);
        std::fs::remove_file(&root).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let original = directory_identity(&root).unwrap();
        assert_eq!(directory_identity(&link), Some(original));
        std::fs::rename(&root, &backup).unwrap();
        assert_eq!(directory_identity(&link), None);
        std::fs::create_dir(&root).unwrap();
        assert_ne!(directory_identity(&root), Some(original));
        assert_eq!(directory_identity(&link), directory_identity(&root));
        assert_eq!(directory_identity(&backup), Some(original));
    }

    #[test]
    fn tracking_reader_records_missing_files_before_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let requested = Arc::new(RequestedPaths::default());
        let reader = TrackingReader {
            reader: FileAssetReader::new(directory.path()), requested: requested.clone(),
        };
        let path = Path::new("missing/model.usda");
        assert!(matches!(bevy::tasks::block_on(reader.read(path)), Err(AssetReaderError::NotFound(_))));
        assert_eq!(requested.invalidations(&AssetSourceEvent::AddedFolder("missing".into())),
            [path.to_owned()].into());
        std::fs::create_dir(directory.path().join("missing")).unwrap();
        std::fs::write(directory.path().join(path), b"#usda 1.0\n").unwrap();
        assert!(bevy::tasks::block_on(reader.read(path)).is_ok());
        assert_eq!(requested.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn folder_events_invalidate_only_requested_descendants() {
        let requested = RequestedPaths::default();
        for path in ["models/a.usda", "models/nested/b.usda", "models-old/c.usda", "next/d.usda"] {
            requested.record(Path::new(path));
        }
        let descendants: BTreeSet<PathBuf> = ["models/a.usda", "models/nested/b.usda"].map(Into::into).into();
        for event in [AssetSourceEvent::RemovedFolder("models".into()),
            AssetSourceEvent::AddedFolder("models".into())] {
            assert_eq!(requested.invalidations(&event), descendants);
        }
        let mut renamed = descendants.clone();
        renamed.insert("next/d.usda".into());
        assert_eq!(requested.invalidations(&AssetSourceEvent::RenamedFolder {
            old: "models".into(), new: "next".into(),
        }), renamed);
        let mut unknown = descendants;
        unknown.insert("models".into());
        assert_eq!(requested.invalidations(&AssetSourceEvent::RemovedUnknown {
            path: "models".into(), is_meta: false,
        }), unknown);
    }

    #[test]
    fn removal_and_rename_invalidate_dependency_paths() {
        let old = Path::new("models/old.usda");
        let new = Path::new("models/new.usda");
        assert_eq!(invalidated_paths(&AssetSourceEvent::RemovedAsset(old.into())), [Some(old), None]);
        assert_eq!(invalidated_paths(&AssetSourceEvent::RemovedUnknown {
            path: old.into(), is_meta: false,
        }), [Some(old), None]);
        assert_eq!(invalidated_paths(&AssetSourceEvent::RenamedAsset {
            old: old.into(), new: new.into(),
        }), [Some(old), Some(new)]);
        for event in [AssetSourceEvent::RemovedUnknown { path: old.into(), is_meta: true },
            AssetSourceEvent::RemovedFolder(old.into()), AssetSourceEvent::ModifiedAsset(old.into())] {
            assert_eq!(invalidated_paths(&event), [None, None]);
        }
    }
}
