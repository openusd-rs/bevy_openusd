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
