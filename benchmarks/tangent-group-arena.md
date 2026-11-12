# Tangent group membership arena

**Status:** exact-output and focused allocation checks pass, but the real-asset
kernel gate fails the cow elapsed target and both process-memory limits. The
candidate remains isolated and is not adopted or rerun unchanged.

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

## Real-asset result: gate failed

Both release executables build from isolated viewer commit `210521d`, with the
same source and compiler settings. Each passes 743 default-feature library
tests; 25 existing ignored tests do not run in that suite. These are the same
tests in two dependency configurations, not 1,486 distinct tests. The tested
lockfiles differ only by replacing registry MikkTSpace with the local candidate.
The viewer lockfile is restored after building; the main checkout's manifest,
lockfile, runtime source and OpenUSD pin are unchanged.

The committed harness at `aa08853` completes all eight processes without errors
or application diagnostics. It validates 800 measured clocks, 32 warmups and
22,876 tangent jobs, with exact corresponding output digests, work counts and
four-job / 16 MiB input bounds. All 268 asset-manifest entries, executable hashes
and source manifests verify after the run. Fresh patch replay matches the
candidate source byte-for-byte.

| Workload | Original median ms | Candidate median ms | Median change | p95 change | Mean peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cow | 30.292536 | 28.788691 | -4.9644% | +17.13% | +7.22% |
| Bird | 6.638448 | 5.718692 | -13.85% | -41.79% | +10.33% |

These are means of two process summaries, not pooled medians or native frame
times. Cow nearby pairings improve 4.91% and 5.02%; bird pairings improve 20.86%
and 5.70%. The cow mean misses the unchanged 5% target; rounding it to 5% would
incorrectly call a failure a pass. Both mean peak RSS increases exceed 5%.
The cow p95 increase remains recorded even though it is not a separate kernel
threshold. Whole-process wall times fall 3.37% / 18.69%, but include untimed
setup and do not establish application performance.

Starting one-minute host load ranges from 4.50 to 5.90 on the i7-13700KF.
Default affinity remains CPUs 0–23, the worker count stays four, and no owned
build/capture overlaps measurements. Unrelated activity is recorded and left
alone; it is not used to waive any failure. The two builds finish before timing
and take 3m54s / 3m38s.

`driver.exit = 0` and `analysis.exit = 1` retain the successful execution and
three failed acceptance conditions separately. `verification.exit = 0` means
the analyzer rejects altered clocks, altered digests and doubled candidate
timings; it does not accept the original performance result. Allocation
controls separately reject no reduction and retained storage, and the corpus
rejects an altered output bit.

Saved executable SHA-256:

```text
before c092b2331eb27d4b9c07735151d52298ea3a3fdcd99b926f307f7c480b4cdbaf
after  1cdbe987f498f50f43bfb5f65fcda6e7866070a0120d8da6147afe63a2aceb5e
```

## Sparse-group diagnosis and next action

A post-timing diagnostic varies usable UV faces in a 2,048-triangle grid. With
none, the original makes 27 allocation/reallocation calls and requests
1,891,648 bytes; the candidate makes 28 calls and requests 1,940,800 bytes.
The extra 49,152 bytes exactly match the eagerly reserved `3 * 2048 * usize`
membership arena. One/eight usable-face cases also request more total storage
than the original. Output bits match and all call-local allocations are freed;
peak live requested bytes stay 655,360 for these sparse cases. The final
`sparse-corpus-allocations.log` retains those diagnostics and repeats the corpus,
six original allocation gates and negative controls successfully.

This proves unnecessary reservation when few/no groups form; it does not prove
that this is the cause of the real-asset RSS increases. Fewer allocator calls
and lower synthetic live payload do not imply lower whole-process memory.
The next implementation must address actual membership occupancy and process
memory behavior before another real-asset comparison. Do not repeat this source
unchanged, combine it with a previously rejected candidate without separate
qualification, or proceed to editor/render integration on this failed gate.
Stable whole-editor controls and native/lifecycle/browser acceptance remain
open, along with the full workstreams in `OPTIMIZATION.md`.
