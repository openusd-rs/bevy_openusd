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
