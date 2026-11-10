# Library and viewer optimization

## Objective and boundaries

Optimize the whole `bevy_openusd` library/viewer pipeline, not only hummingbird
playback. Preserve the current upstream OpenUSD dependency, public behavior,
authored geometry/material fidelity, edit/reload/save safety, instance isolation,
resource lifetimes and supported platform paths. No reduced-quality defaults or
disabled validation presented as algorithmic speedups.

Work remains local, in focused unsigned conventional commits with title-only
messages. Do not push. Do not control the user's desktop or window manager;
interactive acceptance uses the user's input and application capture controls.
Leave unrelated applications/builds alone. Use `.make.lua` through `oslo make`.

The user accepted the hummingbird freeze repair (animation and window respond).
That does not prove smooth frame pacing. See
[the investigation](benchmarks/hummingbird-playback.md) for measured update costs
and the still-failing strict real-asset CPU/GPU image comparison.

## Workstreams and completion evidence

- [ ] **Reproducible baselines.** Record revision, asset hash, build profile,
  feature set, device, resolution, quality settings, warmup/sample counts and
  concurrent load. Separate CPU scene-update time, total/native frame cadence,
  GPU passes, first-open latency, retained payload and whole-process RSS. Retain
  raw samples, not only averages. Historical Bevy 0.19 measurements are routing
  aids, not current baselines.
- [ ] **Load and scene construction.** Profile source/package IO and composition,
  texture decode/transform/mips, mesh/subset construction, instance projection,
  readiness and editor publication. Optimize the dominant measured work without
  changing fidelity or hiding blocking work outside the reported interval.
- [ ] **Animation and seeks.** Profile skin/morph queries, palettes, topology,
  normals/tangents, materials and Bevy asset/entity updates. Separate invariant
  preparation from time-varying data with tested invalidation. Exercise forward,
  reverse, repeated and unique clocks and multiple independently timed instances.
- [ ] **Rendering.** Measure actual native and GPU cost, including uploads,
  shadows/transparency, deformation and instancing/culling. Preserve visual
  correctness and resolve or explicitly retain failing acceptance evidence;
  a compile-only check is not renderer acceptance.
- [ ] **Editor and reload.** Measure idle, snapshot/inspection publication,
  selection/hierarchy work, representative edits, undo/redo and dependency reload.
  Preserve dirty-layer protection, last-good recovery and unrelated state.
- [ ] **Memory and lifecycle.** Measure representative retained payload and peak
  RSS; stress unique clocks, repeated opens/closes and reloads. Bound caches,
  retain useful sharing, release obsolete assets and preserve mutation tracking.
- [ ] **Build/platform behavior.** Measure release rather than extrapolating from
  debug. Check developer-profile bottlenecks separately. Revalidate wasm and its
  browser rendering path; distinguish browser software rendering from native GPU
  results. Do not claim untested OS/device performance.
- [ ] **Final regression and performance audit.** Run repository correctness,
  native oracle/reload/export, GPU capture, browser and lifecycle gates covering
  changed behavior. Repeat before/after performance runs under comparable
  conditions, account for noise and confirm no important workload regresses.
  Record remaining limitations against this full list, not just completed edits.

Working playback target: 60 Hz for the representative small animated release
scenes on the available machine. This is a target, not a current result or a
guarantee for arbitrary USD scenes. Set measurable workload-specific budgets
from the baseline before each optimization; do not redefine them afterward to
fit the result. Opening large scenes must also retain responsive progress and
correct readiness reporting.

## Representative matrix

- Hummingbird: packaged textures, many skinned meshes, transparency, live playback.
- Cow and robot arm: locate the available complete sources and exercise independent
  skeletal/transform animation rather than tuning only for the bird.
- `flagship_showcase.usda` and deformation fixtures: CPU/native/GPU reference,
  morphs, animated normals/tangents and material values.
- `material_subsets.usda` plus available ANYmal/Spot sources: subset payload and
  source/instance lifecycle; verify source availability and hashes first.
- `PointInstancedMedCity.usdz` and compact-instancing fixtures: scale/culling.
- Available textured machine or kitchen scene: large static loading/texture path.
- Focused editing/reload fixtures: warm idle, edits, undo/redo, failure and repair.

## Current checkpoint

- Local performance repairs and material/image invalidation regressions are
  committed through `d5b87b6`.
- Preceding-work hardening passed: 781 no-default tests, 874 all-feature tests,
  strict Clippy, native deformation and watcher checks, and wasm compilation.
  The additional hummingbird CPU/GPU image comparison remains failing as recorded.
- `c804fe1` adds debug/release profile selection to the example build/run recipes.
  Invalid-profile rejection and both profiles were exercised. The clean release
  build completed in 17m27s rather than reusing an old Bevy artifact.
- `5d5d4b6` adds optional raw seek samples (`USD_BENCH_RAW_SAMPLES=1`) and process
  high-water RSS to `editor_benchmark`. The latter includes prior fresh-App samples
  in the same process; it is not per-document peak allocation or VRAM. Default
  workspace/all-target tests pass: 868 passed, 20 ignored. A debug probe emitted
  eight measured records with clocks, nanoseconds and a separate CSV summary.
  Logs: `/tmp/bevy-optimization-benchmark-tests.log` and
  `/tmp/bevy-optimization-raw-probe.log`.
- Release CPU characterization completed under
  `/tmp/bevy-optimization-baseline.KRJgwm/`, with revision/toolchain/host metadata,
  entry-file hashes, per-workload logs, raw seeks and status records. Eight
  workloads cover bird, cow, arm, showcase, morphs, subsets, kitchen and city.
  Referenced dependency closures are not yet hashed. This run shares host load.
  The initial bird case overlapped a one-off probe and was repeated serially;
  the latter records 56.814–67.931 ms median updates. Cow medians are
  82.124–106.466 ms with an unexplained 2.915-second outlier. Saved baseline
  executables and `run-headless-benchmark` (`1ed2f92`) support alternating
  comparisons without rebuilding. See [the baseline report](benchmarks/whole-pipeline.md).
  GPU, native cadence and complete lifecycle baselines remain outstanding.
- Final debug bird profiling through `d5b87b6` records 52.642 ms median and 61.210
  ms max headless updates. Across eight seeks, skin routes take 278.657 ms, mesh
  routes 78.715 ms and materials 39.203 ms. Mesh reads take 50.650 ms inside route
  time, not in addition to it. There are 944 mesh interning misses for 328 mesh
  projections, inserting 15,448,960 bytes: investigate intermediate/rest geometry
  construction and upload churn before adding more cross-frame caches.
- `d2256aa` reuses immutable rest geometry through the existing bounded assembly
  cache before color/skin/morph mutation. Default tests pass (869 / 20 ignored);
  all 32 alternating release matrix processes succeed with identical retained
  asset counts/payloads. Before/after captures are pixel-exact in five cases;
  cow still warns about its unavailable MDL dependency. Additional cache payload
  is about 1 MiB for bird and 3.3 MiB for cow. See the baseline report for exact
  measurements and limits. Hardening passes (782 no-default / 875 all-feature
  tests), as do strict Clippy, four native deformation checks, wasm compilation
  and pixel-exact synthetic CPU/GPU captures at 0, 5 and 10.
- Next targets: intermediate mesh asset churn and skin/tangent preparation,
  plus the remaining source-verification cost. GPU pass
  samples are now recorded, but native animated frame pacing and full lifecycle
  acceptance remain open. Retain all workstreams above; this is not whole-pipeline
  completion.
- Texture checkpoint: `d709b52` isolates finalization cost. `e573343` streams the
  first mip's decode and reuses the exact sRGB code table, reducing paired release
  bird opening from about 4.9 to 2.9 seconds and cow opening from about 2.5 to
  1.5 seconds, with lower peak RSS and identical retained payload. Default tests
  pass; before/after captures are pixel-exact, and hardening passes (787
  no-default / 880 all-feature tests plus strict Clippy), as do nine native watcher
  tests and wasm compilation. At this checkpoint, the cow's initial texture reload
  remained a separate follow-up, with a roughly two-second worst seek. See
  [texture finalization](benchmarks/texture-finalization.md).
- Initial texture verification: `9ded82f` records the hashes of decoded source
  bytes; `4eac681` preserves package-entry overrides; `1eff934` checks edited paths,
  undo/redo and failed refresh publication. Unchanged external textures no longer
  trigger image/material/mip regeneration after Open. Production watcher settings
  and fidelity are unchanged. Default tests pass (878 / 20 ignored) after a
  test-only projection-budget race repair (`bc95517`). Hardening at `1eff934`
  passes (791 no-default / 884 all-feature tests, strict Clippy), as do nine native
  watcher tests and wasm compilation. See [texture verification](benchmarks/texture-reload.md)
  for measurements, evidence and remaining animation/native-rendering limits.
- Skinning checkpoint: `e66e070` retains warmup timings, `dc89bfb` separates GPU
  preparation phases, and `69708b6` reports exact CPU fallback reasons. `739deac`
  reuses only the current mesh binding inside a read-only projection pass;
  authored influences, deformation and eager residency are unchanged. Default
  tests pass (881 / 20 ignored), hardening passes (793 no-default / 887 all-feature
  tests and strict Clippy), as do four native deformation checks, nine watcher
  checks, 15 native export checks and wasm compilation. All 16 alternating release
  processes pass with matching final geometry/image payload and peak asset counts.
  Bird tangent-deformation time falls, but host load prevents a reliable
  whole-frame speedup claim. Five before/after renders are pixel-identical; the
  cow's unavailable MDL diagnostic remains. See
  [skinning preparation](benchmarks/skinning-preparation.md) for exact evidence,
  memory/measurement limits and unresolved CPU/GPU fidelity checks. Larger tangent
  assembly, CPU fallback, native cadence and the full workstreams remain open.
