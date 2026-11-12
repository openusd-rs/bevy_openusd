# Native opening milestones

## Scope

The earlier `capture_source_profile` open/validation/projection sum is not
file-to-ready latency. It omits asynchronous source loading, other source phases,
frame scheduling and rendering. The bird's recorded 35.1% partial-phase regression
in [scoped relationships](skel-relationships.md) remains unresolved; a new metric
does not replace or waive that failed observation.

`viewer_capture` now records wall-clock milestones in each `.capture.txt`:

| Field | Interval or event |
| --- | --- |
| `opening_bootstrap_ms` | Entry to `main` until immediately before `AssetServer::load`; includes configuration, plugin/device startup and capture-target setup. |
| `opening_request_to_scene_ready_ms` | Asset request until `Last` observes the expected number of roots, all `UsdSceneState::Ready`. Includes asynchronous source work and all projection updates. |
| `opening_request_to_pipelines_observed_ms` | Request until main observes a compiled, nonempty, error-free pipeline-cache report from a render extraction with scene, camera and environment ready. |
| `opening_request_to_screenshot_requested_ms` | Request until the existing settled screenshot is scheduled. |
| `opening_request_to_readback_ms` | Request until the screenshot callback receives the image, before encoding or writing files. |

These are cumulative, overlapping intervals, not additive phases. Bootstrap has
a separate origin. First observations are retained, including when playback or
clock reversal subsequently changes the scene. Missing milestones stay `pending`;
timeouts print the partial report rather than treating missing work as zero.

The render probe extracts a scene-readiness marker with the scene. An already
compiled grid/camera pipeline before USD projection cannot alone satisfy the
post-scene milestone. This confirms a corresponding render cleanup report, not
GPU completion, readiness of every render asset, first visible geometry, or
native editor responsiveness. The final readback supplies an image for visual
validation but intentionally includes the existing sixty-ready-frame settle and
any requested playback/GPU measurement. It must not be presented as loading-only
latency. No capture/readiness/quality policy is relaxed by these timestamps.

The capture still uses unlimited projection and the native offscreen 60 Hz
schedule runner. A windowed editor with its normal projection budget is a
separate acceptance path. The metadata states these limitations explicitly.
The existing optional source report now also exposes its recorded override and
texture-publication durations; those still exclude asynchronous loader work.

## Validation and comparison

Unit tests cover missing and out-of-order observations, first-observation
retention, exact interval arithmetic, bootstrap/settle/readback separation and
stale or incomplete pipeline reports. Run `oslo make test-capture-tools` and
workspace hardening, then build the release `viewer_capture`.

For native checks, use unchanged fixed cameras/poses and source hashes, capture
bird/cow/subsets, verify all milestones are finite and ordered, and compare the
resulting images against the saved pre-instrumentation executable at tolerance
zero. Do not overlap task-owned compilation with timing runs. Record host load,
device, binary hash, build profile and process/log status.

Historical saved executables do not contain these milestones. A performance
comparison needs the same recorder built into both compared library revisions;
old partial sums cannot serve as an end-to-end baseline. First establish that
the new recorder preserves rendering and reports valid intervals, then construct
that comparison without changing dependency, quality or scheduling settings.

## Recorder verification

Recorder commit `9cd2119` passes 8 compact-instancing and 40 viewer-capture tests,
capture preflight/session/log tests, workspace hardening (835 no-default and
950 all-feature/all-target executions), strict Clippy, formatting and a release
capture build. These configurations overlap. Logs:
`/tmp/bevy-optimization-opening-tests-fixed.log`,
`/tmp/bevy-optimization-opening-harden.log` and
`/tmp/bevy-optimization-opening-build.log`. The initial compile failure is kept
in `/tmp/bevy-optimization-opening-tests.log`: the Bevy 0.20 extraction derive
requires `#[extract_app(RenderApp)]`. The corrected code also tests real
cross-world resource extraction, not just a manually populated probe.

`/tmp/bevy-optimization-opening-captures.ff6RTX/` retains ten fresh native
processes: saved pre-recorder/current instrumented captures for bird, cow, arm,
material subsets and three independently timed subset roots. The binaries are
SHA-256 `5ff060d6ebd4c3280ca8ea8dcfcbbe477d205bf4c80f29a6716a86d36a2f2003`
and `c38022b4a5819bc79d62fddcf521a147a141810a0a6c733fcb48ee37893f25c3`.
Both have the same production library, dependency lock, fixed cameras/poses,
forward renderer, 4x MSAA, validation and 1280x720 target on RTX 4080/Vulkan.
No task-owned build overlaps; one-minute host load ranges 19.07–23.04.

All five rendered pairs are pixel-exact at tolerance zero; bird and cow images
were inspected. All processes and runtime-log checks pass, all source manifests
verify, and `analysis.json` validates finite/ordered milestones, source phase
sums and identical instance/visible-mesh configuration. Cow retains one missing
MDL warning per run; the diagnostic driver therefore exits 1, not an unqualified
pass. The recorder does not fix that warning or the existing CPU/GPU discrepancy.

| Case | Request to all roots Ready, ms | Request to pipeline observation, ms | Five source phases, ms |
| --- | ---: | ---: | ---: |
| Bird | 6074.624 | 7775.170 | 740.021 |
| Cow | 1535.178 | 1891.458 | 172.452 |
| Subsets | 18.178 | 606.186 | 0.856 |
| Arm | 85.224 | 1207.945 | 45.445 |
| Three subset roots | 21.980 | 697.856 | 3.213 |

This is recorder validation, not an opening speedup comparison. The bird's
5.335 seconds outside the five source-publication timers includes asynchronous
loading and scheduling/other main work; it is not attributed to one operation.
There is also substantial post-scene pipeline-observation delay, including on
small controls. These results make loader and renderer startup separate next
investigation targets rather than treating the old partial sum as the entire
opening cost.

## Matched relationship-cache comparison protocol

Compare the current scoped-relationship library with the identical recorder and
`crates/usd_bevy/src/read/skel.rs` restored from `9f78ca9`. That is the only
production-file difference. Save both binaries, source hashes and the exact
baseline patch; restore and rebuild the current production source before runs.
Use fresh-process before/after/after/before order for bird, cow, arm and subsets,
with the same cameras, settings and input manifests as above. Keep main-entry
bootstrap, request-to-Ready, post-scene pipeline observation, final readback and
source phases separate. Retain the existing greater-than-5% regression limit
for the request-to-Ready and pipeline milestones; this additional matrix cannot
retroactively erase old failed observations or establish windowed acceptance.

### Matched results

`/tmp/bevy-optimization-opening-pair.qBn67r/` records the exact baseline patch,
source/blob hashes, build logs and sixteen fresh capture processes. The rebuilt
baseline SHA-256 is
`838632946bb429a366fd50dca497d032195486e6398d562d4fecb7e0bc33a10d`;
the candidate is the `c38022b4...` recorder binary above. The current source was
restored and rebuilt before measurement; its executable is byte-identical to
the saved candidate. Only `read/skel.rs` differs between the production builds.
There is no task-owned compilation during captures. Host load is high and
variable (44.65–81.77), so the comparison does not isolate a causal cache effect.

| Case | Before request-to-Ready, ms | After request-to-Ready, ms | Change of pair means | Before pipeline observation, ms | After pipeline observation, ms | Change of pair means |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Bird | 4844.208 / 8540.778 | 6781.802 / 8777.130 | +16.2% | 6416.544 / 10425.129 | 8924.358 / 11281.518 | +20.0% |
| Cow | 4879.910 / 4963.832 | 5536.860 / 5065.488 | +7.7% | 5969.713 / 5748.353 | 6700.719 / 6351.634 | +11.4% |
| Arm | 270.478 / 69.355 | 123.339 / 126.814 | -26.4% | 960.443 / 481.896 | 1830.094 / 1682.447 | +143.5% |
| Subsets | 18.985 / 33.355 | 23.383 / 18.974 | -19.1% | 314.249 / 533.614 | 1030.783 / 879.954 | +125.4% |

Six of eight declared milestone checks exceed the 5% limit. This is a failed
acceptance matrix, not clearance of the earlier partial loading regression.
Bird's three-phase partial sum also regresses 18.5%; cow's improves 33.6% even
though its end-to-end request-to-Ready gets worse. The metrics are not
interchangeable. The much larger arm/subset pipeline changes occur after their
source-publication work, confirming a separate startup/render scheduling cost
that must not be attributed to skeleton relationship reads without evidence.

All sixteen raw milestone sets pass arithmetic/order/configuration checks.
All twelve comparisons against each case's first baseline image are pixel-exact
at tolerance zero, including both candidate captures and the second baseline.
All process/log/source checks pass; cow retains its four MDL warnings. The driver
and analysis exit 1 for recorded warnings and six performance failures. Full
JSON data, bootstrap/source phases and images remain in the evidence directory.

The unexplained request-to-Ready interval can also contain main-world texture
finalization in `PostUpdate`, not just asynchronous IO/decode. The existing
`TextureSamplingTiming` counter in `route/sampling.rs` already covers this work
but was only enabled by `editor_benchmark`; expose it in the native capture
before assigning the residual interval to the loader. Earlier mip optimization
and retained fidelity evidence are in [texture finalization](texture-finalization.md).

## Native texture-finalization attribution

`20c43b2` enables the existing `TextureSamplingTiming` resource alongside
`USD_PROFILE_SOURCES` in `viewer_capture`. It snapshots the image count and wall
duration at the first all-roots-Ready observation and separately reports the
cumulative counter at screenshot request. Later settling/playback work cannot
silently be charged to initial loading. No texture algorithm or policy changes.

Updated hardening passes 835 no-default / 951 all-feature/all-target executions,
strict Clippy and formatting; release build passes. Logs:
`/tmp/bevy-optimization-opening-textures-ready-harden.log` and
`/tmp/bevy-optimization-opening-textures-ready-build.log`. Earlier successful
counter-only verification is retained in the corresponding `textures-harden`
and `textures-build` logs. The metadata test also checks optional Ready snapshots.

`/tmp/bevy-optimization-opening-textures.M1sj5I/` retains three diagnostic
captures with the existing `USD_PROFILE_LOADING=1` validation profiler enabled.
Binary SHA-256:
`aa9e2e3f7041f0f9c94a0909fdd6de856bbc64785610cd508da783aec8260c57`.
Source/lock/quality/camera settings remain as above. No own build overlaps;
host load spans 36.24–59.69. These extra profiler settings are not a new
before/after performance comparison.

| Case | Request to Ready, ms | Texture finalization at Ready, ms | Share | Source publication phases, ms | Completed validation, ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bird | 4626.779 | 2361.033 | 51.0% | 519.686 | 5.721 |
| Cow | 9299.963 | 6685.995 | 71.9% | 691.014 | 374.415 |
| Arm | 237.573 | 0 | 0% | 109.436 | 60.424 |

Both textured scenes finalize five images. Bird inputs total 335,544,320 bytes
and append 111,848,100 bytes; cow inputs total 203,423,744 and append 67,807,908.
The Ready and screenshot counters agree exactly, so none of this recorded work
comes from the later settle interval. The arm finalizes no textures. Remaining
request-to-Ready time after these disjoint counters is 1740.339 / 1548.539 /
67.713 ms respectively and is not yet attributed to individual operations.

All three image comparisons against the earlier recorder are pixel-exact at
tolerance zero. Process/log/source checks pass and `analysis.json` validates the
Ready/final snapshots and phase arithmetic. The cow's retained MDL warning
keeps the diagnostic driver at exit 1; the analysis exits 0.

`route/sampling.rs::finalise` does this work synchronously in `PostUpdate`, outside
the scene projection budget. Its time includes mip generation, sampler updates
and generated-image fingerprinting, not just one arithmetic loop; preemption
also contributes to wall time. The next production optimization should target
this measured path while keeping exact finite-image mip bytes, full resolution,
filtering, color space, generated-image tracking and reload behavior. The prior
decode lookup-table/full-float-buffer optimization is already present; do not
repeat it or claim that a faster arithmetic loop alone establishes responsive
windowed loading. Render/pipeline delays and the failed matched opening gates
remain separate unfinished work.
