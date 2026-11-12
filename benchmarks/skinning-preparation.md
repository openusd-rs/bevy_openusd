# Skinning preparation and warmup accounting

## Measurement changes

`e66e070` retains the four seek warmups instead of discarding their evidence.
`USD_BENCH_RAW_SAMPLES=1` emits `seek_warmup_sample` and
`seek_warmup_texture_sample` before the existing, unchanged measured-sample
records. A warmup total/maximum is always printed. Route counters are reported
before their measured-phase reset. Warmup route counters are cumulative from
opening, as are the existing after-idle counters; per-update texture records are
deltas. The CSV's measured percentiles still exclude warmup and remain comparable.

The release cow probe in `/tmp/bevy-optimization-warmup-probe.log` contains all
four warmup records and eight measured records. Its 205.9 ms worst warmup is now
visible rather than missing from the reported evidence. Default tests at this
checkpoint pass 879 tests (20 ignored).

`dc89bfb` adds opt-in `GpuSkinPreparationTiming`, enabled by
`USD_PROFILE_ROUTES=1` in `editor_benchmark`. It separates mesh reads, skin
sampling, rest assembly, morphs, vertex mapping, normal correction, tangent
deformation/mesh construction/correction, influence packing, interning and
publication. Failed attempts are counted too. The counters do not change
deformation or fallback selection. The absence/presence/error paths are tested;
default tests pass 880 tests (20 ignored).

`69708b6` reports up to 16 fallback paths, reasons, vertex counts and their own
visibility component after measurement. The total is reported even when truncated.
Own visibility is not inherited visibility; route visibility counters separately
classify the full hierarchy. These diagnostics do not change the CSV or timed work.

## Findings

Evidence: `/tmp/bevy-optimization-skin-baseline.jsyyZz/`. These release probes run
with unrelated host jobs active, so their timing is characterization, not final
performance acceptance or native frame cadence. No other job was stopped.

Across eight measured seeks, the first cow probe records 112 GPU-preparation
attempts and 16 failures. Tangent mesh construction takes 250.779 ms cumulatively;
skin sampling takes 21.567 ms and tangent deformation 26.419 ms. The entire
`SkinRoute` takes 591.510 ms; time outside GPU preparation includes CPU fallback
and outer-route work. These are CPU timings, not GPU command durations.

The first bird probe records 328 preparation attempts and 24 failures. Skin
sampling takes 41.847 ms, tangent deformation 40.027 ms and tangent mesh
construction 21.911 ms. The entire `SkinRoute` takes 221.045 ms.

Follow-up visibility/fallback probes identify:

- Cow: 13 of 14 animated meshes are hierarchy-hidden. Two hidden `Cow_F_LOD0`
  meshes, under `/World/Cow_F/RigRoot` and `/World/Cow_001/RigRoot`, each have
  22,926 render vertices and fall back because weights are not normalized.
- Bird: all 41 animated meshes are visible. `body`, `head_feathers` and
  `tail_transition` fall back because their influence stride exceeds four. A
  rendered-mesh count is not evidence that every mesh uses GPU skinning.
- The existing opt-in `DeferHiddenMeshes` policy removes hidden cow preparation
  in a control run. This changes residency policy and payload, so it is not a
  substitute for optimizing or measuring the eager library path. Whether to use
  it by default in the viewer still needs production-plugin, reveal, editing,
  readiness, count-panel and rendering checks.

Do not normalize, truncate or otherwise modify authored weights merely to force
GPU eligibility. Existing CPU fallback preserves those cases.

## Bounded binding reuse

`739deac` reuses the most recently resolved mesh binding within the existing
read-only `SkelSampleScope`. GPU preparation and its CPU tangent/fallback work
can share that query. Only one binding is retained, not a per-mesh or per-clock
cache; it is released with the pass. Pose sampling and deformation arithmetic
are unchanged. Tests cover scope nesting, stage isolation, eviction, release,
uncached reads and metadata edits between passes.

Default workspace/all-target tests pass 881 tests (20 ignored). Evidence:
`/tmp/bevy-optimization-binding-tests.log`. The saved release binaries compare
`69708b6` against the binding-only patch in
`/tmp/bevy-optimization-binding-abba.V0WJpA/`. All 16 processes pass: bird, cow,
simple skin and blended animated morph, each in before/after/after/before order.
Each fresh App retains four warmups separately and records 100 measured seeks.
The eager hidden-mesh policy and profiling settings are identical. No task-owned
build overlaps measurement. Unrelated jobs remain active; load averages range
roughly 22–38, so these results do not establish a stable FPS improvement.

| Measurement | Before range | After range |
| --- | ---: | ---: |
| Bird tangent deformation, total across 100 seeks | 478.641–932.092 ms | 129.020–215.036 ms |
| Bird median CPU update | 37.244–61.924 ms | 35.421–59.452 ms |
| Cow tangent deformation, total across 100 seeks | 317.822–474.373 ms | 257.314–301.461 ms |
| Cow median CPU update | 83.884–115.105 ms | 83.770–84.610 ms |
| Simple-skin median CPU update | 0.393–0.507 ms | 0.218–0.361 ms |
| Blended-morph median CPU update | 0.723–3.648 ms | 0.813–1.906 ms |

The removed repeat binding resolution is visible in bird tangent deformation;
initial skin sampling still resolves every mesh. Cow tangent mesh construction
remains much larger. Whole-update tails and control timings vary substantially
with host scheduling; neither universal improvement nor absence of regressions
can be established from these ranges. Warmup maxima are retained in raw logs and
`summary.tsv`, rather than hidden in excluded samples.

Final geometry/image payload and peak asset counts match for all four cases.
GPU preparation/fallback counts also match: bird 4,100/300, cow 1,400/200.
Cow, simple-skin and morph retained cache bytes match. Bird's existing 32-entry
tangent cache has variable occupancy in both binaries (841,392–845,736 bytes),
so total retained cache-byte equality is not claimed. Bird process peaks are
978.0–978.2 MB before and 979.2 MB after; cow peaks are 553.4–554.6 MB before and
553.2–554.0 MB after. These are decimal Linux VmHWM values, not VRAM or a bound
on live allocations.

Entry hashes pass post-run verification. The prior cow directory manifest also
verifies all 267 files after this run; it is not a dependency-closure audit.

`harden` passes 793 no-default tests (21 ignored) and 887 all-feature/all-target
tests (29 ignored), strict Clippy, formatting and whitespace checks. The four
native OpenUSD deformation oracle tests also pass, as does wasm32 compilation.
Logs: `/tmp/bevy-optimization-binding-{harden,native,web}.log`. A wasm build is
not browser rendering or performance acceptance. The five asset-watcher and four
editor texture-watcher tests also pass:
`/tmp/bevy-optimization-binding-watchers.log`. Native export checks pass 14
persistence cases and one reference custom-data round trip:
`/tmp/bevy-optimization-binding-export.log`.

Saved-binary Vulkan captures on the RTX 4080 at 1280x720, 4x MSAA and RGB
tolerance zero are pixel-identical for bird at 0, 5 and 10, cow at 5, and the
combined skin/morph-normal fixture at 5. Evidence:
`/tmp/bevy-optimization-binding-captures.xUS86m/`. All ten captures have compiled
pipeline/readback proof, and the bird and cow images were inspected. Entry hashes
verify after capture. The cow's missing `OmniPBR.mdl` diagnostic is unchanged in
both binaries, so the combined capture gate exits 1 rather than claiming clean
renderer acceptance. These use `UsdAssetPlugin`, not native interactive playback.

CPU/GPU comparisons of the current saved binary at RGB tolerance one are in
`/tmp/bevy-optimization-binding-deformation.26LcqP/`. The synthetic combined
skin/morph-normal fixture is pixel-exact at 0, 5 and 10. The bird still fails
with exactly the previously recorded counts and maxima:

| Time | Pixels over tolerance, of 921,600 | Maximum RGB error |
| --- | ---: | ---: |
| 0 | 2,053 | 33 |
| 5 | 1 | 3 |
| 10 | 1 | 19 |

All twelve captures have pipeline/readback proof and the expected GPU/CPU route
selection. The time-zero diff was inspected again: the disagreement is on grid
lines seen through wing transparency. Its cause remains unresolved. The gate
exits 1; no tolerance was relaxed and no failure was waived. Before/after equality
does not turn the existing CPU/GPU discrepancy into a passing fidelity check.

## Remaining work

The full objective remains in [`OPTIMIZATION.md`](../OPTIMIZATION.md). Native
cadence, eager CPU fallback cost, tangent construction, lifecycle/platform
acceptance and existing render diagnostics remain unresolved. The follow-up
[tangent investigation](tangent-reuse.md) separates assembly from generation,
tests bounded exact reuse with repeat-only admission, and records unique-clock
and repeated-seek comparisons plus current regression/render checks.
Repeated-time cache hits alone do not establish a playback improvement.
