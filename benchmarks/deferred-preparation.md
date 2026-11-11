# Parallel preparation with deferred hidden geometry

## Contract and implementation

The library remains eager by default. `DeferHiddenMeshes` stays opt-in; the viewer
does not enable it in this change. The withdrawn larger tangent queue remains
withdrawn, with the original four-mesh default and eight-mesh hard batch cap.

Previously the deferral resource disables all parallel animation preparation,
including already resident visible meshes. Preparation now carries the target
entity with each route context. With deferral enabled, only entities already
carrying `Mesh3d` are eligible for ahead-of-route preparation. Initially hidden
and newly revealed meshes retain the ordinary synchronous route path, which
evaluates current visibility/purpose before decoding or promoting geometry.
Resident meshes remain retained and updated even when hidden again.

This avoids predicting visibility from stale ECS state or executing visibility
routes out of order. No source read, mesh allocation or tangent job is prepared
ahead for a nonresident entity. Shared residency budgets, custom registries,
missing/single-thread pools, zero limits and oversized input keep their existing
fallbacks. USD/World reads remain on the main thread; worker inputs remain owned.

## Acceptance declared before measurement

- Compare all mesh attributes, indices, morphs and CPU/GPU fallback state with
  serial projection across unique, reversed and repeated clocks. Mix hidden and
  resident siblings, edit while hidden, reveal at the current clock, and hide
  again while checking that retained meshes keep updating.
- Preserve animated ancestor visibility, purpose filtering, independent-root
  clocks, source reload, close/despawn, shared promotion budgets and custom routes.
  Prove nonresident hidden meshes have zero geometry decodes/preparation jobs.
- Run hardening, native deformation/watchers/export and wasm gates. Keep library
  defaults and viewer policy unchanged until separate visible/reveal acceptance.
- Use saved release binaries, the same opt-in deferral setting for both, and
  individually paired full-editor updates with reversed opening/update orders.
  Target at least 15% lower bird paired median in both execution-order strata
  and both fresh pairs, without >5% cow/arm/subset paired median regressions.
  Retain secondary timing failures; do not waive earlier failed observations.
  Require unchanged payload/cache/asset counts between opt-in versions, bounded
  jobs/input bytes, zero diagnostics, and no >5% peak RSS increase.
- Compare actual native animation final poses against independent pre-change
  renders at matching clocks and zero RGB tolerance. This does not clear older
  grid/CPU-GPU/MDL failures or establish native-window/browser FPS. The full
  `OPTIMIZATION.md` workstreams and 60 Hz target remain open.

## Correctness checkpoint

Hardening passes 854 no-default and 976 all-feature/all-target executions,
strict Clippy, formatting and whitespace checks. The new CPU/GPU regression
mixes six hidden and five resident siblings, edits orientation while hidden,
reveals at a changed clock, reverses/repeats clocks and hides retained meshes
again. Full mesh/fallback state matches serial projection. Before reveal only
five meshes decode; after promotion all eleven stay current. Only meshes already
resident before the update enter preparation, with the original job/byte limits.
Adding a residency budget to resident meshes restores the serial fallback.

Independent-root clock/reload/close coverage now runs both eager and deferred
resource configurations for CPU and GPU projection. Existing ancestor-visibility,
purpose, shared-budget, source-reload and custom-route tests remain intact.
This establishes correctness coverage, not a measured speedup or rendered pass.
Evidence: `/tmp/bevy-optimization-deferred-preparation.mJketj/harden.log`.
