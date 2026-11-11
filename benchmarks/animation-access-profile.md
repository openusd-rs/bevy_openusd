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

## Result

Evidence: `/tmp/bevy-optimization-animation-access.tzNL5k/`. The saved release
editor SHA-256 is
`8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1`.
Both processes complete 1,000 unique measured updates and four warmups, with
zero diagnostics. The independent analyzer validates the clock sequences,
summaries, all four reporting phases, exact key-byte/row/call totals and zero
dropped or saturated counters. Adding one to a reported aggregate call count
is rejected by the analyzer's negative control. Geometry/image payload and
non-timing preparation/read/skin counters match the saved original editor.

| Measured phase | Bird | Cow |
| --- | ---: | ---: |
| Projection scopes | 1,000 | 1,000 |
| Animation-query lookups | 82,000 | 28,000 |
| Query-construction requests | 1,000 | 2,000 |
| Joint-transform requests | 1,000 | 2,000 |
| Authored-value checks | 41,000 | 14,000 |
| Blend-weight requests | 0 | 0 |
| Distinct animation prims with joint/weight requests | 1 | 2 |
| Retained diagnostic keys / key-text bytes | 4 / 798 | 8 / 1,032 |

The bird requests the animation under
`/hummingbird_anim_hover_idle_long/hummingbird_rig/hummingbird_skinned_mesh/hummingbird_bind/root_1/Animation`.
The cow requests `/World/Cow_001/Animations/Eating_01` and
`/World/Cow_F/Animations/Stand_02`, not all 150 authored animation prims.
Each queried animation's authored-value checks stop at `translations`; the
profile does not count unevaluated sibling checks. The table's per-update
averages are not individually recorded per-update call distributions.

Joining these requests to the earlier sample-map inventory gives three bird
component candidates with 1,484,000 numeric sample bytes, and six cow candidates
with 2,223,228 bytes. Bird scales have a static 1,200-byte default and no sample
map; that static data is not in the numeric sample-byte total. Cow scale maps
have one 294-byte sample each. These totals exclude map slots, capacity, static
values, keys and query/allocator overhead, and remain neither exact decoder
counts nor a hard retained-query-memory bound.

The diagnostic's measured CPU medians are 20.587 ms / 53.716 ms and process
peak RSS 956,704 / 541,660 KiB for bird/cow. These are instrumented observations,
not an optimization comparison. Unrelated build/test jobs are present in the
host snapshots and were left alone; no owned build overlaps either run.

Full hardening passes formatting, strict Clippy and 887 / 1,017 test executions
in overlapping configurations, including scope isolation, nested disabled
scopes/unwinding, release of profile ownership, key limits and saturation.
The release benchmark build and wasm check also finish successfully. The
original tool handle disappeared during hardening, but the same live driver
PID was observed until it finished; it was not restarted. The `set -e` chain
reaches the release build only after hardening succeeds, and its final build
marker and process termination are retained in `build-observation.txt`.

## Next integration boundary

On-demand reuse should follow the requested animation paths, not a scan of all
authored clips. Existing scope-local query reuse already reduces construction
to one request per used animation per measured update on average. Persistent
reuse still needs capacity-aware admission, invalidation/re-admission after
edits, actual stage identity rather than these aggregated root labels, and
close/reload ownership. The current upstream query's opaque retained source is
still an accounting constraint; these smaller payload totals do not solve it.

The repeated authored-value checks are measured work, not permission to
re-enable the previously withdrawn authored-flag cache or waive its failed
gate. No persistent query cache, decoder replacement, dependency change or
native frame-rate improvement is introduced by this diagnostic.
