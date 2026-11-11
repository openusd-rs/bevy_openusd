# Scoped skeleton relationship sampling

## Evidence and proposed change

`9f78ca9` splits the previous eligibility timer into non-overlapping
deformation-kind checks, mesh decoding and GPU skin sampling. Standalone
`test-library` passes 720 no-default and 728 all-feature tests, and the release
capture build passes. These counts are not the workspace hardening counts.
Logs are `/tmp/bevy-optimization-eligibility-{tests,build}.log`.

The native diagnostic captures are under
`/tmp/bevy-optimization-eligibility-captures.Smm4Oq/`. Cow, bird and subsets
complete 300 measured playback frames plus 30 warmups with unchanged quality,
cameras, validation and default affinity. All 900 raw clock/frame rows validate,
all three settled final poses match the saved pre-refinement executable exactly
at RGB tolerance zero, and source manifests verify. Cow retains its MDL warning
and the driver exits 1. Host load is 31.49–42.68, with no task-owned compilation.

Bird mean phase costs per 330 changed-root updates are 4.199 ms for kinds,
2.785 ms for mesh decoding and 4.958 ms for skin sampling. Its measured main
median/p95 are 25.381/29.924 ms. Cow remains dominated by tangent dispatch/join;
its noisy main median is 133.051 ms. These are diagnostics, not improvements.

`read::skel::inherited_rel` repeatedly resolves local skeleton/animation
relationships while walking the same ancestors. `deformation_is_time_varying`
and initial GPU skin sampling both use those walks. The existing read-only
`SkelSampleScope` caches skeleton definitions/poses and one bound mesh, but not
these relationship reads.

The candidate memoizes successful local first-target reads, including
absence, only inside that existing stage-specific scope. It preserves
authored target order, parent traversal, empty/blocked fallback behavior and
uncached errors. Retention is capped at 4,096 entries and 1 MiB of key/target
text, not total allocator memory. Over-budget entries use fresh reads. No
historical clocks or persistent stage cache is added; the map is discarded
with the pass. Custom registries retain their existing uncached path.

`ffb32c6` implements this candidate. Three new regressions compare fresh walks
with normal/reversed query order, multiple/absent/empty targets, nested and
independent stages, subsequent edits, zero/entry/text caps, oversized fallback,
scope cleanup and an actual custom route mutating the next prim's relationship.
Existing CPU/GPU deformation and source/reload tests remain unchanged.
`harden` passes 835 no-default and 944 all-feature/all-target tests, strict
Clippy, formatting and whitespace checks. The configurations overlap; the log
is `/tmp/bevy-optimization-skel-relations-harden.log`. Refreshed native checks
pass four deformation-oracle, five asset-watcher, four editor-texture-watcher,
fourteen export and one custom-data reference round-trip cases. Wasm compilation
and the release capture build also pass. The ledger is
`/tmp/bevy-optimization-skel-relations-gates.tsv`, with corresponding
`skel-relations-{test-native-deformation,test-watchers,test-native,check-web,build}.log`
files under `/tmp/bevy-optimization-`. The saved-binary comparison below retains
the remaining performance failures; these checks are not full acceptance.

## Acceptance declared before implementation

- Compare cached and uncached resolution across ancestors, absent and empty
  targets, multiple targets, independent/nested stages and edits between passes.
- Verify limits, fallback and scope cleanup; preserve custom-route mutation
  visibility and the existing mesh/deformation/native-oracle assertions.
- Run repository hardening and native/wasm checks. Compare actual candidate
  renders at exact clocks with the saved pre-change executable.
- Use the saved `9f78ca9` release executable, identical native playback settings
  and alternating runs with no overlapping task-owned compilation. Target at
  least 10% lower bird main-update median and no repeatable greater-than-5%
  control regression. This is an incremental target, not a replacement for
  60 Hz, full opening, browser/window, resource or other outstanding gates.
- Retain the prior cow/subset regressions and all new failed observations.

The separate discovery that `PrimvarScope` currently wraps initial projection
but not animation is not included in this candidate. It needs its own bounded
and custom-route-safe assessment rather than conflating two optimizations.

## Native alternating comparison

`/tmp/bevy-optimization-skel-relations-abba.6NRMB5/` retains sixteen fresh
playback processes, before/after/after/before per case. The pre-change
`9f78ca9` executable SHA-256 is
`e0a2e8371af3451ab074b22b62555ea4ff1f872d2a349fc02acdfc325594be20`;
the candidate containing `ffb32c6`, saved at `3f73990`, is
`5ff060d6ebd4c3280ca8ea8dcfcbbe477d205bf4c80f29a6716a86d36a2f2003`.
Both use the refined phase instrumentation, default affinity, RTX 4080/Vulkan,
validation, forward rendering, 4x MSAA, 1280x720, matching full-object cameras,
30 warmups and 300 measured frames. No task-owned compilation overlaps.
Recorded host load spans 25.67–59.09; unrelated jobs are left untouched.

| Case | Before main median, ms | After main median, ms | Mean-of-medians change |
| --- | ---: | ---: | ---: |
| Bird play | 70.551 / 56.652 | 57.289 / 29.895 | -31.5% |
| Cow play | 52.819 / 125.264 | 52.653 / 98.578 | -15.1% |
| Material subsets play | 1.083 / 1.147 | 1.090 / 1.074 | -3.0% |
| Arm idle | 0.798 / 3.299 | 0.937 / 1.843 | -32.2% |

The bird's aggregate exceeds the declared 10% incremental target, but this is
one noisy matrix, not an isolated causal gain or a universal playback claim.
The unchanged arm-idle path and mesh/tangent phases also vary strongly. Bird
mean kinds/skin-sampling phases fall 24.8%/25.9%, while its unchanged mesh-read
phase falls 22.1% and tangent dispatch/join falls 11.4%. These observations
cannot attribute the entire main-time improvement to relationship reuse.
Bird and cow p95 main costs improve only about 3%; their candidate tails and
median offscreen intervals remain well above the 60 Hz budget.

Bird partial open/validation/projection totals regress **35.1%**: before
586.386/468.571 ms, after 743.648/681.827 ms. This is an unresolved loading
observation, not waived as noise. It is also not a measurement of complete
file-to-ready/editor opening. The other partial source totals improve, but
neither they nor this matrix clear the older kitchen/bird opening, affine seek
or cow/subset playback regressions.

`raw-analysis.json` validates all 4,800 raw frames, clocks, summaries, interval
threshold counts and published GPU/render-CPU arrays, with maximum clock error
5.685e-14. `phase-analysis.json` checks matching phase/job counts, zero tangent
failures, non-overlapping phase sums within the enclosing timer and identical
visible CPU/GPU mesh counts across each case. Worker wall times still overlap
and cannot be summed into main-thread cost.

All six candidate bird/cow/subset settled final poses match independent
pre-change renders at their exact clocks with zero changed RGB pixels at
tolerance zero. The two subset comparisons are in `extra-comparisons.tsv`;
the same ledger records three pixel-exact arm-idle comparisons against the
first baseline. Representative candidate bird/cow images were inspected.
All sixteen process/log checks and source manifests pass. Cow retains its
missing MDL warning, so the diagnostic driver exits 1; the extra-comparison
driver exits 0. These checks do not resolve the separately tracked CPU/GPU
bird pixel discrepancy or native-window/browser/lifecycle acceptance.

Full acceptance remains open. Follow-up must investigate the loading
observation and remaining kind/mesh/tangent costs with controlled evidence,
not repeatedly rerun until a favorable result appears.

The subsequent [matched opening comparison](opening-milestones.md#matched-results)
instruments both production versions with the same request-to-Ready recorder.
Bird/cow aggregate request-to-Ready regress 16.2%/7.7%, and all four pipeline
milestones exceed the 5% limit under high variable host load. Its twelve settled
image comparisons remain pixel-exact. These new failures remain open alongside
the original partial-phase regression; neither timing scope replaces the other.
