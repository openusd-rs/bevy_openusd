# Distinct-clock editor retention baseline

Measured 2026-09-11 on an Intel i7-13700KF, Rust 1.96, Bevy 0.19.1,
release profile, headless editor App. The fixture is
`assets/morph_tangent_normals.usda`: two triangles, authored UV/normal offsets
and a material subset. GPU-prepared means CPU preparation of GPU morph data;
there is no render device, GPU execution, UI or VRAM measurement.

Each process runs three fresh-App samples. Each sample opens the document and
performs 100 idle updates, then four warmup seeks at 0/5/10/5. `seek` measures
100 updates repeating that sequence. `seek-unique` measures 1,000 distinct
times from 0.01 through 10. These are distinct measured clocks, not guaranteed
cache misses: warmup already visited 5 and 10.

Only App::update is timed. Command enqueue, identity/clock/Ready checks, asset
counts and RSS reads are outside the timer. Percentiles use nearest rank.
Peak asset counts are sampled after updates, not allocation counts. RSS is
whole-process Linux VmRSS, not allocator-attributed memory or peak RSS; global
thread pools and allocator reuse can affect later samples in a process.
Host load was not isolated. Do not infer a speedup or production-scene behavior.

Run sequentially through Make, in CPU-repeat, CPU-unique, GPU-prepared-repeat,
GPU-prepared-unique order:

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 cpu seek'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 cpu seek-unique'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 gpu-prepared seek'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 gpu-prepared seek-unique'
```

Logs: `/tmp/editor-{cpu,gpu-prepared}-{seek,seek-unique}-final.log`.
Each run passed the document identity, Ready state and exact-clock checks.
All samples in a row had the same asset/cache counts and cache payload:

| Mode | Clocks | Peak/final mesh assets | Cached meshes | Cache payload bytes | Peak materials/images |
| --- | --- | ---: | ---: | ---: | --- |
| CPU | repeated | 8 | 7 | 1,152 | 2 / 1 |
| CPU | distinct | 2,004 | 2,003 | 312,528 | 2 / 1 |
| GPU-prepared | repeated | 10 | 10 | 3,240 | 2 / 1 |
| GPU-prepared | distinct | 3,004 | 3,004 | 1,009,224 | 2 / 1 |

First-sample timing and RSS observations:

| Mode | Clocks | Median us | p95 us | Max us | RSS before bytes | RSS after bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| CPU | repeated | 511.718 | 760.858 | 1122.298 | 50,999,296 | 51,052,544 |
| CPU | distinct | 529.817 | 661.777 | 1271.601 | 50,671,616 | 53,989,376 |
| GPU-prepared | repeated | 500.491 | 536.523 | 645.312 | 50,827,264 | 50,855,936 |
| GPU-prepared | distinct | 506.682 | 612.733 | 1639.758 | 51,019,776 | 56,762,368 |

The cache counters account for almost all retained mesh assets. Current
`ProjectionCache` holds strong handles and clears on either an 8,192-entry
limit or its default 256 MiB payload limit. This sweep does not reach either
limit, so its growth is not evidence of an unbounded leak. It does expose
historical deformation-state retention hidden by the repeated-clock benchmark.
The payload budget excludes allocator and GPU overhead. Further work must
evaluate streaming geometry retention without losing live-instance sharing,
then validate longer runs and representative scenes; this is not completion of
the performance acceptance checklist.

Four benchmark tests pass in debug and release, including distinct-clock
scheduling, RSS parsing/overflow, and a full 1,000-seek editor run. Logs:
`/tmp/unique-seek-tests-final.log`, `/tmp/unique-seek-release-tests.log`.

## Cache-only handle pruning

The follow-up implementation prunes cache-only mesh handles in `UsdPlugin`'s
Last schedule. Entity/external strong references preserve ownership and sharing;
Bevy asset tracking handles later removal. Entry/byte budgets remain. This drops
historical cache states rather than bypassing interning for every animated mesh.
An unused historical state may receive a new handle when revisited. Maintenance
scans the cache each frame; material-cache retention is unchanged.

Repeated the four commands above, three samples each, after ordinary validation
and before GPU validation. Logs: `/tmp/pruned-editor-{cpu,gpu-prepared}-{seek,seek-unique}.log`.
All samples retain two mesh entities and agree on the counts below:

| Mode | Clocks | Peak/final mesh assets | Cached meshes | Cache payload bytes | Peak materials/images |
| --- | --- | ---: | ---: | ---: | --- |
| CPU | repeated | 6 | 2 | 312 | 2 / 1 |
| CPU | distinct | 6 | 2 | 312 | 2 / 1 |
| GPU-prepared | repeated | 6 | 2 | 600 | 2 / 1 |
| GPU-prepared | distinct | 6 | 2 | 600 | 2 / 1 |

The storage count is measured at the end of the update; it is not identical to
the cache count or live Mesh3d count. Asset removal is deferred by Bevy tracking.

First-sample timing/RSS observations after pruning:

| Mode | Clocks | Median us | p95 us | Max us | RSS before bytes | RSS after bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| CPU | repeated | 526.171 | 552.036 | 610.871 | 50,642,944 | 50,724,864 |
| CPU | distinct | 529.672 | 845.249 | 1415.007 | 50,884,608 | 50,995,200 |
| GPU-prepared | repeated | 516.225 | 625.427 | 1084.054 | 50,376,704 | 50,565,120 |
| GPU-prepared | distinct | 516.005 | 785.539 | 1203.858 | 50,786,304 | 50,987,008 |

This demonstrates reduced mesh/cache retention for this fixture, not faster
updates: timings vary and host load was not isolated. It does not measure GPU
memory, arbitrary static-scene scan cost, long-duration allocation behavior or
material/texture-cache reclamation. The distinct-clock test now checks both
deformation modes on this fixture, with at most 16 peak mesh assets and four
cache entries. All four benchmark tests pass in release:
`/tmp/cache-prune-release-tests.log`.

The full ordinary suite passes 524 tests (13 ignored), and all 19 GPU live-clock
cases pass after pruning. The morph-tangent live capture also matches its
pre-pruning image exactly at RGB tolerance 0; `/tmp/cache-prune-before-after.log`.
These checks preserve the tested visual states and sharing behavior, not all
possible scene/performance workloads.

## Animated material retention

The same release benchmark was run with `assets/animation_showcase.usda 3 cpu
seek-unique`. Before ownership-aware material-cache maintenance, each sample
peaked at 1,003 standard material assets; afterward each peaked at four. Mesh
assets remained five and image assets zero. Logs:
`/tmp/material-retention-before.log`, `/tmp/material-retention-after.log`.

Sample-zero update median/p95/max changed from 420.874/499.251/734.979 us to
440.395/762.054/923.693 us. Whole-process RSS before/after seeks was
48,455,680/49,324,032 bytes before the change and 50,581,504/50,765,824 after.
These are separate processes with uncontrolled host load: this demonstrates
lower material retention, not faster updates or lower absolute RSS.

Standard and flat-normal cache maintenance releases cache-only handles in
`Last`, leaving entity/external ownership and normal asset tracking intact.
The 1024-entry caps remain. Unit tests cover ownership and stale assets in both
caches; the benchmark regression checks at most 16 peak standard materials in
both CPU and GPU-prepared modes. This texture-free fixture does not measure
texture lifetime, flat-material asset counts, renderer allocations or VRAM.
