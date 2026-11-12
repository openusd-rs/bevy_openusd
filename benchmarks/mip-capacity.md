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

The recorder adds one `image_buffer_scope` line under `USD_PROFILE_IMAGES=1` in
`editor_benchmark`. It counts main-world image assets, images with `Some(data)`,
payload length, capacity and unused capacity. Empty reserved buffers count toward
capacity; missing buffers do not. Existing CSV columns and image grouping output
are unchanged. Profiling remains outside the measured update intervals.

The planned matched CPU matrix uses one sample per fresh process, both idle and
quick-seek modes for bird/cow/arm/subsets, in ABBA order for each case/mode. Quick
seek retains four warmup and eight measured updates. Both saved release binaries
share the recorder, dependencies and features; only complete-chain reservation
differs in production code. Stage both at one owned executable path, with no
task-owned compilation overlapping measurement. Preserve raw logs, source/binary
hashes, process status, warnings and host load. Require unchanged payload/asset/
geometry counts and no capacity increase; report opening/update/RSS observations
separately and retain greater-than-5% timing regressions rather than averaging
them away across workloads. This CPU editor matrix is not GPU/window performance.

Recorder hardening passes 839 no-default / 956 all-feature/all-target test
executions and strict Clippy. The new accounting fixture covers no assets,
missing data, empty reserved data and a four-byte image with spare capacity.
Log: `/tmp/bevy-optimization-image-capacity-final-harden.log`. No production
library code or dependency changed in the recorder step.

## Matched real-scene capacity measurement

`/tmp/bevy-optimization-mip-capacity-abba.VeUwqI/` retains all thirty-two fresh
processes and their raw records. Both versions use recorder `626198c`. The
baseline temporarily removes only the reservation call and checked-size helper;
its production sampling prefix is byte-identical to `86315b2`. The source is
restored before measurement, and rebuilding the candidate reproduces its saved
executable byte-for-byte. Source patches, shared-input manifests and all three
build logs remain with the artifacts. Binary SHA-256 values:

- Before: `000693db33413a9acd58dce6a93f7983d4ec03bf3126f8949093bd2711eddc2b`.
- After: `7bcda02ba5fd391092a9ddf90eebf1d30385aef86f6475171acf0261c0634162`.

Every case/mode completes ABBA at the same owned executable path, using the
protocol above. No task-owned compilation overlaps. One-minute host load spans
16.51–36.42; unrelated applications remain untouched. There is no GPU device or
explicit process warmup, and OS page-cache state is not controlled.

| Case, both idle and after quick seeks | Images with data | Unchanged payload, bytes | Before capacity, bytes | After capacity, bytes | Capacity change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bird | 9 | 715,827,876 | 939,524,096 | 715,827,876 | -23.8% |
| Cow | 6 | 338,340,516 | 473,956,352 | 338,340,516 | -28.6% |
| Arm / subsets | 0 | 0 | 0 | 0 | not applicable |

All observations agree on those capacities and payloads. The change removes
223,696,220 / 135,615,836 bytes of unused image-buffer capacity for bird/cow,
without changing image count or encoded payload. **This is not an equivalent
RSS reduction.** Bird peak-RSS pair means change +0.02% idle / +0.11% quick;
cow changes -0.20% in both modes. Physical residency remains approximately
unchanged in this matrix; allocator and resident-memory effects cannot be inferred
from `Vec::capacity` alone.

### Remaining comparison failures

The full comparison is not accepted. Eleven timing pair means exceed the declared
5% regression boundary:

| Case/mode | Observed increases |
| --- | --- |
| Bird idle | CPU open +23.7%; idle update +10.4% |
| Bird quick | CPU open +21.6%; seek median +6.2%; seek p95/max +14.3% |
| Cow idle | Idle update +21.9% |
| Cow quick | CPU open +14.5%; idle update +258.7%; seek median +76.3% |
| Subsets quick | CPU open +14.5% |

These are observed elapsed-time failures, not isolated causal effects of the
allocation change. The untouched arm quick-seek median also falls 52.1%, and
the subset median falls 67.2%. No workload or failed observation is removed.

Six retained-cache equality checks also fail, all for quick-seek tangent-cache
payload. Bird ends at 2,771,400 / 2,743,320 bytes for the two identical baseline
executables and 2,732,448 / 2,741,568 for candidate. Cow is 16,328,016 / 15,639,852
before and 16,560,840 / 16,469,532 after. Every value remains below the existing
16 MiB byte budget; the varying cache state still prevents claiming an otherwise
identical retained-cache footprint. Geometry, image/peak-asset counts, image
payload and other recorded cache payloads match in every cohort.

The first analyzer stops at the first cache mismatch; its source and driver
output are retained as `analyzer-first.mjs` and `driver-first-analysis.log`.
The follow-up analysis accumulates every mismatch instead of stopping early,
without removing any failure criterion, and still exits 1. All thirty-two
records and 192 raw warmup/measured updates validate structurally. Process,
runtime-log and source/binary manifest checks pass with no CPU-run warnings.
This does not repair the separately recorded native cow material warning.

### Next animation-order investigation

`live.rs::resample_animation_system` directly iterates the public
`AnimatedPrims(HashSet<String>)` into animation batching. Its order can differ
between fresh processes. `PrimEntities` itself is path-ordered, and
`prepare_batch` already sorts worker results before publication; neither makes
the incoming animated-path order deterministic. Tangent-cache admission and LRU
retention depend on visitation order. This is a concrete candidate explanation
for the cache-footprint variation, not yet an isolated causal proof.

Investigate deterministic path ordering for built-in live routes without changing
the public `AnimatedPrims` type or custom callback ordering. Verify repeated
fresh-process cache state, rendered output, edit/reload behavior and update cost
before accepting such a change. Do not change cache policy merely to silence the
comparison or treat capacity savings as completion of the broader goal.
