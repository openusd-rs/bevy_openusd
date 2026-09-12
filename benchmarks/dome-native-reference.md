# Native dome reference controls

`assets/dome_reference.usda` adds a fixed front camera and explicit material
binding APIs to the existing directional dome fixture. The camera is at (0,1,6),
looking down -Z, with 45-degree vertical field of view and 16:9 aspect ratio.
The dome rotates from 0 to 180 degrees at times 0 and 10, intensity 10000.
`assets/dome_reference_unit.usda` overrides default and sampled intensity to 1.
These fixtures do not change production lighting or the source dome fixtures.

The native capture wrapper accepts `USD_NATIVE_CAMERA_LIGHT=on|off`, default on.
It records the selection in settings.txt and rejects other values before launch
or output-directory creation. Off passes usdrecord's --disableCameraLight.
This removes the added camera light, not every ambient term: the OpenUSD
[25.05.01 recorder source](https://github.com/PixarAnimationStudios/OpenUSD/blob/v25.05.01/pxr/usdImaging/usdAppUtils/frameRecorder.cpp#L376-L393)
still supplies SCENE_AMBIENT to SetLightingState.

## Reproduction

Add the installed Weston binary directory to PATH if needed. Output paths must
be new. Run separately for time 0 and 10:

```sh
USD_NATIVE_CAMERA_LIGHT=off make --eval='native-dome:; @/bin/bash scripts/capture_native_storm.sh assets/dome_reference_unit.usda target/NEW_NATIVE /ReferenceCamera 0' native-dome
USD_CAPTURE_DOME=/Env USD_CAPTURE_CAMERA=/ReferenceCamera USD_CAPTURE_SHADOWS=off make --eval='bevy-dome:; @nixVulkan target/release/examples/viewer_capture assets/dome_reference.usda target/NEW_BEVY.png 0' bevy-dome
```

## Observations, 2026-09-12

- Bevy `target/dome-bevy-reference-{0,10}.png`: both inspected; diffuse and metal
  spheres are visible. The blue reflection region changes side with rotation.
- Native `target/dome-storm-reference-0/frame.png`, camera light off, intensity
  10000: white polygonal silhouettes. This alone cannot establish overexposure.
- Native `target/dome-storm-unit-0/frame.png`, intensity 1, camera light off:
  still white silhouettes. Native usdcat --flatten confirms intensity 1 at both
  sample times, so the override did compose.
- Native `target/dome-storm-unit-10/frame.png`: black frame.
- Native `target/dome-storm-unit-camera-on/frame.png`: black frame at time 0
  with the camera light enabled. The failure is not limited to disabling it.
- All three unit-intensity runs exited zero and had no diagnostic beyond the
  camera, renderer and recording-time messages. Logs and settings remain beside
  each image; wrapper stdout is in `/tmp/dome-storm-unit-*.log`.

Native references are not accepted as dome-light fidelity evidence. No renderer
orientation or intensity correction follows from these results. Exposure,
prefiltering, tessellation, tone mapping and native capture reliability still
need isolation before a cross-renderer comparison can prove parity.

## EXR isolation follow-up

The equivalent half-float EXR yields visible colored spheres at both rotations:
`target/dome-storm-exr-{0,10}/frame.png`, both inspected. OIIO --diff against
the HDR passes. A mipmapped EXR repeat also renders colored spheres at both
times (`target/dome-storm-mipped-{0,10}/frame.png`). Switching back to HDR
after these four EXR captures reproduces white silhouettes at time 0:
`target/dome-storm-hdr-after-exr/frame.png`. This isolates a format-dependent
native path in this fixture; it does not establish the native decoder's cause.

Both EXR variants emit a missing-mip warning (level 1 for scanline, level 3
for mipmapped). Those warnings are retained rather than reported as clean runs.
The native reflection placement differs from Bevy, but this is not yet proof
that Bevy's coordinate mapping violates USD. Installed domeLight.h describes
longitude zero toward +Z and positive longitude toward +X; the installed Storm
domeLight.glslfx uses atan(z,x) in its texture sampling helper. The complete
native transform/sampling chain still needs tracing before changing mapping.

Durable control: `assets/dome_reference_exr.usda` uses unit intensity and
`assets/dome_directional.exr`, generated without color conversion:

```sh
make --eval='dome-exr:; @oiiotool assets/dome_directional.hdr -d half -o assets/dome_directional.exr' dome-exr
```

Use the native command above with dome_reference_exr.usda. No EXR conversion is
inserted into the runtime loader. Native brightness and pixel parity remain
unverified; the Bevy comparison above uses intensity 10000, not unit intensity.
The committed fixture was captured independently at both times in
`target/dome-storm-exr-canonical-{0,10}/frame.png`; both show colored spheres and
were inspected. Make-driven OIIO comparison against the original HDR passes.

Validation: wrapper syntax passes through Make; invalid camera-light selection
returns 2 without creating target/dome-invalid-light-probe; runtime settings
confirm both on and off paths. Native flatten accepts the unit fixture. No Rust
code changed and the full Rust suite was not rerun for this capture-only change.
