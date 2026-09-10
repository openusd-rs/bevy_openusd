//! Filesystem asset sources with dependency invalidation on file removal.

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

/// Read-only native file source that reloads dependents when a file is removed.
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
                    while let Ok(event) = events.recv_blocking() {
                        if let AssetSourceEvent::RemovedAsset(path) = &event {
                            if output.try_send(AssetSourceEvent::ModifiedAsset(path.clone())).is_err() {
                                break;
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
