# Bevy 0.20 migration audit

## Status and boundaries

Work is local and unverified until the gates below pass. Do not push, merge remote
PRs, or release this migration without the user's approval. The library is used by
downstream applications; compiling the editor alone is not acceptance.

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
- [ ] Workspace/all-target checks, formatting and strict all-feature Clippy.
- [ ] Workspace tests with default, no-default and all features; doctests.
- [ ] Vendored asset tests and source ordering/failure/repair regressions.
- [ ] Native OpenUSD export/reopen checks and filesystem watcher tests.
- [ ] wasm32 check and browser bundle; distinguish build from browser acceptance.
- [ ] Real GPU shader/pipeline checks: flat forward/prepass/deferred/OIT, sheen,
  medium/transmission, strands with/without authored normals, GPU instancing,
  culling and shadows, skinning/morphs, dome lighting and compact instancing.
- [ ] Inspect captured images and compare deformation/transparency reference
  fixtures; pipeline compilation alone does not prove rendering equivalence.
- [ ] Native editor window, shared-GPU viewport, resize, capture, close/save
  protection and interactive editing smoke tests.
- [ ] Record downstream integration and platform coverage limitations. Linux
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
run plus native exports is in `/tmp/bevy-020-openusd-verify.log`; do not assume
that run has passed until its final result is recorded.

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
position/normal samples now exit cleanly. Full numerical checks are pending in
`/tmp/bevy-020-openusd-native-deformation.log`.

Next: finish the new-dependency gates, full GPU/native editor checks, strict
Clippy, watchers, native numerical comparisons and image inspection. Keep
follow-up commits focused and local; no pushes or remote merges.
