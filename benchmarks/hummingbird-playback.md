# Hummingbird playback investigation

## Status

The user reported that clicking Play froze the native window for roughly ten
seconds per frame. The earlier migration's rendering and correctness checks did
not establish playback performance for this asset.

The local fixes below reduce headless scene-update time substantially. Native
captures show different bird poses, but native resampling intervals were still
roughly 0.1–0.25 seconds under concurrent system load. Smooth interactive playback is
**not accepted**. Nothing has been pushed; these are separate local commits.

The user subsequently confirmed: "Animates and window responds." This accepts
the observed freeze repair, not a measured smooth-playback or frame-rate target.
The follow-up goal is broader library/viewer optimization.

## Reproduction

Asset: `assets/external/hummingbird.usdz`. The live projection reports 84 prims,
44 animated. Run the native editor through the project workflow:

```sh
USD_VIEWER_PANE=timeline nix develop --command oslo make run \
  --args assets/external/hummingbird.usdz
```

Click Play manually. Desktop/window-manager input automation is not part of this
check. The patched viewer was launched on 2026-11-10 with Wayland and Vulkan
validation enabled. Evidence: `/tmp/bevy-hummingbird-patched.pSC6kJ/`, containing
`viewer.log`, `loaded.png`, and `playing.png`. These temporary files are local
evidence, not committed assets. Initial loading still takes approximately twenty
seconds.

## Changes

- `a66ce3d`: add `seek-quick` to the existing editor benchmark: four warmup seeks
  and eight measured seeks, cycling through time codes 0, 5, 10, 5.
- `9abc6b0`: share material preparation between built-in routes within a live
  projection/sample, including instance-root samples. Custom routes retain their
  mutation-safe behavior and bypass this scope.
- `2e968cf`: recognize generated images after mip/sampler finalization rather
  than recreating them each frame. Validate the base pixels, finalized layout and
  finalized pixel hash; retain no strong image handles or duplicate pixel buffers
  in the generated-image index. Reschedule authored sampler-wrap changes.
- `c2c5f23`: retain static bound materials across samples. Invalidate on USD
  notices, texture-handle replacement, missing dependencies or material edits;
  animated materials remain time-sampled. Bound the memo and keep it stage-local.
- `07d05ec`: optimize the newly split upstream `openusd-schemas` crate in the
  development profile, matching the existing `openusd` setting.
- `c3bbdee`: share skeleton definitions, animation queries and skeleton-order
  poses within one read-only built-in pass. Preserve stage/time isolation and
  mesh-specific influences, binding transforms, normals and tangents.
- `3a4a5e4`: remove wall-clock races from two projection fixtures by selecting an
  unlimited projection budget in those tests only. The production budget is
  unchanged.
- `d5b87b6`: invalidate cached materials after tracked source/generated image
  edits, including missed image events. Without image event tracking, textured
  results are only shared within a pass. Validate cached color-transformed image
  contents through the generated-image cache instead of checking existence alone.

No upstream dependency was rolled back. CPU/native deformation equivalence is
still required; removing correct normal/tangent handling is not a performance
fix.

## Measurements

```sh
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
USD_PROFILE_ROUTES=1 nix develop --command oslo make run-example \
  --name editor_benchmark \
  --args 'assets/external/hummingbird.usdz 1 gpu-prepared seek-quick'
```

These are **headless CPU scene updates**, not displayed FPS. One run per step,
eight measured samples after warmup, debug profile with the package optimization
overrides in `Cargo.toml`. Other projects were running/building concurrently;
these are diagnostic observations, not a controlled benchmark or latency bound.

| Step | Median update | Maximum update | Image payload after seeks |
| --- | ---: | ---: | ---: |
| Shared per-pass materials | 2552.573 ms | 3072.469 ms | 1,252,698,780 B |
| Reuse finalized images | 623.922 ms | 843.807 ms | 715,827,876 B |
| Retain static material samples | 128.111 ms | 138.500 ms | 715,827,876 B |
| Share skeleton queries/poses | 126.602 ms | 138.260 ms | 715,827,876 B |
| Optimize upstream schemas | 82.356 ms | 98.844 ms | 715,827,876 B |
| Final cache-invalidation regression checkpoint | 52.642 ms | 61.210 ms | 715,827,876 B |

Logs, in table order:

- `/tmp/bevy-hummingbird-scoped-profile.log`
- `/tmp/bevy-hummingbird-mips-profile.log`
- `/tmp/bevy-hummingbird-static2-profile.log`
- `/tmp/bevy-hummingbird-skel-profile.log`
- `/tmp/bevy-hummingbird-optimized-profile.log`
- `/tmp/bevy-hummingbird-final-profile.log`

The final row includes `d5b87b6`. Its lower time is another uncontrolled-load
observation; it does not attribute a further speedup to invalidation handling.

The original editor profile (`/tmp/bevy-hummingbird-editor-profile.log`) measured
initial loading, **not animated frames**: 26.9 seconds total, including about
8.2 seconds in mesh/material/subset routes. Do not use it as a frame-time
baseline. The user's ten-second-frame observation is separate native evidence.

In the final headless run, skin routes consumed 432.332 ms over eight updates,
mesh routes 126.878 ms, and material routes 57.340 ms. Remaining work includes
deformation preparation and native frame pacing. The eight-sample run is not a
long-running memory/lifecycle acceptance test.

## Validation

Completed checks:

- `test-library`: 669 passed / 20 ignored without default features, 677 passed /
  29 ignored with all features; no failures. Log:
  `/tmp/bevy-hummingbird-image-events-tests3.log`.
- `test-native-deformation`: four native OpenUSD position/normal oracle checks
  passed again at the final checkpoint. Log:
  `/tmp/bevy-hummingbird-final-native-deformation.log`.
- `test-watchers`: five asset-watcher and four editor-texture-watcher checks
  passed again. Log: `/tmp/bevy-hummingbird-final-watchers.log`.
- `check-web`: wasm32 compilation passed again. Log:
  `/tmp/bevy-hummingbird-final-web.log`.
- `test-deformation` on `skel_morph_blended_normals.usda`: CPU and GPU captures
  match exactly at time codes 0, 5 and 10. Evidence:
  `/tmp/bevy-hummingbird-deformation-20261110/`.

The final workspace `harden` rerun passed: 781 no-default tests (21 ignored),
874 all-feature/all-target tests (29 ignored), strict Clippy, formatting and
whitespace checks. Log: `/tmp/bevy-hummingbird-final-harden3.log`. Earlier runs caught a new
fixture missing its authored attribute, then an existing color-texture cache
that reused externally modified images. Both were corrected; the final library
results above include those corrections.

New regression coverage exercises shared preparation, animated material values,
custom-route mutation, USD edits, texture replacement, external material edits,
generated-image lifetime and mutated pixels/mips/layouts, sampler-wrap changes,
tracked in-place image edits and missed events, and nested stage/time-isolated
skeleton scopes.

### Additional real-asset comparison: not passing

```sh
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
USD_WGPU_VALIDATION=1 USD_WGPU_DEBUG=1 \
nix develop --command oslo make test-deformation \
  --asset assets/external/hummingbird.usdz \
  --output /tmp/bevy-hummingbird-asset-deformation-20261110 --times '0 5 10'
```

All six captures completed with real GPU deformation selected in the GPU cases.
The strict RGB tolerance of 1 failed:

| Time | Pixels over tolerance | Maximum RGB error |
| --- | ---: | ---: |
| 0 | 2,053 / 921,600 | 33 |
| 5 | 1 / 921,600 | 3 |
| 10 | 1 / 921,600 | 19 |

Inspection of the time-zero diff locates the visible disagreement on grid lines
seen through the transparent wing blur. The source of that discrepancy has not
been established; there is no corresponding pre-change capture baseline. Do not
waive the tolerance or describe this additional check as passing. The captured
time-zero and time-ten poses visibly differ, demonstrating deformation rather
than just an advancing clock, but not smooth live playback.

A source-grounded hypothesis to test is transparent depth sorting. The pinned
Bevy 0.20.1 `bevy_pbr/src/material.rs` queues transparent meshes using the world
transform of the render mesh asset's `aabb_center`. Its infinite-grid renderer
uses `TransparentSortingInfo3d::Sorted` with a zero center. GPU skinning retains
rest positions in the mesh asset, whereas the CPU route publishes deformed
positions; `gpu_skin::attach` currently supplies no posed `final_aabb` override.
This can produce different sort keys, but has not yet been proven to cause the
observed pixels. Compare actual phase order and posed bounds before choosing a
fix. Any repair must preserve independently timed instance sharing and must not
hide the grid or relax the image tolerance.

Smooth-playback acceptance and the real-asset image comparison remain outstanding.
The open interactive viewer was built at the
`c3bbdee` code state; the later image-invalidation correction is covered by the
library regression rather than that already-running process. No release-profile,
other-platform or downstream-application performance acceptance is claimed.
