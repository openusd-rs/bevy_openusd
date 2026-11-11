# Bounded parallel mesh preparation

## Scope and status

This targets the larger tangent-generation cost left by the CPU-input reuse
checkpoint, not another repeated-pose cache optimization. The tightly paired
release results below support a scheduling improvement; the earlier failed
saved-revision trials are retained and are not retroactively passed. Native
frame-rate acceptance and the full objective in `OPTIMIZATION.md` remain open.

The chronological checkpoints below describe the evidence available at each
step. For the subsequent wing/grid sorting repair and current remaining
single-pixel image failures, see [posed sorting centers](deformed-sort-centers.md).

The later [deferred preparation](deferred-preparation.md) change allows ahead-of-
route work for already resident meshes under opt-in deferral. Nonresident meshes
and shared residency budgets retain synchronous routing; library defaults stay
eager. Historical all-deferral fallback descriptions below refer to older code.

## Follow-up phase instrumentation

The asset-root classification-cache comparison and its two targeted repeats
show conflicting cow whole-update results and large changes in the unchanged
mesh-preparation phase. Those regressions remain unaccepted; see
[the retained results](instance-animation.md). An opt-in
`MeshPreparationPhaseTiming` resource separates eligibility/USD sampling,
CPU deformation, mesh assembly, tangent-cache lookup, traversal bookkeeping,
tangent dispatch/join and ordered publication. These main-thread phases do not
overlap. Tangent-job wall times, job failures and vertex counts are separate:
worker times may overlap one another and must not be added to the main phases
or described as thread CPU time. The timer retains only aggregate counters and
a fixed set of phase names, not meshes or per-frame histories.

The existing preparation resource and public settings remain unchanged.
`USD_PROFILE_ROUTES=1` enables the new resource in `viewer_capture`; reporting
explicitly includes animation warmups. Dispatch, mesh generation, cache policy
and publication order are unchanged. The new tests compare profiled/unprofiled
geometry over changing/reversed clocks and cover empty/failed batches without
worker jobs; a capture test checks the separate metadata scopes.

The first hardening attempt passes both new library tests but exposes another
existing subset fixture assuming two roots finish projection in one shared
10 ms turn. It fails before resolving `/M`, not on any changed geometry
assertion. The fixture now requests an unlimited test projection budget and
asserts both roots reach Ready before its original subset/material/independent
child checks. Production projection budgets are unchanged. The failed run is
retained in `/tmp/bevy-optimization-mesh-phases-harden.log`. The second attempt
passes 831 no-default tests but strict Clippy rejects the added eighth capture
system argument. The optional phase resource now belongs to the existing
`CaptureTiming` system-parameter group rather than adding a lint suppression.
That attempt remains in `/tmp/bevy-optimization-mesh-phases-ready-harden.log`.

Final hardening passes 831 no-default and 940 all-feature/all-target tests,
strict Clippy, formatting and whitespace checks. These configurations overlap.
The log is `/tmp/bevy-optimization-mesh-phases-final-harden.log`.
`13fd91a` contains only the fixture readiness correction; `274562f` contains the
phase resource and capture reporting. The release capture build and wasm
compilation pass (`/tmp/bevy-optimization-mesh-phases-{build,check-web}.log`).
The timers are diagnostic instrumentation, not another production speedup.

### Native phase evidence

`/tmp/bevy-optimization-mesh-phase-captures.IQlaun/` records three release
production-playback processes at `d8a4dfe`, using the full-object cow/bird views
from the classification comparison and the default subset view. Each process
has 30 animated-ready warmups and 300 measured frames, with RTX 4080/Vulkan,
validation enabled, forward rendering, 4x MSAA and 1280x720. No task-owned build
overlaps. Recorded load is lower than the preceding comparisons, 4.69–6.38;
these runs locate costs, not demonstrate a new speedup or clear failed repeats.

The profiled executable SHA-256 is
`f24a32760a843c84421771b7c80b5c66c01e6d097501423239154fb43f1c65e6`.
All three settled final poses match the saved pre-phase-cache executable
`5db8900aa0fb0b8763904f5c16f4621b4e96e451fac41e161db6c6b7f4828b6e`
at their actual final clocks, with zero changed RGB pixels at tolerance zero.
The candidate images were inspected. All 900 raw frame/clock rows and published
pass arrays validate, maximum clock error 5.685e-14. Phase/job counts agree with
the enclosing instrumentation, no tangent job fails, and non-overlapping phase
sums fit within the complete batch timer. Raw and phase analyses are retained
as `raw-analysis.json` and `phase-analysis.json`; asset manifests verify.
All process/log checks pass, but the missing cow MDL warning still yields
diagnostic driver status 1.

These phase averages divide all batch durations, including warmup, by the
330 changed-root updates. They are not medians of the 300-frame measured window.

| Mean per changed update, ms | Cow | Bird | Subsets |
| --- | ---: | ---: | ---: |
| Eligibility, USD mesh/skin sampling | 7.775 | 11.568 | 0.024 |
| CPU deformation | 3.298 | 1.891 | 0 |
| Assembly | 1.080 | 0.267 | 0 |
| Tangent-cache lookup | 0.114 | 0.058 | 0 |
| Tangent dispatch/join | 31.003 | 6.325 | 0.001 |
| Complete preparation | 43.283 | 20.127 | 0.027 |

Cow dispatch/join is 71.6% of batch time, while bird eligibility is 57.5%.
This rules out treating tangent generation as the sole next target for both
assets. At this checkpoint eligibility includes deformation-kind queries,
decoded mesh reads and GPU skin sampling/normal-correction preparation.
The subsequent split and scoped relationship-reuse candidate are recorded in
[skeleton relationship sampling](skel-relationships.md).

Cow runs 4,620 tangent jobs over 35,374,680 submitted vertices, with 17,266.715 ms
aggregate worker wall time and 17.268 ms maximum individual job. Bird runs
13,528 jobs over 5,552,860 vertices, with 2,903.800 ms aggregate worker wall time
and 8.995 ms maximum job. Those overlapping job times are not thread CPU time,
not additive to dispatch/join, and do not by themselves isolate scheduling
overhead. Subsets submit no tangent jobs: their earlier whole-update regression
cannot be explained by expensive tangent generation.

Measured main median/p95 are 51.150/56.902 ms for cow and 24.547/28.497 ms for
bird. Their offscreen interval medians are 52.765 and 25.223 ms, so both still
miss 60 Hz even in this lower-load observation. This does not replace the
unaccepted alternating comparisons, full opening, native-window pacing,
browser runtime, lifecycle or strict CPU/GPU image gates.

## Execution contract

Only built-in animation batches use preparation. Public single-prim projection
and patch calls stay synchronous. Custom registries, missing/single-threaded
compute pools, zero limits, deferred-hidden residency and residency budgets use
the existing serial route path. Initial scene projection is unchanged.

The first implementation reads USD and assembles owned deformed meshes for up
to eight prim contexts at a time. Default limits admit four tangent jobs and 16 MiB of
worker input mesh payload per batch. This is not a total-memory bound: decoded
USD arrays, deformed reads, tangent outputs and MikkTSpace scratch are additional.
An oversized input stays in its context for serial generation without repeating
its deformation or assembly. No worker sees a stage, World or asset handle.

Cache lookup and recurrence admission stay on the main thread. Misses run the
same Bevy MikkTSpace function on separate meshes in a scoped compute-pool call.
All tasks join before publication; results finish in input order. Routes then
run in their original prim and registry order, including material, skin, subset
and instancing consumers. A batch may observe two misses for equal inputs before
either result is cached; this does not bypass exact input equality or the cache
byte budget, but cache accounting/performance requires measurement.

Prepared GPU samples, failures and deformed meshes are consumed only by their
own context. They are not retained across clocks or independently timed roots.
Both editor animation and `UsdAssetPlugin` instance-clock updates use batching.
GPU morph-only and subdivided paths retain their existing preparation. Missing
or invalid deformation keeps the existing fallback/error behavior.

`MeshPreparationTiming` records complete prepass/dispatch/join wall time, job
counts, cache hits and maximum worker input payload. `editor_benchmark` reports
and resets it alongside route timings. Moving work out of SkinRoute does not
make it disappear from the reported update interval; compare whole updates and
the separate batch time, not reduced per-route times alone.

## Required evidence

- Byte-exact serial/batched geometry, including morph attributes, across unique,
  reverse and repeated clocks; UV, normal, orientation, method and influence edits.
- Actual two-thread executor coverage; missing/single-thread/zero/oversized and
  residency fallbacks; custom routes retaining synchronous stage visibility.
- Independent production-plugin roots, subsets, source reload and close cleanup.
- Repository hardening, native oracle/watchers/export and wasm checks.
- Alternating saved release binaries on bird, cow, CPU/GPU deformation and small
  controls, including unique clocks, retained payload and peak RSS. No task-owned
  compilation during comparisons; retain host-load observations and raw samples.
- Before/after native renders and actual interactive cadence. The existing cow
  MDL diagnostic and strict bird CPU/GPU mismatch remain unresolved gates.

## Initial verification

`0035a9e` splits cache lookup/admission from result insertion without changing
the serial generator. `7483e05` adds batch preparation and three regression
tests. `d740239` reports its complete batch cost in the benchmark. The initial
workspace/all-target run passes 889 tests with 20 ignored:
`/tmp/bevy-optimization-parallel-tests.log`.

Further production-plugin coverage is still being verified. Its first run
fails because the new test used `UsdSource::snapshot` for a wrapper referencing
filesystem fixtures: snapshots deliberately disallow unresolved filesystem
dependencies. The serial control fails before any parallel comparison. The
test now uses the filesystem-backed constructor and an explicit unlimited
initial projection budget; this is not a change to production loading policy.
The original failure is retained in
`/tmp/bevy-optimization-parallel-harden.log`.

The next run passes the new lifecycle test but exposes an existing geometry
fixture assuming both roots finish within one shared 10 ms loading turn. The
missing `/PI` is in a still-projecting serial-control root; initial projection
does not use the new batching path. `c2e3142` gives that geometry-only test an
explicit unlimited budget and asserts both roots are Ready before inspecting
them. It does not change production budgets. The failed run is retained in
`/tmp/bevy-optimization-parallel-harden-fixed.log`.

`51da2f4` completes hidden-residency and production-instance tests. The final
hardening rerun passes 803 no-default tests (21 ignored), 897 all-feature/
all-target tests (29 ignored), strict Clippy, formatting and whitespace:
`/tmp/bevy-optimization-parallel-harden-recheck.log`. Coverage includes CPU and
GPU routes, live/reversed root clocks, referenced geometry, subsets, source
replacement, closing one root without affecting another, and no further jobs
after both roots close. Native oracle/watchers/export, wasm and new captures
still need rerunning for this production change.

## Release experiment target

Before running the first comparison, the focused performance target is at least
15% lower cow GPU unique-clock median update cost, with no repeatable greater
than 5% regression on bird or small deformation controls. This is a checkpoint
target, not a replacement for the native 60 Hz objective. Lower per-route time
alone does not count: use complete update samples, account for the batch wall
time and inspect process CPU, retained geometry and peak RSS. Exact outputs and
bounded worker inputs remain required regardless of elapsed results. Concurrent
host load must be retained; no task-owned compilation may overlap comparisons.

## Sparse-batch correction

The first saved-binary matrix completes all 24 processes under
`/tmp/bevy-optimization-parallel-abba.m3FZjw/`: six workloads, ABBA order, four
warmups and 1,000 unique authored-range clocks per process. Before production is
`c9006b3`, after production is `7483e05` (recorded HEAD `d4c69a0`). No task-owned
compilation overlaps. Unrelated host jobs are untouched; load averages range
20.23–80.51, so timing noise is substantial. All 24 final geometry/image payload
counts, peak asset counts and retained cache payload counts match within each
workload (`payload-check.txt`); these counters are not byte-exact image proof.
Entry/package and cow-directory hashes verify after the run.

| Unique-clock median updates (ms) | Before range | First candidate range |
| --- | ---: | ---: |
| Cow GPU | 80.829–187.530 | 147.850–187.296 |
| Bird GPU | 30.861–43.193 | 30.438–50.026 |
| Morph GPU | 0.602–0.879 | 0.786–0.934 |
| Cow CPU | 75.014–100.058 | 76.196–79.273 |
| Bird CPU | 44.380–45.115 | 30.907–48.654 |
| Morph CPU | 0.824–0.902 | 0.757–1.478 |

The cow GPU target is not met. Do not accept this version as a speedup or excuse
the result solely through host load. Raw samples, means, tails, warmups, process
CPU and peak RSS remain in the logs, `summary.tsv` and `cpu-cache.tsv`. Peak RSS
is at most 551.805 MB for candidate cow GPU versus 551.150 MB before, and 978.399
MB for candidate bird GPU versus 974.856 MB before; this is not a memory saving.

The candidate's counters expose a structural problem independent of timing
noise: 21,000 batches for only 13,986 jobs across 1,000 cow clocks, with at most
three jobs in any batch. Eight *prims* rarely contain several of the cow's 14
animated meshes among 164 animated prims. Most nonempty batches therefore move
serial work onto another thread without providing meaningful concurrency.

`2900832` scans at most 128 lightweight prim contexts and ends
each publication group after at most four mesh prims by default (hard cap eight).
Mesh prims count toward that bound even when they fail, hit the cache or need no
tangents. Worker inputs retain the same byte cap. A single job runs inline; with
several jobs the caller executes the largest input while other jobs run on the
pool. It does not execute arbitrary global-pool work on the caller. Result
publication still follows original prim/route order. A sparse-scene regression
test requires four meshes separated by 20 non-mesh prims each to share one batch.
It also checks the eight-mesh hard cap with an unlimited caller job limit and
zero tangent-cache budget. Parallel and inline generation counts distinguish
real concurrency from merely moving a single job. Deformation-kind decisions
are kept within the same read-only context and reused during route publication
instead of querying those eligibility conditions again.

The correction was not compiled during the first comparison. The saved binaries
stayed immutable; its hardening/build rerun begins only after all 24 processes
finish. Hardening passes 804 no-default tests (21 ignored), 899 all-feature/
all-target tests (29 ignored), strict Clippy, formatting and whitespace:
`/tmp/bevy-optimization-parallel-grouped-harden.log`. The release benchmark
build completes too (`parallel-grouped-build.log`). `8a06876` records parallel
versus inline work in both benchmark and capture metadata, including captures
that reverse clocks on existing scene roots. No elapsed or render result for
the correction is accepted yet. Its first release comparison isolates the
primary cow GPU gate; the other controls remain required afterward.

## Corrected cow experiment

The four-process cow GPU ABBA run completes under
`/tmp/bevy-optimization-parallel-grouped-abba.ucCKEr/`, against the same saved
`c9006b3` baseline. Each process has four warmups and 1,000 unique clocks. No
task-owned compilation overlaps; host load ranges 18.33–52.65. All final
geometry/image payload counts, peak asset counts and retained cache payload
counts agree, and entry/package/cow-directory hashes verify.

Before medians are 89.013 and 81.714 ms; corrected medians are 93.076 and
60.636 ms. Averaging those per-process medians gives 85.364 versus 76.856 ms,
about 9.97% lower, **not the predeclared 15% target**. One candidate run is
promising but the other is slower; neither this aggregate nor the faster single
run establishes accepted native playback performance. Full means, tails,
warmups and process CPU remain in `summary.tsv`, `cpu-cache.tsv` and raw logs.

Utilization is corrected: both candidates perform 3,996 parallel batches and
13,986 jobs, with zero inline jobs during measured seeks, peak four jobs and at
most 1,682,992 input bytes. Total batches fall from 21,000 to 4,000–5,000. Whole
batch wall time is still 49.331–78.619 seconds across the 1,000 seeks. The 127 MB
resident reading in one candidate is below its retained image payload and its
554 MB process high-water mark; do not call it a memory saving.

Other corrected performance workloads and native cadence remain pending.
The next timing probe needs tighter temporal pairing of serial/batched work
rather than treating drift between long processes as an algorithmic gain.

## Corrected native and render validation

Four native deformation oracle checks, nine watcher checks, 15 native export
checks and wasm compilation pass. Logs are
`/tmp/bevy-optimization-parallel-grouped-{test-native-deformation,test-watchers,test-native,check-web}.log`;
the status ledger is `parallel-grouped-gates.tsv`. Wasm compilation is not browser
runtime acceptance.

`/tmp/bevy-optimization-parallel-captures.Zpsb1g/` contains six saved-binary
before/after comparisons: bird, cow and combined animated skin/morph fixture,
each with explicit GPU and CPU routes. Three independently timed roots load at
0/5/10; after 30 ready frames, their clocks reverse to 10/5/0 before capture.
All six comparisons are pixel-exact at RGB tolerance zero. All 12 images have
compiled-pipeline/readback and live-clock-reversal proof. After metadata confirms
actual parallel batches for bird/cow and inline generation for the single-mesh
morph fixture. Entry hashes verify. The device is RTX 4080, Vulkan/NVIDIA
595.112.07, at 1280x720, forward rendering, 4x MSAA with WGPU validation.

The bird/cow images were inspected: all three bodies render, although the nearest
bird's wing reaches outside the frame. These are scoped comparisons, not complete
camera-angle coverage or native frame-cadence evidence. Both cow modes still
warn about the missing `OmniPBR.mdl` dependency, in both binaries; the combined
capture gate therefore exits 1 rather than being called entirely green.

The strict CPU/GPU comparison of the three-bird view fails at one pixel, maximum
RGB error 3, tolerance one. That different framing does not replace the original
single-bird gate. The original single-bird time-zero GPU and CPU captures were
rerun with the corrected binary: **2,053 pixels still fail**, maximum RGB error
33, tolerance one. Its inspected diff remains on grid lines through the wing
transparency. This is unchanged from the preceding checkpoint and unresolved;
neither comparison tolerance nor grid visibility was changed.

## Next work

- Add a tightly paired elapsed probe using independent serial/batched editor
  apps, identical clocks and balanced first-update order, preserving document
  identity checks and raw samples. Identify it as a same-binary scheduling
  comparison, not a replacement for saved-revision or native-frame evidence.
- Complete corrected bird/CPU/small-control comparisons and establish an actual
  elapsed benefit before accepting the parallel implementation as an optimization.
- Resolve the strict rendering discrepancy with observed phase/bounds evidence;
  do not hide the grid, relax tolerance or mutate shared mesh bounds blindly.
- Continue the complete loading, editor, memory/lifecycle and platform workstreams
  in `OPTIMIZATION.md`; this checkpoint does not finish the overall objective.

## Tightly paired diagnostic

`editor_benchmark ASSET 2 gpu-prepared seek-timeline-paired` compares the current
serial control (`ParallelMeshPreparation.max_jobs = 0`) with the default batched
path in two independent editor apps. Each app owns its document and caches. Both
receive four warmups followed by the same 1,000 distinct authored-range clocks.
At every clock the two complete `App::update` intervals run sequentially, with
the first app balanced serial/batched/batched/serial across each four clocks.
The second outer sample reverses both app construction and first-update order.
Raw paired clocks, order and nanoseconds are always retained, including warmup.
Every seek checks readiness, exact clock and unchanged document identity.

This is a same-binary scheduling diagnostic, not a historical revision comparison
or native frame-rate measurement. It excludes startup/open/idle from seek timing,
does not run two app updates concurrently, and reports whole-process RSS for the
two retained apps together. Payload counts and sampled peak asset counts are
reported separately; their equality does not establish byte-level geometry or
render parity. Existing saved-binary comparisons and native rendering gates remain
required. Eager projection is mandatory: deferred residency/budget overrides are
rejected rather than silently disabling the parallel control. A single-thread
compute pool can still take the production serial fallback; recorded job counts
must establish that a purported parallel experiment actually dispatched work.

The shared benchmark profile reset now also clears GPU palette-update, residency
and material timing counters after warmup. Previously these optional cumulative
profiles included open/idle work under the measured-seeks label; whole-update
samples and acceptance decisions never subtracted those counters.

### Paired release results

`79e80bc` passes hardening (804 no-default tests, 902 all-feature/all-target
tests, strict Clippy and formatting). The release matrix completes all six cases
under `/tmp/bevy-optimization-paired.yeD1V6/`: two independently reopened pairs
per case, 12,000 measured pairs / 24,000 updates. Source verification passes for
129 root/package entries and 267 cow-directory entries. Each pair has equal
final payload counts and sampled peak asset counts. No task-owned compilation
overlaps; unrelated host work remains untouched. Case-boundary one-minute load
readings range 6.36–41.48, not a continuously sampled load range.

| Case | Serial medians, ms | Batched medians, ms | Median cost reduction |
| --- | --- | --- | --- |
| Cow GPU-prepared | 79.106 / 87.989 | 56.477 / 68.709 | 28.6% / 21.9% |
| Bird GPU-prepared | 28.414 / 28.128 | 26.057 / 26.095 | 8.3% / 7.2% |
| Cow CPU | 78.521 / 77.075 | 56.724 / 49.237 | 27.8% / 36.1% |
| Bird CPU | 26.105 / 26.009 | 24.003 / 23.943 | 8.0% / 7.9% |
| Morph GPU-prepared | 0.555 / 0.806 | 0.573 / 0.811 | -3.3% / -0.6% |
| Morph CPU | 0.492 / 0.644 | 0.490 / 0.654 | 0.3% / -1.7% |

Cow GPU batching wins 984/1,000 and 913/1,000 individual pairs. Its benefit
persists in both first-update order strata and each 250-clock quarter: quarter
median reductions range 19.1–28.9%. `paired-analysis.tsv` retains these strata,
means, tails and median paired ratios; raw logs retain every clock and order.
Both primary paired medians exceed the predeclared 15% threshold, with no
repeatable >5% median regression in these controls. This supports keeping the
parallel path as a measured scheduling improvement. It does **not** retroactively
pass the failed saved-revision trial or establish native 60 Hz playback.

Reopened apps do not necessarily produce identical batch partitions. The second
cow GPU sample records 2,997 parallel batches plus 1,998 inline jobs, versus
3,996 parallel batches and no inline jobs in the first. Both process 13,986 jobs.
`AnimatedPrims` uses a hash set and the batching path preserves incoming prim
order while chunking lightweight descriptors, so independent app ordering is
another source of variation, not only host load. Future changes must preserve
publication semantics rather than silently reordering custom routes. Bird runs
record 9,980 parallel batches and 998 inline jobs; the one-mesh control uses only
inline generation. Serial controls record zero preparation jobs.

RSS is combined across two apps and affected by paging. In particular, one cow
reading drops below retained image payload while its process high-water mark
exceeds 1 GB; this is not a memory saving. Remaining requirements include
saved-revision control comparisons, actual native cadence, the unresolved strict
rendering discrepancy, and the complete workstreams in `OPTIMIZATION.md`.
