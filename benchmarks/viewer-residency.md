# Viewer residency policy investigation

## Scope and boundaries

Compare the current eager policy with the existing hidden-mesh deferral policy,
both using the original parallel scheduler and resident-only preparation from
`d5ad5ab`. This is a broader policy comparison, not another run of the failed
incremental serial-versus-parallel deferral experiment. Its failed 15% bird target
remains failed in [deferred preparation](deferred-preparation.md).

Library and viewer defaults remain unchanged during this investigation. A viewer
policy must preserve authored visible fidelity, logical hierarchy and selection,
current-clock reveal, editing/history, dirty-layer protection, last-good reload
and replacement cleanup. Initially hidden meshes may remain unprepared, but
resident meshes must remain retained and current after re-hide. No LOD switch,
weight normalization, quality reduction or larger tangent queue is introduced.

## Editor command verification

`52f1fbc` exercises two live editor apps through `EditorBridge` with eager and
deferred resources, for CPU and GPU projection. It selects and edits a hidden
skinned mesh without materializing it, seeks, reveals at the current clock,
undoes/redoes visibility, and compares mesh bytes, morph data, palettes and
fallback state against the eager app. Reversed/repeated clocks preserve geometry.
Dirty-source reload is rejected without losing edits or entities; undo permits
reload, malformed input preserves the last good scene, repair preserves document
and selection, and opening a replacement removes the old meshes/deferred markers
and GPU skins. Hardening passes 855 / 977 executions plus strict Clippy/fmt.
These are real command-path tests, not native viewport reveal readbacks.

The paired-policy probe passes hardening with 855 no-default and 978 all-feature
executions plus strict Clippy/fmt. Its regression runs both opening orders and
CPU/GPU modes, including generated material subsets; missing visible geometry
and same-sized position corruption are rejected. The composed-source example
now waits for both roots to become ready under the normal shared projection
budget and a zero-budget, one-prim-per-update regression (`cf382de`). Its original
single-update failure and the probe's corrected subset-count assertion remain in
`/tmp/bevy-optimization-viewer-residency.Uqos3X/probe-harden-ready.log` and
`probe-harden-fixed.log`; the passing run is `probe-harden-subsets.log`.

## Paired policy experiment declared before measurement

Use one saved release `editor_benchmark` executable with
`ASSET 1 gpu-prepared seek-residency-paired`. Two independent editor apps share
one process/production compute pool. They differ only in initial mesh deferral;
both retain default four-job preparation. After four warmups, alternate individual
updates at 1,000 distinct authored-range clocks in ABBA order. A second fresh
process with `USD_BENCH_PAIR_REVERSE=1` reverses opening and starting update order.
External deferral/budget/prewarm overrides are rejected.

Outside both timers, compare logical paths, local transforms and visibility,
all hierarchy-visible mesh/subset identities (not camera/frustum culling),
topology/attribute/morph bytes,
skin palettes, morph weights and fallback errors at every clock. Reject unmapped
visible meshes rather than claiming partial coverage. This geometry comparison
does not replace native material/render validation. Retained hidden payloads are
expected to differ; report their counts rather than require false equality.
RSS is combined for the two apps, not a per-policy saving or whole-viewer claim.

Prospective primary target: at least 50% lower cow paired update median in both
execution-order strata and both fresh processes, with no >5% bird/arm/subset
paired median regression. Independently verify all clocks, orders, geometry
checks, raw timings and summaries. Record p95/max, opening/idle, combined RSS,
asset/cache bounds and diagnostic failures without silently dropping outliers.
No task-owned build may overlap timing; unrelated host work is left alone.

A passing CPU policy signal cannot change viewer defaults by itself. Native
reveal/selection/undo behavior, unchanged framing and visible material quality,
reload/resource safety and the full remaining performance/platform gates still
need evidence. Prior grid, CPU/GPU, MDL, lifecycle and 60 Hz gaps remain open.
