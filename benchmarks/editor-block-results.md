# Sustained-block editor control results

## Outcome: controls fail; candidate remains isolated

Twenty unchanged-editor processes complete 20,000 measured updates and 80
warmups, with matching clocks, payloads, work counts and animation operations.
All processes exit zero without application diagnostics. The predeclared
timing gate nevertheless fails **15 of 20 order strata**. This is not an
original-versus-candidate run and cannot establish a candidate speedup or
regression. No production source or dependency is changed.

The [protocol](editor-blocks/README.md) was committed at `8ce7976` before
measurement. The driver/protocol tests are at `a0b0529`; analysis and Make
integration are at `e1175c5`. Four protocol tests pass, including explicit ABBA
ordering, complete buffered-command consumption and rejection of malformed or
out-of-order events. Driver, analyzer and verifier syntax checks pass.

Evidence: `/tmp/bevy-optimization-editor-blocks.kvoNGn/`.

## Timing evidence

Each workload has two fresh process pairs, each split into two 500-clock
first-block-order strata. Every stratum must keep the median paired
right/left ratio within 1% of unity and the p95 ratio within 5%. Negative
changes are not candidate improvements: both processes run identical code.

| Workload | Failed strata | Median paired change range | p95 change range |
| --- | ---: | ---: | ---: |
| Bird | 1 / 4 | -0.57% to -0.06% | -5.14% to +2.01% |
| Cow | 2 / 4 | -0.62% to +0.31% | -5.20% to +10.58% |
| Robot arm | 4 / 4 | -1.17% to +0.13% | -30.08% to -7.41% |
| Morph | 4 / 4 | -4.52% to -0.10% | -21.55% to +19.38% |
| Material subsets | 4 / 4 | -20.46% to +8.01% | -2.56% to +37.44% |

All eight bird/cow median comparisons meet the declared limit, but three of
their tail comparisons fail. Every light-workload stratum fails at least one
limit. These results do not qualify the protocol by workload selection,
pooling away order strata, dropping resumed updates or rounding near misses.
The complete raw samples, individual process quantiles, opening/idle costs,
peak memory and block wall intervals remain in `perf/` and `analysis.json`.

One-minute host load ranges from 6.98 to 10.80 across the thirty retained
snapshots. Unrelated builds remain running and untouched; no owned builds or
captures overlap measurement. Main-thread affinity remains 0-23, and every
process reports sixteen compute-pool threads. The data show instability but
do not identify whether host contention, scheduling, process state or another
factor causes it. No causal attribution follows from host load alone.

## Integrity and verifier evidence

Both lanes use the same saved original executable, including the same on-disk
path, throughout the experiment:

```text
8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1
```

Actual executable hashes, PID/start identities and default main-thread
affinity match at startup, after warmup, midpoint and before the last block.
Every readiness marker and authored clock is checked, including all first and
resumed updates. Block intervals are ordered and nonoverlapping, and their
recorded update sums match the raw samples. CSV quantiles and warmup totals
match those samples. All twenty process IDs are distinct.

Every paired payload field, non-timing preparation count and animation-access
operation matches. There are no dropped or saturated animation records and
no query-cache records. All 743 asset-manifest entries, original build inputs,
tracked runtime/vendor sources and the premeasurement harness manifest verify
after measurement. Runtime source still matches the saved baseline at
`7d1dbfd`; root manifests and dependency pins are unchanged.

The negative controls reject an altered clock, changed block schedule and
changed payload. Doubling right-lane timings fails all twenty timing strata.
The verifier passing does not turn the actual control failures into passes:

```text
harness-tests-final.exit = 0
driver.exit              = 0
analysis.exit            = 1
verification.exit        = 0
```

## Consequences and remaining scope

Do not build/adopt the normal-keyed tangent candidate on the strength of these
controls, repeat this unchanged until it passes or relax the declared limits.
Its [kernel result](tangent-normal-memo.md) remains separate: cow/bird medians
improve 17.16% / 22.44%, while cow p95 increases 19.93%. Neither establishes
whole-editor frame pacing or GPU performance.

Before another elapsed comparison, obtain evidence that distinguishes
scheduling/host interference from application work and demonstrates a stable
measurement environment. Additional kernel variations are not the next step.
The full optimization goal also retains independent downstream packaging,
native visual, reload/export, lifecycle/memory and browser acceptance work.
This failed timing qualification neither closes those tasks nor prevents
their correctness work. No release, push or desktop automation occurs.
