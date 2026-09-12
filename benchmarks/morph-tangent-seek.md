# Morph tangent editor seek baseline

Measured 2026-09-10 with the current editor benchmark on Bevy 0.19.1, Rust 1.96,
debug profile, headless App. Asset: `assets/morph_tangent_normals.usda` (two
triangles, authored UV/normal offsets, one material subset). No render device,
GPU execution, compositor, UI, release optimization or isolated host load.
Validation and other work shared the machine; these are diagnostic observations,
not a controlled speedup claim or a representative production-scene benchmark.

Each run creates a fresh App, opens the document, performs 100 idle updates, then
four warmup seeks and 100 measured seeks cycling 0/5/10/5. The timer surrounds
only App::update. Document identity, Ready status and clock are checked outside
the timer on every seek. Percentiles use nearest rank. Payloads count retained
CPU mesh attributes, indices, inline morph attributes and image bytes after seeks;
allocator/handle overhead, source/stage memory, RSS and VRAM are excluded.

Run through Make, one sample per process, alternating CPU/GPU-prepared order:

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 1 cpu seek'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 1 gpu-prepared seek'
```

Pair order: CPU/GPU, GPU/CPU, CPU/GPU. Logs:
`/tmp/editor-seek-{cpu,gpu-prepared}-{1,2,3}.log`.

| Mode | Pair | Median us | p95 us | Max us |
|---|---:|---:|---:|---:|
| CPU | 1 | 6803.963 | 8316.814 | 14892.462 |
| CPU | 2 | 6706.215 | 6984.398 | 11768.904 |
| CPU | 3 | 6700.692 | 6907.639 | 7212.186 |
| GPU-prepared | 1 | 6433.609 | 6720.515 | 11492.476 |
| GPU-prepared | 2 | 6506.843 | 9382.678 | 11520.213 |
| GPU-prepared | 3 | 6529.049 | 7099.466 | 8521.478 |

All runs retain two mesh entities and one subset. CPU retains 8 mesh assets,
26 vertices, 1248 vertex bytes, 120 index bytes, 0 morph bytes and 8 image bytes.
GPU-prepared retains 10 mesh assets, 34 vertices, 1632 vertex bytes, 168 index
bytes, 1440 morph bytes and 8 image bytes. No unreferenced vertices were counted.
GPU morph entity counts are 0 versus 2; this verifies preparation, not GPU work.

These three pairs do not isolate the cost of tangent regeneration from stage
sampling, routing, cache lookups or editor inspection. They cannot establish
an optimization benefit or production frame budget. Next measurements need
release builds, larger authored-UV deformation scenes, sustained clock streams,
allocation/RSS tracking and actual GPU/frame measurements.

## Release profile follow-up

The optimized build at source commit `ad97199` completed through
`make build CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark'`.
Toolchain: rustc 1.96.0 (ac68faa20), Linux 7.0.0-30-generic. Build log:
`/tmp/editor-seek-release-build.log`. Each result reports `profile=release`.
The same three fresh-process alternating pairs and clock sequence were repeated
with `--release --example editor_benchmark`, after compilation had completed.
No agent-started tests/builds overlapped the six measured runs; ambient host load
was not controlled. Logs: `/tmp/editor-seek-release-{cpu,gpu-prepared}-{1,2,3}.log`.

| Mode | Pair | Median us | p95 us | Max us |
|---|---:|---:|---:|---:|
| CPU | 1 | 565.161 | 993.687 | 1066.407 |
| CPU | 2 | 567.792 | 855.684 | 1113.823 |
| CPU | 3 | 582.677 | 1222.401 | 1339.846 |
| GPU-prepared | 1 | 589.169 | 1011.783 | 1457.538 |
| GPU-prepared | 2 | 529.111 | 874.623 | 987.480 |
| GPU-prepared | 3 | 557.115 | 1041.921 | 1312.512 |

All retained asset/payload counts match the debug runs above. GPU morph entity
counts remain 0 for CPU and 2 for GPU-prepared. CPU open times were
6.424/8.006/8.947 ms; GPU-prepared open times were 6.200/6.164/6.308 ms.
These measurements establish an optimized small-fixture baseline, not a causal
CPU/GPU-prepared speed comparison. The GPU-prepared route still performs work on
the CPU and does not execute GPU commands here. Larger scenes, unique sustained
clocks, allocation/RSS tracking and end-to-end rendering remain unmeasured.
