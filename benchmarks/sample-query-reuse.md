# Upstream animation-query reuse probe

**Status:** the bird attribute kernel passes; the cow timing is incomplete
because it exceeds the probe's resource guard. No production query cache is
enabled, and the combined opportunity gate is not passed.

The current OpenUSD pin already provides `usd::AttributeQuery`: a timed read
memoizes the resolved source and replays it while the upstream prim-revision
stamp remains valid. Numeric time samples use the same upstream interpolation;
default reads and value clips retain their ordinary resolution paths. Cloning
a query resets its memo. `openusd-schemas::skel::AnimQuery` does not use this
API: its component/weight reads still construct attributes and resolve again.

This is a possible way to avoid repeated full-map decoding without a dependency
fork. It is not selective decoding: a query retains the complete sample table.
The current public API exposes neither retained-byte accounting nor an admission
budget. A persistent integration must bound memory, retain edit/reload and
instance isolation, and preserve the upstream query rather than reimplement
composition or interpolation locally. No runtime query cache is added here.

## Bounded opportunity measurement

Build `sample_query_probe` with `oslo make build-sample-query-probe`; its unit
tests run with `oslo make test-sample-query-probe`. Both run inside `nix develop`.
Use the same release executable for hummingbird and cow, twice each with
reversed first-read order. Each process reads the four animation channels on
every composed SkelAnimation at four warmup clocks and 1,000 unique stage-range
clocks. The default resolver, dependencies, interpolation and quality stay
unchanged. No owned build or native capture overlaps the measurements.

Within each update, time the direct attribute batch and the upstream-query
batch separately in ABBA order. Compare option presence, numeric variant and
every payload byte after the timer, and retain length-framed fingerprints.
Unsupported value types fail the probe rather than being omitted. Record all
raw samples, cold warmups, first-read orders and nearest-rank median/p95/max.
The returned-value allocation is timed; equality, hashing, printing, initial
inventory and stage opening are separate. This is an attribute-read kernel,
not full-editor timing, native FPS or acceptance of a production cache.

Inventory every selected attribute's sample count and numeric payload bytes,
including half/double precision. These are composed-map logical bytes, not a
measurement of query-owned allocation, metadata, capacity, RSS or a hard cache
bound. Preserve process peak RSS separately. Include all fixture layers/assets
in before/after manifests; keep the source and lockfile unchanged.

The opportunity gate requires exact values throughout and at least 25% lower
query median batch time in both first-read strata, in both fresh runs, for each
asset. Passing only identifies a worthwhile integration candidate; failing
rejects this opportunity at the declared threshold. Do not relax it, aggregate
away an order failure or rerun unchanged for a favorable answer. Full-editor,
resource/lifecycle, native visual and platform gates remain separate.

The initial cow run exceeds the probe's 512-attribute guard before measurement.
Keep that failure and the original executable; do not raise the guard or repeat
the reversed cow timing unchanged. Finish only the already-declared second bird
run. A separate `ASSET inventory 0` mode then inventories the cow one sample
map at a time without constructing persistent queries or taking performance
samples. It does not turn the failed cow timing into a passing result.

## Measured result

Evidence: `/tmp/bevy-optimization-sample-query.6c8Dyk/`. The original timing
executable is `probe`, SHA-256
`6deaa6a30b81cb598c5208951233b1b0fdbb03e6a5151a879ed3b05d90b83e6f`.
Its source is retained as `timed-probe-source.rs` and in local commit `f51342b`.
The initial test build fails because the edit helper consumes an attribute
handle; the fixture now clones that handle, preserving the query being tested.
The corrected focused tests and release build pass. Later full hardening,
including the inventory mode, passes formatting, strict Clippy and 885 / 1,015
test executions in overlapping no-default and all-feature configurations.

Two bird processes complete 2,000 measured clocks and eight warmups, comparing
16,064 attribute answers exactly, including option presence, variant and numeric
bits. Their raw fingerprints, four order-stratum summaries, stage range and
sample-map inventories match. The signed-zero/kind/presence unit test detects
alterations, and the query matches direct reads after edits and interpolation
changes. Unsupported value types still fail rather than disappearing from the
comparison. Both successful processes have zero application diagnostics.

| Fresh process | First batch | Direct median ms | Query median ms | Change |
| --- | --- | ---: | ---: | ---: |
| Bird 0 | Direct | 0.237857 | 0.006575 | -97.24% |
| Bird 0 | Query | 0.241570 | 0.006669 | -97.24% |
| Bird 1 | Direct | 0.241801 | 0.007042 | -97.09% |
| Bird 1 | Query | 0.244149 | 0.006991 | -97.14% |

This exceeds the 25% bird-kernel opportunity target in all four strata. It is
only about 0.23 ms saved for these batches, not a 97% application improvement or
a measurement of all OpenUSD decoding. No rendered playback or integrated
cache is measured. Cold warmups, p95/max and process totals remain in the raw
logs and `partial-analysis.json`. The bird processes peak at 51,308 / 51,820
KiB RSS, including both policies, the inventory and the stage; this is not a
before/after memory comparison.

The first cow process exits one before sampling with `probe attribute limit
exceeded`. Its reversed timing is not run. The independent partial analyzer
therefore exits one and explicitly records two successful processes, one failed
process and one unrun process. Neither the 512-attribute guard nor the timing
threshold is raised to convert that failure into a pass.

## Inventory and integration boundary

The separately built `inventory-probe` has SHA-256
`47d68e3e19b365baef266f37c202e499322ba7b77d8d627f351dea0da1b2559c`.
It completes the cow inventory without constructing persistent queries. The
analyzer validates every attribute, four channels per animation, exact totals,
zero diagnostics and unchanged manifests:

| Asset | Animation prims | Attributes | Authored sample entries | Logical numeric sample bytes |
| --- | ---: | ---: | ---: | ---: |
| Bird | 2 | 8 | 1,060 | 2,968,000 |
| Cow | 150 | 600 | 82,058 | 56,232,988 |

The largest single cow attribute contributes 752,640 numeric sample bytes.
The streaming inventory peaks at 60,844 KiB process RSS; it does not measure
retaining all 600 query sources. Logical payload excludes map slots, keys,
capacities, static values, query metadata, the stage and allocator overhead.
Both phases retain the archive and all 267 cow-directory files in their asset
manifests; the successful bird reads also report their sole layer identifier.
The original upstream pin and lockfile are unchanged.

The inventory includes every composed SkelAnimation, not only animations the
current meshes bind. Do not pre-create a persistent query for every authored
animation or equate that inventory with actual per-frame reads. Next identify
which attributes the existing projection requests and how often, then design
bounded on-demand reuse with explicit admission, invalidation and close/reload
ownership. The opaque upstream cache still needs a defensible memory bound.
Full-editor gains, native fidelity and all broader acceptance gates remain open.

The subsequent [projection-operation trace](animation-access-profile.md) finds
one requested bird animation and two requested cow animations in the current
playback paths. Their component-map payloads are substantially smaller than
this all-authored inventory, without establishing a complete cache memory bound.

## Admission and lifetime constraints

Source review is against the pinned `900ea31591348e3fbf416d4661d703aa8700771b`,
not an assertion about a later upstream revision. `SourceResolver` clones the
winning sample map; `AttributeQuery` holds it alongside its source site and
private validity stamp. Public `time_samples()` returns another cloned map,
retimed in place. Numeric arrays are not expanded by that retiming, but their
logical payload omits map slots and source-site metadata. Measuring that public
copy is not inspecting the query's allocations.

Three regression cases in `sample_query_probe` exercise the proposed admission
boundary, using the unchanged upstream API:

- `query_refresh_can_exceed_initial_sample_budget` warms one query on 24 numeric
  sample bytes, then grows the same attribute to 98,304 bytes. The same query
  returns the new 49,152-byte interpolated array and then respects a value block.
  A one-time 1,024-byte admission test would admit the initial source, not the
  edited one. The ordinary, non-proxy prim's public `index_revision` also stays
  unchanged: it reports the index's build revision, not the private value
  revision that invalidates the query. Query correctness does not enforce that
  external memory budget.
- `proxy_path_revision_does_not_guard_query_readmission` performs the same
  growth through an instance proxy. Its visible prim's `index_revision` stays
  `None` before and after, while upstream correctly refreshes the query from
  the changed source. Equality of those public revisions cannot authorize reuse
  of an old byte charge; the internal stamp also tracks the redirected source
  and population epoch.
- `equal_root_labels_do_not_share_query_state_or_lifetime` opens two independent
  stages with equal root identifiers and attribute paths, then edits only one.
  The queries remain independent. A weak sentinel owned by the first stage's
  change sink stays alive after dropping its caller's stage and attribute
  handles, until its query is dropped. A global query registry would therefore
  also retain stages unless explicitly cleared; root labels are not identities.

These cases compare exact values at forward, reverse, repeated and out-of-range
clocks. They do not measure retained query bytes, process memory or playback
speed, and are not a full reload/lifecycle acceptance suite.

Evidence: `/tmp/bevy-optimization-query-admission.UAtjks/`, with the tests in
local commit `5e8d4d2`. The first compile fails because the sink closure needs
explicit argument types. The next run passes five tests but fails an incorrect
expectation that a value edit changes the public index revision. Inspection of
upstream `built_revision`, `prim_revision` and `restale_values` confirms separate
build/value stamps; the regression now asserts the observed stable build stamp
alongside changed values and sample payload. Both failed logs are preserved.
The final run passes all six focused tests, formatting and strict workspace
Clippy with all targets/features. Dependency and upstream-source manifests are
unchanged. Full hardening, wasm and real-asset timing are not rerun for this
test-only change; previous performance and acceptance results stand as recorded.

Persistent integration needs a bounded upstream source-cache contract, rather
than a local copy of USD resolution:

1. Account retained source storage with checked arithmetic, including capacities,
   map slots and owned source metadata. Unknown accounting must decline caching,
   not admit an uncharged value. Distinguish retained storage from transient
   decoding, caller-owned return values, allocator overhead and whole-process RSS.
2. Enforce admission on every source resolution/refresh under the same validity
   stamp used by the query, including instance redirection. Release obsolete
   storage and its charge; an edit cannot silently grow an admitted entry.
3. Preserve direct-read semantics when admission is refused, including defaults,
   blocks, clips, offsets, precision casts and interpolation. Do not truncate
   values, reduce quality or cache an error as an empty answer.
4. Bound aggregate entries/bytes, bind ownership to the actual stage instance,
   and release queries on eviction, close and replacement. Do not form a cycle
   from a stage-owned sink back to a query owning that stage.

The existing API does not provide that accounting/admission contract. Do not
enable persistent query reuse from the measured payload totals or visible-path
revision alone. An upstream API change needs its own isolated implementation
and validation before adoption; the dependency pin and production path remain
unchanged. Other measured optimization work need not wait on this API boundary.
