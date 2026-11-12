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
evidence, not an isolated latency bound. The original matrix is retained unchanged.

## Source-grounded hypothesis

`crates/usd_bevy/src/route/instancer.rs` collects existing instances by stable ID,
but reinserts `ChildOf(entity)` for every instance, including unchanged parents.
The pinned Bevy 0.20.1 relationship discard hook removes the child from its
target collection. For a vector collection that operation performs a reverse
linear search followed by `Vec::remove`; insertion appends the child again.
Repeating that operation for all siblings can do quadratic work. The analogous
prototype-hierarchy path also reinserts its parent links.

At the initial checkpoint this source path is a candidate, not yet a measured
attribution to the specific hook. The isolated scaling experiment and subsequent
production checkpoint follow below; they do not attribute all route cost to it.

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

## Primary instance-link repair

`3562782` retains unchanged instance `ChildOf` components, while new instances
receive the complete transform/visibility/parent bundle together. It collects
the exact source-ID child order after unrelated children and uses Bevy's public
bulk replacement only when the final order differs. Stale instances are detached
in that bulk operation before despawning; full instance clearing similarly
preserves unrelated children and detaches owned siblings before destruction.
This adds temporary order/partition vectors, not a retained cross-frame cache.

Two regressions cover steady updates, reordering, growth/shrinkage, malformed
last-good recovery, stable IDs/runtime components, unrelated children and
2,048-child descendant cleanup. An actual `Discard<ChildOf>` observer verifies
that retained/reordered links emit no replacement events, while removed IDs
discard exactly their removed links. Existing nested-prototype, independent
clock, masking and geometry tests remain in the hardening suite. The separate
prototype-part parenting loop has not been optimized by this commit.

`harden` passes 819 no-default tests and 919 all-feature/all-target tests, strict
Clippy, formatting and whitespace checks. These are overlapping configurations,
not distinct-case totals. Log: `/tmp/bevy-optimization-instancer-harden.log`.
The first new-test compilation failed on a vector conversion and a borrowed-key
type; both fixture errors were corrected before this passing run.

Native gates pass: four deformation checks, five asset and four editor texture
watcher checks, fourteen native exports plus one reference custom-data round
trip, and wasm compilation. Release benchmark and debug capture builds complete.
The ledger is `/tmp/bevy-optimization-instancer-gates.tsv`; individual logs use
`/tmp/bevy-optimization-instancer-*.log`. The original editor opening and bird
image failures remain.

## Saved-release city and control matrix

`/tmp/bevy-optimization-instancer-abba.ZI0ygi/` completes twenty processes and
16,400 measured seek updates: city has 100 repeated clocks per process, four
small fixtures have 1,000 unique clocks each. Every process has four warmups and
one fresh App, in before/after/after/before order. Before production is `6d4f8ae`
and after is `3562782`. No task-owned compilation or rendering overlaps.

All five source/package hashes verify. Retained geometry/image/cache payloads
and peak asset counts agree across every workload's four processes, including
the city's 40,008 mesh entities and eight shared mesh assets. Raw seek samples,
binary hashes, logs and `analysis.tsv` are retained. Profiling/logging is enabled
in both lanes. Unrelated one-minute host load ranges 25.76–48.88 at case boundaries.

| Workload | Before update medians, ms | After update medians, ms | Change in average medians |
| --- | --- | --- | --- |
| City | 1,369.624 / 864.449 | 50.533 / 53.636 | -95.3% |
| Hierarchy | 1.385 / 0.881 | 0.736 / 0.727 | -35.4% |
| Affine subsets | 0.706 / 0.756 | 1.025 / 0.735 | +20.4% |
| Deformation | 0.695 / 0.585 | 0.689 / 0.703 | +8.8% |
| Material subsets | 0.697 / 0.317 | 0.371 / 0.654 | +1.1% |

The city exceeds its predeclared 75% reduction target by a substantial margin
in both repetitions. Its opening falls from 97.174/70.607 ms to 80.502/72.403 ms.
All small-control average openings also improve in this run. This measures the
complete headless seek update, not native FPS; 50–54 ms still leaves significant
work and does not establish a 60 Hz rendered result.

The **full small-control gate remains unresolved**: affine and deformation
average medians exceed the 5% limit, with large within-lane host variation.
Their independent three-fresh-App-per-process ABBA follow-up retains this
original failure rather than redefining or discarding the gate.

City process high-water RSS is 132.2/129.8 MB before and 130.2/129.7 MB after.
The first baseline's final resident reading falls to 86.3 MB despite a higher
earlier reading; paging makes it unsuitable for a retained-memory-saving claim.
Small controls have approximately 1.1 MB lower RSS after the change, but this is
not attribution to the temporary child-order vector or full lifecycle proof.

## Small-control follow-up

`/tmp/bevy-optimization-instancer-controls.gL3QC5/` completes eight processes,
24 fresh Apps and 24,000 measured unique-clock updates. The immutable binaries,
profiling settings and ABBA order are unchanged. Both source hashes and payload
signatures verify. No task-owned compilation or rendering overlaps; case-boundary
one-minute host load is 36.63–40.17.

Each cell below is the median of per-App seek medians (six per lane for the all
column); even-sized groups average their two middle observations.

| Workload | All Apps before / after, ms | Change | First App before / after, ms | Later Apps before / after, ms |
| --- | --- | --- | --- | --- |
| Affine subsets | 0.643 / 0.754 | +17.1% | 0.525 / 0.533 | 0.643 / 0.760 |
| Deformation | 0.653 / 0.514 | -21.3% | 0.669 / 0.514 | 0.653 / 0.520 |

Deformation's regression does not repeat, but affine's aggregate/later-App
regression does. The **full regression gate is still not accepted**. Individual
affine after medians vary 0.335–1.753 ms and even unchanged snapshot work varies;
that is evidence to investigate host scheduling, not permission to waive the
failure. A separate P-core-affinity diagnostic with an unaffected non-instancer
control will test that explanation. It cannot replace default-host acceptance
or change the production task-pool policy.

The completed diagnostic is
`/tmp/bevy-optimization-instancer-affinity.zhdJKl/`: eight processes and 24 fresh
Apps, with only the benchmark processes restricted to P-core logical CPUs
0/2/4/6. Both source hashes and payload signatures match. Using the same
median-of-App-medians statistic, affine is 0.318553 ms before versus 0.320301 ms
after (+0.55%); the unaffected non-instancer subset control is 0.171608 versus
0.172907 ms (+0.76%). Both are within 5% in this configuration. This supports a
scheduling/environment contribution but does not prove the exact cause or erase
the two default-affinity failures. The diagnostic changes available worker
parallelism as well as CPU placement, so it is not a default-native acceptance
substitute. No production scheduling settings or unrelated processes changed.
Its GNU time records include the Oslo/Nix wrapper process tree; those CPU/RSS
figures are not isolated application cost and are not used for speedup claims.

## Render comparison

`/tmp/bevy-optimization-instancer-captures.OEown1/` completes eight captures on
the native RTX 4080 / NVIDIA 595.112.07 Vulkan device, with validation enabled,
forward rendering, four-sample MSAA and 1280×720 output. City at authored time
184, two independently timed hierarchy roots reversed after 30 ready frames,
affine subsets at zero and deformation at five all have **zero changed pixels**
between before/after binaries at strict RGB tolerance zero. All source hashes,
pipeline readiness and renderer-log checks pass. The hierarchy metadata records
final clocks `[10.0, 0.0]`; this exercises updates, not just initial projection.

The actual captures were inspected: the authored city camera shows the dense
building field; the hierarchy, affine panels and deformed columns remain present.
The fixed affine view crops part of the rightmost panel equally in both lanes;
these comparisons establish preservation, not exhaustive framing or native frame
cadence. Capture timing is not used as a performance result.

The earlier attempt under
`/tmp/bevy-optimization-instancer-captures.HUQgCx/` timed out because the script
requested the nonexistent `/MediterraneanHills/main_cam`. Source inspection
identifies `/Cameras/main_cam`; correcting that capture-only configuration gives
the successful run above. The original failed artifact is retained. No camera,
render-quality or tolerance change was made to the library or user's desktop.
