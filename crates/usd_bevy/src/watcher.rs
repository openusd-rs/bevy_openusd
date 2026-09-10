//! Filesystem asset sources with dependency invalidation on file removal and rename.

use std::{path::Path, thread::JoinHandle, time::Duration};
use bevy::asset::io::{AssetSourceBuilder, AssetSourceEvent, AssetWatcher};
use bevy::asset::io::file::{FileAssetReader, FileWatcher};

struct RemovalWatcher {
    receiver: async_channel::Receiver<AssetSourceEvent>,
    worker: Option<JoinHandle<()>>,
    _watcher: FileWatcher,
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
/// Folder deletion and processed asset sources are not supported by this adapter.
pub fn file_source(root: impl AsRef<Path>) -> AssetSourceBuilder {
    let root = FileAssetReader::new(root).root_path().clone();
    let reader_root = root.clone();
    AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(&reader_root)))
        .with_watcher(move |output| {
            let (input, receiver) = async_channel::unbounded();
            let watcher = match FileWatcher::new(root.clone(), input, Duration::from_millis(300)) {
                Ok(watcher) => watcher,
                Err(error) => {
                    log::error!("USD file watcher could not watch {}: {error}", root.display());
                    return None;
                }
            };
            let events = receiver.clone();
            let worker = std::thread::Builder::new().name("usd-asset-events".into())
                .spawn(move || {
                    'events: while let Ok(event) = events.recv_blocking() {
                        for path in invalidated_paths(&event).into_iter().flatten() {
                            if output.try_send(AssetSourceEvent::ModifiedAsset(path.to_owned())).is_err() {
                                break 'events;
                            }
                        }
                        if output.try_send(event).is_err() { break; }
                    }
                });
            match worker {
                Ok(worker) => Some(Box::new(RemovalWatcher {
                    receiver, worker: Some(worker), _watcher: watcher,
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
