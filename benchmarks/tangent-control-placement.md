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
