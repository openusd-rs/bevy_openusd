# Standalone consumer integration

## Fixed documentation gap

The root README previously showed only the `bevy_asset` override. The current
library also requires the `bevy_pbr` morph-publication repair. An application
depending on `usd_bevy` does not inherit either override from this repository:
[Cargo reads patches at the consuming workspace root](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section).

The README now lists both local path overrides beside the local library
dependency, pins the demonstrated Bevy version to 0.20.1 and explains that all
three must use the same checkout/revision. It does not advertise these unpushed
changes as available from a remote revision or a published release. The
Mara override is viewer-specific and is not needed by the standalone consumer.
This corrects the integration instructions; it does not make patches transitive.

## Independent workspace and actual dependency resolution

`benchmarks/downstream-consumer` declares its own workspace, package and
lockfile. Its only direct dependencies are Bevy and the public `usd_bevy`
library. Bevy's explicit runtime features are `std`, `async_executor` and
`multi_threaded`; remaining features are supplied by the library, not the
viewer. The fixture does not change the root manifests or production source.

Evidence: `/tmp/bevy-optimization-downstream.2xkWO2/`.

Actual `cargo metadata` for the positive consumer contains 310 native-target
packages, exactly one Bevy/Bevy-internal/asset/PBR version at 0.20.1, both
local override paths and official OpenUSD revision
`900ea31591348e3fbf416d4661d703aa8700771b`. The consumer is the sole workspace
member and resolution root. Neither `usdview` nor Mara is present. The default
graph enables no optional `usd_bevy` features; morph and multilayer material
support are present through the library's normal dependencies.

Two disposable consumer manifests remove one override each. Actual offline
Cargo resolution selects the registry copy of the missing crate, despite the
library still pointing at this repository. The verifier rejects each with the
matching required-patch diagnostic. These are real resolution negatives, not
edited metadata pretending to be Cargo output. They are not rendered morph
tests or behavioral runs against the unpatched crates.

The independent consumer lock was seeded from the root lock and resolved
offline, not refreshed to unrelated dependency versions. Root and consumer
manifests/locks retain their hashes through the graph verification. The
consumer lock is checked in separately at `9bc9623`.

## Public API regression results

`make test-downstream` passes all three integration tests with no optional
consumer features, then the same three with all optional features forwarded:
`file_watcher`, `skeleton_overlay` and `physics_overlay`. These are six test
executions of three distinct tests. All-target/all-feature consumer Clippy
passes with `-D warnings`; the fixture is formatted. The tests exercise:

- Installing the public USD plugins with an independent headless Bevy app.
- Loading a named in-memory asset source through `AssetServer`.
- Two roots with independent clocks and distinct entities but a shared mesh
  asset; exact three-vertex geometry and independent seeks.
- Successful reload preserving matching entities, runtime components, mesh
  identity and independent animation positions.
- Invalid-source reload retaining last-good prim identities and reporting
  `UsdSceneState::Failed`; repair restores `Ready` and clears the failed state.
- Removing one scene root without removing unrelated runtime children or the
  other instance, then releasing the remaining live-stage ownership.
- Gated older successful and failed loads of one source remaining serialized
  while an unrelated source completes, and publishing the final loaded state.

Tests and graph verification are committed at `df1ec63`.

## Retained fixture corrections

The first compile attempted to inspect the private `UsdSceneInstance::revision`
field. The fixture was corrected to observe public prim/entity identities and
newly published prim paths; no library internals were exposed for the test.
The initial compile log is `prepare.log` and the corrected check is
`prepare-public-api.log`.

The first behavioral run incorrectly expected `Ready` after an invalid-source
reload. The library correctly retained the last-good scene while exposing
`Failed`. The corrected fixture checks both retention and the visible failure,
then requires `Ready` after repair. `initial-integration.rs` and `tests.log`
retain the original assertion; `tests-final.log` contains the passing matrix
and strict lint run. This was a fixture correction, not a runtime bug fix or
an instruction to hide errors.

```text
prepare.exit             = 1
prepare-public-api.exit  = 0
graph.exit               = 0
tests.exit               = 1
tests-final.exit         = 0
```

All root baseline inputs and final fixture inputs verify after the checks.
There is no runtime, vendor, root Cargo.toml or root Cargo.lock diff against
the starting checkpoint `d3305f8`.

## Scope still open

This qualifies the documented **local patched consumer configuration** for
these public headless APIs. It does not qualify an unpatched dependency,
published package, existing downstream application, first-visible GPU morph
frame, native image fidelity, browser/device/OS behavior or performance.
The optional feature compile checks do not claim native watcher events or
rendered overlay acceptance. Full cache/RSS lifecycle stress also remains open.

The normal-keyed tangent candidate remains isolated after failed editor timing
qualification; no new optimization variant or dependency change is adopted.
Next close actual downstream render and broader native/lifecycle/browser
acceptance gaps rather than treating metadata and these tests as release proof.
All commits remain local and unsigned; nothing is pushed.
