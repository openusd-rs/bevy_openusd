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

## CPU policy result: primary comparison passes

`7184abe` builds the saved release probe. Eight fresh processes complete on the
i7-13700KF using the production 16-thread compute pool: 8,000 measured pairs /
16,000 full editor updates, plus four warmup pairs per process. All requested
clocks are distinct within each measured run; independent analysis validates
clocks, ABBA order, both opening orders, raw timing summaries and all 8,000
measured visible-geometry comparisons. Runtime/diagnostic exit statuses are zero.
Executable, lockfile, primary assets and cow/robot dependency manifests verify
before/after. No task-owned compilation overlaps timing; unrelated host work
is untouched. Observed one-minute host load ranges from 8.73 to 14.09.

The declared cow median-ratio target of at most 0.50 passes in all four strata:
deferred updates require approximately 32% of eager update time. All control
strata remain within the declared 1.05 ratio, including the slower arm-first
ordering. The previous incremental preparation experiment is still failed;
this result compares deferral with eager loading, not two deferred schedulers.

| Scene / fresh process | Eager-first median ratio | Deferred-first median ratio |
| --- | ---: | ---: |
| Cow 0 | 0.319119 | 0.316126 |
| Cow 1 | 0.321567 | 0.316918 |
| Bird 0 | 1.004326 | 0.997587 |
| Bird 1 | 1.009107 | 0.997019 |
| Arm 0 | 0.964991 | 1.041354 |
| Arm 1 | 0.961869 | 1.044101 |
| Subsets 0 | 0.980024 | 1.026263 |
| Subsets 1 | 0.980472 | 1.033502 |

Whole-run observations below are milliseconds, eager / deferred. Opening and
idle are recorded observations, not individually paired speedup claims. The
primary pass does not hide secondary regressions: cow-1 idle rises, bird maxima
rise, subsets-0 p95 rises, and subsets-1 opening is slower. Raw samples and all
outliers remain in the evidence; no claim of uniformly faster behavior is made.

| Process | Update median | Update p95 | Update max | Open | Idle |
| --- | ---: | ---: | ---: | ---: | ---: |
| cow-0 | 57.029 / 18.132 | 94.520 / 30.139 | 143.426 / 49.567 | 1545.797 / 1295.185 | 0.0474 / 0.0403 |
| cow-1 | 56.446 / 18.087 | 101.649 / 32.450 | 161.998 / 78.413 | 1346.861 / 1273.862 | 0.0369 / 0.0682 |
| bird-0 | 22.707 / 22.758 | 26.299 / 26.220 | 34.807 / 36.824 | 2690.017 / 2665.009 | 0.0556 / 0.0538 |
| bird-1 | 22.779 / 22.829 | 27.010 / 27.027 | 39.063 / 40.164 | 2619.301 / 2647.985 | 0.0475 / 0.0475 |
| arm-0 | 0.546 / 0.549 | 0.741 / 0.766 | 2.602 / 2.663 | 70.358 / 45.220 | 0.0387 / 0.0388 |
| arm-1 | 0.549 / 0.553 | 0.875 / 0.907 | 2.665 / 2.638 | 51.604 / 46.739 | 0.0439 / 0.0360 |
| subsets-0 | 0.176 / 0.176 | 0.291 / 0.323 | 0.451 / 0.446 | 3.449 / 1.097 | 0.0361 / 0.0346 |
| subsets-1 | 0.170 / 0.171 | 0.240 / 0.244 | 0.499 / 0.481 | 1.040 / 3.475 | 0.0430 / 0.0450 |

Every measured clock compares 1 hierarchy-visible cow mesh, 43 bird meshes /
subsets, 29 arm meshes / subsets and 3 subset-fixture render entities. Cow's
resident `Mesh3d` count falls from 14 to 1 and mesh assets from 42 to 4; arm
counts fall from 71 to 32 and assets from 32 to 25. Bird and subset-fixture
payload/cache/peak asset counts match between policies. Hidden geometry is not
required to have equal retained payload: cow vertex/index payload falls from
19,205,232 to 3,161,344 bytes. This excludes GPU allocation, allocator overhead
and other assets. Combined two-app process RSS cannot establish per-policy
memory savings.

Cow preparation jobs fall from 13,986 to 999 over 1,000 measured clocks; bird
retains 40,914 jobs and 82 cache hits in both lanes. Preparation stays within
four jobs / 16 MiB inputs; projection, tangent and assembly payloads stay within
256 / 16 / 128 MiB budgets, and projection entry counts stay below 8,192.
All retained counts match across the two opening orders for each policy.

Evidence directory: `/tmp/bevy-optimization-viewer-residency.Uqos3X/`, including
`driver.sh`, `analyze.mjs`, eight process logs, `analysis.json`, `analysis.log`,
`results.tsv`, load/pressure observations and verified input manifests. Probe
SHA-256: `3066549a398e2e6c4800b460ad71790da0c01e482fbb8d5c148613979f4ba2b4`.

This is a passing CPU policy comparison, not native-window FPS or final policy
acceptance. Cow and bird CPU medians alone still exceed a 60 Hz frame budget.
Library and viewer defaults remain eager. Next: exercise owned native viewport
captures for hidden selection, current-clock reveal, undo/redo and unchanged
framing/materials against eager controls. Do not rerun this passed CPU matrix
unchanged; preserve the secondary observations and all earlier open gates.
