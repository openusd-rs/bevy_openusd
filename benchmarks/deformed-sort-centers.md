# Posed sorting centers

## Problem and constraints

The labelled hummingbird phase diagnostic records different left-wing/grid draw
order between CPU and GPU deformation at time zero. Both grid settings are
identical; the generated wing subsets use posed mesh positions on CPU but rest
mesh positions for the GPU render asset's sorting center. See
[the observed phase keys](hummingbird-playback.md#observed-transparent-phase-divergence).

The correction keeps the existing grid, material depth bias, sorting mode and
geometry. It does not change comparison tolerances, shared mesh AABBs or position
compression metadata. Independent roots and material-subset children require
independent centers even when they share a mesh asset.

## Implementation

`route/deformed_sort.rs` caches one optional world-space center per owned GPU
deformation draw. It follows the CPU renderer's convention: enclose all posed
vertices in mesh-local space, then transform that AABB center to world space.
This is not generally the same as the center of a world-space AABB. Morphs are
applied before skinning, including referenced morph weights and inverse bind
poses. Existing camera/grid bounds continue enclosing drawn vertices only.

The main-world Last system runs after palette/global propagation and asset-event
publication. It rebuilds on mesh/transform/skin/morph changes, referenced weight
or joint changes, and mesh/inverse-bind asset events. Idle frames reuse the
cached center. Invalid or unavailable data yields no override. Cache ownership
follows the existing skin/morph claims and is released with the last claim;
cache entries retain no asset handles. Extraction replaces the per-entity map
each frame and shrinks excess capacity after scene removal.

Before render queuing, original retained phase keys from the previous frame are
restored. After queuing and before sorting, only marked USD draws receive their
posed center in transparent/transmissive phases. Restoring the original key
prevents a removed or invalid override from leaving stale posed data in retained
phase items. Material bias, always-on-top mode and unrelated draws remain intact.
The systems are registered by the GPU skinning plugin only with a RenderApp,
from both build and finish with duplicate registration guarded. This also
covers a renderer installed after the skinning plugin. Headless scene-update
comparisons do not measure this render-path cost.

`DeformedSortTiming` optionally records main-world dependency-check/update time,
center rebuilds and vertices visited. Capture metadata includes these totals;
they are not a native frame-cadence measurement.

## Required evidence

- Unit coverage for shared meshes with independent weights, local-vs-world AABB
  centers, joint and inverse-bind edits, missing inputs, idle reuse, last-owner
  cleanup, and retained transparent/transmissive key restoration.
- Original hummingbird CPU/GPU captures at 0/5/10 with RGB tolerance one and the
  grid visible; retain any residual discrepancy rather than weakening the gate.
- Independent timed roots with reversed clocks, including subset geometry.
- CPU controls unchanged, production plugin registration, strict hardening,
  native deformation/watcher/export checks and wasm compilation.
- Render-path cache/rebuild cost and native playback cadence. Existing headless
  paired timing improvements do not prove this new path's runtime cost.

## Initial checkpoint

Default workspace/all-target tests pass: 900 passed, 20 ignored. The time-zero
bird capture in `/tmp/bevy-optimization-posed-sort-captures.rK9qoF/` passes the
original RGB tolerance of one: zero failing pixels, maximum RGB error one,
versus 2,053 failing pixels before the correction. The CPU control remains
pixel-identical to its pre-change capture at tolerance zero. Both images have
compiled-pipeline/readback proof and clean renderer log checks; the source hash
verifies. The bird image and difference image were inspected with the grid and
transparent wing blur still visible.

Actual GPU phase order now matches CPU: right wing, left wing, grid. Left-wing
distance is -11.429891 and the grid remains -10.7276535; the grid transform,
spacing, fade and sort key are unchanged. No material or vertex data is altered
by the sorting override. Metadata records 40 center rebuilds / 5,292 vertices
across 798 main-world updates, with 10.103 ms accumulated update work. Many
updates precede loading, so dividing that total into an apparent frame rate
would be misleading. The idle-cache regression separately verifies no rebuilds
without dependency changes.

This initial checkpoint fixes the observed time-zero discrepancy, not every
outstanding rendering/performance requirement. Subsequent clock, lifecycle and
independent-root checks follow below; render-path cadence remains unverified.

### Remaining real-asset discrepancy

The same saved renderer completes times 5 and 10 under
`/tmp/bevy-optimization-posed-sort-captures.X4PyGC/`. Actual CPU/GPU transparent
phase order now agrees at both clocks. Both capture/log/pipeline checks pass and
the asset hash verifies, but the unchanged strict comparison still fails:

| Time | Failing pixels | Coordinate | Max RGB error | GPU RGB | CPU RGB |
| --- | --- | --- | --- | --- | --- |
| 5 | 1 / 921,600 | (1150, 489) | 3 | (183, 151, 128) | (186, 153, 129) |
| 10 | 1 / 921,600 | (799, 604) | 19 | (102, 110, 101) | (121, 129, 117) |

These are the same remaining single-pixel failures recorded before the sorting
correction, separate from the repaired time-zero grid/wing overlap. Difference
images were inspected; do not call the complete 0/5/10 gate green or infer a
floating-point/coverage cause without additional evidence. Raster/coverage and
deformation precision are follow-up diagnostics, not a reason to waive tolerance.
These correctness captures used the immutable initial candidate while subsequent
test/registration hardening compiled; their elapsed timings are not performance
comparison evidence.

## Hardened regression checkpoint

Production commits `fe32713` and `291b4af`, capture refactoring `7aa6ba3`, and
regressions `53a34e6` pass `oslo make harden`: 809 no-default tests with 21
ignored, 908 all-feature/all-target tests with 29 ignored, strict Clippy,
formatting and whitespace checks. These are overlapping configurations, not
1,717 distinct test cases. Log:
`/tmp/bevy-optimization-posed-sort-harden-fixed.log`.

Tests exercise independent direct/referenced morph weights on shared assets,
mesh removal/reinsertion and in-place edits, missing joints and weights,
inverse-bind edits, singular transforms and recovery, local-vs-world center
conventions, retained transparent/transmissive keys and last-owner cleanup.
A real asset-plugin fixture compares CPU and GPU centers for independently
timed roots and generated subsets across clock swaps, source reload and closing
one/all roots. Fixtures assert Ready before sampling. A separate registration
test covers a late RenderApp without treating that unit test as GPU proof.

The subsequent repository-native gates pass:

| Recipe | Result |
| --- | --- |
| `test-native-deformation` | 4 native position/normal oracle checks |
| `test-watchers` | 5 asset + 4 editor texture checks |
| `test-native` | 14 export checks + 1 reference custom-data round trip |
| `check-web` | wasm compilation |
| `build-example --name viewer_capture` | final renderer built successfully |

Logs are `/tmp/bevy-optimization-posed-sort-{test-native-deformation,test-watchers,test-native,check-web,final-build}.log`;
exit statuses are in `/tmp/bevy-optimization-posed-sort-gates.tsv`. Wasm
compilation is not a browser-rendering or other-platform performance claim.

## Independent roots and reversed clocks

`/tmp/bevy-optimization-posed-sort-instances.eZBVWF/` contains six captures from
the final renderer through `53a34e6`. Each case opens three roots at times
0/5/10, reverses them after 30 ready frames, and records final times 10/5/0.
Compiled pipelines, successful readback, GPU/CPU path selection and clock
reversal are checked. All three entry hashes and 267 cow dependency hashes
verify. The CPU controls are pixel-exact against their saved pre-fix images.

| Scene | CPU/GPU pixels over RGB tolerance 1 | Max RGB error | CPU before/after tolerance 0 |
| --- | ---: | ---: | --- |
| Hummingbird | 1 / 921,600 | 3 | exact |
| Animated skin + morph fixture | 0 | 0 | exact |
| Cow | 0 | 1 | exact |

The bird retains its prior one-pixel strict failure. Both cow captures also
retain the unavailable `Cow_FBX_PNG/Cow_F/_converted/OmniPBR.mdl` dependency
warning; the combined matrix exits nonzero rather than hiding either limitation.
GPU images were visually inspected: all three independently posed roots are
visible, with the bird's nearest wing extending beyond the fixed framing as in
the prior control. These are not all-angle or smooth-playback acceptance.

The optional posed-center counters record 200 bird rebuilds / 26,460 vertices,
10 morph rebuilds / 30 vertices and 60 cow rebuilds / 306,720 vertices. Those
totals include opening and clock reversal, not continuous animated frames. They
do not establish the cost of the new path during sustained native playback.
