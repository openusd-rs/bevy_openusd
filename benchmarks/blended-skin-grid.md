# Blended-skin preparation cost

## Animated joint palette

`assets/skel_morph_blended_animated.usda` samples both joints' translations,
rotations and nonuniform scales at 0 and 10. The generator's optional third
argument `animated` selects it; omitted/`static` preserves the existing fixture.
All layers are copied into a new directory, without editing existing assets.

```sh
make --eval='animated-grid:; @/bin/bash scripts/make_skin_grid.sh target/NEW_ANIMATED_GRID 100 animated' animated-grid
```

On `target/skin-grid-animated-100/grid.usda`, 1000 distinct sampled times give:

| Mode | Median | p95 | Maximum | Tangent cache payload |
|---|---:|---:|---:|---:|
| GPU-prepared | 21.580 ms | 25.914 ms | 34.755 ms | 729648 bytes |
| CPU | 20.243 ms | 25.868 ms | 33.950 ms | 729648 bytes |

GPU first, CPU second, profiling enabled in both, no concurrent builds/tests.
Both retain six mesh assets, zero unreferenced vertices and no peak growth above
six meshes. This remains editor preparation, not GPU/presentation timing, and
GPU-prepared remains slightly slower. Logs `/tmp/animated-grid-{gpu,cpu}-benchmark.log`.

Four endpoint images `target/animated-grid-{0,10}-{gpu,cpu}.png` were inspected.
The mesh changes from a folded upright strip to a broad slanted panel. CPU/GPU
are RGB-exact at time 0; at time 10 two of 921600 pixels differ, max RGB error 1.
The strict time-10 comparison fails; no threshold was relaxed. The stretched
corner is present in both CPU and GPU output. Captures use /ReferenceCamera,
forward MSAA Off and shadows off. Logs `/tmp/animated-grid-{0,10}-compare.log`.

The shader-simulation regression checks animated normals/tangents at five times
and two weight patterns. A new explicit native oracle compares the small source
fixture's normals to native baked normals at those five times (1e-5 component
tolerance); all four native deformation tests pass. Native sampling also accepts
the generated 10201-point grid at time 10. 499 library tests pass, 19 ignored;
check-all passes. Logs `/tmp/animated-grid-{tests,native-tests,native-points,check}.log`.
Generator syntax and invalid-mode rejection pass without creating output.
No runtime Rust code changed; full workspace tests were not rerun in this step.

## Distinct-time cache retention

At ba49bba runtime code, 1000 distinct seeks from 0.01 through 10 on the same
grid give the following profiled editor preparation results (GPU first, CPU
second; no concurrent builds/tests):

| Mode | Median | p95 | Maximum | Retained tangent payload |
|---|---:|---:|---:|---:|
| GPU-prepared | 20.967 ms | 21.725 ms | 32.105 ms | 729648 bytes |
| CPU | 19.381 ms | 19.908 ms | 28.469 ms | 729648 bytes |

Both retain six mesh assets and zero unreferenced vertices, with no peak mesh
count increase beyond six. GPU morph payload remains 1469232 bytes, CPU zero.
Whole-process RSS increases about 5 MB in each run; this is not GPU memory or
a bound for arbitrary assets. Logs `/tmp/tangent-cache-unique-{gpu,cpu}.log`.

This fixture has a fixed two-joint palette with varying spatial weights and
sparse animated morph data. Distinct time samples are not evidence of a fully
animated large joint palette. GPU preparation remains slightly slower than CPU;
no 60 Hz playback or GPU frame-rate claim follows.

At time 2.37, both `target/unique-grid-237-{gpu,cpu}.png` were inspected and are
RGB-exact across 921600 pixels. Compared to the prior time-5 GPU frame, 20 pixels
differ (max RGB 186), confirming a changed rendered sample rather than a reused
screenshot. Logs `/tmp/unique-grid-237-compare.log` and
`/tmp/unique-grid-time-change.log`; the latter intentionally fails strict equality.
No source implementation changed in this measurement step.

## Cache unchanged rest-mesh tangent generation

MeshRoute assembles geometry without tangents and uses MeshTangentCache to reuse
generated tangents for byte-identical inputs. Hash candidates undergo full mesh
comparison, so hash collisions cannot substitute geometry. Cached CPU input and
tangent buffers are private copies, independent of mutable mesh assets. Failed
generation is not cached. Route order, custom registries and material tangent
diagnostics remain unchanged; public mesh conversion is unchanged.

Retention is FIFO, at most 32 entries and 16 MiB of mesh/tangent payload by
default, with no retained asset handles. Applications may insert
MeshTangentCache::with_byte_budget before projection, including zero to disable
retention. Payload accounting excludes allocator/container overhead and is not
GPU memory. The editor benchmark now reports cached_tangent_payload_bytes.

Same 100-seek profiled grid, GPU-prepared mode, sequential runs without concurrent
builds/tests during measurement:

| Mode | Median | p95 | Maximum |
|---|---:|---:|---:|
| Before (81306ed) | 38.012 ms | 39.043 ms | 41.823 ms |
| Cached tangents | 21.060 ms | 22.393 ms | 26.133 ms |
| Repeat, metrics added | 20.910 ms | 22.809 ms | 28.143 ms |

MeshRoute's 100-seek total falls from 1723.119 ms to 29.983 ms; SkinRoute stays
near 1969 ms. The cache retains 729648 CPU payload bytes for this fixture.
This is editor preparation, not presented frame time or proof of 60 Hz playback.
Logs: `/tmp/joint-normal-after-benchmark.log`,
`/tmp/rest-tangent-cache-{benchmark,repeat}.log`.

Inspected `target/rest-tangent-cache-{gpu,cpu}.png` at time 5, forward MSAA Off,
shadows off, /ReferenceCamera: CPU/GPU and before/after GPU are RGB-exact across
921600 pixels. Logs `/tmp/rest-tangent-cache-{compare,before-after}.log`.
498 library tests pass (18 ignored), including exact reuse, forced hash collision,
UV changes, external output mutation, zero budget, eviction and failed generation.
Logs: `/tmp/rest-tangent-cache-tests.log` and release build logs
`/tmp/rest-tangent-cache-{build,metrics-build}.log`.

The first full workspace run failed the nested-package reload test during its
repair wait (497 passed, one failed); log `/tmp/rest-tangent-cache-all-tests.log`.
The isolated unchanged test passes, followed by an unchanged full rerun with
663 passed, 18 ignored, 33 suites. check-all and viewer build also pass.
Logs: `/tmp/rest-tangent-cache-reload-recheck.log`,
`/tmp/rest-tangent-cache-all-tests-repeat.log`,
`/tmp/rest-tangent-cache-{check,viewer-build}.log`. The reload failure's cause
has not been isolated; a successful rerun is not a claim that it was fixed.

## Reuse joint normal inverses within a sample

CPU normal skinning and GPU normal-correction preparation lazily cache each
referenced joint inverse-transpose for the duration of one call. Unused singular
joints are not evaluated; referenced singular joints still fail. Rigid bindings
retain inversion of the blended matrix. CPU nonrigid normals also omit the
unused blended position-matrix calculation. No state survives a time sample.

One profiled 100-seek GPU-prepared pair on the same grid measured:

| Revision | Median | p95 | Maximum |
|---|---:|---:|---:|
| Before | 38.232 ms | 40.082 ms | 40.696 ms |
| After | 38.012 ms | 39.043 ms | 41.823 ms |

This is effectively unchanged overall preparation cost, not a meaningful
speedup claim. CPU tangent rebuilding remains. Logs:
`/tmp/joint-normal-{before,after}-benchmark.log`.

Both `target/joint-normal-grid-{gpu,cpu}.png` were inspected. CPU/GPU and
before/after GPU comparisons are RGB-exact across 921600 pixels at tolerance 0;
logs `/tmp/joint-normal-{grid-compare,before-after}.log`. Capture settings:
time 5, /ReferenceCamera, forward MSAA Off, shadows off, unchanged grid source.

496 library tests pass (18 ignored), all three native baked-deformation tests
explicitly pass, check-all and release benchmark/capture builds pass. Logs:
`/tmp/joint-normal-{tests,native,check,build}.log`. The cache regression covers
lazy population, reuse, singular failure and a fresh cache for the next sample.
Full workspace tests were not rerun for this bounded library change.

## Skip discarded rest tangents

Mesh assembly now has an internal tangent-generation flag. Public conversion
keeps its existing behavior; GPU skin preparation disables rest-pose tangents
only when it will replace them with fully deformed tangents. The correction's
CPU-reference geometry and final tangent calculation remain unchanged. The
shader-simulation test now starts without tangents and covers both uniform and
varying joint weights at five times.

Profiled 100-seek measurements on the same grid:

| Order | Mode | Median seek | p95 seek | Maximum seek |
| --- | --- | ---: | ---: | ---: |
| 1 | CPU | 38.308 ms | 49.283 ms | 56.853 ms |
| 2 | GPU-prepared | 40.163 ms | 52.822 ms | 75.085 ms |
| 3 | GPU-prepared | 63.207 ms | 82.159 ms | 144.883 ms |
| 4 | CPU | 47.463 ms | 68.261 ms | 112.838 ms |

The first GPU median is below the preceding 56.289 ms result, but the reversed
pair is substantially noisier. Other CPU-heavy work was observed on the shared
host afterward; that observation does not isolate the cause of each sample.
Do not treat this as a robust percentage speedup or a stable playback rate.
The implementation deterministically removes one discarded tangent-generation
call for the affected bindings.
Logs: /tmp/skip-rest-tangents-{cpu,gpu-prepared}-benchmark.log and
/tmp/skip-rest-tangents-repeat-{cpu,gpu-prepared}-benchmark.log.

At time five, forward MSAA Off, shadows off and /ReferenceCamera, CPU/GPU and
before/after GPU images are RGB-exact (0/921600 changed). Both new images were
inspected: target/skin-grid-skip-rest-{gpu,cpu}.png;
/tmp/skin-grid-skip-rest-{compare,before-after}.log.
494 library tests, three native checks, check-all and release example build
pass: /tmp/skip-rest-tangents-{tests-final,native,check,build}.log.

A separate candidate reused sampled GPU buffers to build final tangents. It
measured 55.637 ms versus 56.289 ms beforehand and introduced maximum RGB-1
rounding differences. That candidate was removed rather than retain extra
complexity for a change within observed timing variation. Its diagnostic logs
remain under /tmp/reuse-tangent-input-* and /tmp/skin-grid-reuse-tangents-*;
the final code retains the original CPU-reference tangent calculation.

## Skip discarded morph tangents

When blended skinning will regenerate fully deformed tangents, GPU morph
preparation now skips its earlier morph-only tangent pass. Standalone morphs
and bindings without that later tangent pass retain their existing behavior.
The correction's final tangent computation and rendered normal math are
unchanged. The shader-simulation regression exercises the skipped-pass path.

On the same 100-by-100 source, one subsequent profiled CPU/GPU-prepared pair
with 100 repeated seeks measured:

| Mode | Median seek | p95 seek | Maximum seek |
| --- | ---: | ---: | ---: |
| CPU | 37.728 ms | 40.402 ms | 44.485 ms |
| GPU-prepared | 56.289 ms | 60.411 ms | 70.748 ms |

GPU-prepared median is 23.1% lower than the 73.172 ms baseline, but remains
slower than CPU preparation. This is not GPU frame time or proof of responsive
large-rig playback. Logs: /tmp/skip-morph-tangents-{cpu,gpu-prepared}-benchmark.log.

Both new posed images were inspected. At time five, forward MSAA Off,
shadows off and /ReferenceCamera, CPU/GPU remain RGB-exact and the GPU image
is RGB-exact with the baseline (0/921600 changed):
target/skin-grid-skip-morph-{gpu,cpu}.png;
/tmp/skin-grid-skip-morph-{compare,before-after}.log.
494 library tests, three explicit native tests, check-all and release example
build pass: /tmp/skip-morph-tangents-{tests-final,native,check,build-final}.log.

## Fixture and method

At da06181, `scripts/make_skin_grid.sh` generates a self-contained USDA fixture
directory with copied small source layers, a 100-by-100 grid (10,201 points,
20,000 triangles), UVs, normals, two differently rotated/scaled joints and
weights varying across X. It retains a sparse morph target, constant normal-map
material and an animated material subset. Original assets are not modified.

```sh
make --eval='skin-grid:; @/bin/bash scripts/make_skin_grid.sh target/NEW_GRID 100' skin-grid
make build CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark'
USD_PROFILE_ROUTES=1 make --eval='skin-bench:; @target/release/examples/editor_benchmark target/NEW_GRID/grid.usda 1 cpu seek' skin-bench
USD_PROFILE_ROUTES=1 make --eval='skin-bench:; @target/release/examples/editor_benchmark target/NEW_GRID/grid.usda 1 gpu-prepared seek' skin-bench
```

Each run measures 100 repeated seeks through 0,5,10,5 after four warmup seeks.
CPU runs first, GPU-prepared second; one run each. Workspace tests/builds had
finished before measurement. Route profiling is enabled in both runs. These
are whole editor-update preparation times, not GPU rendering, presentation
cadence or an isolated normal-correction microbenchmark.

## Baseline

| Mode | Median seek | p95 seek | Maximum seek | SkinRoute total / 100 seeks |
| --- | ---: | ---: | ---: | ---: |
| CPU | 37.402 ms | 40.672 ms | 50.923 ms | 1926.831 ms |
| GPU-prepared | 73.172 ms | 76.678 ms | 81.682 ms | 5452.592 ms |

The GPU-prepared route is about 1.96 times slower to prepare in this ordered
pair. Both retain six mesh assets and 40,810 vertices, with no unreferenced
vertices. GPU preparation retains two morphed entities and 1,469,232 bytes of
morph payload; CPU retains neither. Asset sharing does not remove the sampled
geometry/tangent work. This is a demonstrated preparation bottleneck, not a
claim that GPU execution is slower. Optimizing duplicated deformation/tangent
preparation remains necessary before claiming responsive large-mesh playback.

Logs: `/tmp/skin-grid-100-{cpu,gpu-prepared}-benchmark.log` and
`/tmp/skin-grid-benchmark-build.log`. Source: `target/skin-grid-native-100/`.
The native CPU sampler accepts the generated large stage and reports 10,201
points at time five (`/tmp/skin-grid-native-100-sample.log`).

The corrected generated source was captured in Bevy at time five, forward
MSAA Off, shadows off and /ReferenceCamera. Both images were inspected and
are RGB-exact (0/921600 changed): target/skin-grid-native-100-{gpu,cpu}.png;
/tmp/skin-grid-native-100-compare.log. This is one posed image, not sustained
rendering or a native image comparison.

## Generator validation

The script accepts 1–256 cells per side and refuses an existing output path.
Shell syntax, zero-size rejection, existing-directory rejection and a one-cell
native bake were checked. The first generator used adjacent prim declarations
on one line, accepted by the Rust reader but rejected by native USD. That form
was replaced with separate lines. The corrected small-native result is
`/tmp/skin-grid-small-native.log`; earlier `target/skin-grid-100/` captures are
retained but are not the canonical native-compatible source.
