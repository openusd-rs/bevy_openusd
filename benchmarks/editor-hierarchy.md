# Editor hierarchy sampling

## Evidence and scope

The broad release characterization at `8b664bc` uses the saved `79e80bc`
headless executable; subsequent production changes concern render-only posed
sorting. Evidence is `/tmp/bevy-optimization-broad-profile.tOEza4/`. Kitchen,
instanced city, UR10 arm, showcase and subset workloads all complete, with
entry/package hashes verified. External reference closures are not fully hashed.
No task-owned build overlaps. Unrelated host load is high: case-boundary
one-minute averages range 44.84–50.13, so these are diagnostic timings rather
than an isolated latency or speedup result.

Across three arm samples, 3,015 command-driven snapshots take 1,980.99 ms;
3,000 measured complete seek updates take 6,768.62 ms. The snapshot total also
includes opening/four warmups per sample, so their approximately 29% ratio is
an attribution aid, not a precise non-overlapping percentage. Kitchen initial
snapshots take 13.6–59.2 ms for 2,742 prims. Initial kitchen composition
validation and mesh projection are larger separate costs, not solved here.

`EditorSession::snapshot_at` currently traverses the complete hierarchy and
queries every prim's visibility for every seek/selection. The owned public
snapshot must remain complete. Selected attributes, sampled matrices, authoring
targets, undo state, save/conflict state and diagnostics must stay current.

## Bounded reuse under validation

Reuse only the current composed hierarchy paths and time-independent visibility
values within one editor session. Read animated visibility at each requested
time, including defaults, single-sample values and value clips. Classification
failures must fall back to a fresh read rather than introduce new snapshot
errors. Untimed defaults and numeric-time values have separate lazy cache slots;
an untimed value is never assumed to be the numeric value merely because sample
introspection reports zero samples. The existing full traversal/visibility
semantics remain the oracle.

Share the texture index's authored/structural revision and composition key:
layer identifiers and revisions, structural revision, load rules, population
mask and muted layers. Flush before checking it; saturated revisions never
permit reuse. Detect changes during a rebuild before retaining results.
Cache no selected-property values, historical poses, asset handles or files.
Retain at most 65,536 paths and 8 MiB of path text per session; larger hierarchies
use the uncached path. These are retained-entry/text limits, not a total allocator
or whole-process memory bound. The cache dies with the session.

## Predeclared acceptance

- Cached and uncached hierarchy/visibility results agree for default, forward,
  reverse and repeated clocks, including a single authored sample and clips.
- Direct layer edits, variants, payload/load rules, population masks, muting,
  namespace changes and document replacement cannot reuse stale hierarchy.
- Selected attributes/matrices, edit/undo/redo, failed inspection recovery and
  disk/reload safeguards retain existing behavior; hardening/native gates pass.
- Warm unchanged snapshots avoid stage-wide traversal/static visibility queries;
  bounds and uncached fallback have explicit regression coverage.
- Alternating saved release binaries target at least 25% less warm snapshot
  median time and 10% less arm seek-update median, with no repeatable greater than 5%
  opening/small-control median regression. Keep raw samples and host-load data;
  no task-owned compilation during comparisons. A failed target stays failed.
- This does not establish native frame cadence or complete the workstreams in
  `OPTIMIZATION.md`. Existing bird pixel failures and cow MDL warning remain.

## Initial implementation checkpoint

The first workspace/all-target test run passes 907 tests with 20 ignored:
`/tmp/bevy-optimization-inspection-tests.log`. Five new regressions compare
cached hierarchy/visibility with fresh stage reads across static/animated/single
samples, clips, direct edits, variants, payload rules, muting, undo/redo and both
retention limits. A subsequent hardening pass additionally separates default
and numeric cache slots and covers masked stages, native instance proxies,
activation/removal, document isolation and failed/repaired source reloads.
The subsequent `harden` run passes 817 no-default tests with 21 ignored and
916 all-feature/all-target tests with 29 ignored, strict Clippy, formatting and
whitespace checks. These are overlapping configurations, not 1,733 distinct
test cases. Log: `/tmp/bevy-optimization-inspection-harden.log`.

Subsequent native gates pass: four deformation-oracle tests, five asset-watcher
plus four editor-texture-watcher tests, fourteen native export tests plus the
reference custom-data round trip, and wasm compilation. The fresh release
benchmark build succeeds. Logs are
`/tmp/bevy-optimization-inspection-{test-native-deformation,test-watchers,test-native,check-web,after-build}.log`;
the status ledger is `/tmp/bevy-optimization-inspection-gates.tsv`. Native/browser
frame cadence and saved-release performance acceptance remain separate gates.

A fresh pre-change release executable is saved under
`/tmp/bevy-optimization-inspection-before.ylni3Q/`; the authoritative
path is `/tmp/bevy-optimization-inspection-before-path`. Its recorded revision
is `8b664bc`, so the comparison need not attribute unrelated changes
between an old historical executable and this focused patch.

## Alternating release results

`/tmp/bevy-optimization-inspection-abba.70KixS/` completes all 28 processes:
seven workloads, one fresh App per process, before/after/after/before order.
Before is `8b664bc`; after production is `6d4f8ae` (recorded HEAD `b06e8ec`).
Arm/showcase/bird/cow each use four warmups and 1,000 unique authored-range clocks;
kitchen/city/subsets use four warmups and 100 repeated 0/5/10/5 clocks. There are
17,200 measured updates in total. No task-owned compilation overlaps.

All seven entry/package hashes, both robot-directory files and 267 cow-directory
hashes verify. Every workload's retained geometry/image/cache payload counts and
peak asset counts agree across its four processes; these counters are not
byte-exact render proof. Case-boundary one-minute load averages range 9.62–71.49.
Raw samples, profiling logs, binary hashes and `analysis.tsv` are retained.
Timing below includes matching optional timers/logging in both binaries and
does not measure a native display or GPU frame.

| Workload | Before update medians, ms | After update medians, ms | Before snapshot medians, ms | After snapshot medians, ms |
| --- | --- | --- | --- | --- |
| Arm | 1.973 / 2.090 | 1.245 / 1.274 | 0.676 / 0.655 | 0.067 / 0.072 |
| Kitchen | 16.181 / 53.501 | 1.347 / 8.307 | 14.879 / 45.085 | 0.708 / 3.230 |
| City | 1,482.535 / 688.786 | 1,172.396 / 678.345 | 0.166 / 0.094 | 0.053 / 0.045 |
| Showcase | 0.926 / 0.937 | 0.890 / 0.856 | 0.114 / 0.114 | 0.054 / 0.053 |
| Subsets | 0.218 / 0.278 | 0.215 / 0.190 | 0.036 / 0.046 | 0.024 / 0.021 |
| Bird | 30.198 / 37.658 | 26.695 / 33.029 | 0.507 / 0.683 | 0.195 / 0.258 |
| Cow | 78.881 / 67.513 | 75.355 / 68.303 | 8.019 / 7.764 | 7.545 / 6.568 |

The average of the two arm update medians falls 38.0%; its snapshot counterpart
falls 89.6%. Both arm repetitions and all snapshot medians support the intended
reuse, exceeding the arm's predeclared 10% update / 25% snapshot targets. Kitchen
snapshot medians also improve substantially. Cow's snapshot reduction is only
10.6% by the same averaging rule, below 25%; this cache does not remove every
inspector cost. Do not attribute large bird/city wall-time differences solely
to the much smaller snapshot interval or ignore the varying host load.

The **complete regression gate is not accepted from this run**. Average opening
times increase 24.4% for kitchen, 28.2% for subsets and 19.7% for bird, beyond
the 5% limit. Untouched kitchen projection time itself varies 0.662–3.185 seconds,
so this experiment does not cleanly separate candidate overhead from host
variation; that does not erase its failed gate. An additional saved-binary,
open-only ABBA comparison with three fresh Apps per process follows below.
Preserve both results rather than replacing this one afterward.

Small-control process RSS is around 1.4–1.8 MB higher after the change, while
larger workloads overlap or vary with paging. This is not a memory saving, nor
proof that all extra RSS belongs to cached path text. Retained cache entries are
bounded and released with their session; whole-process/lifecycle memory remains
an independent acceptance workstream.

The city seeks reveal a separate dominant point-instancer bottleneck, not an
inspector problem. See [parent-link investigation](instancer-parenting.md) for
the measured route cost and source-grounded next experiment. Existing strict
bird image failures, the cow MDL warning and native cadence remain unresolved.

## Opening-only follow-up

`/tmp/bevy-optimization-inspection-openings.vx3THG/` completes twelve processes
and 36 fresh-App openings, using the same saved binaries and ABBA order. Each
workload has six opens per lane; every process exits zero, all three entry hashes
verify and retained payload/peak asset counts match. No task-owned compilation
overlaps. Case-boundary one-minute load averages range 10.37–15.10. Raw logs,
binary hashes and the process ledger remain in that directory.

Medians below average the middle two observations for even sample counts.
First-App and later-App strata distinguish process initialization from reuse;
neither process lifetime high-water RSS nor these warm filesystem-cache opens
are a cold-machine measurement.

| Workload | All six before / after, ms | Change | First App before / after, ms | Later Apps before / after, ms |
| --- | --- | --- | --- | --- |
| Kitchen | 574.102 / 652.212 | +13.6% | 568.735 / 602.125 | 704.531 / 732.217 |
| Subsets | 3.363 / 3.127 | -7.0% | 5.578 / 5.538 | 3.083 / 3.011 |
| Bird | 3,120.994 / 3,380.258 | +8.3% | 3,392.586 / 4,968.341 | 3,108.017 / 3,068.660 |

The subset regression does not repeat. Kitchen and bird still exceed the 5%
aggregate limit, so opening acceptance remains **unresolved**, not green.
Bird's later-App median improves 1.3% while its first-App result worsens 46.4%;
this variance does not establish causation by the hierarchy cache. Kitchen's
first/later strata worsen 5.9%/3.9%. Separate cold classification cost and host
variation before deciding whether to defer cache preparation or reject it.
