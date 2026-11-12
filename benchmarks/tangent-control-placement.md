# Tangent control CPU placement diagnostic

The [full-editor comparison](tangent-projected-groups.md#full-editor-result-cow-improves-control-gates-fail)
passes cow's median target but fails GPU-morph and subset controls. The latter
executes no measured tangent jobs, while the morph reader interval also grows
at identical decode counts. Both executables use the same compiler/linker.
The host has core and atom CPU classes; the failed run did not record thread
placement. None of this proves a scheduling cause or clears those failures.

## Protocol

Use the same saved release executables, without rebuilding or changing
production code. Keep eager residency, default affinity, profiling flags,
1,000 unique authored-range clocks and four warmups per fresh process. Leave
unrelated jobs and host settings untouched. No owned build or capture overlaps.

For GPU morph and material subsets, use before/after/after/before in each of
two separate instrumented passes:

1. `perf stat --no-scale` collects instructions/cycles from both hybrid PMUs,
   software task-clock and raw scheduler counters across inherited threads.
   Require complete running-time coverage across the two PMUs. User-filtered
   context-switch/migration counts cannot establish scheduler behavior.
2. `perf record` samples user CPU-clock at 999 Hz with CPU IDs, without stack
   unwinding. Validate lost-event/period totals, the owned process identity and
   CPU-class membership. Report all-thread and main-thread placement separately
   (`tid == pid` identifies this editor executable's main thread).

Both instrumentation scopes include opening, idle, warmups, seeks and shutdown,
not only the timed updates. Sampling measures observed user CPU execution,
not continuous wall-time residency. Counter and sample passes are different
processes; do not combine them as if they describe the same execution.

Validate all 16 processes, 16,000 update samples and 64 warmups, exact clocks,
raw summaries, matched geometry/image/cache payload and work counts. Preserve
every timing, diagnostic, coverage failure, source/asset hash and host snapshot.
Compare instruction totals and CPU-class distributions; do not use instrumented
wall times as a replacement performance gate, waive the earlier failures or
repeat an unchanged matrix until a favorable result appears.

A placement correlation is not proof of the cause of the earlier unrecorded
run. If it supports a controlled follow-up, declare that experiment separately.
This diagnostic does not enable the candidate or establish native FPS,
rendered correctness, memory/lifecycle or release acceptance.

## Default-affinity diagnostic result

Evidence: `/tmp/bevy-optimization-tangent-placement.qt4AlU/`.
All 16 processes pass with zero matched diagnostics; the independent analyzer
validates 16,000 measured updates and 64 warmups, payload/work equality,
complete unscaled PMU running-time coverage and zero lost CPU samples.

| Control | User instructions before | User instructions after | Change | Stat-run update median change | Sample-run update median change |
| --- | ---: | ---: | ---: | ---: | ---: |
| GPU morph | 3,416,741,473 | 3,416,475,333 | -0.0078% | +0.21% | -0.68% |
| Subsets | 1,854,306,386 | 1,854,376,057 | +0.0038% | +1.43% | -0.81% |

These are means of two complete-process counts per binary, not seek-only
instruction counts. Counter-run task-clock changes -1.09% / +1.37% and cycles
-0.40% / +0.67% for morph/subsets. The large earlier median regressions are not
reproduced in these instrumented runs; this does not erase them or establish
instrumentation overhead relative to the separate uninstrumented experiment.
New opening/idle/tail regressions also remain in the full `analysis.json`.

The eight sampling runs have 161–376 main-thread samples each. The main
thread's atom-class sampled-period fractions are 3.53%, 8.87%, 6.38%, 8.53%
for morph and 1.73%, 0.60%, 1.86%, 4.68% for subsets, in ABBA order. Thus
these fast instrumented runs predominantly sample the main thread on the
core-class CPUs. That does not reveal where the earlier slow runs executed.
All-thread atom fractions are retained separately, not substituted for main
thread placement. No substantial extra instruction count explains the earlier
55.57% / 27.77% wall-time observations in this diagnostic.

## Declared same-binary core-placement control

Next use only the unchanged original executable for a controlled placement
experiment. Run each control in core/atom/atom/core order, binding only each
owned child process and its threads to logical CPU 0 or 16 respectively.
Keep the same source, asset, clocks, instrumentation and GPU-prepared mode.
Both one-CPU masks expose the same available parallelism; record the mask and
CPU count, and require complete counters confined to the selected PMU class.
No unrelated process, global affinity policy or hardware setting changes.

This changes placement intentionally and is not a default-affinity performance
gate. It tests whether unchanged code can exhibit a large timing difference
under a controlled placement change; do not credit any result to the tangent
candidate, claim pure microarchitectural causation independent of core load/
frequency, or infer the unrecorded placement of the original failed run.
Preserve both classes' full timings, counts, payload and diagnostics. A useful
result should inform a placement-aware comparison, not waive existing gates.

## Same-binary placement result

The `pinned/` driver and analyzer both exit zero. All eight processes validate
8,000 measured updates and 32 warmups, matching payload/work counts and zero
diagnostics. Each child reports exactly its selected one-CPU mask and one
available processor. Unscaled PMU running-time coverage is complete and 100%
on the selected class; the other class contributes zero runtime. Only the
unchanged original executable is run in this experiment.

| Control | Core CPU 0 median ms | Atom CPU 16 median ms | Median change | Task-clock change | Instruction change |
| --- | ---: | ---: | ---: | ---: | ---: |
| GPU morph | 0.355908 | 0.577999 | +62.40% | +62.44% | +0.0467% |
| Subsets | 0.165242 | 0.321512 | +94.57% | +92.69% | +0.0321% |

Both atom runs reproduce the slower level: morph medians 576.919 / 579.078 us
versus 355.596 / 356.219 us on the core CPU; subsets 321.604 / 321.419 us
versus 160.816 / 169.668 us. All p95/max/opening/idle/RSS metrics, frequency
snapshots, counters and the exact per-process input identities remain in
`pinned/analysis.json`, logs and manifests. No candidate speedup is measured
here. CPU class, individual-core load/frequency and placement are not separated
from one another, so do not describe this as a pure microarchitecture result.

This establishes that changing placement alone for unchanged code can produce
large wall-time changes in these controls. It does **not** identify the earlier
uninstrumented run's placement or clear its failed acceptance gate. The
one-CPU masks also constrain startup parallelism relative to the default;
this is not evidence for the normal worker-pool configuration.

## Next controlled comparison

Use the existing stepped editor protocol to initialize both original and
candidate with their default affinity and normal worker pools. At the first
ready marker, validate the owned benchmark PID/executable, then bind **only
its main thread** to the same selected CPU for both versions. Preserve and
verify every observed non-main thread's original affinity and the reported
pool-thread counts. Apply this separately on core CPU 0 and atom CPU 16.

Alternate individual equal-clock updates in ABBA order and reverse startup/
first-update order in a second fresh pair. Retain an identical-binary control,
all clocks, readiness, payload, per-order results and mask snapshots. Never
pin unrelated processes or replace this with a globally single-threaded app.
This can test the implementation with matched main-thread placement while
preserving normal worker pools. It remains a controlled diagnostic, not an
automatic waiver of the prior default-affinity failures or proof of native FPS.

The bounded follow-up covers GPU morph and subsets, each on CPU 0 and CPU 16,
with two reversed-startup pairs per placement. First run eight identical-original
pairs, then eight original/candidate pairs: 32 processes, 32,000 measured updates
and 128 warmups. Within each pair, alternate individual updates in ABBA order.
Keep startup, warmup, midpoint and pre-final affinity snapshots; reject changed
worker masks, inherited single-CPU worker masks, mismatched pool counts, input
identities, clocks, payload/work counts, diagnostics or incomplete processes.

Analyze both first-update orders separately (500 measured pairs each) in each
fresh-process pair. Require all identical-binary strata's median paired ratios
to lie within 0.95–1.05 before interpreting the candidate. The candidate meets
this diagnostic's median-control bound only if no stratum exceeds 1.05; retain
p95/max, opening/idle and memory observations independently. A failed identical-
binary control makes the comparison inconclusive rather than authorizing a
retry, discarded samples or looser bounds. Regardless of outcome, this is
headless, explicitly paced evidence under controlled main-thread placement,
not the default-affinity or native-rendering acceptance gate.

## Main-thread-only result: identical control fails

Evidence: `/tmp/bevy-optimization-tangent-main-affinity.sjRXbX/`.
The driver completes all 32 processes, 32,000 measured updates and 128 warmups
with no application diagnostics. Each process reports 16 compute-pool threads;
all 160 affinity snapshots contain 25 total threads. Startup masks are 0–23,
only the verified owned main TID changes to CPU 0 or 16, and every observed
worker retains 0–23. Post-warmup, midpoint and pre-final snapshots also place
the main thread on its selected CPU. The controller's affinity is unchanged.
The executable identities, manifests, recursive fixture-layer hashes, clocks,
payload/work counts and raw-to-summary timing checks pass. The analyzer's
negative control changes one paired clock and correctly fails that comparison.

The diagnostic still fails: two identical-original strata exceed the declared
5% bound, at +5.36% for subsets and +5.28% for GPU morph on CPU 0. Six candidate
strata also exceed it. Consequently `driver=0`, `analysis=1`, with the expected
`negative_control=1`; this comparison cannot qualify the optimization.

| Pair type | Control | CPU | Whole-run median change | Range of order-stratified median paired changes |
| --- | --- | ---: | ---: | ---: |
| Identical original | GPU morph | 0 | -0.18% | -4.25% to +5.28% |
| Identical original | GPU morph | 16 | -0.07% | -4.13% to +4.26% |
| Identical original | Subsets | 0 | +0.29% | -4.78% to +5.36% |
| Identical original | Subsets | 16 | +0.45% | -3.48% to +4.54% |
| Original/candidate | GPU morph | 0 | +0.26% | -8.34% to +8.78% |
| Original/candidate | GPU morph | 16 | -0.07% | -5.16% to +4.94% |
| Original/candidate | Subsets | 0 | +0.18% | -9.32% to +11.77% |
| Original/candidate | Subsets | 16 | +0.71% | -4.29% to +6.13% |

Whole-run changes compare the means of the two fresh-process medians. Each
stratum instead contains 500 equal-clock pairs with one first-update order;
there are four strata per table row. In every pair the second-running lane has
the higher median paired cost, including identical binaries. Pool/placement
control therefore does not remove order sensitivity in this explicitly paced
protocol. Do not pool it away, subtract the identical phase from the candidate
phase, or infer the cause of the earlier unrecorded default-affinity failures.

The complete p95/max/opening/idle/RSS observations remain in `analysis.json`,
including candidate GPU-morph CPU-0 maximum +30.64%, subsets CPU-0 maximum
+11.02% and subsets CPU-16 idle +32.12%. Opening/idle precede the main-thread
pin and are not matched-placement measurements. All 32 host snapshots retain
load/pressure and matching build/viewer process inventories; those inventories
are empty at the sampled boundaries, not proof of exclusive host use.

No production source, dependency or worker policy changes. The candidate stays
unintegrated, all earlier failed gates remain open, and no further unchanged
placement or editor batch is authorized by this result. Further optimization
should target another measured cost, such as OpenUSD sample decoding/copying,
rather than treating an inconclusive timing protocol as implementation proof.
