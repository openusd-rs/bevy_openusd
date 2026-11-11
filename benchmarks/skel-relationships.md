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
files under `/tmp/bevy-optimization-`. Saved-binary performance/render comparison
is still pending; passing these checks is not elapsed-performance acceptance.

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
