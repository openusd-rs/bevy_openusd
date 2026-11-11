//! Bounded counters for animation operations inside projection scopes.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use bevy::prelude::Resource;
use openusd::usd::Stage;

type Key = (String, String, &'static str);

struct Counts {
    records: BTreeMap<Key, u64>,
    key_bytes: usize,
    scopes: u64,
    calls: u64,
    dropped: u64,
    saturated: bool,
    max_entries: usize,
    max_key_bytes: usize,
}

impl Default for Counts {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            key_bytes: 0,
            scopes: 0,
            calls: 0,
            dropped: 0,
            saturated: false,
            max_entries: 4096,
            max_key_bytes: 1024 * 1024,
        }
    }
}

fn increment(value: &mut u64) -> bool {
    match value.checked_add(1) {
        Some(next) => {
            *value = next;
            false
        }
        None => true,
    }
}

impl Counts {
    fn drop_call(&mut self) {
        self.saturated |= increment(&mut self.dropped);
    }

    fn record(&mut self, root: &str, path: &str, operation: &'static str) {
        self.saturated |= increment(&mut self.calls);
        let Some(bytes) = root
            .len()
            .checked_add(path.len())
            .and_then(|v| v.checked_add(operation.len()))
            .filter(|bytes| *bytes <= self.max_key_bytes)
        else {
            self.drop_call();
            return;
        };
        let key = (root.to_owned(), path.to_owned(), operation);
        if let Some(calls) = self.records.get_mut(&key) {
            self.saturated |= increment(calls);
            return;
        }
        let total = self.key_bytes.checked_add(bytes);
        if self.records.len() >= self.max_entries || total.is_none_or(|v| v > self.max_key_bytes) {
            self.drop_call();
            return;
        }
        self.key_bytes = total.unwrap();
        self.records.insert(key, 1);
    }
}

/// Optional animation-operation counters, capped at 4,096 keys and 1 MiB of key text.
#[derive(Resource, Default)]
pub struct AnimationAccessProfile(Arc<Mutex<Counts>>);

/// One root-identifier/path/operation aggregate; equal root identifiers are combined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationAccess {
    pub root: String,
    pub path: String,
    pub operation: &'static str,
    pub calls: u64,
}

/// Owned diagnostic counters; no stage or decoded sample values are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationAccessSnapshot {
    pub records: Vec<AnimationAccess>,
    pub key_bytes: usize,
    pub scopes: u64,
    pub calls: u64,
    pub dropped: u64,
    pub saturated: bool,
}

impl AnimationAccessProfile {
    pub fn snapshot(&self) -> AnimationAccessSnapshot {
        let counts = self.0.lock().expect("animation access profile");
        AnimationAccessSnapshot {
            records: counts
                .records
                .iter()
                .map(|((root, path, operation), calls)| AnimationAccess {
                    root: root.clone(),
                    path: path.clone(),
                    operation,
                    calls: *calls,
                })
                .collect(),
            key_bytes: counts.key_bytes,
            scopes: counts.scopes,
            calls: counts.calls,
            dropped: counts.dropped,
            saturated: counts.saturated,
        }
    }
}

struct Active {
    stage: Stage,
    root: String,
    counts: Arc<Mutex<Counts>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

pub(crate) struct Scope(Option<Active>);

impl Scope {
    pub(crate) fn begin(stage: &Stage, profile: Option<&AnimationAccessProfile>) -> Self {
        let active = profile.map(|profile| {
            let mut counts = profile.0.lock().expect("animation access profile");
            counts.saturated |= increment(&mut counts.scopes);
            Active {
                stage: stage.clone(),
                root: stage.root_layer().identifier().to_owned(),
                counts: profile.0.clone(),
            }
        });
        Self(ACTIVE.with(|current| current.replace(active)))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|current| *current.borrow_mut() = self.0.take());
    }
}

pub(super) fn record(stage: &Stage, path: &openusd::sdf::Path, operation: &'static str) {
    ACTIVE.with(|current| {
        if let Some(active) = current.borrow().as_ref().filter(|a| a.stage == *stage) {
            active
                .counts
                .lock()
                .expect("animation access profile")
                .record(&active.root, path.as_str(), operation);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_keep_existing_counts_and_report_lost_keys() {
        let mut counts = Counts {
            max_entries: 1,
            max_key_bytes: 8,
            ..Default::default()
        };
        counts.record("r", "path", "op");
        counts.record("r", "path", "op");
        counts.record("r", "else", "op");
        counts.record("r", "oversized", "op");
        assert_eq!(
            (
                counts.records.len(),
                counts.key_bytes,
                counts.calls,
                counts.dropped
            ),
            (1, 7, 4, 2)
        );
        assert_eq!(
            counts.records.values().copied().sum::<u64>() + counts.dropped,
            counts.calls
        );
        counts.calls = u64::MAX;
        counts.record("r", "path", "op");
        assert!(counts.saturated);
    }

    #[test]
    fn scopes_isolate_stages_mask_restore_and_release() {
        let a = Stage::builder().in_memory("profile-a.usda").unwrap();
        let b = Stage::builder().in_memory("profile-b.usda").unwrap();
        let path = openusd::sdf::path("/Anim").unwrap();
        let profile = AnimationAccessProfile::default();
        let weak = Arc::downgrade(&profile.0);
        record(&a, &path, "pose");
        {
            let _outer = Scope::begin(&a, Some(&profile));
            record(&a, &path, "pose");
            record(&b, &path, "pose");
            {
                let _disabled = Scope::begin(&a, None);
                record(&a, &path, "pose");
            }
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _inner = Scope::begin(&b, Some(&profile));
                record(&a, &path, "pose");
                record(&b, &path, "weights");
                panic!("scope unwind");
            }));
            assert!(unwind.is_err());
            record(&a, &path, "pose");
        }
        record(&a, &path, "pose");
        let snapshot = profile.snapshot();
        assert_eq!(
            (snapshot.scopes, snapshot.calls, snapshot.dropped),
            (2, 3, 0)
        );
        assert_eq!(snapshot.records.len(), 2);
        assert!(!snapshot.saturated);
        assert_eq!(
            snapshot
                .records
                .iter()
                .find(|r| r.operation == "pose")
                .unwrap()
                .calls,
            2
        );
        drop(profile);
        assert!(weak.upgrade().is_none());
    }
}
