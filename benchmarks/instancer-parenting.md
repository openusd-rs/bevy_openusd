# Point-instancer parent-link update investigation

## Measured discovery

The broader editor-hierarchy ABBA comparison discovers a separate scale problem
in the saved **before** renderer/editor path. For
`assets/external/PointInstancedMedCity.usdz`, 40,008 mesh entities share eight
mesh assets. A fresh opening takes 134 ms in the first process, but 100 seeks at
0/5/10/5 have a 1,482.535 ms median and 4,044.756 ms maximum complete update.

Evidence:
`/tmp/bevy-optimization-inspection-abba.70KixS/city-1-before.log`.
The point-instancer route accounts for 155,276.074 ms across those 100 seeks;
the next largest mesh route takes 170.275 ms total. Thus this is not explained
by inspector snapshot work. The old idle-only city characterization did not
exercise this expensive path. Host load is shared and high; this is attribution
evidence, not an isolated latency bound. The active matrix is retained unchanged.

## Source-grounded hypothesis

`crates/usd_bevy/src/route/instancer.rs` collects existing instances by stable ID,
but reinserts `ChildOf(entity)` for every instance, including unchanged parents.
The pinned Bevy 0.20.1 relationship discard hook removes the child from its
target collection. For a vector collection that operation performs a reverse
linear search followed by `Vec::remove`; insertion appends the child again.
Repeating that operation for all siblings can do quadratic work. The analogous
prototype-hierarchy path also reinserts its parent links.

This source path is a concrete candidate, not yet a measured attribution to the
specific hook. Isolate a large sibling set and compare unchanged versus changed
parents before claiming the full route cost is caused by it. No production
parent-link change has been made at this checkpoint.

The pinned public `replace_children` implementation computes indexed existing/
final sets, preserves retained `ChildOf` links and replaces target ordering in
bulk. It is a candidate for source-order restoration and bulk detachment without
manually mutating relationship internals. The caller must include unrelated
children it intends to retain. Shrink/clear paths also need inspection: repeated
individual child removals can have the same sibling-list cost.

## Required repair constraints and evidence

- Preserve stable instance IDs/entities, runtime-added components, source-order
  child ordering, unrelated children, prototype/subset hierarchy and independent
  roots. Merely omitting `ChildOf` replacement could leave child order stale when
  authored IDs reorder; test that explicitly.
- Only update actual parent changes. Restore ordering in bulk without repeating
  linear sibling removals. Newly spawned, removed and externally reparented
  nodes must retain consistent forward/inverse relationships and cleanup.
- Keep valid-last-state behavior for malformed IDs/transforms, masking and
  prototype failures, and preserve current CPU/GPU-instancing policies.
- Exercise repeated/unique/reversed clocks, ID reorders/growth/shrinkage,
  nested prototypes, source reload and close. Verify actual city renders plus
  focused library/native gates; asset-count equality alone is not visual proof.
- Before changing code, record a scaling fixture and a saved release baseline.
  A focused target is at least 75% lower city seek median, with no repeatable
  greater than 5% small-instancer regression. Keep the original failure and raw
  samples; do not redefine the target afterward or disable validation/instances.
- This is another workstream in `OPTIMIZATION.md`, not whole-library completion.
