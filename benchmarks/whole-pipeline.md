# Whole-pipeline optimization measurements

## Scope and status

The hummingbird freeze repair is accepted by the user. The broader optimization
is not complete. These measurements establish CPU/update and retained-payload
baselines, not displayed frame rate, GPU memory, browser performance or smooth
interactive playback. The workstream checklist is in [`OPTIMIZATION.md`](../OPTIMIZATION.md).

All work remains local. The strict real-bird CPU/GPU image comparison is still
failing as recorded in [the playback investigation](hummingbird-playback.md).

## Release baseline, 2026-11-10

Code: `5d5d4b6`, upstream OpenUSD revision unchanged. Rust 1.98.1, Linux x86-64,
Intel i7-13700KF. The release example uses the default workspace features.
Other applications and builds were active on the host; this is characterization,
not an isolated latency bound. No unrelated process was stopped.

Evidence: `/tmp/bevy-optimization-baseline.KRJgwm/` contains environment metadata,
entry-file SHA-256 hashes, raw seek samples, workload/status lists and logs.
Dependency closures are not fully hashed. All eight workload processes succeeded.
Saved `editor_benchmark.before`, `source_benchmark.before` and
`viewer_capture.before` executables permit comparisons without rebuilding the
baseline. The capture executable is a debug build; the benchmarks are release.

Each workload uses three fresh Apps within one process, 100 idle updates, then
four warmup and eight measured seeks at clocks 0, 5, 10, 5. The morph fixture uses
1,000 distinct clocks in (0, 10] instead. Static cases have no seeks. Medians below
are per-App; ranges cover the three Apps. Opening excludes plugin initialization
and GPU/UI work. Retained payload covers all assets, not only visible geometry.

| Workload | Open range, ms | Median update range, ms | Notes |
| --- | ---: | ---: | --- |
| Hummingbird, serialized repeat | 6,170.705–6,849.016 | 56.814–67.931 | 715,827,876 B image payload |
| Cow eating | 2,526.138–2,771.001 | 82.124–106.466 | 338,340,516 B image payload |
| UR10 pick | 79.483–88.363 | 1.277–1.612 | 71 mesh entities, 36 subsets |
| Flagship showcase | 4.582–18.141 | 0.920–2.324 | One GPU morph entity |
| Morph tangent/normal fixture | 2.348–6.027 | 0.504–0.765 | 1,000 unique clocks; seven peak mesh assets |
| Material subsets | 3.041–12.315 | Not measured | Three mesh assets |
| Kitchen | 545.172–845.306 | Not measured | 1,456 mesh assets; no loaded image payload |
| Instanced city | 54.171–69.976 | Not measured | 40,008 mesh entities sharing eight mesh assets |

The original bird matrix case overlapped a separate one-off probe. Do not use it
for a speedup claim; `hummingbird-serial.log` is the later serialized repeat.
The cow's first measured sample includes a 2,915.143 ms outlier. Its cause is not
established and the sample has not been discarded. Fresh before/after runs are
required before attributing differences to a change.

`rss_process_peak` reads Linux VmHWM. It includes earlier Apps in that process;
it is neither per-document peak allocation nor VRAM. Morph asset counts remain
bounded over the unique-clock probe, but this does not prove general open/close
or reload resource behavior.

`source-lifecycle.log` additionally exercises 256 source instances per root with
one and four independently projected roots, replacement reload, shared assets,
stable entities, runtime components and cleanup. All assertions passed. Warm
one-root loads are about 15–16 ms and reloads about 15–19 ms; four-root loads and
reloads are about 54 ms. This synthetic source test excludes disk IO and GPU work.

## Reproduction

Use the same developer/test debug settings as the preceding validation to reuse
the existing artifacts; omitting them triggers a separate debug-info build.

```sh
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
nix develop --command oslo make build-example --name editor_benchmark --profile release
USD_PROFILE_ROUTES=1 USD_BENCH_RAW_SAMPLES=1 \
  nix develop --command oslo make run-example --name editor_benchmark --profile release \
  --args 'assets/external/hummingbird.usdz 3 gpu-prepared seek-quick'

# Saved release executable, no rebuild:
USD_PROFILE_ROUTES=1 USD_BENCH_RAW_SAMPLES=1 \
  nix develop --command oslo make run-headless-benchmark \
  --binary /tmp/bevy-optimization-baseline.KRJgwm/editor_benchmark.before \
  --args 'assets/external/hummingbird.usdz 1 gpu-prepared seek'
```

Route timers overlap nested mesh-read work; do not add them together. Raw samples
are emitted after the measured `App::update`, not inside its timed interval.
The headless runner does not configure graphics or change validation settings.

Hardware CPU sampling of the already-running viewer was denied by the host's
perf permissions. No security settings or desktop state were changed. Use the
in-process counters for the current analysis.

## First optimization under evaluation

Reuse immutable assembled base geometry before material color, skin and morph
mutation, through the existing 128 MiB / 256-entry content cache. Preserve separate
assembly modes, copy-on-write outputs, complete geometry equality and the
zero-budget path. Do not remove sample-dependent normals or tangents.

Acceptance for this step: eliminate repeated base assembly for cache-resident,
unchanged inputs; preserve geometry/material output and asset counts; quantify
additional bounded cache memory; and repeat before/after timing in alternating
order across the matrix. Total update speed and frame-rate targets remain
separate acceptance criteria, not inferred from cache hit counts.
