# Kubota matched-camera material comparison

Runtime: d84be09, release `viewer_capture`, Bevy 0.19.1, RTX 4080 Vulkan.
Native: installed OpenUSD 25.05.01 Storm through `capture_native_storm.sh`.
This comparison establishes a remaining mismatch, not rendering acceptance.

## Shared setup

`assets/kubota_reference_setup.usda` authors a Z-up camera at (4.2,-6.3,3.95),
aimed at (0,0,1.5), with 45-degree vertical field of view and 16:9 aspect ratio.
The camera uses a 0.1..10000 clipping range. A uniform warm EXR dome has intensity
10. The machine archive is an unmodified copy of the user's Kubota package.

Both tools read `target/kubota-reference/matched.usda`, whose strongest layer
explicitly sets `upAxis = "Z"` and `metersPerUnit = 1`, and sublayers the setup
and machine. Do not rely on the setup sublayer alone for stage up-axis metadata.
`target/kubota-reference/setup_final.usda` is a copy of the committed setup.
The EXR and package are copied into the same directory, keeping resolver access
inside the capture's source boundary. No original machine files were changed.

Native selects `/ReferenceCamera` with camera light disabled. Bevy selects the
same camera and `/ReferenceDome`, disables studio illumination through the dome
capture path, uses EV100=-0.2630344 (unit exposure), and no tone mapping.
Both images are 1280x720 at time zero. Bevy retains its studio background/grid;
native has a black background and its native ambient-light behavior. Transparent
regions therefore are not a clean material-only comparison.

## Inspected results

- `target/kubota-matched-native/frame.png`
- `target/kubota-matched-bevy.png`

Camera framing and major geometry visually align. The hood differs in color,
and glass and decals differ visibly. To avoid background and alpha-channel
confounds in the numerical probe, a 16x16 opaque hood region at (570,320) is
compared using RGB only. The native PNG has an additional alpha channel.

OpenImageIO reports mean encoded RGB error 0.0458333, RMS 0.0573344, and maximum
0.1372549236 (35/255). All 256 pixels differ. At the maximum-error pixel, native
RGB is approximately (255,121,80), versus Bevy (255,86,49). Red is clipped here;
these encoded-byte errors are not a linear-HDR lighting measurement. The strict
ROI comparison fails. No importer or runtime lighting parameters were changed.

Both render commands exit zero. Native logs two rear-wheel unused-point-range
warnings and a missing EXR mip-level warning; this is not warning-free native
validation. Earlier intensity-100 and intensity-1 controls were respectively
overexposed and dark, and are not the final comparison. One intermediate wrapper
was rejected by native USD for a one-line attribute declaration; the final
multiline setup and wrapper render successfully in both tools.

Next: isolate the opaque material's normal/occlusion/roughness inputs and native
ambient contribution before assigning the difference to a particular shader or
changing the importer. Do not equate a Ready state, memory fix, or matching camera
with full-scene material parity.

## Reproduction

Prepare a new directory with copies of `kubota_reference_setup.usda`,
`dome_warm.exr`, and the machine archive renamed `machine.usdz`. Its root layer is:

```usda
#usda 1.0
(
    upAxis = "Z"
    metersPerUnit = 1
    subLayers = [@kubota_reference_setup.usda@, @machine.usdz@]
)
```

With new output paths and the installed Weston tools on PATH:

```sh
make build CARGO='cargo --offline' APP_TARGET='--release --example viewer_capture'
USD_NATIVE_CAMERA_LIGHT=off make --eval='native:; @/bin/bash scripts/capture_native_storm.sh NEW/root.usda NEW-native /ReferenceCamera' native
USD_CAPTURE_CAMERA=/ReferenceCamera USD_CAPTURE_DOME=/ReferenceDome USD_CAPTURE_EV100=-0.2630344 USD_CAPTURE_TONEMAPPING=none make --eval='bevy:; @nixVulkan target/release/examples/viewer_capture NEW/root.usda NEW-bevy.png 0' bevy
```

Logs: `/tmp/kubota-matched-native.log`, `/tmp/kubota-matched-bevy.log`,
`target/kubota-matched-native/record.log`, `/tmp/kubota-hood-rgb.log`, and
`target/kubota-matched-bevy.capture.txt`.
