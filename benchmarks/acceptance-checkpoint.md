# Acceptance checkpoint at 30ca5ed

The integration goal is not complete. This checkpoint does not narrow the
acceptance checklist in BEVY_WORK.md or the release blockers in SUPPORT.md.

## Current gates

- Fresh make test-all fails nested_package_dependency_reload_preserves_two_live_instances:
  498 library tests pass, one fails, 19 ignored. Both roots remain Failed during
  a package repair wait, with AssetServer reporting FileNotFound in the archive.
  Log: `/tmp/acceptance-refresh-tests.log`.
- Explicit workspace ignored-library tests pass all 19: 14 native exports,
  one native reference metadata test and four native deformation tests.
  Native filesystem watcher tests are feature-gated and not covered by this run.
  Log: `/tmp/acceptance-refresh-native.log`.
- make check-all passes: `/tmp/acceptance-refresh-check.log`.
- Current release viewer build and visible Fendt evidence:
  `benchmarks/current-machine-viewer.md`.

## Reproducible repair failure

Earlier isolated stress used usd_bevy-16a5326795ddd708, whereas the workspace gate
uses usd_bevy-02fe53806703c793. Cargo fingerprints show different dependency
artifacts despite the same crate-level feature list. They are not interchangeable
validation configurations. Directly running the workspace binary's nested repair
test passes once, then fails on iteration two with the same repair timeout:
`/tmp/workspace-binary-repair-{1,2}.log`.

Reproduction after building the workspace tests (binary hash is checkout-specific):

```sh
make --eval='workspace-repair:; @target/debug/deps/usd_bevy-02fe53806703c793 --exact asset::tests::nested_package_dependency_reload_preserves_two_live_instances --nocapture' workspace-repair
```

Priority is to isolate repair-event/load-completion ordering in this configuration,
not increase the timeout or count the narrower passing runs as a fix. No root
cause or production fix is established yet.

## Other confirmed open work

SUPPORT.md lists external disk-conflict detection as missing. Inspection confirms
SaveState compares in-memory authored content, and persistence::atomic_write
stages/syncs/replaces files without a loaded-source or pre-publication content
conflict check. Atomic publication alone does not prevent lost external edits.
This needs a separate save-safety implementation after the reproducible reload
failure. GPU preparation, broader render parity and intermittent native black
frames retain the qualifications in the existing support matrix.
