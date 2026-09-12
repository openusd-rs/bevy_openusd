# Ropa material projection

## Avoid duplicate initial projection

The live loader projected the stage, then reapplied all routes when its initially
unset AppliedPurposes state was compared with the current display purposes.
The initial animation sample was also left unrecorded. project_on_load_system
now records the purpose/time snapshot used by the initial projection. Later
changes still reapply the affected routes; a regression verifies first-pass
counts, idle behavior, sampled geometry and a subsequent guide-visibility toggle.

On the same fixed source below, release open time is 16441.770ms versus the
color-lookup baseline's 29818.080ms. MeshRoute matches fall from 1184 to 592,
with 2227 total route attempts instead of 4455. Retained mesh assets/vertices
stay at 1015/918650. This is one additional headless measurement, not a full
viewer-startup or general performance guarantee. Log:
/tmp/ropa-single-projection-profile.log.

Both whole-host and compositor Ropa captures were inspected with intact model,
grid and outliner: target/ropa-single-projection-{host,desktop}.png. Capture log:
/tmp/ropa-single-projection-capture.log. All 491 library tests pass (15 ignored),
check-all and release viewer build pass; logs use /tmp/initial-projection-*
and /tmp/single-projection-* prefixes.

## Fixed-source color transform comparison

The user's Ropa source changed externally to 153124035 bytes (mtime
2026-09-12 10:01:56 UTC), so the later color-transform measurement is not compared
directly with the older results below. A byte-identical copy was made at
target/ropa-color-benchmark-source.usdz, SHA256
e72ee9b52cc153b5cbc18e0aa1db254a0119e0032dc7f5250fe138dff1115292. This hash was
verified again after both runs. The user's source was not modified.

With the existing scalar/alpha optimizations retained, the new RGBA8 color
transfer fast path was temporarily disabled for the baseline and restored for
the second run. Both used the release editor_benchmark command below with that
fixed copy and one gpu-prepared sample. No concurrent test/build workload was
intentionally launched during either measurement.

| Phase | Original conversion ms | Color lookup ms |
|---|---:|---:|
| Editor open | 70275.850 | 29818.080 |
| Material route application | 33219.894 | 12401.403 |
| Subset route application | 31852.852 | 12392.861 |

This ordered pair measures a 57.6% lower headless open time, not UI startup or
GPU upload. Allocations and image cache retention still vary; GPU clocks and
filesystem caching were not controlled. It is not a repeated-sample estimate.

The optimization builds per-channel byte-to-half-float tables for RGBA8 linear
and sRGB sources. Other formats and table overflow retain the original path.
Tests compare every byte value, negative/HDR transforms and same-image edits
against the original conversion. All 490 library tests pass with 15 ignored.
Matched OIT/shadows-off GPU captures of the fixed source have zero changed RGB
pixels out of 921600 at tolerance zero; the optimized image was inspected.

Artifacts: target/ropa-color-fixed-{before,after}.*, target/ropa-color-fixed-diff.png.
Logs: /tmp/ropa-color-fixed-{before,after}-{profile,capture}.log,
/tmp/ropa-color-fixed-compare.log and /tmp/color-lookup-library-tests.log.
The preliminary unpaired run is retained in /tmp/ropa-color-lookup-profile.log;
it is not the basis of the matched speedup above.

## Earlier scalar and alpha comparisons

Asset: `/home/bresilla/machines/usd/ropa_harvester.usdz`.

Command, before and after the RGBA8 scalar lookup change:

```sh
USD_PROFILE_ROUTES=1 make run RUN_WITH=env CARGO='cargo --offline' \
  APP_TARGET='--release --example editor_benchmark' \
  ARGS='/home/bresilla/machines/usd/ropa_harvester.usdz 1 gpu-prepared'
```

| CPU phase | Before (ms) | After (ms) |
|---|---:|---:|
| Editor open | 166891.971 | 87773.971 |
| Material route application | 87740.150 | 44895.260 |
| Subset route application | 74651.461 | 38340.342 |
| Mesh route application | 1179.445 | 1185.241 |

After adding RGBA8 base-color/alpha lookup, the same command measured
66870.708ms open, 31607.697ms material application and 30730.591ms subset
application. This is a further 23.8% reduction from the scalar-only measurement,
not a repeated-sample benchmark. Mesh counts and geometry payloads are unchanged;
material/image retention counts again differ. Log:
`/tmp/ropa-alpha-lookup-profile.log`.

The alpha fast path preserves sRGB bytes directly and uses a 256-entry transfer
table for linear bytes. HDR and other formats retain the existing path. Tests
cover every byte value, color-space conversion, alpha replacement, in-place
source edits, missing/truncated bytes and unsupported formats. All 179 route
tests pass in `/tmp/alpha-lookup-route-tests.log`.

A matched OIT/shadows-off GPU capture at time zero is RGB-identical to the
scalar-only capture: zero changed pixels out of 921600, tolerance zero.
`target/ropa-alpha-lookup-gpu.png` was visually inspected; raw capture and diff
artifacts share its prefix. Logs: `/tmp/ropa-alpha-lookup-{gpu,compare}.log`.
This checks the alpha optimization on this view, not native renderer fidelity
or the original scalar optimization against a pre-optimization GPU baseline.

This single ordered pair shows a 47.4% lower headless open time. It includes
texture decoding and projection, not GPU upload, shader compilation, UI startup
or frame rate. No concurrent build/test was intentionally run during either
measurement. Clocks and filesystem cache state were not controlled. Both runs
retained 1015 mesh assets and 918650 vertices; image/material cache counts
differed, so these results do not establish memory reduction or pixel parity.

Scalar packing now builds a 256-entry transfer table for RGBA8 linear/sRGB
images and reads only the selected channel. Other formats, unavailable bytes
and transfer-table overflow use the existing per-pixel path. The table is local
to each call; it cannot retain stale values after source image edits.

Tests compare all 256 byte values across four channels, two color spaces and
four scale/bias pairs against Bevy's original pixel conversion. Additional
tests cover unsupported/truncated inputs and overflow fallback. The library
gate passes 485 tests with 15 ignored, and all six editor benchmark tests pass.

Logs: `/tmp/ropa-{initial-route,scalar-lookup}-profile.log`,
`/tmp/scalar-lookup-library-tests.log`, `/tmp/initial-profile-tests.log`.

The first post-change whole-window capture failed its near-black gate:
`target/ropa-lookup-desktop.png`, `/tmp/ropa-lookup-capture.log`. Its client
region was inspected and is black; a Ready/UI update marker is not visual
acceptance. No success is inferred from that capture.

A subsequent direct GPU capture passed and was visually inspected:
`target/ropa-lookup-gpu.png`, `/tmp/ropa-lookup-gpu.log`. It shows the yellow
body, gray panels, decals, railings and hitch. Command: release `viewer_capture`
with the same source, time zero, `USD_CAPTURE_RENDERER=oit`,
`USD_CAPTURE_SHADOWS=off`, `USD_CAPTURE_TIMEOUT_SECS=240`. This is rendered
scene evidence, not a matched baseline pixel comparison or whole-window proof.
