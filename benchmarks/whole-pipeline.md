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

## Base-geometry reuse: `d2256aa`

Reuse immutable assembled base geometry before material color, skin and morph
mutation, through the existing 128 MiB / 256-entry content cache. Preserve separate
assembly modes, copy-on-write outputs, complete geometry equality and the
zero-budget path. Do not remove sample-dependent normals or tangents.

Acceptance for this step: eliminate repeated base assembly for cache-resident,
unchanged inputs; preserve geometry/material output and asset counts; quantify
additional bounded cache memory; and repeat before/after timing in alternating
order across the matrix. Total update speed and frame-rate targets remain
separate acceptance criteria, not inferred from cache hit counts.

### Alternating release comparison

Evidence: `/tmp/bevy-optimization-assembly-abba.Ci92Ga/`, including the saved
after executable, binary hashes, exact source diff, environment/load records and
raw samples. Every workload ran before, after, after, before in separate processes,
one fresh App each. Animated scenes use four warmup and 100 measured repeated
seeks, except the morph fixture's 1,000 unique clocks. All 32 processes succeeded.
No builds or other benchmarks from this task overlapped these runs; unrelated
applications remained active. The small number of process repetitions and shared
host load limit speedup attribution.

| Workload | Before median range, ms | After median range, ms |
| --- | ---: | ---: |
| Hummingbird | 62.074–68.533 | 51.735–61.286 |
| Cow eating | 79.647–80.459 | 78.991–79.011 |
| UR10 pick | 0.873–0.877 | 0.845–0.865 |
| Flagship showcase | 0.858–0.882 | 0.824–0.874 |
| Morph tangent/normal fixture | 0.277–0.282 | 0.276–0.276 |

Kitchen opening was 542.349–579.827 ms before and 549.420–567.840 ms after;
city opening was 64.039–65.231 ms before and 63.128–63.788 ms after. These
overlapping/noisy observations do not establish a static-loading speedup.

The bird now records 7,900 assembled-mesh clone hits across 100 seeks, versus
200 before: 77 additional base builds avoided per update, with no post-warmup
assembly misses. Cow records 2,626 clone hits, previously zero. Final geometry,
image payloads, entity/asset counts and peak asset counts match across all four
runs of each workload. Sample-dependent normal/tangent work is unchanged.

Bird retained assembly payload rises from 4,576 to 1,072,892 B; cow rises from
zero to 3,411,392 B, within the unchanged cache limit. Small-workload process RSS
is about 1–2 MB higher even where assembly payload is unchanged; retain this
observation rather than claiming memory usage is identical. Bird process peaks
overlap around 1.17 GB. No memory-bound or general lifecycle acceptance follows.

The bird still creates 11,800 intern misses over 100 seeks, unchanged. Intermediate
asset/upload churn remains a separate target. Cow's first measured seek is
2,931–2,996 ms in **every** before/after process, followed by roughly 150–168 ms
samples before settling. This is reproducible, not just a one-off discarded
outlier; its cause remains unproven. The cow median difference is too small to
claim a meaningful playback improvement. Neither bird nor cow reaches 60 Hz CPU
updates, let alone displayed frames.

### Render comparison

Evidence: `/tmp/bevy-optimization-assembly-captures.e5VLqc/`. Saved pre-change and
post-change debug capture binaries, Vulkan validation enabled, NVIDIA RTX 4080
with driver 595.112.07, forward rendering, 4x MSAA, fixed camera, 1280x720.
Before/after RGB is **pixel-exact at tolerance zero** in all five cases:
hummingbird at 0 and 10, cow at 5, combined skin/morph/normal fixture at 5, and
material subsets at 0. The bird reports 40 visible GPU-skinned meshes in both
versions. Images were inspected; the two bird poses differ visibly.

The cow capture gate is **not clean**: both binaries warn that
`Cow_FBX_PNG/Cow_F/_converted/OmniPBR.mdl` is unavailable. The comparison script
correctly returns failure for that diagnostic even though the images match.
This pre-existing source limitation was not hidden or waived.

Each capture also records 30 raw GPU-timestamp samples per pass after warmup.
Bird main opaque-pass medians are 0.037888/0.036864 ms before/after at time zero,
and 0.045056/0.045056 ms at time ten. These are individual static render-pass
durations, **not total GPU frame time or animated frame pacing**. They establish
neither a GPU speedup nor native interactive acceptance. The separate strict
CPU/GPU real-bird mismatch remains open.

### Correctness checkpoint

`test-all` passes: 869 tests, 20 explicit ignores, no failures. Log:
`/tmp/bevy-optimization-assembly-tests3.log`. New regression coverage checks all
three assembly modes, exact output, output-mutation isolation, cache hits and
zero-budget behavior; existing tests cover geometry edits, normals, influences,
subsets, morphs and independent instances.

The first run exposed an existing subdivision fixture assuming two roots finish
projection in one update under the shared 10 ms budget. `c158484` selects an
unlimited budget in that fixture only; production scheduling is unchanged.
An intervening run without the recorded debug settings was stopped before tests
to avoid rebuilding a separate dependency profile; it is not a test result.

Additional gates completed successfully at this checkpoint:

- `harden`: 782 no-default tests (21 ignored), 875 all-feature/all-target tests
  (29 ignored), strict Clippy, formatting and whitespace checks.
- `test-native-deformation`: four native OpenUSD position/normal oracle checks.
- `check-web`: wasm32 compilation, not browser runtime/performance acceptance.
- `test-deformation`: synthetic CPU/GPU images match exactly at clocks 0, 5 and
  10. Evidence: `/tmp/bevy-optimization-assembly-deformation-20261110/`.

Logs: `/tmp/bevy-optimization-assembly-{harden,native,web,deformation}.log`.
The open interactive viewer remains the earlier accepted freeze-repair process,
not a relaunch of this cache change. Its window and the user's desktop were left
alone. Full native frame pacing, unique-clock real assets, repeated open/close,
browser rendering and the remaining optimization workstreams are still pending.
