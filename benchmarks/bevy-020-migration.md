# Bevy 0.20 migration audit

## Status and boundaries

The local Bevy 0.20.1/WESL and current upstream OpenUSD migration passes the
recorded Linux/Vulkan and software-WebGL2 acceptance gates. Known skips and
untested platforms/downstream applications are listed under Coverage limits.
Do not push, merge remote PRs, or release without the user's approval. The
library is used by downstream applications; this is not a downstream release
signoff.

Follow-up: the user's hummingbird playback check exposed severe frame stalls.
The [playback investigation](hummingbird-playback.md) records local performance
fixes and remaining interactive acceptance gaps. The gates below do not establish
smooth playback for this asset.

Preserve Dave Lee's (`dave-gray101`) original PR commit and author identity in the eventual
merged history. The user explicitly wants his contribution credited, rather
than squashed into a commit attributed to someone else. Keep our follow-up
fixes separate. No public branch or PR has been changed.

The original commit was fetched locally as `contributor/feat-bevy-0.20`:
`5cbdee3f4c49912b153d03b6f8f07e51e1605512`. Local merge `cc9c80e` now retains that
exact commit as a parent; `git merge-base --is-ancestor` confirms its ancestry on
`develop`. The original author identity and contribution are unchanged. The user
requests small, focused local commits throughout the work, not one giant final
commit. Our fixes, dependency pins, regression tests and tooling are separate
commits. The upstream commit's existing sign-off is untouched; our commits are
unsigned, title-only conventional commits without trailers.

Starting point: `develop` at `5b3ebbc`, Bevy 0.19.1, OpenUSD revision
`d10fa4c2838e1e18a5c4fb4aa47534f94fde8995`.

## Reviewed sources

- [bevy_openusd draft PR #4](https://github.com/openusd-rs/bevy_openusd/pull/4),
  revision `5cbdee3f4c49912b153d03b6f8f07e51e1605512`.
- [Mara draft PR #1](https://github.com/bresilla/mara/pull/1), revision
  `0ba23f4de3971a965d54fbe8cb01deb7b9c76741`.
- [egui-scale PR #7](https://github.com/zakarumych/egui-scale/pull/7), revision
  `d2c7b861eaf01cc8149c375290961bdd4bc71723`.
- [Bevy migration guide](https://bevy.org/learn/migration-guides/0-19-to-0-20/).

As checked on 2026-11-09, all three PRs are open and unmerged. PR #4 has no
reported status checks. Mara's PR reports a merge conflict with current main.
Its code can be evaluated at the pinned revision without merging either PR.

## Architecture and invariants

`usd_bevy` owns the reusable library. `source` captures source bytes and package
dependencies; `asset` loads, validates and projects stages; `live`, `sync` and
`route` maintain the stage-to-entity mapping. `authoring`, `editor`, `reload`
and `persistence` preserve USD edits, layer save states, undo/redo, dependency
reloads and instance identity. `usd_macro` emits inline USD. None of those
contracts should be weakened to accommodate rendering changes.

The root `usdview` application embeds Bevy in Mara, sharing GPU resources with
egui rather than owning a separate Bevy window. Bevy, Mara, egui-wgpu and direct
wgpu dependencies must agree on one GPU API version. `examples/web` is a separate
wasm consumer and must be checked on wasm32, not just built natively.

The vendored `bevy_asset` serializes loads per source path, including the source
ID but excluding the label. Preserve fresh reads after acquiring the lock,
success/failure event ordering, parallel unrelated sources and weak lock cleanup.
The existing regression tests are part of the migration gate.

## Draft review findings

1. **Do not replace the current worktree with the PR branch.** It predates the
   newer OpenUSD revision and associated authoring/persistence adaptations.
   Several test hunks no longer apply. Keep the current OpenUSD pin and integrate
   rendering/API changes onto `develop`.
2. **WESL is not a filename-only change.** Twelve library modules use the old
   import/preprocessor dialect; their import paths, conditionals, material
   constants and registration paths must change together. The standalone GPU
   culling shader is plain WGSL and remains supported. The compact-instancing
   example also needs migration and typed storage-buffer layout checks.
3. **Flat-material transparency is incomplete in the draft.** It still gates OIT
   on `OIT_ENABLED` and sends unpremultiplied blend colors. Bevy 0.20 requires the
   material-specific OIT flag and premultiplied input. Test ordinary blending,
   OIT order independence, opaque/masked materials and prepass/deferred variants.
4. **Transmission can silently disappear.** `Camera3d` no longer enables screen
   space transmission automatically. Add it explicitly to application/capture
   cameras that previously had it. Do not attach render cameras in the USD camera
   route: that route intentionally only projects authored camera metadata.
5. **Rust API changes span more than materials.** Shape/bounds imports,
   `MeshAabb::get_aabb`, generic extraction, reflection conversion errors,
   component enumeration and wgpu readbacks all need checking.
6. **The UI upgrade changes behavior.** Screenshot callbacks replace input
   events; late completions must remain request-scoped. Repaint timing and texture
   delta ownership need regression tests as well as a real editor launch.
7. **The asset patch needs upstream comparison.** The draft rebases it onto
   0.20.0. Published 0.20.0 and 0.20.1 asset `src/` trees compare identically;
   update package metadata to 0.20.1 and verify that only the local load-order
   patch differs from the published source. Retain licenses and correct VCS data.
8. **Test tooling has drifted.** Existing capture scripts still invoke an old
   GNU Make interface, but current recipes live in `.make.lua`. Expose an Oslo
   example runner and use that for repeatable GPU checks.

## Acceptance gates

- [x] Record pre-migration workspace test results (one failure; see below).
- [x] Workspace/all-target checks, formatting and strict all-feature Clippy.
- [x] Workspace tests with default, no-default and all features; doctests.
- [x] Vendored asset tests and source ordering/failure/repair regressions.
- [x] Native OpenUSD export/reopen checks and filesystem watcher tests.
- [x] wasm32 check and browser bundle; distinguish build from browser acceptance.
- [x] Real GPU shader/pipeline checks: flat forward/prepass/deferred/OIT, sheen,
  medium/transmission, strands with/without authored normals, GPU instancing,
  culling and shadows, skinning/morphs, dome lighting and compact instancing.
- [x] Inspect captured images and compare deformation/transparency reference
  fixtures; pipeline compilation alone does not prove rendering equivalence.
- [x] Native editor window, shared-GPU viewport, resize, capture, close/save
  protection and interactive editing smoke tests.
- [x] Record downstream integration and platform coverage limitations. Linux
  results do not establish Windows/macOS or browser runtime acceptance.

## Commands

Use `nix develop --command oslo make <recipe>` outside the project shell.
`test-all`, `harden`, `test-native`, `check-web` and `test-assets` cover separate
gates. `run-example --name viewer_capture --args 'ASSET OUTPUT.png TIME'` builds
and runs the offscreen capture with native graphics on NixOS.

## Execution record

- The Bevy 0.19 baseline (`test-all`) stopped in the library: 649 passed,
  1 failed, 20 ignored. Log: `/tmp/bevy-019-baseline.log`. The failing instancer
  test assumes that two scenes finish projecting in a single update, despite
  the shared 10 ms projection budget. Its fixture now selects an unlimited
  projection budget; it passes in the migrated workspace suite. No production
  budget changed. Local commit: `2dfb861`.
- Applied the PR's rendering/API changes without its older OpenUSD pin, lockfile
  or bulk filesystem-path test edits. The current OpenUSD revision is retained.
- Pinned Mara's draft revision and vendored only its Bevy host to retain the
  render-start pacing fix missing from the draft. `test-host` exercises its
  pacing tests. The transitive egui-scale revision is fixed by `Cargo.lock`.
- Compared the rebased asset source with published Bevy 0.20.1: only the expected
  per-source load-order patch differs in `src/server/mod.rs`.
- Initial 0.20 check stopped before compilation: the old Nix shell provided
  Rust 1.96.0, while Bevy/WESL require 1.97.1. Updated the rust-overlay lock and
  selected Rust 1.98.1; the rerun is in `/tmp/bevy-020-check.log`.
- Corrected the flat shader's material OIT flag and premultiplication, restored
  screen-space transmission on viewer/capture/web cameras, and added a late
  screenshot-callback isolation test. GPU and test verification are pending.
- Workspace/all-target `check-all` passed on Rust 1.98.1 / Bevy 0.20.1 after
  resolving the new projection helper and egui ID APIs, without compiler warnings.
  Log: `/tmp/bevy-020-check-clean.log`. Subsequent fixture/layout tests still need
  to compile and run in the full test gate.
- The full workspace/all-target test suite passed with `CARGO_BUILD_JOBS=2`,
  `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` to bound linker memory.
  Result: 841 passed, 0 failed, 20 ignored. Log: `/tmp/bevy-020-tests.log`.
  This includes the material-route fixture and compact storage-layout tests,
  not runtime shader compilation. Ignored native tests are separate gates.
  The first test-build process exited with signal status 143 before running
  tests; confirmed that no Cargo/rustc process remained, then resumed the same
  command using the completed dependency artifacts. Its previous log is retained
  at `/tmp/bevy-020-tests-interrupted.log`.
- `test-render --output NEW_DIRECTORY` now captures the material fixture through
  forward, prepass, deferred and OIT paths, plus GPU instancing/shadows,
  skinning/morphs, dome lighting and the order-independence comparison. Its CPU
  fixture test checks that the intended custom material routes are selected.
  Render validation uses `USD_WGPU_VALIDATION=1` and `USD_WGPU_DEBUG=1` after
  project-environment loading. The first render gate is running, with evidence
  under `/tmp/bevy-020-render-20261109`.
- `check-web` passed on wasm32, with two dead-code warnings for native-only
  reload polling (`WatchState` and `watch`). Log: `/tmp/bevy-020-web-check.log`.
  This does not establish browser runtime acceptance.
- `test-capture-tools` passed after replacing hard-coded `/bin/bash` launches
  with PATH lookup on NixOS. Preflight, process-group cleanup, exit propagation,
  failed-handshake protection and viewer-log rejection checks pass.
  Log: `/tmp/bevy-020-capture-tools.log`. Local commit: `7c21328`.
- The tested Rust toolchain pin is a separate local commit: `2e9a4ab`.
- The local contributor merge preserves newer `develop` code and its OpenUSD
  revision. It also includes the PR's portable filesystem-path tests that were
  initially excluded from patch application. The conflict for the already-removed
  package entry-budget test remains resolved as on newer `develop`; no current
  assertion was removed. The restored portability edits require rerunning tests.
  Separate follow-ups rebase the patched asset package (`1116902`), retain the
  Mara host pacing fix (`e6a8b2d`) and pin the dependency graph (`4aaef99`).
- `test-assets` passed: 110 passed, 0 failed, 1 ignored. Log:
  `/tmp/bevy-020-assets.log`. Library-specific source ordering tests are also
  required; this vendored suite alone is not the full ordering regression gate.
- The first real Vulkan run on the RTX 4080 failed the capture gate: helper WESL
  modules were embedded but never loaded, and a material shader request could
  start before its asset registration. Evidence:
  `/tmp/bevy-020-render-20261109/materials-forward.log`. The repair registers
  assets before installing material plugins and retains helper shader handles
  once per app without leaking them. Headless use without shader assets remains
  supported. New tests cover registration ordering, duplicate setup and retained
  library loading; GPU revalidation is pending in the `-libraries` evidence run.
- The wasm32 check now passes without warnings after native polling is excluded
  from wasm compilation (`0bbb684`): `/tmp/bevy-020-web-check-clean.log`.
- The browser bundle initially failed before compilation because Trunk parses
  `NO_COLOR` as a boolean, whereas the environment supplied `1`. The make recipes
  now use `NO_COLOR=true` (`ed3ef2e`); the release bundle is building.

## Expanded verification checkpoint

- `test-library` passed without default features (651 passed, 20 ignored) and
  with all features (659 passed, 29 ignored), including WESL registration and
  helper-retention regressions. Doctests passed. Log: `/tmp/bevy-020-library.log`.
  These counts precede the latest hierarchy timing-fixture repair.
- The Vulkan material gate now captures forward, prepass, deferred and OIT,
  plus instancing and all three skin/morph render paths without shader errors.
  Evidence: `/tmp/bevy-020-render-20261109-libraries/`. That run stopped at a
  tooling mistake: the dome selector needs the authored `/Env` path, not `1`.
  Corrected it and started a fresh full run at `/tmp/bevy-020-render-final/`.
  The fresh run also uses the corrected light intensity consistently: USD
  intensity 1.2 projects to 12,000 lux, not the original fixture's 12 million.
- `harden` found another two-root fixture assuming projection finishes in one
  update (`hierarchy_prototypes_share_meshes_and_keep_independent_animation`).
  Like the earlier baseline failure, its fixture now selects an unlimited
  projection budget; the production budget and assertions are unchanged.
  Rerun: `/tmp/bevy-020-harden-rerun.log`.
- The Mara host's ordinary tests passed 9 with 2 ignored. The explicit GPU
  tests initially had no camera, so the RGBA test correctly received no image.
  Camera-equipped tests now exercise actual readback, non-aligned rows, resize
  and GPU-to-CPU switching, and reject renderer failure rather than silently
  skipping. Both assertions passed, but the process then crashed with an NVIDIA
  NVVM error while asynchronous pipeline work remained. This is not a passing
  gate; a pipeline-completion wait is being checked before test teardown.
- The wasm release bundle passed: `/tmp/bevy-020-web-build.log`. Chromium WebGL2
  then loaded both HTTP Kitchen and in-memory scenes, orbited and resized without
  runtime errors. All three screenshots in `/tmp/bevy-020-browser-basic/` were
  inspected. The adapter is SwiftShader: this establishes software WebGL2
  behavior, not hardware browser performance or compute/OIT support. The default
  Chromium run also selected SwiftShader. A fresh profile needed
  `--password-store=basic` to avoid navigation hanging in this environment.

### Existing OpenUSD package-export blocker

`test-native` passed 13 export checks but failed
`native_export_repackages_snapshot_archives` before calling native `usdcat`:
`unsupported layer extension "usda]"`. Log: `/tmp/bevy-020-native.log`.
The same test in the preserved Bevy 0.19 baseline executable
`target/debug/deps/usd_bevy-5fc903b8e07189e3` fails identically:
`/tmp/bevy-019-native-package-baseline.log`. This is a verified existing failure,
not evidence of a Bevy 0.20 regression, but the native export gate is not green.

The pinned OpenUSD packager splits a package-relative source path without
removing its closing bracket. Upstream fixes include
[`cde477f`](https://github.com/mxpv/openusd/commit/cde477f9d1b23de505ea2fabe8664a6cfa416be3)
and [`a304ba2`](https://github.com/mxpv/openusd/commit/a304ba2100083a3fa719ee7ea02a844c3f1c03fa).
Even the first fix is 11 commits past our pin, including the large live-layer
handle API migration. Keep the OpenUSD pin for this Bevy migration rather than
silently introducing that separate API migration or a packaging workaround.
Do not suppress the failing test or describe native export as fully accepted.

The user subsequently required staying current with upstream OpenUSD, including
its API changes: update upstream rather than retaining the old pin or adding a
compatibility workaround. Upstream `mxpv/openusd` main was verified at
`900ea31591348e3fbf416d4661d703aa8700771b` on 2026-11-09; this is now the selected
revision across all manifests. Adapt the new layer APIs and rerun persistence,
authoring, undo/reload and all affected gates.
Do not treat the earlier Bevy-only results as verification of the new dependency.

### Bevy checkpoint before the OpenUSD update

- Full `test-render` passed, including dome selection and OIT order independence:
  `/tmp/bevy-020-render-final.log`. OIT reversed triangles are RGB-identical;
  ordinary blending changes 60,552 pixels as the control requires. Forward,
  prepass, OIT and dome images were inspected. Remaining images still require
  visual inspection. The native UI capture displays a functioning shared-GPU
  viewport, but the material fixture's strands are missing there, unlike the
  offscreen capture; investigate before native rendering acceptance.
- GPU/CPU blended-joint deformation is RGB-exact at 0, 5 and 10 with shadows and
  MSAA off and the authored camera: `/tmp/bevy-020-deformation/results.tsv`.
- Mara GPU tests pass 2/2 with normal asynchronous, pipelined rendering and clean
  process exit after the render-world pipeline-status wait:
  `/tmp/bevy-020-host-gpu-status.log`. Earlier direct access to `RenderApp` from
  the main app failed because pipelined rendering had moved it to its thread.
- Native UI compositor and viewport readback checks pass, with two compositor
  captures: `/tmp/bevy-020-native-ui.log` and `/tmp/bevy-020-native-ui.png`.
- Compact overview is RGB-exact against expanded instances, and offscreen
  culling works. The edge-view case fails its restored visibility assertion:
  `/tmp/bevy-020-compact-final/edge-compact.log`. This gate remains incomplete.
- Workspace no-default tests pass 653 library and 110 editor tests. `harden`
  then fails strict Clippy with 107 library-test diagnostics, including new
  Rust chunk-iterator lints and existing structural lints. All-feature workspace
  tests were not reached: `/tmp/bevy-020-harden-rerun.log`.
- Native asset watcher tests pass 5/5; editor texture watcher tests fail 2/4
  (change-tick preservation and retry/recovery): `/tmp/bevy-020-watchers-oracle.log`.
- The native deformation C++ oracle builds, but its process aborts with
  `free(): invalid pointer` in all four Rust integration checks:
  `/tmp/bevy-020-native-deformation.log`. Investigate the SDK/ABI before claiming
  numerical parity; the separate CPU/GPU image comparisons do not replace it.

### Upstream OpenUSD integration

The manifests and lockfile now pin upstream `900ea31591348e3fbf416d4661d703aa8700771b`.
Persistence, reload, editor inspection and projection use `LayerHandle` and
short-lived guards. Save-as relocates a copied layer in a separate registry so
the original stage is neither edited nor held busy by the copy's edits.

Observer installation now uses fallible constructors before publishing a new
document. Existing `new` constructors retain their public signature and document
the failure condition; `try_new` exposes the upstream error. Observer closures
hold weak state references, so even a drop during an active layer edit releases
the wrapper's data. If upstream refuses removal while busy, the inert weak
observer remains until stage destruction and the failed removal is logged.

Live queue checks/drains flush upstream notifications before inspecting the
queue. Fallible variants preserve queued edits when flushing fails. Editor
snapshots and texture indexing also flush, material reads invalidate on pending
changes, and failed save-state hashes are retried at the same revision.

`check-all` passed after the API migration. `test-library` passed 651 no-default
tests before the added observer regressions, then 667 all-feature tests including
all eight new regressions (20/29 intentional ignored checks respectively):
`/tmp/bevy-020-openusd-library.log`. The added tests cover direct/shared-layer
projection, busy construction/drain, observer lifetime, editor history, revision
tracking, material-cache invalidation and save-state recovery. A fresh complete
run plus native exports is in `/tmp/bevy-020-openusd-verify.log`. All 14 native
export tests pass there, including the previously failing archive repackaging
case. The native reference-custom-data check also passes, followed by 659
no-default and 668 all-feature library tests. The latter includes the additional
unchanged-texture publication regression added during this run.

The wasm target also checks with the new dependency:
`/tmp/bevy-020-openusd-web-check.log`. Browser runtime and rendered acceptance
still need to be repeated; earlier captures used the previous dependency.

### Native oracle compiler ABI

The native oracle crash was traced to `UsdGeomXformCache` destruction, not a
numerical discrepancy. OpenUSD 25.05's `arch/defines.h` selects GNU extension hash
containers for GCC and standard unordered containers for Clang. The shell's
Clang-built helper therefore disagreed with the GCC-built native SDK's object
layout, despite using the same libstdc++ and USD headers. Backtrace:
`/tmp/bevy-020-native-oracle-debug-backtrace.log`.

The Nix shell now supplies `USD_NATIVE_CXX` and matching `USD_NATIVE_CPPFLAGS`,
`USD_NATIVE_LDFLAGS`, and `USD_NATIVE_LDLIBS`; only the native oracle recipe uses
them. No global C++ compiler override is applied. Rebuilding with
`nix develop --command oslo make build-native-deformation` succeeds, and direct
position/normal samples now exit cleanly. All four native numerical comparison
tests pass against the new Rust OpenUSD dependency:
`/tmp/bevy-020-openusd-native-deformation.log`. They cover combined skin/morph
positions, combined normals, blended joints and animated blended-joint normals
at five times from 0 through 10.

Next: finish the new-dependency gates, full GPU/native editor checks, strict
Clippy, watchers, native numerical comparisons and image inspection. Keep
follow-up commits focused and local; no pushes or remote merges.

### Live-editor rendering and watcher repairs

All nine watcher checks now pass in `/tmp/bevy-020-openusd-watchers-fixed.log`.
Unchanged texture installs previously removed and reinserted their resource,
changing its Bevy tick even when every image handle was reused. Both explicit
refresh and disk-reload publication now leave an identical resource untouched.
A regression checks both paths, actual image replacement and removal. The native
document-preservation test retains both watcher mechanisms. The event-watcher
retry test disables the independent disk poller, which otherwise refreshed its
pixels before the event watcher had rearmed, defeating that test's premise.

The missing native-viewer strands and neutral medium were caused by material
registration being confined to `UsdAssetPlugin`. The live-editor path uses
`UsdPlugin` and `LiveStagePlugin`; these now share idempotent registration of all
four custom materials when PBR is installed. The old route fixture's manually
created material assets did not exercise this startup contract. A new test checks
each public projection plugin independently. Its initial expectation of four
helper libraries was corrected to the three actually used by these materials;
GPU instancing owns the fourth helper separately.

The new native UI gate passed with two compositor frames and viewport readback:
`/tmp/bevy-020-openusd-native-ui.log`. Both the viewport and second compositor
image were inspected: round/oriented strands and the green transmissive medium
are now visible. Evidence: `/tmp/bevy-020-openusd-native-ui.viewport.png` and
`/tmp/bevy-020-openusd-native-ui.second.png`. The latest no-default suite passes
661 tests; the all-feature suite passes 669 tests, with 20 and 29 explicit skips
respectively. Doctests also pass. Evidence:
`/tmp/bevy-020-openusd-material-plugins.log`.

The compact example now marks its authored batch bounds `NoAutoAabb`; otherwise
Bevy replaces them with prototype-only bounds. Its first new run captured the
overview but failed the strict log gate on a transient built-in PBR WESL import
warning: `/tmp/bevy-020-openusd-compact/overview-compact.log`. Bevy classifies
this as `ShaderImportNotYetAvailable` and retries, but the checker still rejects
it; it has not been weakened. The retry in
`/tmp/bevy-020-openusd-compact-retry.log` passes both overview captures and their
comparison (zero changed pixels out of 480,000). The compact edge view restores
four visible batches and keeps the offscreen batch culled, but its log again
fails on the startup import warning. The expanded edge capture and comparison
did not run. This gate remains incomplete until final pipeline readiness,
both edge views, their comparison and visual inspection are established.

### Strict lint checkpoint

The `lint --format json` recipe exposes the same workspace, all-target and
all-feature Clippy gate as `harden`, retaining `-D warnings`. The current baseline
reports 68 library errors and 108 library-test errors (including overlapping
diagnostics), not a passing lint gate. Evidence:
`/tmp/bevy-020-openusd-clippy.jsonl` and `/tmp/bevy-020-openusd-clippy.log`.
The first focused cleanup converts fixed-size chunk iteration to array slices,
preserving the existing handling of incomplete trailing chunks. Its workspace
check, 661 no-default tests, 669 all-feature tests and doctests pass in
`/tmp/bevy-020-chunks-verify-rerun.log`. Array references require an explicit
dereference in the packed-pixel equality check; compilation caught that detail
before this verification rerun. The strict lint gate still needs another run
and further cleanup. No lint rules have been suppressed.

### Library and viewer lint cleanup

The strict gate now checks every target with `--keep-going`, so an example
failure no longer hides diagnostics from the other examples. The library and
viewer (including their test targets) have no remaining diagnostics in
`/tmp/bevy-020-lint-viewer.jsonl`. The workspace gate still exits nonzero: fifteen
diagnostics remain across `viewer_capture`, `gif_frames`, `compact_instancing`
and `editor_assembly`. These are not a passing workspace lint result.

Focused commits remove redundant references, flatten guarded branches, name
query/cache types and group Bevy system parameters without changing their
resource access. The outliner shares one traversal context and no longer needs
its previous argument-count allowance. No new lint allowances were added.
The texture conversion table is indexed by byte then channel; its existing
exhaustive linear/sRGB regression checks all byte values, transforms and edits.
The sun threshold retains rejection of unordered floating-point comparisons.
The material hash uses the `BuildHasher` trait's single-value operation.

The post-cleanup library rerun passes 661 no-default and 669 all-feature tests,
with 20 and 29 explicit skips respectively, and passes doctests:
`/tmp/bevy-020-lint-library-tests.log`. The viewer refactors still require the
full workspace tests and fresh native interaction/render checks. Native ignored
tests, wasm/browser runtime and rendering acceptance remain separate gates.

The compact edge image from the earlier retry was inspected and contains the
expected visible blue boxes: `/tmp/bevy-020-openusd-compact-retry/edge-compact.png`.
This does not replace the missing expanded-edge comparison or resolve its
startup shader-warning failure. Nothing has been pushed.

### Strict lint and pipeline-readiness checkpoint

Strict workspace/all-target/all-feature Clippy now passes, first in
`/tmp/bevy-020-lint-examples.log`, then again with the new capture probe in
`/tmp/bevy-020-pipeline-probe-checks.log`. No rules were suppressed. Upstream
OpenUSD `main` was rechecked with `git ls-remote` on 2026-11-10 and still resolves
to the pinned `900ea31591348e3fbf416d4661d703aa8700771b`.

Both capture examples now collect pipeline state in the render world after
rendering, require a nonempty fully compiled cache, and recheck it when the
screenshot readback arrives. Queued pipelines and retryable missing imports
prevent readiness, as do permanent shader errors. Compact capture waits for
three ready updates after the culling cycle and has a 60-second bound; the
viewer retains its existing 60-ready-update and timeout requirements.

The log checker distinguishes recovered import warnings from permanent errors.
A warning requires a later positive compiled-pipeline count followed by a
successful capture. Missing, malformed, zero-count, stale or out-of-order proof
fails, and a later warning invalidates earlier proof. Permanent failures still
fail even with subsequent proof markers. Capture unit tests, probe state tests,
preflight, process cleanup and log-checker regressions pass in
`/tmp/bevy-020-pipeline-probe-checks.jsonl`. GPU integration is the next gate;
these checks alone do not establish rendering acceptance.

### Compact GPU acceptance and workspace rerun

The first probe-backed compact run was terminated with status 143 while its
final expanded capture was waiting for Cargo's build lock. Its compact edge
capture had already recovered the built-in import warning and reported 23
compiled pipelines. The gate now builds both tools once and snapshots the
executables into its evidence directory, with SHA-256 identities, before any
capture. Paired captures no longer reacquire Cargo's lock or change binaries.

The complete retry passes all four capture/log checks and both comparisons:
`/tmp/bevy-020-compact-pipeline-final.jsonl` and
`/tmp/bevy-020-compact-pipeline-final/`. Both views have zero pixels outside the
existing RGB tolerance of 1, over 480,000 pixels each. Overview RGB is exact;
edge maximum RGB error is 1. The overview and both edge images were inspected.
Offscreen geometry is fully culled; compact visibility restores 8 overview
batches and 4 edge batches. Both compact runs recovered actual import warnings
before recording their 23-pipeline proof and successful readback. This accepts
the compact forward-PBR fixture, not the separate USD/shadow/deformation gates.

The broader `harden` run passed its no-default workspace tests (663 library,
110 viewer, plus doctests), then strict Clippy. Its all-feature library phase
found four fixtures reading incomplete scene projections after one update:
`/tmp/bevy-020-full-verification.log`. Those geometry fixtures now explicitly
request unlimited projection and assert `UsdSceneState::Ready` before inspecting
their entities. The production 10 ms budget, finite-budget tests and geometry
assertions are unchanged. Full workspace verification must be rerun; later
commands in the failed verification chain did not execute.

### Full verification after the fixture repair

The complete sequential rerun exits successfully:
`/tmp/bevy-020-full-verification-rerun.log`. `harden` passes formatting,
no-default workspace tests (773 passed, 21 ignored), strict all-target/all-feature
Clippy, and all-feature/all-target tests (865 passed, 29 ignored). Default
workspace/all-target tests pass 859 with 20 ignored. These overlapping suites
must not be summed into a count of unique tests.

The same chain passes vendored asset tests (110 passed, 1 ignored), ordinary
Mara host tests (9 passed, 2 ignored), native export/custom-reference tests
(15 passed), filesystem watchers (9 passed), native deformation comparisons
(4 passed), and warning-free wasm compilation. The native tests explicitly run
their ignored cases; passing the ordinary suites alone would not cover them.

A fresh isolated library check passes 661 no-default and 669 all-feature tests,
plus doctests. Both explicit Mara GPU tests pass with validation enabled and
clean process exit. Evidence: `/tmp/bevy-020-final-native-gates.log`. The following
native resize gate stops at tool preflight because `xwininfo` is unavailable;
it does not reach the application and subsequent UI commands do not execute.

The Linux development shell now declares Weston, Xwayland and xwininfo rather
than depending on manually added store paths. All three resolve in a fresh
shell (`/tmp/bevy-020-ui-tools.log`). Close and resize preflights name missing
commands. Their native runtime rerun is separate from this environment check.
Browser runtime, full scene rendering and native interaction remain pending.

### Native and browser acceptance; visual render failure

The native UI chain passes after adding the missing tools:
`/tmp/bevy-020-native-ui-final.log`. The oversized-window capture shows OIT
disabled at its 128 MiB budget and a working scene after restoring 1440x920.
The OS-close prompt and cancel images were inspected; the prompt is present
before cancellation and absent afterward, while discard exits cleanly. WESL
material compositor and viewport captures show flat geometry, sheen, the
transmissive medium and both strand types. The second compositor capture also
passes the image checks.

The actual numeric-array edit replay passes with `USD_VIEWER_PANE=inspector`
and `/Root` selected: `/tmp/bevy-020-final-native-edit-inspector.png`. The image
shows the edited orange cube, `1 0.3 0.05`, and one modified layer. The first
attempt omitted the inspector setting; its inputs zoomed the viewport instead
and are not editing acceptance. The source asset remains unchanged on disk.

The fresh wasm release bundle passes (`/tmp/bevy-020-web-build-final.log`).
Browser smoke passes for both HTTP and in-memory roots, orbit and resize:
`/tmp/bevy-020-browser-final-retry/result.json`. All three images were inspected.
The adapter is SwiftShader WebGL2, not hardware browser coverage. A first launch
failed before opening Chromium because an old store path was absent; the retry
resolves Chromium and Node from the pinned Nixpkgs input.

Visual inspection of `/tmp/bevy-020-full-render-final-retry/` finds missing
strands under a deferred camera and dark strands with normal-prepass/OIT modes,
despite successful pipeline checks. The generic wrapper lets forward-only
extensions inherit the deferred default, and strand fragments load normals
from a prepass they do not participate in. Sheen also has only a forward
fragment shader. New material-route assertions and two fixed interior-strand
image regions cover these failures. This render gate is not accepted yet.

An earlier render/deformation chain was deliberately stopped after its native
edit capture because the queued camera path was incorrect; the corrected
deformation camera is `/ReferenceCamera`. Retained partial evidence is in
`/tmp/bevy-020-edit-render-final.log` and `/tmp/bevy-020-full-render-final/`.

### Forward-only extensions and final visual proof

The new route assertion fails before the fix (`Auto` instead of `Forward`), and
the strand-region check rejects the dark prepass image with 0/25 lit pixels:
`/tmp/bevy-020-forward-regression-before.log` and
`/tmp/bevy-020-strand-region-before.log`. The former also exposes another
independent-clock fixture assuming complete projection within one update. Its
opacity fixture now selects unlimited projection and asserts ready roots;
the production budget is unchanged (`aff2dc9`).

`d44b534` keeps extensions with a custom forward fragment but no custom deferred
fragment in the forward pass, including on deferred cameras. Flat materials
retain their deferred implementation. The original standard material is not
mutated; tests cover source preservation, cache reuse, shared handles, edits
and restoration for Auto, Deferred and Forward inputs. Strand specialization
uses its fragment normals rather than loading another surface's prepass normals.
Stochastic coverage, prepass exclusion and shadow geometry are unchanged.

Isolated library tests pass 662 no-default and 670 all-feature tests, with 20/29
explicit skips, plus doctests. Strict workspace/all-target/all-feature Clippy
passes: `/tmp/bevy-020-forward-regression-fixed.log`.

The complete GPU gate exits zero in `/tmp/bevy-020-render-forward-fixed.log`.
Both authored-normal and round-strand regions pass 25/25 lit pixels in all four
renderer modes. The corrected prepass, deferred-camera and OIT images were
inspected. Forward materials, instancing, all six skin/morph images, dome and
all transparency controls are byte-identical RGBA to the inspected images in
the preceding run. OIT remains RGB-exact under reversed triangle ordering;
ordinary blending differs at 60,552 pixels. The preceding deformation comparison
also reports RGB-exact GPU/CPU images at 0, 5 and 10; all three GPU images were
inspected. Its enclosing shell returned 143 after writing the three results,
so a clean independent gate remains part of final verification.

The release browser bundle was rebuilt after this fix
(`/tmp/bevy-020-web-build-forward.log`), and the software WebGL2 smoke passes
again at `/tmp/bevy-020-browser-forward/`; initial, orbit and resized images
were inspected. Upstream main was rechecked on 2026-11-10 and remains `900ea315`.

### Coverage limits

- Native runtime evidence is NixOS/Linux on an NVIDIA RTX 4080 with Vulkan.
  Windows and macOS were not exercised.
- Browser runtime evidence is Chromium SwiftShader WebGL2. Hardware browser
  rendering, compute/OIT browser support and browser performance are not claimed.
- The optional Moana dataset is absent at `/pkg/datasets/moana`; its dataset-
  dependent Ptex check remains skipped. Synthetic Ptex tests run in the library
  suite. Ordinary ignored native export, watcher and deformation checks are
  exercised by their explicit recipes rather than counted as ordinary passes.
- Bevy's unchanged vendored `handle_try_apply_verifies_type_id` test remains
  ignored upstream for issue #24111. The macro crate's illustrative doctest is
  also marked ignored. These are not counted as passing tests.
- Repository examples, isolated library features and native USD interoperability
  were tested. The user's separate downstream application checkouts were not
  changed or validated; those integrations remain a release consideration.
- No push, public merge, tag or release has been performed. Local contributor
  ancestry still includes Dave Lee's original commit unchanged.

### Final verification checkpoint

The final `harden`, default workspace test and wasm-check chain exits zero:
`/tmp/bevy-020-final-workspace.log`. No-default workspace tests pass 774 with
21 ignored (including the illustrative doctest); all-feature/all-target tests
pass 866 with 29 ignored; default all-target tests pass 860 with 20 ignored.
Formatting, workspace checks, strict Clippy and wasm compilation all pass.
Do not sum these overlapping suites as unique tests.

The independent deformation recipe now exits zero:
`/tmp/bevy-020-deformation-accepted.log`. GPU/CPU pairs are RGB-exact at all three
time codes and byte-identical to the previously inspected captures. The final
integration chain also exits zero (`/tmp/bevy-020-final-integration.log`):
15 native export/reference checks, 9 watcher checks, 4 native numerical
deformation checks, 2 explicit host GPU checks, 110 vendored asset tests and
9 ordinary host tests pass. The two GPU tests' ordinary-suite skips are covered
by their explicit run; the upstream asset skip remains as documented above.

The published Bevy 0.20.1 asset source was compared again with the vendored tree:
only `src/server/mod.rs`, containing the local load-order patch, differs.
Contributor ancestry and the current upstream OpenUSD pin were rechecked.
The final capture-tool/compact/native-window chain exits zero:
`/tmp/bevy-020-final-capture-native.log`. It passes 31 capture/probe unit tests,
shell preflight/process cleanup/log-checker regressions, compact comparisons and
native compositor/viewport readbacks. The compact overview and edge captures
are byte-identical to the inspected earlier images; overview RGB is exact and
edge maximum RGB error remains 1 against expanded instances. Both final native
material images were inspected at `/tmp/bevy-020-native-accepted.png` and its
`.viewport.png` companion. The second compositor capture also passes. The local
migration checkpoint is complete within the documented coverage; nothing has
been pushed, tagged, released or merged remotely.
