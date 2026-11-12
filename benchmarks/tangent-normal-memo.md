# Normal-keyed tangent projections

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
