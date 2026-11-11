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
