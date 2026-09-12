# Blended-skin preparation cost

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
