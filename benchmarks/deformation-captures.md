# CPU/GPU deformation capture sweep

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
