# Tangent generation and bounded reuse

## Scope

This follows [skinning preparation](skinning-preparation.md). Tangent generation
must preserve the existing MikkTSpace result, authored influences, CPU fallback,
normal/morph behavior, full mesh layout and eager library residency. The broader
objective and native playback target remain in [`OPTIMIZATION.md`](../OPTIMIZATION.md).

## Phase isolation

`edb462c` separates tangent mesh assembly from tangent generation. Both still run
the same assembly and Bevy MikkTSpace functions, including failure handling.
The existing blended-joint tests compare profiled/unprofiled tangent bytes at
multiple clocks. Default tests pass 881 tests (20 ignored):
`/tmp/bevy-optimization-tangent-profile-tests.log`.

The saved release probe in `/tmp/bevy-optimization-tangent-phases.z2ZEJe/` records
the following cumulative CPU durations over eight measured seeks:

| Workload | Tangent mesh assembly | Tangent generation |
| --- | ---: | ---: |
| Cow | 4.784 ms | 207.422 ms |
| Bird | 1.058 ms | 18.079 ms |

Generation is the larger phase. These short runs share unrelated host load;
they characterize work rather than establish displayed FPS or a latency bound.

## Rejected initial cache trial

The first trial routes GPU tangent correction and CPU fallback meshes through
the existing exact-input tangent cache without changing its 32-entry FIFO
policy. Its release executable, source patch and probes are retained in the
same evidence directory as `editor_benchmark.fifo32`, `fifo32.patch`, and
`{bird,cow}-cached.log`. Default tests pass 883 tests (20 ignored):
`/tmp/bevy-optimization-tangent-cache-tests.log`.

Bird records zero tangent hits and 640 misses across eight measured updates;
adding deformed inputs to that small cache churns the working set. Cow records
84 hits and 140 misses, without a demonstrated tangent-generation gain. The
nonpaired durations are confounded by host load. This is not accepted as a
speedup; the trial is retained rather than hidden in a successful-only report.

## Candidate cache policy

`6e71684` refreshes recently used entries on hits and allows up to 128
entries, while retaining the existing 16 MiB default CPU-payload budget and
caller-configured byte budget. The entry limit bounds the number of cache entries;
payload accounting excludes allocator and metadata overhead.
Returned tangents remain owned copies. Hash matches must also pass exact input
comparison; changed positions, normals, UVs, topology and other mesh data cannot
reuse stale values. No clock-keyed or unbounded history is added.

Tests exercise an 80-mesh working set, frequently used meshes mixed with unique
inputs, and 1,000 distinct geometries under zero, small and large byte budgets.
The CPU route is compared byte-for-byte with direct MikkTSpace output through
unique/reversed/repeated clocks and UV, normal and orientation edits, both with
CPU selected explicitly and with GPU preparation rejected for unnormalized
authored weights. GPU blended-joint/morph tests compare cached and uncached
tangent output too.

The revised short probe records 168 hits / 56 misses for cow and 477 hits / 163
misses for bird over eight measured seeks, rather than the FIFO trial's zero bird
hits. These are workload counters, not controlled timing evidence. Default tests
pass 884 tests (20 ignored): `/tmp/bevy-optimization-tangent-lru-tests.log`.

The first 1,000-clock cow trials of unconditional deformed-result retention
record 14,000 misses alongside 14,000 hits, with 16.5–16.7 MB of retained tangent
payload. This motivates a separate admission policy for deformed inputs: keep
at most 128 recent miss fingerprints, and retain a generated result only after
its fingerprint recurs. A fingerprint alone never establishes a cache hit; full
input comparison is still required. Unique input streams therefore need not
copy every generated mesh into the retained cache. Zero-budget configurations
retain neither results nor admission history.

`bc9963f` adds repeat-only admission and the GPU/CPU route integration. It passes
885 default workspace/all-target tests (20 ignored):
`/tmp/bevy-optimization-tangent-admission-tests.log`. Its new regression checks
1,000 unique inputs under zero and small budgets, followed by repeated input;
unique misses retain no mesh payload, admission history stays bounded, and
cached results remain equal to direct MikkTSpace output. Full validation results
and the unchanged rendering blockers are recorded below.

## Unconditional-retention comparison

The first full alternating matrix is retained in
`/tmp/bevy-optimization-tangent-abba.CrBjq9/`. It compares the saved `edb462c`
release binary with `6e71684` plus unconditional deformed-cache integration,
before the admission revision. Eight workloads each run before/after/after/before,
with four separately recorded warmups and either 100 repeated seeks or 1,000
distinct clocks spanning the authored timeline. No task-owned compilation
overlaps these runs; unrelated jobs remain active, with load roughly 12–58.

All 32 child processes report success. Entry-file hashes and the 267-file cow
directory manifest verify afterward. The outer execution session returned 143
despite the completed matrix and post-run manifests; its cause is unknown.
`session-exit-note.txt` records that anomaly rather than claiming an exit-zero
outer gate. Raw logs and `summary.tsv` preserve all samples and memory figures.

| Median CPU scene update | Before range | Unconditional-retention range |
| --- | ---: | ---: |
| Cow, repeated poses | 96.567–129.369 ms | 64.231–80.315 ms |
| Bird, repeated poses | 28.676–54.175 ms | 43.884–52.874 ms |
| Cow, unique timeline | 79.072–98.022 ms | 86.669–106.835 ms |
| Bird, unique timeline | 41.644–48.171 ms | 31.006–32.215 ms |
| Cow, CPU-only unique timeline | 85.019–143.530 ms | 80.854–98.695 ms |
| Bird, CPU-only unique timeline | 32.967–34.822 ms | 30.371–31.434 ms |
| Blended morph, unique timeline | 0.522–0.797 ms | 0.723–0.908 ms |
| Simple skin, unique timeline | 0.354–0.361 ms | 0.342–0.397 ms |

This is not a universal speedup: host scheduling strongly changes even control
runs, and the cow unique-time result does not establish improvement. The cow's
unique-time candidate inserts about 5.7 GB of cumulative tangent payload across
1,000 seeks while retaining only 16.5–16.7 MB; its 14,000 misses are not useful
cross-clock reuse. The repeat-only admission revision targets that unnecessary
copy/retention work without changing generated tangents. This earlier matrix
does not validate that revision.

A repeated-seek speedup alone is not native playback improvement. Final evidence
must retain unique-time comparisons, cache payload, process RSS and the existing
bird CPU/GPU mismatch and cow MDL warning.

## Repeat-only admission comparison

Evidence: `/tmp/bevy-optimization-tangent-admission-abba.baAx2B/`. This repeats
the eight-case, 32-process ABBA matrix against the same saved `edb462c` baseline,
using the final `bc9963f` production source. All children and the outer script
exit successfully. The 129 entry-file checks and 267-file cow directory checks
pass afterward. Geometry/image payload, retained mesh counts, peak mesh/material/
image asset counts, and mesh/assembly cache payload match across all four runs
of each case (`payload-check.txt`). Those checks are not a complete dependency
closure or allocator audit.

No task-owned compilation overlaps measurement. Unrelated jobs remain active;
one-minute load averages range 8.30–34.19. `summary.tsv` retains warmup, mean,
median, p95, maximum, cache payload, RSS and high-water RSS, with raw samples in
each log. `cpu-cache.tsv` adds GNU Time user-plus-system CPU accounting for the
entire saved executable, including opening, idle updates, warmups, seeks and
shutdown. It excludes the Make/Nix launcher, but is neither seek-only CPU time
nor displayed frame cadence. CPU frequency and host interference still affect it.
The timer invocation uses `--args=...` because a separate argument starting
with `-f` is parsed as an option by the Make argument parser.

| Workload | Before median update, ms | After median update, ms | Before process CPU, s | After process CPU, s |
| --- | ---: | ---: | ---: | ---: |
| Bird, CPU-only timeline | 35.093–35.104 | 29.382–30.708 | 41.33–41.68 | 35.02–37.10 |
| Bird, repeated poses | 56.743–75.263 | 57.223–61.756 | 10.38–12.08 | 9.81–10.88 |
| Bird, GPU-prepared timeline | 36.109–36.116 | 30.184–33.529 | 40.07–44.50 | 35.12–41.66 |
| Cow, CPU-only timeline | 78.240–83.538 | 77.190–78.188 | 84.46–90.05 | 79.45–82.37 |
| Cow, repeated poses | 77.371–161.371 | 34.121–67.397 | 10.13–17.40 | 7.12–8.62 |
| Cow, GPU-prepared timeline | 82.764–111.132 | 82.526–83.847 | 86.80–115.31 | 86.10–88.59 |
| Blended morph, timeline | 0.469–0.492 | 0.464–0.529 | 0.65–0.69 | 0.65–0.71 |
| Simple skin, timeline | 0.323–0.354 | 0.260–0.268 | 0.43–0.44 | 0.40–0.40 |

The measured 1,000-clock phases insert **zero** tangent-cache entries in all
timeline cases. Cow rejects retention of 13,986 one-use deformed results and
hits 14 results already cached during warmup; bird rejects 40,914 GPU-prepared
or 40,918 CPU-only results and hits 82. Unchanged/rest inputs account for most
other tangent-cache hits. Do not attribute those hits to reuse of unique poses.
Warmup results remain bounded and retained; admission does not mean zero total
cache payload. Repeated seeks still incur eviction: cow reuses 700 deformed
results and misses 700; bird reuses 2,050 and misses 2,050. Neither scene's full
three-pose working set fits its applicable byte/entry limits.

| Workload | Before tangent payload, MB | After tangent payload, MB | Before peak RSS, MB | After peak RSS, MB |
| --- | ---: | ---: | ---: | ---: |
| Bird, CPU-only timeline | 0.841–0.848 | 2.690 | 974.0 | 972.9–973.7 |
| Bird, repeated poses | 0.841–0.844 | 2.740–2.744 | 979.8–980.1 | 979.7–981.7 |
| Bird, GPU-prepared timeline | 0.810–0.844 | 2.690 | 975.8–976.0 | 976.0–976.6 |
| Cow, CPU-only timeline | 2.853 | 8.559 | 550.6–550.7 | 550.1–550.5 |
| Cow, repeated poses | 2.853 | 15.575–16.745 | 553.2–554.0 | 553.4–554.7 |
| Cow, GPU-prepared timeline | 2.853 | 8.559 | 551.3–553.3 | 554.0–555.2 |

MB here is decimal. Cache payload excludes allocator/metadata overhead and
VRAM; peak RSS is whole-process Linux VmHWM. The admission fingerprints add at
most 128 `u64` values, separately from mesh payload accounting. A lower RSS in
one process does not prove lower live allocation. Compared with unconditional
retention, the unique-time cow stops copying approximately 5.7 GB of cumulative
cache payload and retains about half as much tangent payload. Compared with the
pre-integration baseline it still retains more, in exchange for repeated-input
reuse. Blended-morph retained tangent payload is 432 bytes versus 216; the
simple-skin control retains none.

These measurements support bounded exact reuse and removal of unnecessary
cache copies. They do not establish universal speedup, a hard frame-time bound,
or the 60 Hz native target. In particular, cow unique-time CPU preparation is
still roughly 83 ms/update and small-morph timing ranges overlap. The policy
does not skip eager hidden geometry, normalize authored weights, lower quality
or change which meshes fall back to CPU deformation.

## Regression and rendering checks

At `bc9963f`, `harden` passes 797 no-default tests (21 ignored), 891
all-feature/all-target tests (29 ignored), strict Clippy, formatting and
whitespace checks. Four native OpenUSD deformation oracle tests, nine native
watcher tests, 15 native export checks and wasm32 compilation also pass. Logs:
`/tmp/bevy-optimization-tangent-admission-{harden,test-native-deformation,test-watchers,test-native,check-web}.log`.
Wasm compilation is not browser rendering/performance acceptance.

Before/after captures are retained in
`/tmp/bevy-optimization-tangent-admission-captures.7GWUsx/`. The saved before
renderer is production `739deac`; subsequent profiling extraction preserves
output. The current renderer is `bc9963f`. Both use `UsdAssetPlugin`, Vulkan on
an RTX 4080 with NVIDIA 595.112.07, 1280x720, forward rendering, 4x MSAA and WGPU
validation. All eight comparisons are pixel-exact at RGB tolerance zero:

- GPU-prepared bird at 0, 5 and 10, cow at 5, combined skin/morph-normal fixture
  at 5.
- Explicit CPU controls: bird at 0, cow at 5 and the combined fixture at 5.

All 16 images have compiled-pipeline/readback proof and the expected CPU/GPU
route selection. Entry hashes verify afterward. The after bird/cow images were
inspected. Both cow controls retain the same unavailable `OmniPBR.mdl` warning,
so the combined capture gate exits 1 despite pixel-exact before/after output.

The additional current-binary CPU/GPU bird comparison at time zero still fails:
2,053 of 921,600 pixels exceed RGB tolerance one, with maximum error 33. Its
inspected diff is again grid lines through the transparent wing blur. This is
identical to the previous failure, not a newly passing fidelity gate. No
tolerance was relaxed or grid removed. The transparent-sort hypothesis in
[the playback report](hummingbird-playback.md) remains unproven.

## Next work

The full scope remains in [`OPTIMIZATION.md`](../OPTIMIZATION.md). Unique-time
preparation still dominates: the recorded cow run spends 66.954 seconds in
`SkinRoute` across 1,000 updates, including 27.880 seconds generating GPU-correction
tangents; the bird run spends 23.104 seconds in `SkinRoute`. These are nested
CPU intervals, not additive GPU cost. Further work must address actual
time-varying preparation and CPU fallback rather than repeatedly increasing a
pose-cache budget. Native cadence, renderer fidelity, editor/loading/lifecycle
and browser/platform acceptance remain open. Nothing was pushed.
