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
Both original and first-candidate release probes build successfully. Two focused non-ignored tests
pass: sparse/oversized scheduling still matches serial tangents, and the digest
detects index identity, added attributes, signed-zero bytes and index-format
changes. Both binaries pass these two tests. Two real-asset diagnostics remain
explicitly ignored in those focused runs. Production manifests and lockfile
were then restored byte-for-byte. Full hardening passes formatting, strict
Clippy and 885 / 1,012 test executions (overlapping configurations). The
experimental dependency is not included in that production hardening run.

The original dependency copy is `original-mikktspace/`, the explicitly marked
experimental copy is `candidate-mikktspace/`, and `candidate.patch` preserves
the exact source delta. The candidate adds one call-local scratch vector of
two three-component vectors per face in the largest group. It does not retain
memory across meshes or frames.

## Projection-only result: gate missed

All eight processes exit zero without matched diagnostics. `analyze.mjs`
validates 800 measured clocks plus 32 warmups, exact clock sequences and
nearest-rank summaries. Every output digest and job/batch/input-bound field
matches the original at its corresponding clock, including repeated/reverse
warmups. No task-owned builds overlap timing; unrelated host work is unchanged.
Input, dependency-directory, source and executable manifests verify afterward.

| Workload | Original median ms | Candidate median ms | Median change | p95 change | Maximum change | Mean peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Cow | 30.704142 | 28.753741 | -6.35% | -5.01% | -1.17% | +1.12% |
| Bird | 6.170330 | 5.770128 | -6.49% | -5.64% | -10.06% | +0.81% |

The nearby cow pairings improve 6.18% and 6.53%; bird improves 5.89% and
7.08%. Whole-process wall time improves 4.28% and 1.99% respectively, but
includes loading, setup and hashing outside the kernel timer. These are
mean-of-two process summaries, not pooled medians or native frame times.
The cow result misses the predeclared 10% kernel target. `driver=0` and
`analysis=1` are retained in `benchmark-status.txt`; the analyzer reports that
specific failure. The candidate is not integrated and no target is relaxed.

Saved probe SHA-256:

- Original: `c5db07ba7db25cb837e9c3302124804f4ee4c2284cf79218ead9850740e61d0d`.
- Projection-only: `05df8763230475e3287d5da8468f37b890d9caf79bf5ac22ebc5e22b3a768e8e`.

## Separate temporary-list reuse candidate

The same source loop allocates and drops a member-index vector for each face,
even when an identical subgroup was already found. The second candidate adds
a reusable temporary list to the projection-only candidate, cloning its sorted
members only when creating a distinct subgroup. Membership, sorting, numeric
evaluation and output order remain unchanged. Extra scratch is one call-local
`usize` per face in the largest group; it is not retained across meshes/frames.
This is a distinct implementation, not an unchanged rerun of the failed one.

Use the same original probe and the unchanged kernel gates above, including
10% cow improvement, both bird pairing bounds, output digests and RSS. Retain
this second candidate separately under `combined-mikktspace/` and its results
under `reuse/`. Do not overwrite the projection-only failure. No production
integration, native capture or full-editor timing is authorized by this probe
alone; the broader checks remain necessary if the combined kernel gate passes.
