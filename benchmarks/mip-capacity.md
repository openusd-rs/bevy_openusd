# Mip output buffer capacity

## Target and invariants

`add_mips` appends encoded levels to the original image's `Vec<u8>`. Repeated
pushes can geometrically grow that buffer beyond the complete chain's payload,
retaining unused capacity for the image's lifetime. This is distinct from source
resolution, encoded payload, float-level scratch space, allocator overhead, RSS
or GPU memory. The existing source-only first reduction and exact sRGB encoder
remain unchanged.

Plan: after validating the supported image and its base byte count, calculate
the complete chain's encoded size with checked arithmetic and reserve only the
additional required bytes, before allocating any float levels. Reuse an existing
larger reservation without shrinking or copying it. Do not change filtering,
source texels, mip order, levels, wrapping, generated-image tracking or readiness.
Unsupported, malformed, already-mipped and single-texel images remain unchanged.

## Acceptance declared before implementation

- Reproduce geometric over-allocation with the current implementation.
- Check all five supported formats with square, odd and one-dimensional images,
  both minimally sized and already over-reserved source buffers. All output bytes
  and levels must match the independent scalar reference; repeat finalization
  must not alter output or capacity.
- On the tested allocator, a fresh image's reported buffer capacity must equal
  its complete encoded length, unless an existing larger reservation is reused.
  This tests requested buffer capacity, not physical allocator or process usage.
- Exercise checked size calculation at zero, narrow and overflowing dimensions.
- Run hardening and native reload/deformation/export plus wasm compilation.
  Compare native bird/cow/arm/subset images against saved pre-change captures.
- Record actual retained capacity and timing before/after on representative
  textured scenes, without overlapping task-owned compilation. Do not infer a
  process-memory or startup improvement from small fixtures alone, and retain
  earlier performance failures. Full acceptance remains in `OPTIMIZATION.md`.

## Reproducer and candidate

The pre-change `oslo make test-library` run reproduces over-allocation for a
1x9 RGBA8 sRGB image: complete output is 64 bytes, while the retained buffer
capacity is 72 bytes. Log: `/tmp/bevy-optimization-mip-capacity-before.log`.
That first configuration reports 724 passing tests and two failures: the new
capacity assertion and an existing point/curve fixture reading entities after
one update without guaranteeing projection completion. The latter fixture is
corrected separately in `86315b2` with an unlimited test projection budget and
explicit Ready assertions for both roots, not a production budget change.

The candidate computes each level's encoded byte count with checked multiply/add
and calls `reserve_exact` once before reduction. There are no source/texture
format, encoder, filter or runtime-resource additions. The test covers sixty
format/dimension/reservation combinations against the scalar reference and
checks repeat-finalization capacity. Checked-size tests cover narrow chains,
base multiplication overflow and total-chain addition overflow. A 4096x4096
RGBA8 chain requires 89,478,484 bytes; that calculation alone is not an observed
real-scene capacity or RSS reduction.

## Correctness checkpoint

Final hardening passes 839 no-default / 955 all-feature/all-target test executions,
strict Clippy and formatting. Both new mip tests and the repaired point/curve
fixture pass in both configurations. The allocation fixture uses byte value 51,
which also forms finite half-float channels; nonfinite half behavior remains
covered separately under the existing class/value contract. The configurations
overlap and are not distinct test inventories.

Four native deformation tests, five asset watchers, four editor texture watchers,
fourteen native exports and the custom-data reference round trip pass. Wasm
compilation passes. Ledger: `/tmp/bevy-optimization-mip-capacity-gates.tsv`;
logs: `/tmp/bevy-optimization-mip-capacity-final-{harden,test-native-deformation,test-watchers,test-native,check-web}.log`.
The earlier hardening log also passes before changing the allocation fixture's
constant to a finite half-float value.

Native rendered comparison, real-asset retained buffer capacity/timing, RSS and
window/browser performance remain unverified for this change. Neither the
smaller fixture capacities nor passing correctness checks clear any earlier
performance failure or complete the memory/lifecycle workstream.
