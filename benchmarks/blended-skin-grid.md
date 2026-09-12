# Blended-skin preparation cost

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
