# Reusing resolved CPU skinning inputs

## Source finding and candidate

After the [tangent-cache checkpoint](tangent-reuse.md), unique-time skeletal
preparation remains dominant. `cpu_skin_sample` already resolves and validates
joint influences and remaps skeleton transforms into mesh joint order. It then
calls `SkinningQuery::skinned_points`, which reads/flattens the influences and
remaps the transforms again before calling the actual skinning function.

`c9006b3` routes non-rigid classic-linear points directly through the
same upstream `skin_points_lbs` function, using the already-resolved arrays.
It does not implement alternative skinning arithmetic, normalize weights,
change morph-before-skin ordering, retain new caches or modify the upstream
dependency. The dependency remains OpenUSD revision
`900ea31591348e3fbf416d4661d703aa8700771b`. Source evidence is in that revision's
`skel/skinning_query.rs::skin_elements` and
`skel/skinning/pose.rs::PrimPose::skin_points`.

Constant influences and dual-quaternion skinning retain the query path.
Unsupported methods and existing invalid-influence handling remain unchanged.
The normal sample still consumes the original authored influences and mesh-order
transforms. This removes duplicate input preparation, not the larger
time-varying tangent generation itself.

The new regression compares point **bits** with the upstream query across
vertex and constant influence fixtures and animated skin/morphs; classic-linear,
dual-quaternion and unsupported methods; original, reversed and missing-joint
orders; 41 unique times plus reverse/repeated clocks; unnormalized sampled
weights and a scaled/rotated/translated geometry-bind edit between passes.

Default workspace/all-target tests pass 886 tests (20 ignored), including the
new upstream-query comparison. Evidence:
`/tmp/bevy-optimization-cpu-inputs-tests-final.log`. The release benchmark builds
successfully: `/tmp/bevy-optimization-cpu-inputs-build.log`. A wait connection
reset during the test run; the original compiler/test process was followed to
completion without restarting it. The release build was started only after
the original process was absent and the complete test results were present.

Full hardening, performance comparison and rendered output checks follow below.
The full objective remains in [`OPTIMIZATION.md`](../OPTIMIZATION.md); native
frame pacing, the bird transparency discrepancy and cow MDL diagnostic remain
open. Nothing has been pushed.

## Unique-time comparison

Evidence: `/tmp/bevy-optimization-cpu-inputs-abba.bVA3xC/`. Saved release binaries
compare production `bc9963f` with `c9006b3`, in before/after/after/before order.
Each process opens a fresh App, retains four warmups separately, then measures
1,000 distinct clocks. Eager hidden-geometry residency and profiling settings
match. No task-owned compilation overlaps measurements; unrelated jobs remain
active, with one-minute load averages 7.96–44.02.

The first constant-influence control was rejected by the **before** binary
because its root layer provides no finite increasing timeline. That failed run
is retained. `/tmp/bevy-optimization-constant-timeline.usda` adds only explicit
root time bounds 0–10 over the unchanged constant-influence asset. The wrapper
and its hash are copied into the evidence directory. Resuming the remaining
cases yields 28 successful processes plus the one preserved control-input
failure, not a retrospectively all-green original matrix.

All 28 successful processes have matching final geometry/image payload,
retained mesh counts, peak mesh/material/image counts and all three retained
cache payloads within each case (`payload-check.txt`). Root entry hashes, the
267-file cow directory manifest and the wrapper hash verify afterward. These
are not complete dependency-closure or allocator audits.

| Workload | Before median update, ms | After median update, ms | Before process CPU, s | After process CPU, s |
| --- | ---: | ---: | ---: | ---: |
| Bird, CPU-only timeline | 27.961–38.084 | 27.263–37.267 | 33.45–41.52 | 32.74–41.63 |
| Bird, GPU-prepared timeline | 27.313–27.846 | 26.365–35.880 | 31.62–35.74 | 30.07–43.25 |
| Constant-influence CPU control | 0.333–0.421 | 0.205–0.313 | 0.40–0.49 | 0.31–0.39 |
| Cow, CPU-only timeline | 79.226–153.122 | 76.692–78.777 | 83.91–126.97 | 85.45–96.53 |
| Cow, GPU-prepared timeline | 77.057–78.552 | 77.859–78.854 | 82.02–82.78 | 81.25–82.82 |
| Blended morph, CPU-only timeline | 0.426–0.453 | 0.707–0.848 | 0.59–0.63 | 0.78–0.92 |
| Blended morph, GPU-prepared timeline | 0.435–0.456 | 0.432–0.493 | 0.57–0.64 | 0.57–0.67 |

Process CPU is GNU Time user-plus-system for the saved executable, including
opening, idle, warmups, seeks and shutdown. It is not seek-only time or displayed
frame cadence. `summary.tsv` and `cpu-cache.tsv` retain means, tails, warmups,
RSS/high-water RSS and cache counters; raw samples remain in each log.

Cow GPU tangent deformation falls from 2,329.060–2,355.943 ms to
2,062.447–2,098.694 ms cumulatively across 1,000 seeks. That is a smaller phase
inside total preparation, not a whole-frame improvement. Its median whole
update is essentially unchanged. Bird phase ranges overlap
(1,062.227–1,168.145 ms before, 767.911–1,163.816 ms after). The CPU-only morph
candidate is slower in this matrix, while even the unchanged constant-influence
path varies strongly. Host interference cannot be used to waive a regression;
further counter/repeat evidence is needed before drawing a general conclusion.

No additional persistent cache is introduced. Retained tangent payload remains
8,559,144 bytes for cow and 2,690,064 for bird in both variants. Whole-process
RSS is affected by paging: one bird candidate ends near 101 MB resident despite
approximately 716 MB of retained image payload and a 978 MB process high-water
mark. That low resident reading is not evidence of a memory optimization.

## Hardware-counter validation

The first counter trial in `/tmp/bevy-optimization-cpu-inputs-counters.u3u9P9/`
completes 24 processes, but its default-scaled hybrid-PMU counts are not accepted
as an aggregate instruction comparison. This host exposes separate `cpu_core`
and `cpu_atom` events; their running fractions change as work migrates. Summing
counts individually scaled to the whole enabled interval would overcount.
The attempted aggregation rejected the data rather than presenting that sum.

A revised probe uses `perf stat --no-scale`, preserving the raw counts for each
PMU and adding software task-clock accounting to audit running-time coverage.
The completed probe is `/tmp/bevy-optimization-cpu-inputs-raw-counters.n21LNa/`.
All 32 processes pass: two ABBA blocks per case, four before and four after.
Cow/bird use 100 repeated seeks; the small classic-linear and dual-quaternion
controls use 1,000 unique times. The DQ wrapper is retained in that directory.
No task-owned build overlaps the runs, no CPU affinity or host jobs are changed,
and load averages range 6.08–16.54. Each process's summed raw instruction/cycle
counter running time equals its software task-clock running time; no uncovered
PMU interval was accepted. Entry hashes verify afterward.

| Whole-executable user-space instructions | Before mean | After mean | Change |
| --- | ---: | ---: | ---: |
| Cow, repeated poses | 90,253,104,323 | 89,577,193,222 | -0.749% |
| Bird, repeated poses | 75,814,703,509 | 75,186,500,053 | -0.829% |
| Classic-linear morph, CPU timeline | 3,663,894,224 | 3,597,620,419 | -1.809% |
| Dual-quaternion morph, CPU timeline | 3,755,806,749 | 3,763,691,721 | +0.210% |

Counts include opening, idle, warmups, seeks and shutdown across inherited
threads, not just the modified function. The DQ query path has an additional
method check; its small counter increase is retained, not called a universal
improvement. Its median updates overlap (0.405–0.428 ms before versus
0.406–0.420 ms after). The earlier large CPU-only morph slowdown is not
reproduced: these medians overlap at 0.386–0.404 versus 0.379–0.406 ms, with
fewer instructions after. Neither that repeat nor lower instruction counts
establishes native frame cadence or waives the noisier first matrix.

## Regression and render checks

Default tests pass 886 tests (20 ignored). The first hardening run catches four
needless-borrow lints in the new test. `8e9001f` fixes only those test expressions;
the complete rerun passes 798 no-default tests (21 ignored), 892 all-feature/
all-target tests (29 ignored), strict Clippy, formatting and whitespace checks.
Evidence: `/tmp/bevy-optimization-cpu-inputs-harden-fixed.log`; the failed first
run is retained as `cpu-inputs-harden.log`. Four native deformation oracle tests,
nine watcher tests, 15 native export checks and wasm compilation pass too:
`/tmp/bevy-optimization-cpu-inputs-{test-native-deformation,test-watchers,test-native,check-web}.log`.
Wasm compilation is not browser runtime acceptance.

Saved-binary captures in `/tmp/bevy-optimization-cpu-inputs-captures.Ewo7ht/`
compare `bc9963f` with production `c9006b3`, using `UsdAssetPlugin`, RTX 4080
Vulkan/NVIDIA 595.112.07, 1280x720, forward rendering, 4x MSAA and WGPU validation.
Eight before/after comparisons are pixel-exact at RGB tolerance zero: GPU bird
at 0/5/10, cow at 5, combined skin/morph-normal fixture at 5, and CPU controls
for bird at 0, cow at 5 and the combined fixture at 5. All 16 captures have
compiled-pipeline/readback proof and expected deformation routes. Entry hashes
verify afterward; the after bird/cow images were inspected.

The cow still warns about unavailable `OmniPBR.mdl` in both modes/binaries, so
the combined capture gate exits 1. The strict current-binary bird CPU/GPU
comparison also still fails: 2,053 pixels exceed tolerance one, maximum RGB
error 33. The inspected diff again lies on grid lines through wing transparency.
These failures are unchanged, not resolved by before/after equality.

## Next work

The gain is incremental. Generating tangents and the broader CPU fallback work
remain much larger than the removed query preparation. Stage access is not
`Send`, and custom route order is significant. Any parallel preparation must
keep stage reads and ordered publication on the main thread, limit in-flight
owned mesh work, preserve custom-route and residency behavior, and prove exact
outputs plus native elapsed performance. Loading, rendering fidelity, editing,
lifecycle and browser/platform acceptance remain part of the full objective.
