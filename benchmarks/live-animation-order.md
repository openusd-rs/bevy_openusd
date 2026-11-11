# Live animation traversal order

## Evidence and scope

The [image-capacity matrix](mip-capacity.md#matched-real-scene-capacity-measurement)
records different retained tangent-cache payloads between fresh executions of
the same baseline. Live resampling passes `AnimatedPrims(HashSet<String>)`
iteration directly to built-in animation batching. That order is not stable
across independently seeded sets. The path/entity map is already a BTreeMap,
and worker results already publish in input order; neither sorts this input.
Tangent admission and recency depend on visitation order.

Investigate a path-ordered view for built-in live animation only. Preserve the
public HashSet resource, custom callback order, route registration order,
same-clock short circuit, membership snapshots, changes made by custom callbacks,
source edits/reloads and independently timed asset roots. Keep current projection,
parallel-job and cache budgets. Do not introduce persistent ordering caches or
retain stage/entity history. Sorting scratch holds references for the current
pass only; the existing owned animated-set snapshot remains unchanged.

## Acceptance declared before implementation

- Reproduce unordered built-in traversal. Verify lexical order independently of
  hash seed/insertion order, empty sets, and externally changed membership.
- Exercise actual custom callbacks with and without the built-in registry.
  Preserve their current iteration order and resource visibility; mutations to
  animated membership during a callback affect the next time change, not the
  current snapshot. Repeating a clock must not run callbacks again.
- Pass hardening, native deformation, watcher/export and wasm checks. Keep the
  public `AnimatedPrims` and `SchemaRegistry` APIs intact.
- Use saved release CPU editor binaries in matched fresh-process ABBA runs for
  bird/cow/arm/subsets, idle and quick seeks, with the same allocation recorder.
  Candidate tangent footprints must repeat within each workload and remain
  inside existing bounds. Payload, geometry and asset counts must be preserved.
  Do not require the newly ordered cache to retain the same entries as a randomly
  ordered baseline; retain the earlier equality failures as reproduction evidence.
  Record all timing observations and flag greater-than-5% pair-mean increases.
- Validate rendered output through the **live-editor resampling path** after
  actual time changes. Ordinary `viewer_capture` uses asset roots, so its static
  images cannot establish this change's live-route fidelity. Do not automate the
  user's desktop or Play button; use owned application/offscreen capture.

Stable order is a hypothesis about the observed cache variation until tested in
fresh processes. It does not by itself establish faster playback or complete the
loading, memory, browser, native-window or 60 Hz acceptance requirements.

## Reproducer and implementation

The pre-change library run passes 728 tests and fails only the new ordering
fixture: it visits `/b, /A/Child, /Z, /B, /A` rather than lexical path order.
Both actual custom-registry cases already pass, including a callback adding
`/Late` to the public animated set while the current snapshot is being visited.
Log: `/tmp/bevy-optimization-animation-order-before.log`.

The candidate sorts borrowed path references only when `is_builtin_only()` is
true. Custom registries retain their original HashSet iterator without creating
the sorting vector. The existing cloned membership snapshot and lazy path/entity
filter stay in place. Tests cover independently constructed sets, changed
membership, empty sets, custom-only and extended-built-in registries, callback
visibility/mutation of the animated resource, restoration of the live stage/map,
and suppression of repeated-clock callbacks.

Workspace hardening passes 841 no-default / 958 all-feature/all-target test
executions, strict Clippy and formatting. Both new tests pass in both feature
configurations. Log: `/tmp/bevy-optimization-animation-order-harden.log`.
Fresh-process cache determinism, native oracle/reload/export, wasm and actual
live-route rendered fidelity remain pending at this implementation checkpoint.
