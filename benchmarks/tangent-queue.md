# Bounded tangent job queues

## Evidence and execution contract

At `181239d`, the cow spends 74.0% of preparation wall time in tangent
dispatch/join. Fourteen jobs are separated by four barriers per changed update.
The caller waits after at most four mesh prims, even though the existing hard
batch cap is eight and the worker input budget is 16 MiB.

The candidate separates buffered jobs from concurrent workers. A batch admits
at most twice `max_jobs`, capped at eight mesh prims and the unchanged input-byte
budget. Default concurrency remains four: jobs are assigned largest-first to
the least-loaded worker lane by vertex count. Each lane executes sequentially;
the heaviest lane runs on the calling thread. All work joins before results are
published in original input order. No stage or World crosses worker threads.

`MeshPreparationTiming::peak_jobs` continues to count buffered jobs in one batch,
not simultaneously executing generators. Thus its default maximum increases
from four to eight. There is no new thread pool, public configuration field,
retained cross-frame queue, fidelity reduction or change to MikkTSpace math.
The byte budget still excludes decoded USD arrays, prepared reads, tangent
outputs, allocator overhead and worker scratch. The larger queue can retain
more of that transient storage; peak RSS must be measured, not inferred.

Serial, missing/single-threaded pool, custom registry, residency and oversized
input fallbacks remain. Cache policies and bounds remain; a larger batch may
encounter additional duplicate misses before publication, so identical cache
hit counts or occupancy are not presumed.

## Acceptance declared before implementation

- Exercise actual private multi-thread pools and assert generator concurrency
  never exceeds the configured limit; verify zero/single/large limits, bounded
  queues, input bytes and complete ordered publication.
- Compare full CPU/GPU geometry, morphs, errors and fallback state against
  uncached serial projection across unique/reverse/repeated clocks and edits.
  Preserve independent-root reload/close, custom-route and residency checks.
- Run hardening, native deformation/watchers/export and wasm checks.
- Use saved release before/after binaries in common-path fresh-process ABBA
  comparisons for cow, bird, arm and subsets, with 1,000 authored-range clocks.
  Incremental target: at least 15% lower cow whole-update median, no greater-
  than-5% timing regression in controls, unchanged geometry/image/asset payloads,
  bounded cache retention and no greater-than-5% peak RSS increase. Retain every
  failed observation. No task-owned compilation overlaps timing runs.
- Compare actual native animation final poses with independent pre-change
  renders at the same clocks and zero RGB tolerance. Keep existing grid,
  CPU/GPU and cow MDL failures open. This is not native-window/browser acceptance
  and does not replace the 60 Hz target or the full `OPTIMIZATION.md` checklist.

Saved baselines and subsequent evidence live in
`/tmp/bevy-optimization-tangent-queue.D7gWIV/`.

## Correctness checkpoint

Hardening passes 854 no-default and 973 all-feature/all-target test executions,
strict Clippy, formatting and whitespace checks. The two new tests cover load
balancing and actual scoped generators with active-job concurrency counters;
the expanded sparse-batch test verifies two/four/eight worker limits and buffered
job counts. Existing full geometry, independent-root reload/close, custom-route,
residency and oversize fallbacks still pass. Configurations overlap.

`harden-fixed.log` is authoritative. The first compile exposed a missing
`'static` bound on the executor's owned result type; `harden.log` retains that
failure. No performance or native-render acceptance follows from these tests.
