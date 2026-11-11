# Rendered playback measurement

## Scope and measurement contract

Headless editor seeks omit rendering, while fixed-pose GPU captures omit ongoing
animation. Add an opt-in bounded playback measurement to `viewer_capture` using
the production `UsdAssetPlugin`, `UsdPlayback`, renderer, materials and quality
settings. Do not automate the user's desktop or change production scheduling.

This is native-GPU **offscreen** playback: the 60 Hz schedule runner has no
display present/vsync or Mara editor UI. Report it separately from both headless
scene updates and actual native-window cadence. Preserve the default fixed-pose
capture behavior when measurement is disabled.

- Wait for scene/camera/pipeline readiness, then at least 30 consecutive ready
  animated warmup updates before measuring. Loading/compilation is not a frame
  sample; readiness loss during measurement is a failed run, not a skipped frame.
- Use real `UsdPlayback` stage-rate advancement and record every sampled root
  clock and actual Bevy time delta. Signed playback speeds support reverse;
  explicit speed zero is an idle-render control, not evidence of animation.
- Bound the run to 1–600 measured frames and retain raw samples. Measure main
  schedule duration before `First` through after `Last`. Measure begin-to-begin
  intervals separately, including renderer/extraction waits and runner sleep.
  Do not present the main-schedule timer as total frame duration.
- GPU diagnostic timestamps are assigned when completed diagnostics synchronize
  into the main world (`bevy_render::diagnostic::sync_diagnostics`), not when the
  GPU frame began. Collect fresh published pass samples during the measured
  window, report actual per-pass counts, and explicitly avoid claiming exact
  CPU/GPU frame pairing or summing nested pass timings into a whole-frame cost.
  Delayed animated-warmup reports may enter that publication window and late
  tail reports may fall outside it. The frame interval also includes recorder
  bookkeeping; the main timer stops before that end-of-frame bookkeeping.
- Stop playback only after the final frame's begin-to-begin interval is known.
  Preserve final clocks and let the renderer settle before the final screenshot.
  No screenshot readback or per-frame file output enters the measured window.
- Verify parameter bounds, warmup exclusion, lifecycle/readiness failure,
  duplicate GPU measurements, complete final intervals and unchanged disabled
  behavior before relying on the results. Require real motion, successful native
  captures and retained metadata for bird/cow/arm baselines. GPU timestamps alone
  do not prove those scenes visibly animated correctly.

The working small-scene target remains 60 Hz from `OPTIMIZATION.md`. Initial
measurements characterize the remaining gap; they do not redefine success or
waive existing startup/small-scene regressions, strict bird image failures,
the cow's missing MDL warning or broader lifecycle/platform acceptance.

## Implementation checkpoint

`USD_CAPTURE_PLAYBACK_SAMPLES=1..600` enables the recorder; optional
`USD_CAPTURE_PLAYBACK_SPEED=-100..100` defaults to one and zero explicitly
selects an idle control. Combining this with fixed-pose GPU sample collection or
the separate clock-swap fixture is rejected. The new schedules bracket `First`
and `Last`, so posed-sort work and cache maintenance are included in main time.
The grid retains its initial fit during measurement and refits at the final
clock before the settled screenshot. Production playback/scheduling code and
default capture behavior are unchanged.

Seven new tests cover configuration, schedule ordering, readiness loss, final
interval completion, motion versus explicit idle, stale/duplicate/missing GPU
data, invalid durations and root-count changes. `harden` passes 819 no-default
and 926 all-feature/all-target tests, strict Clippy and formatting. The release
capture executable builds; these are overlapping test configurations. Logs:
`/tmp/bevy-optimization-playback-{tests,harden,release-build}.log`.

The first native probe under
`/tmp/bevy-optimization-playback-probe.K0B2qx/` correctly rejects
`point_deformation.usda`: that wrapper has animated referenced geometry but no
authored stage range. At this upstream revision, stage timing metadata resolves
from the session/root layer, not arbitrary sublayer metadata. It must not be
silently counted as successful playback. The `skel_test_simple.usda` positive
control completes 60 measured frames, changes its clock in every sample and
records 59 fresh GPU reports per active pass. It produces a valid capture after
stopping. Median main time is 1.831 ms and median interval 16.728 ms in this
diagnostic run; this is neither full-scene acceptance nor native-window FPS.
