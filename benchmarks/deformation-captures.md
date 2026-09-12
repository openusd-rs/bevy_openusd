# CPU/GPU deformation capture sweep

## Additional blended render paths

At da06181, the two-joint fixture at ten was captured in forward MSAA4,
prepass MSAA Off and deferred MSAA Off, with shadows off and the authored
/ReferenceCamera. All six CPU/GPU images were inspected. Forward and deferred
are RGB-exact; prepass maximum RGB error is 1 and passes tolerance 1.
Artifacts: target/blended-{forward,prepass,deferred}-10-{gpu,cpu}.png;
logs /tmp/blended-{forward,prepass,deferred}-10-compare.log.
This extends the small-fixture path checks, not all animation/normal-map cases.

The larger varying-weight fixture and measured preparation bottleneck are
documented in benchmarks/blended-skin-grid.md.

## Blended-joint normal and tangent repair

`assets/skel_morph_blended_normals.usda` adds two independent joints with
different rotations/nonuniform scales and equal per-vertex weights to the
combined morph/normal-map/subset fixture. Native normal sampling exposed an
actual mismatch: at time zero, native normal X was 0.372133719 while Bevy CPU
returned 0.5331887 (/tmp/native-blended-normal-rust-test.log).

For per-vertex classic-linear skinning, CPU normals now use the weighted sum
of per-joint inverse-transpose transforms. Rigid constant-influence binding
retains the inverse transpose of its combined rigid transform. For GPU vertex
skinning, let B be the blended position matrix and N the native weighted normal
matrix. Base normals and morph normal offsets are multiplied by B-transpose*N
so Bevy's subsequent inverse-transpose B yields N. Active zero/nonfinite
normal results reject instead of reaching shader normalization.

Normal-mapped geometry also needs a consistent tangent frame. When the normal
correction is nonidentity and UVs exist, tangents are recomputed from the sampled
CPU geometry/normals, then transformed by inverse B before GPU skinning. This
keeps GPU positions/morphs active but adds CPU geometry/tangent work for those
bindings; no large-rig performance claim is made. Identity corrections skip
this path. The shader-simulation regression covers normals, tangents and
handedness at five times.

Native numerical checks pass after the fix. The normal-only fix still produced
GPU/CPU image errors up to RGB 6 with the constant normal-map fixture; the
tangent correction removes those differences. Forward, MSAA Off, shadows off,
authored /ReferenceCamera at times 0,5,10 are all RGB-exact (0/921600 changed).
All six images were inspected: target/blended-tangent-{0,5,10}-{gpu,cpu}.png.
GPU metadata reports two skinned/two morphed entities; CPU reports zero.
Logs: /tmp/blended-tangent-{0,5,10}-compare.log. These captures precede the
additional zero-normal rejection guard; they exercise the same valid-value
normal and tangent math, not that guard's failure path.

The final guard-inclusive release was also captured at ten; both
target/blended-final-10-{gpu,cpu}.png were inspected and are RGB-exact
(/tmp/blended-final-10-compare.log). Final ordinary library tests pass 494 with
18 ignored, all three explicit native checks pass, and check-all/release build
pass: /tmp/blended-normal-final-{tests,native,check,build}.log.

The earlier failed native/image diagnostics remain available. This verifies
the two-joint fixture, not arbitrary rigs, spatially varying normal maps,
deferred rendering or the complete deformation/environment acceptance item.

## Native baked normal oracle

The native sampler accepts `--normals`, reads baked built-in mesh normals,
transforms them by the inverse transpose into the original mesh's local frame,
and normalizes them. Singular transforms and zero/nonfinite normals fail.
`assets/skel_morph_native_normals.usda` keeps the combined fixture's geometry,
animation and material but replaces indexed normal primvars with equivalent
built-in vertex normals. The original indexed fixture remains unchanged.

The native point and normal integration tests both pass at 0,2.5,5,7.5,10 with
absolute component tolerance 1e-5. Use the `native_baked_` test filter to execute
both ignored tests with USD_NATIVE_DEFORMATION_TOOL set. Log:
/tmp/native-normal-rust-tests-final.log. At ten, native normal zero is
(0.769800313,0.272165537,0.577350326); the other three are
(0.707106722,0,0.707106841). This covers one nonuniformly scaled joint plus
sparse morph normal offsets, not arbitrary multi-joint normal blending.

An ordinary regression verifies identical CPU normal results between the two
fixture encodings at all five times. The Bevy time-ten built-in-normal image
was inspected and is RGB-exact with the indexed image (0/921600 changed),
using the same release capture executable and authored camera:
target/combined-bevy-builtin-normal-10.png versus
target/combined-bevy-canonical-10.png; /tmp/combined-builtin-normal-compare.log.
Native target/combined-storm-builtin-normal10/frame.png was also inspected:
its triangle shading difference persists, so the encoding substitution does
not establish rendered shading parity. Lighting and surface-normal handling
still need isolated comparison.

Ordinary library tests pass 492 with 17 ignored; make check-all passes.
Logs: /tmp/native-normal-{library-tests,check}.log. Native compiler validation:
/tmp/native-normal-build-final.log. No production Rust behavior changed.

## Native CPU-baked point oracle

`scripts/sample_native_deformation.cpp` uses native UsdSkelBakeSkinning on an
anonymous flattened stage, never saving source layers. It prints sampled points
in the original mesh's local coordinate space. Native baking can move rigid
skinning into the mesh transform rather than its points, so the tool includes
the baked transform and removes the original world transform. Nonfinite times,
invalid mesh paths and singular original transforms are rejected.

The tool materializes existing animated attributes at the requested time before
baking that single instant. Without this step, the native bake at intermediate
time 5 used the default blend weight instead of the interpolated value. Native
output at 0/5/10 now gives first-point positions (0,0,0),
(0.176776677,0,0.176776707), (0.353553355,0,0.353553414), respectively.
This is a CPU point oracle, not a normal/tangent or GPU timing check.

Build against an installed OpenUSD SDK, its Python development library and
compatible TBB headers/library. Set CPPFLAGS, LDFLAGS and LDLIBS for those SDKs:

```sh
make --eval='native-sample-build:; @$(CXX) -std=c++17 $(CPPFLAGS) scripts/sample_native_deformation.cpp $(LDFLAGS) $(LDLIBS) -o target/sample-native-deformation' native-sample-build
USD_NATIVE_DEFORMATION_TOOL="$PWD/target/sample-native-deformation" make test CARGO='cargo --offline' APP_TARGET='-p usd_bevy --lib native_baked_positions_match_combined_skin_and_morph -- --ignored --nocapture'
```

The ignored integration test directly runs native baking at 0,2.5,5,7.5,10 and
compares all four source-order points against the Bevy CPU deformation result
with absolute tolerance 1e-5. It is separate from the native-export test lane.
It was executed and passed (/tmp/native-deformation-rust-test.log). Ordinary
library tests pass 491 with 16 ignored, and make check-all passes. NaN time and
missing mesh probes reject with status 2. These checks do not validate arbitrary
assets, normals, tangents or native GPU rendering.
The SDK used here is OpenUSD 25.05.01, Python 3.13.12, TBB 2022.2 headers and
the SDK's oneTBB 2022.3 runtime. Initial missing-header/linker failures remain
in /tmp/native-deformation-build*.log; the successful sampled build is
/tmp/native-deformation-build-sampled.log, with outputs in
/tmp/native-deformation-sampled-{0,5,10}.log.

## Native Storm deformation reference

The base blendshape skeleton now applies SkelBindingAPI, and the normal-map
mesh/subset apply MaterialBindingAPI. Native Storm previously displayed the
unskinned square at time zero and warned about the missing material APIs.
An overlay adding the schemas produced the rotated/nonuniformly scaled shape
at zero and the morphed shape at ten. The canonical fixture declarations now
include those APIs, and the composed fixture regression checks their presence.

`assets/skel_morph_reference.usda` adds /ReferenceCamera, matching Bevy fixed
eye (1,0.5,5), target (1,0.5,0). Native captures use:

```sh
make --eval='native-storm:; @/bin/bash scripts/capture_native_storm.sh assets/skel_morph_reference.usda target/NEW_NATIVE /ReferenceCamera 10' native-storm
```

Inspected native `target/combined-storm-api-time-{0,10}/frame.png`, canonical
`target/combined-storm-fixture-time-{0,10}/frame.png`, and
`target/combined-storm-repeat10/frame.png`, alongside Bevy
`target/combined-bevy-native-camera-{0,10}.png`.
The visible native endpoint silhouettes agree qualitatively with Bevy. Native
time-ten recording is not yet reliable: the canonical first run was black,
but an unchanged repeat rendered the expected shape, both with exit zero and
no renderer warning. Therefore these are qualified shape observations, not a
repeatable native acceptance gate. Native shading differs (visible triangle
gradient versus Bevy's nearly uniform surface), with different lighting setups;
normal/material parity remains unproven. No reference image was overwritten.

## Forward-only MSAA isolation

The independent `USD_CAPTURE_MSAA` control changes sampling without adding
prepasses. Same-release forward GPU/CPU captures of
`assets/skel_morph_tangent_normals.usda`, time 10, default eye/target and
shadows off give:

| Sampling | Max RGB error | Pixels exceeding tolerance 1 |
| --- | ---: | ---: |
| Off | 1 | 0 |
| 4 | 2 | 1 at (711,442) |

All four images were inspected. GPU metadata reports two skinned and two
morphed entities, CPU reports zero, and metadata confirms the requested MSAA.
Artifacts: `target/combined-normal-forward-{noaa,aa4}-{gpu,cpu}.png` with raw
RGBA/settings sidecars; logs `/tmp/combined-normal-forward-{noaa,aa4}-compare.log`.
The four-sample failure reproduces with the same release build used for Off;
sampling is sufficient to change this fixture's tolerance outcome. This does
not prove the underlying floating-point/rasterization cause, establish native
normal parity or waive the original tolerance failure.

## Combined normal-map, skin and morph fixture

`assets/skel_morph_tangent_normals.usda` combines indexed morph normals,
nonuniform skinning (2,1,0.5), UV tangents, a material subset and double-sided
geometry. Both surfaces use the existing constant-normal material, which is
encoded as a normal-map image. It does not test a detailed spatial normal map.
The fixture regression checks composed weights, normal targets, UV/tangent
attributes, skin matrix scales and normal-map materials at times 0,5,10.

```sh
USD_CAPTURE_TIMEOUT_SECS=180 USD_CAPTURE_RENDERER=deferred USD_CAPTURE_SHADOWS=off \
make --eval='gate:; @/bin/bash scripts/compare_deformation.sh assets/skel_morph_tangent_normals.usda target/NEW 1 0 5 10' gate
```

Measured after 31e5180 with the new fixture, RTX 4080/Vulkan driver 595.84,
1280x720, fixed eye (6,4,8), target (0,1,0), shadows off:

| Renderer | Time | Pixels exceeding tolerance 1 | Max RGB difference |
|---|---:|---:|---:|
| Forward, MSAA4 | 0 | 0 | 0 |
| Forward, MSAA4 | 5 | 0 | 1 |
| Forward, MSAA4 | 10 | 1 | 2 |
| Deferred, MSAA off | 0 | 0 | 0 |
| Deferred, MSAA off | 5 | 0 | 1 |
| Deferred, MSAA off | 10 | 0 | 1 |

The forward gate fails at time 10; its tolerance was not increased. The
prepass/no-MSAA control at time 10 passes with max difference one. This is
consistent with a rasterization/AA-sensitive difference, not proof of its cause.
A deferred back-face view from (-6,4,-8) at time 10 also passes with max one.
GPU capture metadata reports two skinned and two morphed meshes, no flat-material
overrides; the gate checks that the CPU controls have no GPU deformation.

Artifacts: `target/combined-normal-{forward,deferred}/`,
`target/combined-normal-{prepass,deferred}-{gpu,cpu}-control.*` and their diff
images. Logs use matching `/tmp/combined-normal-*` names. Front views at all
three times, the failing pair/diff, no-MSAA GPU control and both back-face views
were inspected. The fixture test passes in `/tmp/combined-normal-fixture-test.log`.
These checks establish bounded combined-feature agreement, not performance,
arbitrary normal-map fidelity or exact forward pixel parity.

## Earlier deformation sweeps

Measured at `19fab40` using `scripts/compare_deformation.sh`: Bevy 0.19.1,
NVIDIA RTX 4080/Vulkan, driver 595.84, fixed 1280x720 camera `(6,4,8)` looking at
`(0,1,0)`, forward rendering and scene shadows. This measures image agreement,
not GPU timing or native USD renderer equivalence.

```sh
make --eval='compare-deformation:; @/bin/bash scripts/compare_deformation.sh assets/skel_morph_subsets.usda target/deformation-matrix-strict 0 0 15 30 45 60' compare-deformation
make --eval='compare-normals:; @/bin/bash scripts/compare_deformation.sh assets/skel_morph_normals.usda target/deformation-normals-strict 0 0 5 10' compare-normals
```

Each pair has 921,600 pixels. Tolerance is zero; any RGB difference fails the
comparison. The first sweep deliberately returns nonzero, preserving its small
differences rather than silently increasing tolerance. The second sweep passes.
All sixteen scene images were visually inspected. Renderer logs contain no
warnings/errors. GPU captures report visible skin/morph components; CPU captures
report neither. These component counts are coverage checks, not draw statistics.

| Fixture | Time | Changed pixels | Maximum RGB error | Mean RGB error |
| --- | ---: | ---: | ---: | ---: |
| skel_morph_subsets | 0 | 2 | 1 | 0.000001 |
| skel_morph_subsets | 15 | 14 | 1 | 0.000005 |
| skel_morph_subsets | 30 | 6 | 1 | 0.000002 |
| skel_morph_subsets | 45 | 14 | 1 | 0.000005 |
| skel_morph_subsets | 60 | 2 | 1 | 0.000001 |
| skel_morph_normals | 0 | 0 | 0 | 0 |
| skel_morph_normals | 5 | 0 | 0 | 0 |
| skel_morph_normals | 10 | 0 | 0 | 0 |

Each output directory contains results.tsv, both PNG/RGBA readbacks, capture
metadata, renderer logs and difference images for every case. Driver/platform,
camera and material changes may alter these results; no broad tolerance is
inferred from this small set. The normal fixture includes authored morph normal
offsets, nonuniform skinning and an animated material subset.

A negative control using `assets/material_subsets.usda` produced identical
CPU/GPU images but correctly failed with `no_gpu_deformation`. Its artifacts
are in `target/deformation-static-control`. Invalid tolerance 256 was rejected
before creating output, and rerunning against an existing output directory was
rejected without changing its results.tsv. Bash syntax and whitespace checks
pass. The final metadata guards were also checked at time 5 in
`target/deformation-final-script`.
