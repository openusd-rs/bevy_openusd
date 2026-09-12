# Kubota matched-camera material comparison

## Textured clearcoat support

Preview clearcoat and clearcoat roughness now resolve texture connections,
selected scalar channels, raw/sRGB declarations and sampled scale/bias. The
scalar packing path supplies linear red-channel coat strength and green-channel
coat roughness textures to Bevy. Both textures join source dependency discovery
and editor reload consumer matching. Geometry warnings include missing coat UVs.
This does not change the renderer's coat BRDF or establish native shader parity.

`assets/clearcoat_texture.usda` uses blue-channel strength and green-channel
roughness from `clearcoat_channels.png`; at time 10 its sampled blue scale is
zero. Camera and dome remain fixed. `clearcoat_scalar_reference.usda` replaces
the connections with scalar strength 1 and roughness 0.2.

All three final captures were visually inspected:
`target/clearcoat-texture-on.png`, `target/clearcoat-texture-reference.png`, and
`target/clearcoat-texture-off-fixed.png`. Texture and scalar frames are
pixel-exact at 1280x720 (`/tmp/clearcoat-texture-compare.log`). The zero-coat
control differs at 43182 pixels and visibly removes the left sphere's coating
(`/tmp/clearcoat-texture-change.log`, expected equality failure). The initial
`off.png` capture is superseded by `off-fixed.png`, which fixes dome rotation.
These separate-time captures are not a new live texture-file reload qualification.

Workspace check and release viewer/capture builds pass:
`/tmp/clearcoat-texture-{check,build}.log`. Capture logs contain CAPTURE_OK.
No full test-suite rerun was performed for this increment.
Bevy's texture feature requires the manifest-only dependency adjustment described
in `vendor/bevy_pbr/VENDORED.md`; upstream rendering source is unchanged.

## Scalar clearcoat input fix

The hood authors clearcoat 0.5 and clearcoat roughness 0.15. The Bevy reader and
conversion previously omitted both inputs. A native-only override setting that
coat amount to zero lowers the hood RGB mean error against the old Bevy frame
from 0.0458333 to 0.00273693. Its maximum remains 20/255; this control isolates a
significant coat contribution without claiming all other behavior matches.
The inspected control is `target/kubota-no-coat-native/frame.png`, with ROI log
`/tmp/kubota-no-coat-roi.log`.

Preview Surface scalar clearcoat and roughness now resolve through the existing
material graph/time-sampling path and map to Bevy StandardMaterial. An authored
coat without roughness uses USD's 0.01 roughness default, verified in the installed
native `usdShaders/resources/shaders/shaderDefs.usda`, rather than Bevy's 0.5.
Unsupported coat textures and nonscalar values produce explicit warnings;
nonfinite scalar coat inputs are rejected. This does not implement textured
clearcoat or claim that the two renderers' coat BRDFs are identical.

The inspected `target/kubota-coat-bevy.png` visibly renders the machine. Against
the original coated native reference, the hood ROI mean error is now 0.0220588,
RMS 0.0285256 and maximum 25/255. This roughly halves the mean error, but all 256
ROI pixels still differ and the strict comparison still fails. Glass and decal
differences remain. No light intensity, camera or source-machine data changed.

The added tests cover graph-connected sampled scalar coat amounts, authored
roughness, USD's omitted-roughness default, an uncoated default and unsupported
texture diagnostics. The library passes 515 tests with 19 ignored; release
viewer and capture builds pass. Logs: `/tmp/clearcoat-tests.log`,
`/tmp/clearcoat-library.log`, `/tmp/clearcoat-release.log`,
`/tmp/kubota-coat-bevy.log`, `/tmp/kubota-coat-roi.log`.

## Baseline comparison

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
