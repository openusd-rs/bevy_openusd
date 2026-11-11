# Asset-root animation classification

## Measured bottleneck

The [rendered playback baseline](rendered-playback.md) places most of the slow
playback interval in the main update, including the transform-only robot arm.
Unlike the live editor's maintained `AnimatedPrims` index, `instance::tick`
currently scans the entire projected map and calls `live::prim_is_animated`
again on every changed root clock. That query checks authored samples,
subsets, deformation inputs, inherited primvars, bound materials and instancer
prototypes; it is not a cheap lookup of the prim's own sample count.

An opt-in `UsdInstanceAnimationTiming` resource (`f81d2d2`) separates
classification from the enclosing material-scope/animation-route work without
changing the iterator or projection decisions. `USD_PROFILE_ROUTES=1` enables
it in `viewer_capture`.
The report covers **all root time changes, including warmup**, rather than
claiming to isolate the recorder's measured window. Classification is already
included in total time; the columns must not be added together. Total excludes
the subsequent residency materialization and the rest of the app/render work.

`/tmp/bevy-optimization-instance-profile.lYjmai/` retains the saved executable,
exact worktree patch, revision/binary/source hashes, raw rows, images, process
results and `classification-analysis.json`. Native RTX 4080/Vulkan, release,
validation enabled, original forward/4x-MSAA/1280x720/default-camera settings,
300 measured frames plus 30 animated warmups per process. No task-owned build
overlaps these runs; case-boundary host load is 21.92–31.52.

| Asset | Changed root updates | Prims classified / animated per update | Mean classification, ms | Mean enclosing animation, ms | Classification share |
| --- | ---: | ---: | ---: | ---: | ---: |
| Arm | 330 | 138 / 9 | 30.022 | 31.087 | 96.6% |
| Bird | 330 | 84 / 44 | 8.832 | 33.390 | 26.5% |
| Cow | 330 | 183 / 164 | 17.973 | 74.234 | 24.2% |

These are instrumented wall-time aggregates under variable host load, not
thread CPU consumption or before/after performance gains. All three processes
complete the measured frames, change every measured clock, pass renderer-log
checks and produce final poses pixel-exact at tolerance zero against the saved
pre-instrumentation executable at the same clocks. Source manifests verify.
The driver still exits 1 because the cow retains its unavailable MDL dependency.

The arm result confirms redundant animation discovery as the next target.
Removing it alone is unlikely to meet 60 Hz for bird/cow: their remaining
animation work is already larger than the target interval. Existing tangent
preparation and other per-frame geometry work still need optimization.

## Next-change acceptance, declared before implementation

- Avoid repeated animation discovery for unchanged asset-root compositions;
  do not skip animated prims, replace actual playback with seeks, reduce quality
  or alter the public stage/edit/reload/instance contracts.
- Bound retained classification state to the current root/composition; retain
  no historical poses or asset handles. Explicitly test the retention cap and
  uncached fallback, root teardown and source replacement.
- Compare cached decisions with fresh classification for forward/reverse clocks,
  single samples, inherited geometry/material animation, deformation, clips and
  point-instancer prototypes. Independently timed roots must remain isolated.
- Invalidate for direct edits, binding/collection changes, variants, payloads,
  masks, muted layers and successful reloads. Failed reloads must preserve the
  last-good root. Preserve custom-route ordering and mutation visibility rather
  than assuming every callback is read-only.
- Saved-release alternating comparisons target at least 25% lower arm main-update
  median, with no repeatable greater-than-5% opening/idle/small-scene regression.
  Keep the full 60 Hz playback target and the existing failing gates unchanged;
  this percentage is an incremental target, not completion of the whole goal.
- Verify actual native images against independent fixed poses and before/after
  images, using meaningful full-object framing for the follow-up matrix. Do not
  equate offscreen intervals with display/vsync/editor-UI acceptance.

## Verification status

The timer regression checks that instrumentation is absent by default, counts
only changed root clocks, preserves independent transforms and excludes idle
updates. The first hardening attempt had a test-only Bevy API spelling error;
the corrected attempt passes that test in both configurations but exposes two
existing one-update projection assumptions in subset/subdivision fixtures.
Those fixtures now request an unlimited test budget and explicitly assert
readiness; their original geometry, skin/morph transition, independent-clock
and runtime-component assertions are unchanged (`5ba8419`). Final hardening
passes 820 no-default and 928 all-feature/all-target tests, strict Clippy,
formatting and whitespace checks. The configurations overlap; they are not
1,748 distinct tests. Logs are retained as
`/tmp/bevy-optimization-instance-timing-{harden,ready-harden,final-harden}.log`;
the final release build is `/tmp/bevy-optimization-instance-timing-final-build.log`.

Native gates also pass: four deformation-oracle cases, five asset-watcher and
four editor-texture-watcher cases, fourteen native export cases and the custom
data reference round trip. `check-web` passes wasm compilation, not a browser
render/performance test. The status ledger is
`/tmp/bevy-optimization-instance-timing-gates.tsv`, with logs
`/tmp/bevy-optimization-instance-timing-{test-native-deformation,test-watchers,test-native,check-web}.log`.

The instrumentation-only checkpoint above does not include classification reuse.

## Classification-reuse candidate

The candidate retains positive and negative classifications in each asset
runtime, capped at 65,536 paths and 8 MiB of path text, not total allocator/RSS
usage. It retains no entities, stage handles, asset handles or historical poses.
The existing live-stage observer increments a saturating composition revision
for every commit and synthetic resync; draining notices does not reset it.
Each lookup also checks layer identifiers, load rules, population mask and muted
layers after flushing. A changed key releases old entries, failed flushes and
saturated revisions disable retention, and a change during classification
prevents publishing that result into the cache. Oversized entries use fresh
classification. New/reloaded runtimes begin empty and teardown drops the cache.

The original lazy traversal/order stays intact. Registries with custom routes
clear/bypass the cache, preserving fresh classification after preceding route
callbacks. Reuse counts are separate from actual classification calls; timed
classification includes lookup/invalidation checks. `967bd89` adds the live
revision and `c624928` adds the cache and capture reuse counters.

Initial candidate tests exposed a fixture construction error, not an oracle
disagreement caused by caching: a root with filesystem fallback did not resolve
relative dependency names that existed only in the supplied snapshots. The
fixture now uses the explicit snapshot-only root policy and validates its full
composition before testing payload transitions. Production resolver behavior
is unchanged. Both failed attempts are retained in
`/tmp/bevy-optimization-animation-cache-{harden,ready-harden}.log`.

Nine new tests plus the updated optional-timer regression pass. They cover
fresh-oracle agreement across deformation/clip/prototype fixtures, single-sample
direct layer edits, drained notices, positive/negative decisions, payloads,
muting, variants, population masks, material collections/shader changes,
retention caps, custom callback mutation/order, independent clocks, failed and
repaired reloads, runtime-component preservation, teardown and revision
saturation/detachment. Final `harden` passes 829 no-default and 937
all-feature/all-target tests, strict Clippy, formatting and whitespace checks;
these are overlapping configurations. The release capture executable builds.
Logs: `/tmp/bevy-optimization-animation-cache-fixed-harden.log` and
`/tmp/bevy-optimization-animation-cache-build.log`.

Native before/after performance, full-object render comparisons and refreshed
native/reload/export/wasm gates remain pending for this candidate.

Before the alternating run, the saved pre-cache executable renders and passes
pipeline/log/readback checks at the new fixed views. Images under
`/tmp/bevy-optimization-animation-camera.MFRxtx/` were inspected: the entire arm,
bird and cow fit the viewport. Eye/target are `(2.4,1.6,3.2)/(0,0.6,0)` for arm,
`(10,6,14)/(0,1,0)` for bird and `(3,2,4)/(0,0.8,0)` for cow. Both compared
versions must use those same views; differences from the earlier default-view
baseline are not attributed to the cache. The cow MDL warning remains present.
