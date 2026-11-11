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
- Record render-node CPU diagnostics separately from GPU durations. These are
  wall-clock intervals around render-node recording, not consumed thread CPU
  time or complete extraction/render-thread time. CPU-only diagnostics cannot
  satisfy the requirement for fresh GPU samples.
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

Eight new tests cover configuration, schedule ordering, readiness loss, final
interval completion, motion versus explicit idle, stale/duplicate/missing GPU
data, CPU/GPU diagnostic separation, invalid durations and root-count changes.
`harden` passes 819 no-default
and 927 all-feature/all-target tests, strict Clippy and formatting. The release
capture executable builds; these are overlapping test configurations. Logs:
`/tmp/bevy-optimization-playback-{tests,harden,release-build}.log`, followed by
`/tmp/bevy-optimization-playback-ready-{harden,build}.log`.

The intermediate CPU-diagnostic hardening attempt exposed a timing-dependent
existing transform fixture: one update did not guarantee both projected roots
were ready under load. The test now uses an unlimited projection budget and
explicitly waits for and asserts both `Ready` states before looking up prims.
All original affine/TRS, independent-clock and runtime-child assertions remain;
no production budget or transform behavior changed (`45c5025`). The failed run
is retained in `/tmp/bevy-optimization-playback-render-cpu-harden.log`.

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

## Recorder native validation

`9f4212f` adds the bounded recorder; `1fad2a5` adds separate render-node CPU
samples and initial stage clocks. Native validation of the initial recorder is
retained in `/tmp/bevy-optimization-playback-validation.b5gFuD/` and the probe
directory above. All successful captures use the native RTX 4080 Vulkan path,
forward rendering and four-sample MSAA at 1280 by 720, with validation enabled.

- Forward and reverse skeleton playback each complete 60 intervals with 60 clock
  changes. Explicit idle completes 60 intervals with no clock changes.
- An independent check of all raw clock rows agrees with actual Bevy time delta,
  stage rate, signed speed and looping range within `7.105e-15` time codes.
  Every interval is complete, positive and at least its enclosed main duration.
- The settled forward final pose is pixel-exact at RGB tolerance zero against a
  fresh fixed-pose capture at the recorded final clock. The initial and final
  poses visibly differ: 156,732 pixels change, with maximum channel delta 200.
  Both images were inspected; the straight skeletal column bends as expected.
- Disabled playback produces a pixel-exact deformation capture against the
  saved pre-recorder image. Invalid sample count and conflicting GPU/playback
  flags fail before GPU initialization.
- Source hashes, pipeline readiness and renderer log checks pass. These validate
  the recorder, not sustained bird/cow/arm playback or an actual display window.

## Bird, arm and cow baseline

The six-process matrix at `1fad2a5` is retained in
`/tmp/bevy-optimization-rendered-playback.C8fiNQ/`: 300 samples per case, 30
consecutive animated-ready warmups, fresh processes, release, native RTX 4080
Vulkan, 1280 by 720, forward, four-sample MSAA and validation enabled. There is
no task-owned compilation during the matrix; unrelated host load remains high
and variable (case-boundary one-minute averages 23.02–53.20). This is an initial
diagnostic baseline, not an alternating before/after comparison or native-window
frame-rate acceptance. The camera is the unchanged fixed default `(6,4,8)`
looking at `(0,1,0)`.

| Case | Main median / p95, ms | Interval median / p95, ms | Mean-derived offscreen Hz | Intervals over 20 / 33.333 ms |
| --- | ---: | ---: | ---: | ---: |
| Bird play | 47.839 / 258.966 | 48.417 / 263.955 | 11.97 | 300 / 300 |
| Bird idle | 1.051 / 3.545 | 16.726 / 16.747 | 59.72 | 0 / 0 |
| Arm play | 17.910 / 25.253 | 18.142 / 25.516 | 52.18 | 75 / 0 |
| Arm idle | 0.792 / 1.861 | 16.724 / 16.738 | 59.79 | 0 / 0 |
| Cow play | 90.333 / 187.357 | 92.082 / 189.118 | 9.61 | 300 / 300 |
| Cow idle | 0.628 / 1.075 | 16.723 / 16.732 | 59.81 | 0 / 0 |

Each play case changes its clock in all 300 samples; each idle case remains at
zero. An independent check of all 1,800 raw rows verifies positive complete
intervals enclosing main time and stage-rate/signed-speed/loop advancement to
within `5.685e-14` time codes. Bird and cow each cross the loop boundary six
times. `raw-verification.json` retains the checks and observed diagnostic counts.
Common active passes publish 240, 284 and 251 samples during bird/arm/cow play,
respectively; the arm's sparse-buffer-update pass publishes 34. No unobserved
passes or missing asynchronous samples are synthesized.

All six captures and renderer log checks complete. Each of the three settled
animated final poses is pixel-exact at tolerance zero against an independent
fixed-pose process at its recorded final clock. Entry/package hashes, both robot
files and the 267-file cow manifest verify before/after. Images were inspected:
bird and cow are textured, the arm changes pose and the objects are visible.
The default framing leaves the arm small and crops part of the bird; a properly
framed follow-up is still required for broader visible-surface/GPU acceptance.
Final-pose equality also does not inspect every intermediate rendered frame.

The driver intentionally exits **1**, not success: both cow runs retain the
unavailable `Cow_FBX_PNG/Cow_F/_converted/OmniPBR.mdl` dependency warning. This
does not invalidate the recorded timings, but complete fidelity/log acceptance
remains open. Existing strict CPU/GPU bird-pixel failures are a separate gate
and are not replaced by this same-path final-pose comparison.

Main-update time nearly fills the slow play intervals. Individual reported GPU
pass medians are below 0.06 ms and render-node CPU medians below 0.03 ms; these
are neither a summed whole-frame total nor a proof about uninstrumented GPU
work. They direct the next investigation toward main-world animation rather
than reducing render quality. The asset-root path currently reclassifies every
projected prim with `prim_is_animated` on each changed clock, whereas the live
editor samples its maintained animated-prims index. That source-level difference
is a profiling hypothesis, not yet an attributed speedup.

Whole-process `perf record` probes failed before launching the capture because
sampling events are unavailable with the current host permissions. The failed
software-event run is retained in
`/tmp/bevy-optimization-playback-perf.tCJeOm/`; a hardware user-event probe also
fails. No host security setting was changed. Opt-in in-process asset-root
classification/total timers are the next diagnostic step.
