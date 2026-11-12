# Editor scheduling diagnostic results

## Result: execution variation remains; no candidate acceptance

The [declared diagnostic](editor-scheduler/README.md) runs only the original
editor. Twenty processes complete 20,000 measured updates and 80 warmups with
zero application diagnostics and exact paired clocks, payloads, preparation
work and animation operations. All identities and input hashes verify.

The scheduler analyzer validates 916 snapshots, each containing 25 threads,
and deltas for 428 measured blocks plus 20 warmup blocks. All twenty final
blocks retain their raw timings but have no post-exit scheduler snapshot, as
declared. The existing timing checks still fail 14 of 20 strata; this
instrumented run is diagnostic, not a replacement acceptance experiment.

Evidence: `/tmp/bevy-optimization-editor-scheduler.Meiu6C/`.
The same original executable remains
`8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1`.
Protocol/counter tests are at `90d8dca`, runner/analyzer at `82d5edf`, and the
input-stream correction below at `fce420d`. No editor build or source change
is needed for this measurement.

## Observed execution and migration

The main thread migrates in **424 of 428 measured blocks**. Only four blocks
have zero migrations and matching CPU endpoints; all four are core-class.
Endpoint CPU numbers cannot identify intermediate core/atom residency for
the other blocks. In particular, these observations do not prove that the
main thread moved between CPU classes rather than between CPUs of one class.

The table reports the migrating-block groups; the four nonmigrating blocks
remain separately represented in `perf/scheduler-analysis.json`.

| Workload | Migrating / observed blocks | Main CPU execution / external window | Counter capture / external window |
| --- | ---: | ---: | ---: |
| Bird | 195 / 196 | 97.16% | 0.40% |
| Cow | 194 / 196 | 99.24% | 0.16% |
| Robot arm | 11 / 12 | 95.53% | 1.28% |
| Morph | 12 / 12 | 92.17% | 1.82% |
| Material subsets | 12 / 12 | 88.58% | 3.31% |

These are summed CPU/window fractions, not medians of per-update ratios.
CPU accounting covers output, bookkeeping and observation windows as well as
updates, and sequential thread reads are not atomic. Worker CPU time is kept
separately and can overlap main-thread execution. It is not added to elapsed
time. Main CPU execution close to the external window means off-CPU time
alone does not describe the observed block costs; it does not identify the
instruction, frequency, cache or contention contribution.

Among equal-clock pairs with migrations on both main threads, block update
sum ratios range from 0.9163 to 1.4835 for bird and 0.6983 to 1.2409 for cow.
Their median main-CPU ratios are 1.0006 and 0.9992, respectively. All
individual CPU, elapsed, migration, voluntary/involuntary switch and capture
values remain recorded, including outliers. No observations are discarded
to manufacture stability.

Host load ranges from 2.69 to 5.55, lower than in the earlier control run, but
that is neither an idle-host guarantee nor a controlled load comparison.
Default affinity remains 0-23; all processes report sixteen compute-pool
threads. CPU maps remain core 0-15 and atom 16-23. No owned build/capture
overlaps, and unrelated jobs and machine settings are untouched.

`kernel.sched_schedstats` remains zero. Raw runqueue fields are retained, but
runqueue waiting is unavailable rather than treated as zero delay. External
window minus main CPU execution includes controller/IO and worker waits;
it must not be reported as scheduling latency. The field definitions are
documented in the [Linux scheduler statistics reference](https://docs.kernel.org/scheduler/sched-stats.html).

## Harness failure and verification

The first attempt, `/tmp/bevy-optimization-editor-scheduler.oAONbm/`, stops
before any warmup or measured sample. It expected a pipe-read wait channel,
but Node-provided child stdin is a Unix stream socket on this host, observed
as `S unix_stream_read_generic`. Both owned children are terminated and reaped;
the failed logs, zero-sample output and `82d5edf` source archive are retained.
The corrected snapshot recognizes that specific input-read channel as well as
pipe reads, not arbitrary sleeping states. A real owned shell child using
Node-provided stdin now exercises this case in the regression suite.

All eight scheduler tests and script syntax checks pass. Tests cover raw
identity/counter parsing, missing/unsafe values, changed identities/masks,
nonmonotonic counters, disabled runqueue accounting, migration endpoints,
separate worker CPU and live socket-backed input. Altered scheduler deltas
and process identities are rejected against the real diagnostic evidence.
The inherited clock, block-schedule, payload and doubled-timing negative
controls also pass.

```text
socket-regression.exit       = 0
driver.exit                  = 0
functional.exit              = 1  # fourteen timing strata fail
scheduler-analysis.exit      = 0
verification.exit            = 0
scheduler-verification.exit  = 0
```

The functional analyzer validates every raw sample, authored clock, readiness
marker, payload/work count and CSV quantile before reporting timing failures.
All 743 asset entries, runtime/vendor files, original build inputs and current
harness sources verify after measurement. No production source or dependency
diff exists against the saved runtime baseline. The earlier failed attempt's
source is retained by archive/commit, not falsely claimed to match the fixed
current harness.

## Next boundary

Do not build/adopt another tangent variation, rerun this unchanged for a
passing timing result, change host-wide scheduler settings or blame unrelated
jobs. The observations narrow the problem toward execution placement and
CPU-work variation, but frequent within-block migrations prevent class-specific
attribution from endpoint snapshots alone. A controlled follow-up needs
matched main-thread placement for sustained blocks, with workers/pool sizes
unchanged, or within-block PMU/CPU sampling that measures the missing residency.
The older single-update pinned experiment is not sustained-block evidence and
its failure remains recorded.

The tangent candidate's kernel gains and cow p95 risk remain unchanged and
isolated. Downstream packaging, native visual/reload/export, lifecycle/memory,
browser and broader optimization acceptance remain open. Nothing is pushed.
