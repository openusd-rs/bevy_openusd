# Skip the discarded reload animation index

Source-root publication previously called reconciliation with the live editor's
animation-index scan, then discarded that index. Source-root clock changes use
their own current-stage animation detection. Reconciliation now accepts an
internal `collect_animation` flag: source publication disables it, while both
live change-reconciliation paths retain it. Geometry and component routing are
unchanged; no animation cache or cross-stage reuse was introduced.

## Verification

- `make test-all CARGO='cargo --offline'`: 381 ordinary tests pass; five optional
  native export tests are ignored, not rerun for this change.
- `make check-all CARGO='cargo --offline'`, `make build CARGO='cargo --offline'`
  and `git diff --check` pass.
- One new test checks reconciliation with/without animation collection, stable
  entities and sampled transforms, and preservation/rebuilding of the index.
- Another starts two static source roots, reloads animated/static/animated
  revisions, then scrubs only one root. It checks correct sampled transforms,
  stable entities, Ready states and an unchanged unrelated live animation/time
  resource. Existing material/prototype/deformation clock tests also pass.

## Paired release measurements

The before executable was copied from the optimized `source_benchmark` built at
`40a13c9` before modifying code. Documentation-only `bc728f5` does not change it.
The after executable includes this change. Same toolchain, host, sibling Mara
revision and benchmark fixture as [the baseline](source-lifecycle.md).
One-minute host load was 27.68; background work and CPU affinity were uncontrolled.

After compilation completed, executables ran in before/after/after/before order,
at 1,024 instances per root. Each execution contains three alternating-order
one/four-root samples and the same lifecycle assertions. Commands used the
repository Makefile without changing it:

```sh
make build CARGO='cargo --offline' APP_TARGET='--release --example source_benchmark'
make --eval='measure-reload:; @$(BENCH_BIN) $(ARGS)' measure-reload BENCH_BIN=/tmp/source-benchmark-before-animation-index ARGS=1024
make --eval='measure-reload:; @$(BENCH_BIN) $(ARGS)' measure-reload BENCH_BIN="$PWD/target/release/examples/source_benchmark" ARGS=1024
```

The temporary before executable is local evidence, not a committed artifact;
rebuild the baseline revision to reproduce that side elsewhere.

```csv
run,roots,sample,load_ms,reload_ms,idle_us,initial_meshes,initial_materials,projected_meshes
before-1,1,0,233.992,172.855,63.531,1,1,1024
before-1,4,0,460.199,632.866,20.815,1,1,4096
before-1,4,1,470.530,601.643,55.398,1,1,4096
before-1,1,1,111.672,233.488,58.651,1,1,1024
before-1,1,2,126.716,271.120,77.214,1,1,1024
before-1,4,2,564.819,744.804,20.728,1,1,4096
after-1,1,0,108.653,110.099,40.513,1,1,1024
after-1,4,0,582.068,435.179,57.378,1,1,4096
after-1,4,1,464.496,434.529,68.589,1,1,4096
after-1,1,1,103.062,109.220,44.117,1,1,1024
after-1,1,2,114.264,105.261,55.431,1,1,1024
after-1,4,2,451.844,508.268,91.260,1,1,4096
after-2,1,0,144.475,119.124,23.214,1,1,1024
after-2,4,0,549.754,482.573,25.791,1,1,4096
after-2,4,1,411.069,446.647,39.051,1,1,4096
after-2,1,1,103.109,150.213,22.365,1,1,1024
after-2,1,2,117.457,137.156,58.248,1,1,1024
after-2,4,2,409.482,432.302,45.924,1,1,4096
before-2,1,0,130.782,196.146,51.325,1,1,1024
before-2,4,0,455.793,781.825,83.629,1,1,4096
before-2,4,1,592.072,611.904,60.899,1,1,4096
before-2,1,1,117.887,178.363,47.954,1,1,1024
before-2,1,2,123.432,235.716,57.588,1,1,1024
before-2,4,2,425.485,785.630,47.105,1,1,4096
```

Four-root reload ranges are 601.643–785.630ms before versus 432.302–508.268ms
after. One-root ranges are 172.855–271.120ms versus 105.261–150.213ms. These
samples support a reload improvement for this fixture, not a universal percentage
speedup, frame-rate claim or a new latency guarantee. Initial load does not use
the changed path; its variation illustrates the measurement noise.

A separate before/after run enabled `USD_PROFILE_SOURCES=1` with ARGS=128:

| Version/sample | Four-root validation ms | Four-root projection ms |
| --- | --- | --- |
| Before 0 | 27.028 | 66.888 |
| Before 1 | 23.747 | 49.163 |
| Before 2 | 22.400 | 51.905 |
| After 0 | 23.024 | 30.216 |
| After 1 | 28.538 | 32.483 |
| After 2 | 22.759 | 29.643 |

The reduction appears in projection rather than validation, consistent with
removing animation discovery from reconciliation. All benchmark assertions passed.
Reload still blocks the App update for hundreds of milliseconds at the larger
size. Clock changes still scan animation inputs, and instance edit reconciliation
still collects a discarded index through `apply_changes`; neither path was changed
here. Rendering, GPU performance and representative complex-scene acceptance
remain open.
