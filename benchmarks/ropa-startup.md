# Ropa material projection

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
