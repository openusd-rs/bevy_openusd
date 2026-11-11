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
