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

Real-asset retained buffer capacity/timing, RSS and window/browser performance
remain unverified for this change. Neither the smaller fixture capacities nor
passing correctness checks clear any earlier performance failure or complete the
memory/lifecycle workstream.

## Native fidelity checkpoint

Candidate `fdf88b7` builds in release and renders bird, cow, arm and subsets at
their fixed time-zero poses. Artifacts:
`/tmp/bevy-optimization-mip-capacity-captures.CvNwEi/`; build log:
`/tmp/bevy-optimization-mip-capacity-build.log`. Candidate executable SHA-256 is
`732ab79485080feaff84b16ccbd1209b293ae1a4c6e8a18dcc96b2c9d6426707`.
The reference is `05b3106`'s after-version warmup from the common-path matrix,
before the capacity change. Dependency lock and asset manifests match.

All four 1280x720 RTX 4080/Vulkan captures are pixel-exact at RGB tolerance zero;
bird/cow images were inspected. Renderer, camera, scene/visible-mesh counts,
texture image/input/appended-byte counts and settings agree. Four raw opening
records validate and Ready/final texture counters agree. Process/log/manifest
checks pass; the cow retains one missing-MDL warning, so the combined diagnostic
driver exits 1. This is fixed-pose fidelity evidence, not a performance comparison,
interactive playback or rendered-browser acceptance. No task-owned compilation
overlaps the captures.

## Next resource measurement

Current opening counters report payload lengths, not image-buffer capacities.
Add separate opt-in benchmark accounting for main-world `Assets<Image>` data
length and capacity, with tests for empty/missing data and pre-reserved buffers.
Preserve existing benchmark CSV fields and distinguish this retained capacity
from allocator overhead, process RSS and GPU storage. A matched pre-reservation/
candidate pair must share that recorder before comparing real-scene capacities
and timings; the small-fixture result is not a substitute. Retain the full
optimization scope and all existing failed gates.
