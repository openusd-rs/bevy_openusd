# Animation operations requested by projection

## Scope

The sample-query inventory covers every authored SkelAnimation, including
animations the current meshes may not bind. Before designing a persistent
cache, count the animation operations the normal projection actually invokes.
`AnimationAccessProfile` is an optional diagnostic resource; it is not a value
cache and retains no stage, query or sample array after a projection scope.

The profile counts five operations in the existing reader paths:

- `query-lookup`: an animation-query lookup, including scope-local cache hits.
- `query-build`: a lookup that reaches the upstream query-construction path.
- `joint-transforms`: an upstream joint-local-transform request. Its component
  reads can short-circuit; this is not three guaranteed attribute decodes.
- `blend-weights`: an upstream blend-shape-weight request before remapping.
- `authored-check`: one actually evaluated authored-value check; short-circuited
  sibling attributes are not recorded.

These are reader-operation counts, not decoder calls or CPU costs. Keys contain
the root identifier, prim/property path and operation. Equal root identifiers
aggregate across instances; they are diagnostic labels, not valid cache keys.
The active scope still filters by actual stage identity and restores nested
scopes, including unwinding and disabled scopes.

Storage is bounded to 4,096 keys and 1 MiB of key text. New keys beyond either
limit increment an explicit dropped counter; existing keys continue counting.
Counter overflow is reported. Default application execution does not install
the resource. The headless editor benchmark enables it only when
`USD_PROFILE_ANIMATION_READS` is set, and resets it after seek warmup with the
other profiles.

## Declared diagnostic

Build one release `editor_benchmark` through the repository make recipe. After
its build and hardening finish, run hummingbird and cow once each with the
existing GPU-prepared headless timeline, default eager residency, normal worker
pools, four warmups and 1,000 unique authored-range clocks. Enable animation,
route and raw-sample diagnostics. Preserve source/executable/asset hashes and
host load; do not overlap owned builds/captures or change unrelated jobs.

Validate completion, readiness/clocks, nearest-rank summaries, no application
diagnostics, positive scopes/calls, zero dropped/overflowed counters, and exact
agreement between row sums and aggregate counts. Retain opening/idle/warmup and
measured-seek phases separately. Compare geometry/image payload and non-timing
work counts with the saved unchanged editor's matching real-asset results.

Map observed joint/weight operations to candidate channels in the separate
sample-map inventory, retaining authored checks separately. A joint request
identifies three possible component channels; do not claim each was decoded,
that all inventoried clips execute, or that their aggregate bytes are a proven
retained-cache bound. The diagnostic changes measured CPU work and is not a
before/after performance gate, native FPS result or permission to integrate an
unbounded query cache. All existing failed gates remain open.
