# CPU/GPU deformation capture sweep

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
