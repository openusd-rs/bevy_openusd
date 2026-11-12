# Streamed tangent groups

## Scope and invariants

The prior flat membership arena fails its elapsed and RSS gates, and reserves
storage even when no usable groups form. This new isolated candidate starts
from original MikkTSpace 1.0.0. It visits each completed group immediately and
reuses that group's membership vector, rather than retaining all group records
and members. No earlier projected-subgroup or arena optimization is included.

Group discovery order, IDs, neighbor recursion, membership order, subgroup
sorting, evaluation order and floating-point operations must remain unchanged.
Later group construction may assign other triangle corners, but cannot change
an already assigned corner or the orientation chosen by a triangle's first
group. The visitor gets immutable triangle data. The original no-group early
return must still skip the degenerate repair passes. Geometry callback order
will be compared explicitly, in addition to all returned tangent bits.

Membership capacity grows only for the largest encountered group and is freed
at call end. Subgroup scratch vectors reserve exactly for the largest group
seen, replacing the original up-front maximum scan. No persistent cache,
worker-policy change, unsafe production code or reduced-quality mode is added.

Evidence: `/tmp/bevy-optimization-tangent-stream.oiBxFD/`.
Production starts at `883154e`; the isolated viewer remains `210521d`, whose
runtime/probe sources match production. Reuse its saved original executable
only after verifying source, lockfile and executable identities. Main runtime,
manifests and the upstream OpenUSD dependency remain unchanged.

## Gates declared before measurement

1. Pass the existing 3,474 finite geometry comparisons, including all six
   thresholds and degenerate/unsupported cases. Compare the complete Geometry
   callback sequence separately, with a deliberately altered trace rejected.
2. Pass the six connected/disconnected allocation gates, including reduced
   calls, no higher peak live requested bytes and no retained generation
   allocation. For sparse UV fixtures with 0, 1 and 8 usable faces, require no
   increased allocation calls, requested bytes or peak live requested bytes.
   Preserve the no-reduction, retained-allocation and output-bit controls.
3. Build the candidate release probe and run the same library tests on both
   saved binaries. Run the unchanged real-asset protocol: eight ABBA processes,
   100 unique clocks plus four warmups each; exact output digests and job/input
   bounds; at least 5% lower cow mean-of-medians; neither bird pairing worse
   than 5%; neither mean peak process RSS higher than 5%. Keep every failed
   result and tail observation. No overlapping owned builds or captures.

These are kernel/correctness gates, not application or native frame-rate
acceptance. A passing result still needs stable whole-editor controls,
downstream packaging, platform and rendered validation. All broader workstreams
and existing acceptance failures remain open. Nothing is pushed.
