# Upstream animation-query reuse probe

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
