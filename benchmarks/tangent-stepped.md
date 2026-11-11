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
