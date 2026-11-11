# Sharing scoped skeleton query handles

## Source finding

The existing `SkelSampleScope` stores owned `SkeletonQuery` and `AnimQuery`
values. Every cache hit clones a query. In the pinned OpenUSD source
`900ea31591348e3fbf416d4661d703aa8700771b`, `AnimQuery::clone` deep-copies its
`Vec<String>` joint order. `SkeletonQuery::clone` shares the definition but
clones its bound animation and mapper. A hit therefore still allocates and
copies immutable query metadata before classification or deformation.

The candidate shares those two private query handles with `Rc` inside the
existing stage-specific read-only scope. No new cache or persistent lifetime
is introduced. Pose samples, mesh mappings, influence reads, arithmetic,
unsupported-route behavior, custom-registry behavior and public APIs stay the
same. An unscoped lookup still builds a fresh query; failed construction still
returns before cache insertion.

## Acceptance declared before measurement

- Prove pointer sharing across cache hits and independently rebuilt mesh
  bindings; preserve absence, nested scopes, stage isolation and later edits.
  Weak handles must expire after scope cleanup when no consumer retains them.
- Run workspace hardening, native CPU deformation and wasm compilation. Existing
  bit-exact upstream-query tests must still pass for changed clocks, mappings,
  influences and geometry-bind transforms.
- Save release binaries before and after the change and compare the same
  whole-editor CPU workloads in alternating order without task-owned builds.
  Retain raw samples, opening/idle/tail/resource regressions and diagnostics.
  Reject a repeatable greater-than-5% control regression. Seek evidence for
  lower preparation cost, not a fabricated aggregate FPS claim.
- This is not a replacement for the existing native 60 Hz, visual, loading,
  resource, downstream or other-platform gates. Those remain open even if this
  local copy-elimination candidate passes its correctness checks.

The fresh pre-change release example is saved with revision `e6ee48e` under
`/tmp/bevy-optimization-shared-queries.cQLdQZ/` (`before-editor`,
`before-build.log`, `before-inputs.sha256`).

## Correctness checkpoint

Workspace hardening passes 882 no-default and 1,009 all-feature/all-target
executions, formatting and strict Clippy. The configurations overlap. The two
new sharing/lifetime/edit regressions pass, as do the existing bit-exact
upstream skinning comparisons. Four native OpenUSD deformation-oracle tests,
wasm compilation and the release editor benchmark build also pass. Logs are
`harden.log`, `native.log`, `wasm.log` and `after-build.log` in the evidence
directory. `candidate.patch` and `after-inputs.sha256` identify the tested code.

The alternating whole-editor CPU comparison is in progress. No measured
application gain or complete renderer/resource/platform acceptance is claimed
at this checkpoint.
