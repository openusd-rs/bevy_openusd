# Editor scheduling diagnostic

The sustained-block identical-editor controls fail 15 of 20 strata. Diagnose
the execution environment rather than repeat those controls for acceptance.
Run the same original executable and declared five-workload, reversed-pair,
ABBA block schedule, now recording per-thread Linux scheduler counters at
every nonterminal block boundary. No editor source, affinity, pool sizing,
host settings or unrelated processes change. No owned build/capture overlaps.

`/proc/PID/task/TID/schedstat` reports CPU execution and runqueue waiting
counters, as described in the [kernel documentation](https://docs.kernel.org/scheduler/sched-stats.html).
However, this host has `kernel.sched_schedstats=0`. Do not interpret a zero or
stale runqueue counter as no scheduling delay, or enable that global setting.
Record its raw value but report runqueue waiting as unavailable. CPU execution,
migration and voluntary/involuntary switch counters are retained separately.

Before sending each block and after its ending ready marker, wait for the
owned main thread to block in its input pipe, then read all thread counters.
Keep raw stat/schedstat/sched records, identity, read intervals and affinity.
Validate stable task identities, monotonic counters and unchanged task sets.
Record last-CPU endpoints and migration deltas; only zero-migration blocks
with matching endpoints can be classified as staying on one CPU class.
Capture both hybrid PMU CPU maps and verify them after the run.

All 20 processes retain every one of their 1,000 measured updates and four
warmups. The final block exits without a following ready marker, so explicitly
mark its scheduler snapshot unavailable, without dropping its timings or
pretending to sample a dead process. This leaves 428 measured blocks and 20
warmup blocks with scheduler deltas, and 20 retained terminal blocks without
them. The existing functional analyzer must still verify all 20,000 updates,
80 warmups, payload/work equality, raw quantiles and zero process diagnostics.

CPU deltas cover the external block interval, including output/bookkeeping and
capture overhead, not only `app.update()`. Record main and worker execution
separately. Window time minus main CPU execution also includes stdin/output,
controller scheduling and waits for worker tasks: it is **not** a runqueue
latency estimate. Sequential per-thread reads are not atomic snapshots.

Compare equal-clock block pairs and retain all migration/CPU-class categories,
including first/resumed updates. Report CPU execution, measured update sums,
wall windows, switch counts and counter-read overhead. This diagnostic cannot
waive failed controls, qualify a candidate or establish native FPS; any future
acceptance experiment requires separately declared stable-environment evidence.
No candidate source is built or adopted in this diagnostic.
