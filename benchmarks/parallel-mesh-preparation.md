# Bounded parallel mesh preparation

## Scope and status

Work in progress after `7474920`. This targets the larger tangent-generation
cost left by the CPU-input reuse checkpoint, not another repeated-pose cache
optimization. No elapsed-speedup or native-frame-rate result is accepted yet.
The full objective remains in `OPTIMIZATION.md`.

## Execution contract

Only built-in animation batches use preparation. Public single-prim projection
and patch calls stay synchronous. Custom registries, missing/single-threaded
compute pools, zero limits, deferred-hidden residency and residency budgets use
the existing serial route path. Initial scene projection is unchanged.

The main thread reads USD and assembles owned deformed meshes for up to eight
prim contexts at a time. Default limits admit four tangent jobs and 16 MiB of
worker input mesh payload per batch. This is not a total-memory bound: decoded
USD arrays, deformed reads, tangent outputs and MikkTSpace scratch are additional.
An oversized input stays in its context for serial generation without repeating
its deformation or assembly. No worker sees a stage, World or asset handle.

Cache lookup and recurrence admission stay on the main thread. Misses run the
same Bevy MikkTSpace function on separate meshes in a scoped compute-pool call.
All tasks join before publication; results finish in input order. Routes then
run in their original prim and registry order, including material, skin, subset
and instancing consumers. A batch may observe two misses for equal inputs before
either result is cached; this does not bypass exact input equality or the cache
byte budget, but cache accounting/performance requires measurement.

Prepared GPU samples, failures and deformed meshes are consumed only by their
own context. They are not retained across clocks or independently timed roots.
Both editor animation and `UsdAssetPlugin` instance-clock updates use batching.
GPU morph-only and subdivided paths retain their existing preparation. Missing
or invalid deformation keeps the existing fallback/error behavior.

`MeshPreparationTiming` records complete prepass/dispatch/join wall time, job
counts, cache hits and maximum worker input payload. `editor_benchmark` reports
and resets it alongside route timings. Moving work out of SkinRoute does not
make it disappear from the reported update interval; compare whole updates and
the separate batch time, not reduced per-route times alone.

## Required evidence

- Byte-exact serial/batched geometry, including morph attributes, across unique,
  reverse and repeated clocks; UV, normal, orientation, method and influence edits.
- Actual two-thread executor coverage; missing/single-thread/zero/oversized and
  residency fallbacks; custom routes retaining synchronous stage visibility.
- Independent production-plugin roots, subsets, source reload and close cleanup.
- Repository hardening, native oracle/watchers/export and wasm checks.
- Alternating saved release binaries on bird, cow, CPU/GPU deformation and small
  controls, including unique clocks, retained payload and peak RSS. No task-owned
  compilation during comparisons; retain host-load observations and raw samples.
- Before/after native renders and actual interactive cadence. The existing cow
  MDL diagnostic and strict bird CPU/GPU mismatch remain unresolved gates.

## Initial verification

`0035a9e` splits cache lookup/admission from result insertion without changing
the serial generator. `7483e05` adds batch preparation and three regression
tests. `d740239` reports its complete batch cost in the benchmark. The initial
workspace/all-target run passes 889 tests with 20 ignored:
`/tmp/bevy-optimization-parallel-tests.log`.

Further production-plugin coverage is still being verified. Its first run
fails because the new test used `UsdSource::snapshot` for a wrapper referencing
filesystem fixtures: snapshots deliberately disallow unresolved filesystem
dependencies. The serial control fails before any parallel comparison. The
test now uses the filesystem-backed constructor and an explicit unlimited
initial projection budget; this is not a change to production loading policy.
The original failure is retained in
`/tmp/bevy-optimization-parallel-harden.log`.

The next run passes the new lifecycle test but exposes an existing geometry
fixture assuming both roots finish within one shared 10 ms loading turn. The
missing `/PI` is in a still-projecting serial-control root; initial projection
does not use the new batching path. `c2e3142` gives that geometry-only test an
explicit unlimited budget and asserts both roots are Ready before inspecting
them. It does not change production budgets. The failed run is retained in
`/tmp/bevy-optimization-parallel-harden-fixed.log`.

`51da2f4` completes hidden-residency and production-instance tests. The final
hardening rerun passes 803 no-default tests (21 ignored), 897 all-feature/
all-target tests (29 ignored), strict Clippy, formatting and whitespace:
`/tmp/bevy-optimization-parallel-harden-recheck.log`. Coverage includes CPU and
GPU routes, live/reversed root clocks, referenced geometry, subsets, source
replacement, closing one root without affecting another, and no further jobs
after both roots close. Native oracle/watchers/export, wasm and new captures
still need rerunning for this production change.

## Release experiment target

Before running the first comparison, the focused performance target is at least
15% lower cow GPU unique-clock median update cost, with no repeatable greater
than 5% regression on bird or small deformation controls. This is a checkpoint
target, not a replacement for the native 60 Hz objective. Lower per-route time
alone does not count: use complete update samples, account for the batch wall
time and inspect process CPU, retained geometry and peak RSS. Exact outputs and
bounded worker inputs remain required regardless of elapsed results. Concurrent
host load must be retained; no task-owned compilation may overlap comparisons.
