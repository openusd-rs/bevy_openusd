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
