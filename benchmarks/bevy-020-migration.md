# Bevy 0.20 migration audit

## Status and boundaries

Work is local and unverified until the gates below pass. Do not push, merge, or
release this migration without the user's approval. The library is used by
downstream applications; compiling the editor alone is not acceptance.

Preserve Dave Lee's (`dave-gray101`) original PR commit and author identity in the eventual
merged history. The user explicitly wants his contribution credited, rather
than squashed into a commit attributed to someone else. Keep our follow-up
fixes separate. No public branch or PR has been changed.

The original commit was fetched locally as `contributor/feat-bevy-0.20`:
`5cbdee3f4c49912b153d03b6f8f07e51e1605512`. The contributor commit is not yet an
ancestor of `develop`. Arrange the final integration history to retain it before
any push. The user requests small, focused local commits throughout the work,
not one giant final commit. Our independent fixes are being committed separately;
the contributor-derived migration remains in the worktree pending integration.

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

Next: finish the GPU gate, then run `test-library`, `test-assets`, `test-host`,
`harden`, `test-native`, watcher tests and the browser/interactive gates. Reuse
the debug-profile environment above for native test/build commands to avoid
rebuilding the same dependencies. Keep follow-up commits focused and local.
