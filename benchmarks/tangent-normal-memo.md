# Normal-keyed tangent projections

**Status:** the declared kernel, output/callback, allocation and process-memory
gates pass. Cow p95 increases 19.93% and remains an unresolved integration risk.
This supports whole-editor investigation, not production adoption or FPS claims.

## Candidate and boundaries

The streamed-group candidate passes its correctness/resource checks but misses
the cow elapsed target. This new candidate adds a group-local projected-pair
memo and reuses temporary subgroup members. Every original geometry callback
stays in place. Projected pairs are reusable only when all three current normal
components have identical bits; a changed normal rebuilds them, and a new group
always starts with an empty key. Signed zero is not collapsed. Sorting, subgroup
membership and accumulation order remain unchanged. This is not the older
projection patch that moved normal reads out of the face loop.

Two call-local scratch vectors reserve for the largest encountered group:
two `Vec3` values and one `usize` per face. They allocate only after a group
exists and are freed when generation returns. No cross-frame cache, worker
policy, quality mode or unsafe production code is added. Arithmetic calls can
be reduced; exact Geometry callback order does not mean identical numbers of
calls to the deterministic `Ops` math functions.

Evidence: `/tmp/bevy-optimization-tangent-memo.ZRHuLP/`.
The candidate applies after `mikktspace-streamed-groups.patch` to original
MikkTSpace 1.0.0. Prior candidates and failed results remain unchanged. Production
starts at `025b662`; viewer/runtime probe remains `210521d` with its saved
original executable, subject to source/lockfile/hash verification. Main runtime,
manifests, OpenUSD dependency and desktop state remain unchanged.

## Gates declared before measurement

1. Extend the finite geometry corpus with changing-normal callbacks and signed
   zero. Match every returned tangent component/orientation bit and the complete
   Geometry callback sequence at all six thresholds. A candidate deliberately
   ignoring normal changes must fail the changing-normal cases.
2. Retain all six allocation gates, sparse-UV no-regression gates, call-local
   release checks and their negative controls. Run strict dependency library
   Clippy, no-std compilation and the release viewer library tests.
3. Use the existing eight-process real-asset ABBA protocol, 100 unique clocks
   plus four warmups each, unchanged four-job / 16 MiB input bounds and exact
   corresponding output digests/work counts. Require **at least 10% lower cow
   mean-of-medians**, neither bird pairing above 5% regression, and neither
   mean peak RSS above 5% overhead. Retain all tails and failures. This raises
   the cow requirement from the standalone streaming trial's 5%; it does not
   relax a failed result. No overlapping owned build/capture or affinity changes.

Passing only permits full-editor investigation, not production adoption, FPS
claims or completion of the broad workstreams. Stable editor controls, native
rendering, packaging/downstream use, lifecycle and browser acceptance remain
required. Nothing is pushed.

## Source qualification

The extended corpus passes 3,528 comparisons covering 703,374 supported corners.
A separate trace run matches all 11,802,154 geometry callbacks in order. The
54 added comparisons exercise alternating signed zero, rapidly changing normal
directions and longer runs of repeated/changed directions. The deliberately
stale-normal implementation fails `changing-normal-2-3` at threshold negative
zero, demonstrating that invalidation is required rather than unused test code.
The stale copy, failure and verification exits are retained separately.

All six allocation gates and the sparse-UV gates pass. On 64-by-64 connected
grids, allocation/reallocation calls fall from 60,852 to 4,270; on disconnected
grids, from 49,199 to 24,612. Peak live requested bytes are 2,621,440 in both
candidate cases, below the original 2,982,368 / 4,096,088. With no usable UV
faces, calls/requested bytes/peak remain exactly 27 / 1,891,648 / 655,360. Every
generation call releases its tracked storage. These checks are not RSS or
elapsed measurements. One-bit output, callback-order, no-allocation-reduction
and retained-allocation controls remain rejected.

Strict all-feature dependency library Clippy and no-default-feature compilation
pass. The source patch at `25e0617` applies after the streaming patch and a fresh
replay matches all candidate source files. The first patch export omitted a
trailing context line and failed to apply; the corrected artifact restores
that line without changing the tested Rust source.

The analyzer now records an explicit 5% or 10% required cow gain, retaining 5%
as its old default. This trial uses 10%. A marked synthetic 7.5% timing fixture
passes the 5% setting and fails the 10% setting with the expected diagnostic;
it is only an analyzer test, not an observed performance result. These checks
are committed before real-asset measurements.

## Real-asset result: declared kernel gate passes

The release candidate builds in 4m05s on Rust 1.98.1. The saved original and new
candidate each pass the same 743 default-feature library tests, with 25 existing
ignored tests. These are overlapping configurations, not 1,486 distinct cases.
Both use viewer source `210521d`; its runtime/probe source matches the main
checkout. The original source/lockfile/executable manifests verify. Only the
MikkTSpace registry source/checksum is removed in the candidate lockfile, and
the viewer's original lockfile is restored before measurement.

All eight real-asset processes exit zero without application diagnostics. The
analyzer checks 800 measured clocks, 32 warmups and 22,876 tangent jobs; every
corresponding output digest, work count and four-job / 16 MiB input bound
matches. Source, executable and all 268 asset-manifest entries verify after
measurement. The analyzer records `required_cow_gain_percent = 10`.

| Workload | Original median ms | Candidate median ms | Median change | p95 change | Mean peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cow | 30.578789 | 25.331986 | -17.16% | +19.93% | +2.69% |
| Bird | 6.014000 | 4.664190 | -22.44% | -21.60% | -1.29% |

Values compare means of two process summaries, not pooled medians or native
frame times. The cow nearby pairings improve 22.36% and 11.86%; bird improves
21.63% and 23.25%. Cow therefore passes the 10% mean-of-medians target, neither
bird pairing regresses, and both mean peak RSS changes stay below 5%.

Cow p95 rises from 35.134476 to 42.138195 ms and its maximum rises 0.82%. These
observations are retained, not reclassified as passes or excluded from later
application acceptance. The kernel protocol did not declare a p95 threshold;
passing it does not establish smooth frame pacing. Bird's maximum falls 23.70%.
Whole-process wall time falls 7.22% / 9.28%, but includes untimed setup and is
not full-editor elapsed evidence. Comparisons with earlier experiments do not
isolate the contribution of any individual part of this combined candidate.

Starting one-minute host load ranges from 5.28 to 7.34. Default affinity and
four workers are unchanged. No owned build/capture overlaps timing; unrelated
build activity is recorded and left alone. `driver.exit`, `analysis.exit` and
`verification.exit` are all zero. The verifier still rejects altered clocks,
altered output digests and doubled candidate timings under the 10% target.

Saved executable SHA-256:

```text
before c092b2331eb27d4b9c07735151d52298ea3a3fdcd99b926f307f7c480b4cdbaf
after  52423d5204b93f841f67950acf7ffac7628a0f550db22fba0b4f1af5fb6efe49
```

## Next acceptance boundary

Move to whole-editor measurement rather than another kernel-only variation.
The earlier per-update alternating-process protocol fails identical-binary
controls, even when only the main thread is pinned. Do not waive those failures,
pool away order strata or claim an application gain from this kernel result.

The existing stepped input contract accepts buffered sequential command lines;
its regression already compares fully buffered commands with ordinary timeline
updates. A controlled block-fed protocol can therefore test sustained editor
work without changing the runtime: validate every ready marker/clock, feed
fixed-size contiguous blocks, alternate process blocks in balanced order and
retain every first/resumed update. Declare the block sizes, control limits and
analysis before running, and qualify identical-original controls first. This
is a different pacing experiment, not permission to rerun failed comparisons
unchanged until they pass. Keep the native-frame distinction explicit.

If controls are stable, build this exact isolated source into the full editor
and test original/candidate workload, tail and memory regressions. The cow p95
increase requires attention. Packaging must also preserve downstream library
behavior; a root-only Cargo patch is not automatically inherited by users.
Native visual, lifecycle, browser and all remaining broad workstreams stay open.
Production runtime, manifests and dependency pins remain unchanged.
