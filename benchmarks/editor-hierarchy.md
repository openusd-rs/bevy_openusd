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
test cases. Log: `/tmp/bevy-optimization-inspection-harden.log`. Native gates and
performance comparisons are still pending.

A fresh pre-change release executable is saved under
`/tmp/bevy-optimization-inspection-before.ylni3Q/`; the authoritative
path is `/tmp/bevy-optimization-inspection-before-path`. Its recorded revision
is `8b664bc`, so the upcoming comparison need not attribute unrelated changes
between an old historical executable and this focused patch.
