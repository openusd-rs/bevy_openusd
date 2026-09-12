# Machinery rendering and texture memory

## Reuse packed outputs before allocation

A second bounded asset-ID index fingerprints scalar input planes. On a hit it
checks the live output's complete image layout and each expected RGBA byte before
reusing the handle. This avoids rebuilding and hashing another large output
buffer. Neither this index nor the generated-image index owns pixel data or strong
handles. Changed inputs, mutated output pixels and changed samplers are covered
by a new regression; source planes with inconsistent lengths are rejected.

The profiled Kubota headless open falls from 12,595.777 ms at bf31365 to
7,220.848 ms, with a 7,127.158 ms repeat. SubsetRoute application falls from
5,238.452 to 2,343.032 ms, and MaterialRoute from 4,559.023 to 2,068.234 ms.
Image payload remains 556,797,440 bytes with 90 images and 78 material assets.
These are bounded loading samples, not interactive frame-rate measurements.

The inspected `target/kubota-inputs-host.png` shows Ready and is pixel-exact to
`target/kubota-shared-host.png`: OpenImageIO mean/RMS/max errors are all zero.
The desktop and rendering-log guards pass; original machine hashes still match.
The library passes 513 tests (19 ignored), check-all and release builds pass.
The full workspace suite was not rerun for this increment; its preceding result
is the 677-test checkpoint recorded in BEVY_WORK.md.

Logs: `/tmp/kubota-route-profile.log`, `/tmp/kubota-inputs-profile.log`,
`/tmp/kubota-inputs-repeat.log`, `/tmp/packed-input-library.log`,
`/tmp/packed-input-check.log`, `/tmp/packed-input-release.log`,
`/tmp/kubota-inputs-ui.log`, `/tmp/kubota-input-pixels.log`.

## Generated-image sharing fix

Generated scalar, alpha, and color textures now share through a bounded index of
4096 content hashes and asset IDs. The index owns neither pixel buffers nor
strong image handles. Reuse checks exact pixels, full texture/view descriptors,
sampler, data order, usage and resize policy. Small conversion caches remain;
large images no longer depend on fitting their full pixel keys into those caches.
Textures are not resized, quantized or assigned different formats.

Kubota's retained image payload falls from 12,434,542,336 to 556,797,440 bytes
(302 images to 90). Its 73 diagnostic payload groups and their combined
465,307,904 bytes are unchanged. Material assets fall from 291 to 78 as image
identity reuse also allows existing material deduplication. The measured single
headless open is slower: 13,494 ms versus the prior 9,074 ms. These are individual
CPU samples, not a frame-rate claim or a controlled loading-time benchmark.

The rebuilt viewer visibly renders Kubota with Ready status in the inspected
`target/kubota-shared-host.png` and `target/kubota-shared-desktop.second.png`.
Ropa's inspected `target/ropa-shared-host.png` is pixel-exact to the pre-fix host
capture: OpenImageIO reports mean, RMS and maximum error all zero.
Original machine archive hashes still match. Native parity remains unqualified
because those camera and lighting setups differ.

Six added regressions cover live sharing, forced hash collisions, pixel mutation,
descriptor/sampler differences, removed assets, index bounds, owner-driven asset
cleanup, and 4096x4096 packing beyond the old pixel-cache budget. The library
passes 512 tests with 19 ignored. Logs: `/tmp/generated-image-library.log`,
`/tmp/generated-image-release.log`, `/tmp/kubota-shared-memory.log`,
`/tmp/kubota-shared-ui.log`, `/tmp/ropa-shared-ui.log`,
`/tmp/ropa-shared-pixel-diff.log`.

## Pre-fix evidence

Current release viewer rebuilt at 5a07516. Both machinery archives pass ZIP
integrity checks and contain every explicitly listed `inputs:file` member.
Original files were not modified; SHA256 checks pass after all captures.

| Asset | SHA256 |
| --- | --- |
| Kubota | bc959a1b9e704c4b3224c44d802149297f8fc455085ae865041ca1230bb5fbc6 |
| Ropa | f9e2ad7917ddbb3606dcf3147c11a446ffd7a2e498d1e5289ad9c78c225cae60 |

## Inspected rendered output

- Native Storm: both `target/kubota-native-current/frame.png` and
  `target/ropa-native-current/frame.png` visibly render machinery. Kubota logs
  two Hydra warnings about unused point ranges on rear wheel meshes; Ropa has
  no warning/error matches in its record log. These successful captures do not
  disprove the previously observed intermittent native blackouts.
- Current Bevy/Mara Kubota: `target/kubota-current-host.png` shows
  `Renderer stopped (OutOfMemory)` and no rendered scene. The viewer log contains
  GPU allocation failures followed by invalid texture upload/view errors.
- Current Bevy/Mara Ropa: `target/ropa-current-host.png` and the inspected
  `target/ropa-current-desktop.second.png` show the yellow harvester, grid,
  hierarchy and Ready status. Both desktop frame checks and the updated log
  check pass. This is an idle smoke check, not interaction or frame pacing proof.

Native captures use automatic front framing and camera lighting. Viewer captures
use its studio environment and oblique framing. They are not pixel-aligned
comparisons and do not establish material parity. All compositor/viewer processes
created by these capture wrappers were cleaned up; user desktop processes were
left untouched.

## Kubota memory diagnosis

The current release headless editor benchmark retains 302 images totaling
12,434,542,336 CPU payload bytes. These are not measured GPU allocation bytes.
The largest repeated payload is RGBA8 4096x4096 (67,108,864 bytes) with 166 copies.
That repetition alone warrants investigation of generated texture reuse; reducing
source resolution is not yet justified. Native Storm rendering the same archive
does not explain Bevy's allocation strategy.

`USD_PROFILE_IMAGES=1` on `editor_benchmark` reports payload groups by dimensions,
format and hash, checking repeated payloads byte-for-byte. Other sampler/descriptor
properties are not part of this diagnostic grouping; sharing still requires their
compatibility. Ordinary benchmark output and scene behavior are unchanged.

The current texture conversion caches store full pixel/plane keys under a 64 MiB
budget. Large generated images can bypass these caches. The generated-image
index above addresses this bypass without retaining full duplicate keys or
changing texture semantics.

## Capture validation fix

The old log guard checked only panic markers, so Kubota's nonblack error UI passed
the wrapper despite the stopped renderer. `check_viewer_log.sh` now rejects Bevy
rendering-error records too. Shell regressions pass; the original Kubota log now
fails with exit 1, while Ropa's log passes. A nonblack window still does not prove
scene readiness, fidelity, or absence of unlogged failures.

Logs: `/tmp/machine-validation-build.log`, `/tmp/kubota-current-ui.log`,
`/tmp/ropa-current-ui.log`, `/tmp/kubota-memory.log`,
`/tmp/kubota-image-exact-profile.log`, and each capture's companion logs.
