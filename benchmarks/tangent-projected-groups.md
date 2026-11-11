# Tangent subgroup projection experiment

## Hypothesis and boundaries

The retained CPU profiles put approximately half of cow user CPU self samples
inside MikkTSpace. Its subgroup filter alone accounts for about 6.5% of total
cow self samples. In Bevy MikkTSpace 1.0.0, `generate_tangent_spaces` projects
each group's face tangents onto the same representative normal repeatedly in
the quadratic subgroup-membership loop. The candidate computes each projected
pair once per group, retaining the original arithmetic, comparisons, sorting,
group membership, accumulation order and default compatibility features.

First test an isolated temporary copy of the dependency, not a production
dependency update. Preserve the original source, copyright and licenses, exact
candidate diff, source/binary/input hashes and all failed results. Do not edit
the Cargo registry. The OpenUSD pin and production scheduler remain unchanged.
No corrected-welding/sorting features, approximate math or quality changes.

## Declared kernel gate

The test-only `real_asset_tangent_kernel` probe uses the existing posed-mesh
reader and four-job / 16 MiB scheduler, with no tangent cache. It does not run
the withdrawn eight-job scheduler. Setup, USD reads, deformation, assembly and
output hashing are outside the timer; this is not whole-app timing or FPS.
All output attributes, indices, topology and morph bytes contribute to a
length-framed BLAKE3 digest. Hashes are a practical byte-equivalence check, not
an independent geometric oracle.

Build one original and one candidate release test executable from otherwise
identical source. Restore production manifests and lockfile before running.
Run before/after/after/before for cow and bird, 100 unique authored-range clocks
plus four warmups per process. Require matching output digests and work counts
at every clock, no errors, the original job/input bounds, at least 10% lower
cow mean-of-medians and no greater-than-5% bird median regression in either
nearby pairing. Retain p95/max/process wall time/RSS and all individual timings;
require no greater-than-5% mean peak RSS increase. Do not overlap owned builds
or captures, change unrelated host work, relax targets or rerun unchanged to
seek a pass.

This gate only decides whether to investigate integration. A successful kernel
test still needs the full-editor comparison (at least 5% cow median reduction,
no repeatable greater-than-5% bird/control regression), correctness/platform
gates, bounded scratch storage and strict native rendered checks. It does not
authorize shipping a dependency fork or clear any existing acceptance failures.

## Harness checkpoint

Evidence directory: `/tmp/bevy-optimization-tangent-projection.HLArUU/`.
The original release probe builds successfully. Two focused non-ignored tests
pass: sparse/oversized scheduling still matches serial tangents, and the digest
detects index identity, added attributes, signed-zero bytes and index-format
changes. Two real-asset diagnostics remain explicitly ignored in that test run.
Full hardening, candidate build and measured original/candidate comparison are
pending; these focused checks are not whole-project acceptance.

The original dependency copy is `original-mikktspace/`, the explicitly marked
experimental copy is `candidate-mikktspace/`, and `candidate.patch` preserves
the exact source delta. The candidate adds one call-local scratch vector of
two three-component vectors per face in the largest group. It does not retain
memory across meshes or frames. Peak process RSS still needs measurement.
