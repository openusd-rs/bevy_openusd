# Acceptance checkpoint

The integration goal is not complete. This checkpoint does not narrow the
acceptance checklist in BEVY_WORK.md or the release blockers in SUPPORT.md.

## Source reload serialization

The workspace now patches Bevy asset 0.19.1 from `vendor/bevy_asset`. Loads of
the same source, including its labels, serialize from reader acquisition through
completion-event publication. Other sources remain concurrent. Both failures and
successes retain their ordinary publication semantics; an older snapshot cannot
finish after a newer snapshot of that source. See `vendor/bevy_asset/PATCHES.md`
for provenance, implementation scope, and the same-source latency tradeoff.

The former broken-state characterization is replaced with gated older-failure
and older-success regression tests that also verify unrelated-source progress.
Both pass in the full workspace binary. The original nested-package two-instance
repair test passes 100 consecutive workspace-binary executions (800 repair cycles):
`/tmp/serialized-repair-{1..100}.log`.

Fresh `make test-all` passes 666 tests across 33 suites, with 19 ignored, including
the previously failing repair case: `/tmp/bevy-serialized-workspace.log`.
`make check-all` also passes: `/tmp/bevy-serialized-check.log`.
This fixes the observed reload race, not
the remaining rendering, save-conflict, or broader qualification requirements.

## Prior gates at 30ca5ed

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

## Isolated load-completion ordering defect

The instrumented workspace stress run failed on iteration 48:
`/tmp/workspace-repair-trace-48.log`. The last repair sequence was:

```text
READ revision=12: broken package
NOTIFY: repaired package
READ revision=14: repaired package
LOADED revision=15
TEXTURE_FAILED revision=13: specified file not found in archive
```

Both roots subsequently timed out with AssetServer Failed and the previous
projected revision 9. An older snapshot completed with failure after the newer
snapshot completed successfully. Temporary trace instrumentation was removed
after capturing this evidence.

The preceding ordinary unit test
`bevy_reload_characterization_stale_failure_overwrites_loaded_state` now isolates
this ordering using a tiny non-USD asset loader. Waker gates hold an older failed
reload until a newer successful reload reaches Assets and LoadState::Loaded.
Releasing the older load changes LoadState to Failed while the newer asset remains
in Assets. The automatic fallback is held separately, then released and drained.
There are no timing sleeps in the loader or USD decoding dependencies.

This is a characterization of a Bevy 0.19.1 defect, not a passing repair acceptance
test. It deliberately asserts the broken state to make the upstream behavior
reproducible; a fix must replace that expectation with stale-completion rejection.
The initial targeted run passes in 0.01 seconds:
`/tmp/bevy-stale-reload-characterization.log`.

That characterization established the need for the serialization fix above.
Ignoring every failure while an asset remains in Assets would hide genuine
broken reloads; the fix instead orders reads and publication in AssetServer.

## Other confirmed open work

Save safety now includes source-resolver byte baselines on viewer opens and
`EditorSession::from_source`, a pre-publication content recheck, and no-clobber
publication for new files. Successful publication advances the baseline using
the staged output hash. Arbitrary `EditorSession::new` stages still lack loaded
disk-byte provenance. Existing-file writers can race the final check and rename;
atomic publication alone is not an atomic content compare-and-swap. Hard-link
identity and ownership/ACL preservation remain outside the current contract.
GPU preparation, broader render parity and intermittent native black frames
retain the qualifications in the existing support matrix.
