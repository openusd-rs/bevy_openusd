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

## Isolated relationship scaling

`examples/parenting_benchmark.rs` compares the original transform/`ChildOf`
bundle replacement with transform updates, conditional parent repair and bulk
order restoration. Each lane owns an independent world. Every update verifies
the exact child order, inverse parent links and transforms outside the timed
interval. The lanes alternate execution order, with two warmups and eight
measured samples per stable/reversed-order case. This does not exercise USD,
prototype geometry or rendering and is not an end-to-end speedup claim.

Release evidence: `/tmp/bevy-optimization-parenting-probe.5Oyo4w/`. All three
processes finish successfully without task-owned compilation overlap;
case-boundary one-minute load averages are 22.62–24.15. The saved editor baseline
has the hierarchy cache (`6d4f8ae` production), so subsequent city comparisons
will not conflate that earlier change with parent-link reconciliation.

| Children | Stable reinsert / retain median, ms | Reversed reinsert / bulk median, ms |
| --- | --- | --- |
| 1,000 | 0.410 / 0.017 | 0.245 / 0.031 |
| 10,000 | 36.864 / 0.173 | 18.007 / 0.311 |
| 40,000 | 625.954 / 0.712 | 320.271 / 1.281 |

The original stable-order path grows approximately quadratically while the
retained-link path scales approximately linearly over this range. This confirms
the isolated hook bottleneck and motivates the production repair; the complete
city route's remaining cost must still be measured independently.
