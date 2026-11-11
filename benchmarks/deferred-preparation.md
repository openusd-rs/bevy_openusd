# Parallel preparation with deferred hidden geometry

## Contract and implementation

The library remains eager by default. `DeferHiddenMeshes` stays opt-in; the viewer
does not enable it in this change. The withdrawn larger tangent queue remains
withdrawn, with the original four-mesh default and eight-mesh hard batch cap.

Previously the deferral resource disables all parallel animation preparation,
including already resident visible meshes. Preparation now carries the target
entity with each route context. With deferral enabled, only entities already
carrying `Mesh3d` are eligible for ahead-of-route preparation. Initially hidden
and newly revealed meshes retain the ordinary synchronous route path, which
evaluates current visibility/purpose before decoding or promoting geometry.
Resident meshes remain retained and updated even when hidden again.

This avoids predicting visibility from stale ECS state or executing visibility
routes out of order. No source read, mesh allocation or tangent job is prepared
ahead for a nonresident entity. Shared residency budgets, custom registries,
missing/single-thread pools, zero limits and oversized input keep their existing
fallbacks. USD/World reads remain on the main thread; worker inputs remain owned.

## Acceptance declared before measurement

- Compare all mesh attributes, indices, morphs and CPU/GPU fallback state with
  serial projection across unique, reversed and repeated clocks. Mix hidden and
  resident siblings, edit while hidden, reveal at the current clock, and hide
  again while checking that retained meshes keep updating.
- Preserve animated ancestor visibility, purpose filtering, independent-root
  clocks, source reload, close/despawn, shared promotion budgets and custom routes.
  Prove nonresident hidden meshes have zero geometry decodes/preparation jobs.
- Run hardening, native deformation/watchers/export and wasm gates. Keep library
  defaults and viewer policy unchanged until separate visible/reveal acceptance.
- Use saved release binaries, the same opt-in deferral setting for both, and
  individually paired full-editor updates with reversed opening/update orders.
  Target at least 15% lower bird paired median in both execution-order strata
  and both fresh pairs, without >5% cow/arm/subset paired median regressions.
  Retain secondary timing failures; do not waive earlier failed observations.
  Require unchanged payload/cache/asset counts between opt-in versions, bounded
  jobs/input bytes, zero diagnostics, and no >5% peak RSS increase.
- Compare actual native animation final poses against independent pre-change
  renders at matching clocks and zero RGB tolerance. This does not clear older
  grid/CPU-GPU/MDL failures or establish native-window/browser FPS. The full
  `OPTIMIZATION.md` workstreams and 60 Hz target remain open.

## Correctness checkpoint

Hardening passes 854 no-default and 976 all-feature/all-target executions,
strict Clippy, formatting and whitespace checks. The new CPU/GPU regression
mixes six hidden and five resident siblings, edits orientation while hidden,
reveals at a changed clock, reverses/repeats clocks and hides retained meshes
again. Full mesh/fallback state matches serial projection. Before reveal only
five meshes decode; after promotion all eleven stay current. Only meshes already
resident before the update enter preparation, with the original job/byte limits.
Adding a residency budget to resident meshes restores the serial fallback.

Independent-root clock/reload/close coverage now runs both eager and deferred
resource configurations for CPU and GPU projection. Existing ancestor-visibility,
purpose, shared-budget, source-reload and custom-route tests remain intact.
This establishes correctness coverage, not a measured speedup or rendered pass.
Evidence: `/tmp/bevy-optimization-deferred-preparation.mJketj/harden.log`.

## CPU comparison: incremental performance target not met

`d5ad5ab` passes four native deformation, nine filesystem-watcher and fifteen
export/reference checks plus wasm compilation. Saved release benchmark and
capture executables build successfully. All sixteen benchmark processes and
runtime diagnostic checks pass; 8,000 pairs / 16,000 updates, authored clocks,
orders and percentile summaries independently validate. Both versions use the
same opt-in deferral setting, unchanged quality and production compute pool.

Payload/cache/image-capacity and peak asset counts match. Peak RSS changes range
from -0.64% to +2.17%, within the 5% limit; worker input and original four-job
bounds hold. Cow submits one resident
job per changed clock (999 jobs / one hit), rather than preparing hidden meshes.
Bird prepares 40,914 jobs / 82 hits; its legacy deferral path is serial.

| Scene / fresh pair | Legacy-first median ratio | Candidate-first median ratio |
| --- | ---: | ---: |
| Cow 0 | 1.0139 | 0.9862 |
| Cow 1 | 1.0104 | 0.9933 |
| Bird 0 | 0.9086 | 0.8885 |
| Bird 1 | 0.9845 | 0.9030 |
| Arm 0 | 1.0396 | 0.9583 |
| Arm 1 | 1.0732 | 0.9423 |
| Subsets 0 | 1.1011 | 0.9229 |
| Subsets 1 | 0.9836 | 0.7796 |

Six paired checks fail: all four bird strata miss the 15% target, one arm and
one subset stratum regress more than 5%. Seven whole-run checks also fail:
bird target/idle/mesh-read time, cow maximum, arm idle, subset idle/maximum.
Bird medians are 25.126 / 50.072 ms before and 22.583 / 47.959 ms after; cow
medians stay approximately 17.7–18.5 ms. Host load rises from 5.03 to 61.64;
no task-owned build overlaps and unrelated workloads remain untouched.

This is not performance acceptance, and the thresholds remain unchanged.
The change remains an unaccepted local checkpoint in the existing opt-in path;
library/viewer defaults stay eager. Do not repeat this matrix unchanged.
Native visible/reveal correctness remains useful for evaluating a separate,
broader viewer deferral policy; it cannot retroactively pass this failed target.
The next capture compares candidate playback with both deferred and eager
pre-change fixed renders at the exact final clocks and zero RGB tolerance.

## Native visible-fidelity checkpoint

Four candidate playbacks complete 300 measured frames plus 30 warmups each on
RTX 4080/Vulkan, forward rendering, 4x MSAA and 1280x720. All 1,200 raw frame
rows, clocks and summaries validate; maximum clock-equation error is 5.685e-14.
For each scene, an independent pre-change process renders the exact final pose
once with deferral and once with eager defaults. All eight RGB comparisons are
pixel-exact at tolerance zero, with matching camera, quality and visible-mesh
metadata. The four candidate images were inspected.

All twelve processes and renderer-log checks pass. The combined diagnostic gate
still fails on the existing missing cow `OmniPBR.mdl` dependency warning in all
three cow runs. This warning is neither suppressed nor treated as a passed gate.
No task-owned build overlaps captures; load varies from 4.69 to 73.24. Bird/cow
main-update medians are 68.530 / 40.999 ms and offscreen intervals 70.988 / 42.972
ms, still above the 60 Hz target. These are candidate-only observations, not
before/after timing gains or native-window cadence measurements.

Evidence is in `captures/`: twelve process logs, four animated raw reports,
eight comparison logs, PNG/RGBA outputs, `results.tsv`, `raw-analysis.json` and
asset/executable manifests. The capture harness source matches the pre-change
renderer; intervening baseline commits only changed tests and CPU benchmarks.
Saved capture SHA-256:

- Before: `d9d57b9ff1e4cf44fce345846c734a64a15ff97b0053c0d9b1e84bf196f03ce8`
- Candidate: `2e7abd1b4908dcbb9594d7abebbbe3d79ef66aa113a52cb7d360cf0de2f06c8d`

This establishes final-pose visible fidelity for these runs, not live viewer
reveal/selection/undo acceptance. A viewer policy change still needs real editor
reveal/edit/reload/selection checks and a paired comparison against current eager
defaults, rather than only the old opt-in serial path. That larger policy change
has not been implemented or accepted; the failed incremental target above stays
failed. Existing grid, CPU/GPU, missing-asset, lifecycle, platform and whole-project
performance gaps remain open.
