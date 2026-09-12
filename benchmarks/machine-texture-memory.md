# Machinery rendering and texture memory

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
budget. Large generated images can bypass these caches. The next step is to
measure and fix reuse without retaining full duplicate cache keys or weakening
texture semantics, then rerun Kubota and inspect the resulting frame.

## Capture validation fix

The old log guard checked only panic markers, so Kubota's nonblack error UI passed
the wrapper despite the stopped renderer. `check_viewer_log.sh` now rejects Bevy
rendering-error records too. Shell regressions pass; the original Kubota log now
fails with exit 1, while Ropa's log passes. A nonblack window still does not prove
scene readiness, fidelity, or absence of unlogged failures.

Logs: `/tmp/machine-validation-build.log`, `/tmp/kubota-current-ui.log`,
`/tmp/ropa-current-ui.log`, `/tmp/kubota-memory.log`,
`/tmp/kubota-image-exact-profile.log`, and each capture's companion logs.
