# Streamed tangent groups

**Status:** correctness, callback order, allocation and process-memory gates
pass, but the cow elapsed target fails. The source remains isolated and must
not be adopted or rerun unchanged.

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

## First checks

The finite corpus passes 3,474 cases and 698,838 supported corners. A separate
trace run also matches all 11,729,458 geometry callbacks in order, including
the writes. The deliberately altered trace is rejected. Both runs still compare
all tangent components and orientation bits; the original one-bit output
negative remains rejected.

The six existing allocation gates pass. On a 64-by-64 grid, connected-case
allocation/reallocation calls fall from 60,852 to 48,429; requested bytes fall
from 11,088,760 to 9,879,944 and peak live requested bytes from 2,982,368 to
2,621,440. Disconnected calls fall from 49,199 to 24,610; requested bytes from
11,697,336 to 8,355,224 and peak live requested bytes from 4,096,088 to 2,621,440.
All generation allocations are released. These are not elapsed or RSS gains.

With no usable UV faces, allocation counts, requested bytes and peak live
requested bytes now exactly match the original (27 / 1,891,648 / 655,360).
One/eight usable-face cases reduce calls and requested bytes with unchanged
peak live requested bytes. The sparse gates therefore pass without reserving
the previous arena's unused 49,152 bytes.

The initial candidate compile fails with missing type inference for the
closure-captured tangent vector. An explicit `Vec<Option<TangentSpace<O>>>`
annotation fixes compilation; no numeric logic changes. `corpus.log` and
`corpus.exit` retain that failure; `corpus-fixed.log` / `.exit` record the
successful rerun, including allocation and negative controls.

Strict all-feature library Clippy and no-default-feature compilation pass for
both isolated MikkTSpace sources. The new sparse-allocation gate rejects the
previous arena implementation on the zero-usable-face fixture; that control's
exit and diagnostic remain in `sparse-negative-control.*`. The new source does
not merely bypass the accounting checks. Fresh replay of the committed patch
matches every candidate source file byte-for-byte.

## Real-asset result: cow elapsed gate failed

Source patch and geometry checks are retained at `cbb60b7`; the unchanged
real-asset driver/analyzer/negative controls are in `benchmarks/tangent-arena/`.
Both executables use isolated viewer `210521d`. The saved original's manifests
and executable hash verify, and its runtime/probe source has no differences
from the main checkout. The candidate build takes 4m08s on Rust 1.98.1; both
binaries then pass the same 743 default-feature library tests, with 25 existing
ignored cases. The candidate lockfile removes only the MikkTSpace registry
source/checksum; the viewer's original lockfile is restored before timing.

All eight processes exit zero without application diagnostics. The analyzer
verifies 800 measured clocks, 32 warmups and 22,876 tangent jobs, with identical
corresponding output digests, work counts, and four-job / 16 MiB input bounds.
Source, binary and the 268-entry asset manifests verify after measurement.

| Workload | Original median ms | Candidate median ms | Median change | p95 change | Mean peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cow | 30.790245 | 29.462650 | -4.31% | -7.48% | -0.75% |
| Bird | 8.433405 | 7.873508 | -6.64% | -10.04% | +0.90% |

Values compare means of two process summaries, not pooled medians or native
frame time. Cow nearby pairings improve 4.44% and 4.19%; bird pairings improve
8.23% and 3.87%. Cow still misses the unchanged 5% mean-of-medians target, so
the implementation is not qualified. Both mean peak RSS bounds pass. The bird
maximum increases 11.98%, which remains recorded even though it is not a
separate kernel threshold. Whole-process wall time falls 3.29% / 4.54%, but
includes untimed setup and is not application elapsed acceptance.

Starting one-minute host load ranges from 7.18 to 9.69. Default affinity and
four workers are unchanged, and no owned build or capture overlaps timing.
Unrelated work is left alone. Comparisons with the preceding trial's percentages
or absolute RSS do not isolate the effect of streaming; only the contemporaneous
original/candidate comparison above is measured here.

`driver.exit = 0`, `analysis.exit = 1` and `verification.exit = 0` distinguish
successful execution, the failed elapsed target, and successful rejection of
altered clocks/digests/doubled timings. No failure is waived. Saved SHA-256:

```text
before c092b2331eb27d4b9c07735151d52298ea3a3fdcd99b926f307f7c480b4cdbaf
after  b5ef99640758d16c0de77b3d2222751d063e622d1002d2f4f949bab26e72dbad
```

## Next implementation boundary

Streaming removes the unused reservation and passes this run's resource limits,
but repeated subgroup projection and temporary member-list construction still
remain. The older projected-subgroup experiment offers a measured hotspot,
not an approved implementation to enable unchanged. In particular, that older
code moves the normal callback outside the face loop and therefore changes
Geometry callback counts; it cannot simply be pasted into this candidate while
claiming the new callback-order contract still holds.

A further candidate should preserve every geometry read, share projected pairs
only for identical current-normal bits within a group, invalidate on a changed
normal, and bound/release its scratch. Exact bit keys must distinguish signed
zero. Any combination needs its own output, callback, changing-normal,
allocation and real-asset gates. The failed elapsed result remains intact.
Stable whole-editor controls, packaging/downstream use, native rendering,
lifecycle/browser checks and all broad workstreams remain unaccepted.
