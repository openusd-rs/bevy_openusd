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

No production animation-index optimization is implemented at this checkpoint.
