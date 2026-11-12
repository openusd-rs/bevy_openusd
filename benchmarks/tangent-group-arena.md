# Tangent group membership arena

## Candidate and boundaries

The original MikkTSpace `build_4_rule_groups` allocates a face-membership vector
for each connected vertex group. The isolated candidate stores those lists in
one call-local vector and gives each group a disjoint range. Traversal, group
IDs, neighbor recursion, subgroup sorting and floating-point evaluation remain
unchanged. This starts from the original 1.0.0 source, not the rejected
projection/list-reuse candidate.

Each append claims one previously unassigned triangle corner;
there can be no more than three entries per nondegenerate triangle. Capacity
is reserved from that already-materialized triangle-corner count. There is no
cross-call retention, new cache, changed worker policy or production dependency
override. Original copyrights/licenses are retained in the isolated source.

Evidence directory: `/tmp/bevy-optimization-tangent-arena.wMIFzj/`.
Production starts at `fe90b03`; its manifests and dependency pin stay unchanged.

## Gates declared before measurement

1. Compare all output components and orientation bits on the existing finite
   geometry corpus at all six thresholds; retain the one-bit negative control.
   Add disconnected and connected many-group meshes and degenerate mixtures.
2. Count actual allocation/reallocation calls separately from timing. Compare
   identical input/output buffers, verify all outputs, and retain total
   requested bytes and peak live allocation bytes. Require fewer allocation
   calls on connected and disconnected many-group meshes, no retained
   allocation after the generation call, and no increased peak live bytes.
3. If those checks pass, compare original/candidate release real-asset tangent
   kernels in fresh before/after/after/before processes, 100 unique clocks and
   four retained warmups each. Require exact output digests and work/budget
   counts, at least 5% lower cow mean-of-medians, no greater-than-5% bird median
   regression in either nearby pairing, and no greater-than-5% mean peak RSS
   increase. Retain every raw timing, tail, error and failed gate.

No owned builds/captures overlap performance runs; leave unrelated work and
affinity unchanged. These gates do not establish whole-editor performance,
native frame pacing or rendered acceptance. Existing failed editor controls,
visual discrepancies and all broad optimization workstreams remain open.

## Finite geometry and allocation checks

The extended corpus passes 3,474 exact comparisons covering 698,838 supported
corners, including connected/disconnected grids and degenerate mixtures. The
one-bit negative control is rejected. Six many-group allocation cases reduce
calls, total requested bytes and peak live requested bytes; storage returns to
zero after each generation call. For 64-by-64 grids, allocation/reallocation
calls fall from 60,852 to 48,436 (connected) and 49,199 to 24,624 (disconnected).
Peak live requested storage falls from 2,982,368 to 2,851,280 bytes and from
4,096,088 to 3,833,944 bytes respectively. These are not RSS or elapsed results.

The first allocator self-test fails because an unused allocation is optimized
away. Its log/exit remain in `allocations.log` / `allocations.exit`. Making the
test pointers opaque fixes the fixture; the counter then observes the expected
allocate/zeroed/reallocate calls and requested-byte peak. The passing rerun is
`corpus-allocations-fixed.log`. Counting excludes preconstructed inputs/outputs,
allocator metadata and transient storage inside the system allocator's realloc.
It is never enabled in the real-asset timing executable.
