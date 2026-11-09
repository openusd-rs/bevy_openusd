# Local Bevy asset patch

Source: crates.io `bevy_asset` 0.20.1, MIT OR Apache-2.0. The root Cargo patch
selects this copy for the whole dependency graph. The published licenses and VCS
metadata are retained.

Compared with the published 0.20.1 crate on 2026-11-09, only
`src/server/mod.rs` differs, by the source-load serialization below. The rebase
originated in bevy_openusd PR #4 at
`5cbdee3f4c49912b153d03b6f8f07e51e1605512`.

## Serialize loads of one source

`src/server/mod.rs` adds an async mutex per source path, including the asset
source ID but excluding the label. `load_internal` holds it from before the
reader is acquired until the completion event has been sent. Different source
paths still load concurrently. Mutex entries are weak and expired entries are
pruned on subsequent load starts; they do not retain assets or idle locks.

This prevents an older snapshot's success or failure from being published after
a newer snapshot from the same source. Queued reloads acquire fresh readers
instead of retaining bytes obtained before their turn. Loader errors are not
suppressed, and the normal untyped reload fallback is preserved.

The tradeoff is serialization within one source: a slow or stuck loader delays
later loads of that source. This patch does not introduce cancellation, timeouts,
coalescing, or a latest-request-wins policy. It does not serialize direct loader
dependency reads independently of their parent load.

Regression coverage lives in `crates/usd_bevy/src/asset.rs`: gated older successful
and failed loads, concurrent unrelated sources, and repeated nested-package
failure/repair while two live instances retain identity and texture ownership.

Remove this patch only after an upstream replacement passes these regressions
and the full workspace gate. Do not edit the shared Cargo registry copy.
