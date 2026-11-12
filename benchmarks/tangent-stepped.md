# Individually paired full-editor updates

The balanced [owned-job diagnostic](tangent-paired.md) supports investigating the
withdrawn queue further, but excludes source reads, caches, publication and the
production task pool. The earlier [full-scene failure](tangent-queue.md) remains.
This experiment pairs individual full updates rather than long process blocks.

## Protocol and declared acceptance

Build the same `editor_benchmark` harness twice: restored production preparation
and exactly the withdrawn queue implementation. Preserve source diffs, binaries,
asset/lockfile hashes, build profiles and feature sets. The queue is experimental,
not a new default; restore production source before running saved executables.

`USD_BENCH_STEPPED=1` requires `ASSET 1 gpu-prepared seek-timeline`. Each process
opens one independent editor and emits `seek_step_ready iteration=N`, flushes
stdout, then accepts exactly `N\n` on stdin. Missing, malformed or out-of-order
commands fail instead of silently skipping measurements. Waiting and output are
outside the existing `app.update()` timer; existing readiness, document and clock
assertions still execute. Ordinary non-stepped measurements are unchanged.

Launch both versions in a fresh pair, then drive one update at a time in ABBA
order at the same clock, including four warmups and 1,000 distinct authored-range
clocks. Reverse both opening and starting update order in a second fresh pair.
Only one process updates at a time; the other waits on stdin. All meshes remain
eager, with the original cache policies, GPU eligibility and quality settings.
Record pool thread counts, host load/pressure, per-process peak RSS, all raw
durations and ordinary profile/payload summaries. No task-owned compilation may
overlap measurements. This is a headless CPU editor test, not native-window FPS.

For cow, require at least 15% lower paired whole-update median in each execution-
order stratum in both fresh pairs. Bird, robot arm and material subsets must have
no greater-than-5% paired median regression. Retain and separately evaluate p95,
maximum, opening, idle, mesh-read and preparation regressions, with the original
5% control threshold; pairing does not erase those gates. Require unchanged
geometry/image/cache/asset counts, bounded queue/input payload, zero diagnostic
failures, and no greater-than-5% peak RSS increase. Independently validate every
clock, order, count and percentile. Do not change thresholds after results.

A passing CPU comparison is not sufficient to enable the candidate: native
animation poses must still match independent pre-change renders at zero RGB
tolerance, and native deformation/watchers/export plus wasm checks must pass.
Existing grid, CPU/GPU, MDL, 60 Hz and whole-project workstreams remain open.

## Harness and identical-binary control

`b0ceb43` passes hardening: 853 no-default and 975 all-feature/all-target
executions, strict Clippy, formatting and whitespace checks. Three new tests
cover explicit configuration, flushed bounded commands, missing/wrong indices,
and a complete 1,000-clock editor comparison against the ordinary harness.
Both paths retain identical geometry/cache/asset counts and readiness assertions.

The controller first runs four subset processes using the same saved binary
under both labels. All processes and diagnostics pass; 2,000 paired updates
validate both starting/opening orders. Paired median ratios range 0.934–1.041,
within the declared control bound. The old whole-run maximum-time check still
fails (+16.8%), despite identical executables. This observation exposes timing
variability; it does not waive the maximum-time gate or authorize a rerun until
green. Full control evidence is retained in `control/`.

## Full-editor result: candidate remains withdrawn

Evidence directory: `/tmp/bevy-optimization-tangent-stepped.lUpGRT/`.
The experimental runtime is exactly the earlier `a4fa55d` scheduler, with the
current test-only paired-probe module retained. It passes hardening separately
(855 / 977 executions, Clippy and formatting). The runtime source is restored
byte-exactly before measurement; the normal benchmark executable is restored
to the baseline copy. No production scheduler change is committed or enabled.

All sixteen processes and diagnostic checks pass. Independent analyzers verify
16,000 measured updates, four warmups per process, 8,000 sequential pairs,
authored clocks, raw percentiles and both execution orders. Each editor uses
the production 16-thread compute pool, not the earlier private four-thread pool.
Geometry/image/cache payload counts, image capacity and peak asset counts match
exactly. Peak process RSS changes less than 0.2%; input budgets remain bounded.
These are count/payload checks, not new rendered comparisons.

| Scene / fresh pair | Legacy-first median ratio | Queue-first median ratio |
| --- | ---: | ---: |
| Cow 0 | 0.8786 | 0.8680 |
| Cow 1 | 0.8804 | 0.8659 |
| Bird 0 | 0.9975 | 0.9747 |
| Bird 1 | 0.9945 | 0.9731 |
| Arm 0 | 1.0821 | 0.9141 |
| Arm 1 | 1.0352 | 0.9495 |
| Subsets 0 | 1.0589 | 0.9560 |
| Subsets 1 | 1.0767 | 0.9504 |

The paired gate has seven failures: all four cow strata miss 15% improvement;
one arm and two subset strata exceed the 5% regression limit. Whole-run checks
also have seven failures: cow median target, cow/arm opening and idle, and
bird/subset maximum update times. Arm/subsets perform zero tangent jobs; their
order effects and the identical-binary control prevent attributing all variation
to the queue. Every failed observation remains recorded.

Cow whole-update medians fall from 55.313 / 55.844 ms to 48.328 / 48.885 ms,
an aggregate 12.5% reduction; preparation totals fall 15.8%. Bird medians improve
only 1.6%. Counts reproduce the earlier experiment: cow 13,986 jobs / 14 hits,
bird 40,914 jobs / 82 hits; barriers fall 4,000 to 2,000 and 11,000 to 6,000.
Default maximum buffered jobs would rise to eight, without meeting the primary
target. This does not establish 60 Hz or justify re-enabling the candidate.

No task-owned build overlaps timing; unrelated applications/builds are untouched.
Raw pressure/load records accompany every pair. Asset and dependency-directory
manifests, lockfile, saved executables and restored source hashes verify before
and after the run. `driver.mjs`, `analyze-pairs.mjs`, `analyze-blocks.mjs`, raw logs,
JSONL pairs, JSON analyses and the exact candidate diff are retained together.
The controller launches through `oslo make run-headless-benchmark`; each version
executes from the same path using atomically replaced inodes. Only its own child
process groups are eligible for cleanup on a controller error.

Saved executable SHA-256:

- Before: `6a3ef7cb1995bd583a9121aefbb2dd8d7eb34e6d9ab0da9664d72910f349e800`
- Candidate: `05958c7bc7c06ddf8639b701a13933f1af5b180ac00a24ca6ee2f7a2e415f408`

This candidate stops before new native captures or platform gates; those were
not run. Do not repeat this scheduler experiment unchanged. The next useful
direction is reducing unnecessary geometry work, including the existing
opt-in hidden-mesh path, while preserving eager library defaults and proving
current-clock materialization, reveal/edit/reload behavior and visible fidelity.
