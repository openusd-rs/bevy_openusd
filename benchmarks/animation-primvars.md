# Animation primvar ownership sampling

## Candidate and acceptance

At `0b4203a`, initial projection shares ancestor primvar authoring/interpolation
answers through `PrimvarScope`, but animation does not. Animated sibling meshes
repeat the same ancestor resolution for absent normals/UV/color/opacity and
inherited constant attributes. Values and time samples must still be read fresh.

The candidate bounds that existing scope to 4,096 answers and 1 MiB of key text,
then enables it for one built-in animation pass, including serial fallback.
Over-budget reads remain uncached. No scope crosses updates, clocks, stages or
custom-route callbacks; no geometry, tangent arithmetic or cache policy changes.
The text cap excludes hash-table metadata, allocator overhead and stage storage.

Declared before implementation:

- Compare ownership with independent uncached walks in both traversal orders,
  including absent, blocked, local and nonconstant-ancestor attributes.
- Exercise nested/independent stages, edits between passes, zero/entry/text caps,
  cleanup and custom callbacks that mutate a later sibling's inherited owner.
- Compare complete assembled mesh payloads across serial/batched animation,
  changing/repeated/reversed clocks and inherited attribute edits.
- Run repository hardening, native deformation/watchers/export and wasm checks.
- Compare saved release executables in fresh-process ABBA order for bird, cow,
  arm and subsets using 1,000 authored-range clocks; preserve raw samples,
  warmups, complete update intervals, payloads, asset counts and RSS. Do not
  overlap task-owned builds or alter unrelated work/host settings.
- Incremental target: at least 10% lower bird mesh-read time and 5% lower bird
  whole-update median, with no greater-than-5% aggregate timing regression in
  controls. Retain failed observations rather than weakening these thresholds.
- Compare native rendered poses at exact clocks with RGB tolerance zero.
  CPU timings are not window FPS. Prior render/performance failures and the
  full optimization checklist remain open regardless of this candidate.

Baseline executables and build logs are retained under
`/tmp/bevy-optimization-primvar-pass.SEBLD3/`.

## Correctness checkpoint

Four new regression tests cover the ownership/cache contract, custom-route
mutation visibility and byte-exact CPU/GPU mesh output against uncached direct
patches across forward/reverse/repeated clocks and inherited interpolation edits.
Hardening passes 852 no-default and 971 all-feature/all-target executions,
strict Clippy, formatting and whitespace checks. Configurations overlap.
The authoritative log is `harden-final.log` in the evidence directory.
Two earlier attempts are retained: a moved test attribute builder and a missing
attribute spec in the new fixture. Both were test construction errors; the
fixture now creates its attribute and clones the consuming builder explicitly.
This checkpoint does not establish performance or rendered acceptance.

## Release comparison

`f3a7501` also passes four native deformation-oracle tests, five asset-watcher
and four editor texture-watcher tests, fourteen export cases, one reference
custom-data round trip and wasm compilation. See `gates.tsv` and its named logs.
No dependency versions changed.

The sixteen-process CPU matrix uses fresh Apps in before/after/after/before
order, four warmups and 1,000 distinct authored-range clocks per process.
All 16,000 measured clocks, durations and percentile summaries validate. All
processes and runtime diagnostic checks pass. Geometry/image payloads, cache
footprints, image capacities and peak asset counts match exactly. Manifests
for executables, shared inputs and the bird/cow/arm/subset sources verify.
The common executable path is `run/editor_benchmark`; no task-owned build
overlaps. Host load is 24.56–49.73, with unrelated work left untouched.

| CPU measurement | Before pair | After pair | Mean-of-pairs change |
| --- | ---: | ---: | ---: |
| Bird mesh reads, total ms | 4412.598 / 3726.514 | 2907.723 / 2219.997 | -37.0% |
| Bird update median, ms | 42.030 / 36.198 | 38.731 / 22.713 | -21.5% |
| Cow mesh reads, total ms | 3918.601 / 4021.724 | 2600.567 / 3398.547 | -24.4% |
| Cow update median, ms | 116.289 / 122.169 | 59.260 / 89.248 | -37.7% |

These meet the two incremental bird targets, but the complete performance
gate **fails twelve checks**. Bird opening/idle regress 23.2%/11.2%; cow seek
p95/max regress 5.6%/89.9%; arm p95/max regress 128.8%/27.4%; subsets opening,
idle, seek median/p95/max and mesh reads regress 9.4%, 44.3%, 16.7%, 55.1%,
202.6% and 24.4%. All raw runs and failed comparisons remain in `analysis.json`.
The unchanged opening and idle paths also vary substantially. This matrix
does not isolate a causal whole-update gain or dismiss any regression as noise.

Peak process RSS changes less than 0.7% in all cases. One cow after-seek RSS
sample drops to 127.6 MB despite unchanged retained payloads; it is not evidence
of a library allocation reduction. `post-run-system.txt` records pressure/swap
state only after measurement, not synchronized evidence explaining that sample.

Saved executable SHA-256 values:

- CPU before: `b51bd9e678a4d29abd1546ca6912534ad44c18346e7f908651f221f1431268d2`
- CPU after: `70becba7d3694fdf3ed879a74697bfba739da0a93836e78288799a0aaa41b92e`
- Native before: `13211bfecbcda0e7f47530e374e199340fe2fd585496ba803ae43275784f66ba`
- Native after: `d9d57b9ff1e4cf44fce345846c734a64a15ff97b0053c0d9b1e84bf196f03ce8`

## Native animation and remaining work

The `captures/` subdirectory retains four actual candidate playback runs on
RTX 4080 / NVIDIA 595.112.07 / Vulkan, with validation, forward rendering,
4x MSAA and 1280x720. Each uses thirty animated-ready warmups and 300 measured
frames. All 1,200 raw frame/clock rows and published GPU/render-CPU arrays
validate; maximum clock-equation error is 5.685e-14. Final poses match independent
baseline renders at their exact clocks: all four pairs have zero changed RGB
pixels at tolerance zero, with matching cameras, quality and visible mesh counts.
Bird and cow images were inspected. Binary/source manifests verify.

All eight processes and panic/render-log checks pass. Both cow runs retain the
missing `OmniPBR.mdl` warning, so the combined diagnostic gate exits 1.
The first analysis attempt expected an unrequested source-loading profile;
the corrected analyzer explicitly records its absence without rerunning captures.
No source-opening timing claim comes from these captures.

Candidate bird/cow main-update medians are 24.462/94.309 ms and offscreen
interval medians 25.182/96.383 ms. Both still miss 60 Hz. These are not paired
before/after timings, editor playback measurements or native window cadence.
Cow tangent dispatch/join remains 74.0% of preparation wall time, including
warmups, identifying the larger remaining playback target. Worker wall times
overlap and are not additive CPU time.

This remains a local candidate, not full performance acceptance. Next work must
address tangent scheduling and isolate small-control overhead/regressions rather
than rerun unchanged matrices until they pass. Prior live-editor grid differences,
CPU/GPU fidelity failures, opening regressions, browser/platform and lifecycle
requirements in `OPTIMIZATION.md` remain open.
