# Morph descriptor publication across reveal and redo

## Scope

This follows the failed native residency matrix in `native-residency.md`. It is
a renderer correctness investigation, not a frame-rate or policy-default result.
Library and viewer defaults remain eager. All work is local and unpushed.

The optional `USD_CAPTURE_MORPH_PROBE=1` requires `USD_SCREENSHOT` and
`USD_CAPTURE_RESIDENCY`. It reads the target mesh and generated material subsets,
then records three render stages: before descriptor preparation, after descriptor
preparation, and after GPU mesh collection. It reports CPU-side render staging,
not a GPU buffer readback. Geometry, weights, joint matrices, descriptor offsets
and cached mesh input uniforms are retained. The report has a 64 KiB text bound,
2 MiB transition-history bound, explicit omissions/truncation flags and latest
per-phase snapshots. Unchanged phase snapshots are deduplicated independently.
Normal viewing installs none of these probe systems.

## Unpatched witness

- Probe commit: `c2b1f54`; hardening: 864 no-default and 987 all-feature/all-target
  executions, formatting and strict Clippy pass. Initial compile failures from
  the system parameter count and a formatting borrow are retained in `harden.log`;
  the corrected gate is `harden-fixed.log`.
- Raw evidence: `/tmp/bevy-optimization-morph-witness.yEwQmF/`.
- Saved release viewer SHA-256:
  `0395e854249557f1220f25be613d694305f19f248d98f504374506496b03cb1e`.
- Cargo.lock SHA-256:
  `589b1ed2cd24e835d6caa460fac7cd91d84590ce6ca7499e024b9cf4b65fc5ef`.
- Six isolated Weston Vulkan runs share one saved executable. Four probe cases
  cover eager GPU reveal, deferred GPU reveal/redo and eager CPU reveal; two
  same-binary probe-off controls cover eager/deferred GPU reveal. Source asset,
  binary and lock manifests pass. No own build overlaps the captures; host load
  is 4.27–4.84. No user desktop or input is controlled.
- The scene, seek sequence (0, 10, 7, 7), camera, command acknowledgements,
  60-update pre-command wait and 120-update capture gate match the earlier matrix.

At the first collected visible frame, both `/Test/Face` and its `Part` subset
have a ready render mesh, allocated morph targets and extracted weights. But the
descriptor preparation stage sees no *previously collected* instance, so it
publishes no descriptor. GPU collection stores `u32::MAX` in each mesh input's
`morph_descriptor_index`. On the following frame descriptors exist and contain
the correct weight `0.7`; the cached mesh inputs still contain `u32::MAX` through
the final capture, more than 100 frames later.

| Case | First cached sentinel frame | Descriptor available frame | Final frame |
| --- | ---: | ---: | ---: |
| Eager reveal | 66 | 67 | 184 |
| Deferred reveal, probe on | 67 | 68 | 184 |
| Deferred redo | 68 | 69 | 186 |

The GPU parent/subset vertex attributes, indices, morph targets, weights and
joint world/inverse matrices match exactly across these three final states.
Entity IDs are normalized only for this source comparison; numeric geometry and
matrices are not rounded. `analyze.mjs`, `analysis.json` and `analysis-fixed.log`
retain the assertions. An initial analyzer incorrectly required every unchanged
phase to be recorded on the same frame; it now uses the latest earlier snapshot
of that phase. The initial failure is retained in `analysis-initial.log`.
All four probe histories have zero omitted events and no truncated text.

All six capture processes pass pipeline/readback checks, but each still has four
broader isolated-host warnings. Three of six exact image comparisons fail:

- Eager probe on/off is pixel-exact, preserving the original wrong shape.
- Deferred probe on/off differs by 39,764 pixels, max RGB error 182: enabling the
  read-only probe changes timing enough to expose the stale index on cold reveal.
  The probe is **not observationally neutral**. Cold deferred success is not a
  stable correctness control.
- The three probed GPU states are pixel-exact with each other, but are wrong:
  both CPU comparisons differ by 39,764 pixels, max RGB error 182.

The driver exits 1 and the failed comparisons and host warnings remain intact.

## Repair boundary

The upstream PBR crate was imported byte-exactly in `6138f1f`, excluding only the
local Cargo `.cargo-ok` marker. Licenses, upstream manifest and VCS metadata are
retained. Repair `efa30b7` changes `src/render/morph.rs` to extract the mesh
asset ID together with its weights, so descriptor preparation no longer depends
on a mesh instance that is collected later in the same schedule.

No deformation weights, normals, shader tolerances, CPU/GPU mode, quality
defaults, visibility timing or per-frame re-extraction policy are changed.
The patch is selected by the workspace Cargo patch; downstream consumers must
apply it in their own root until an upstream replacement exists. See
`vendor/bevy_pbr/PATCHES.md`.

## Patched verification

The same six-case driver ran against the saved patched release binary, after all
owned builds finished, into `fixed/` under the evidence directory above. Host load
was 10.16–13.18 with unrelated builds left alone; these are correctness captures,
not comparable performance samples. All input/asset manifests pass.

- Viewer SHA-256:
  `64a754f0abfa57ee299021630a1d152afe7c02eee246f4b18131f4d30ae8e9ed`.
- Checker SHA-256:
  `cd96fb4876d8ec69e01b8a6fcc799f6a6db27365a5c7794d14448687e088069c`.
- Cargo.lock SHA-256:
  `41ef4e9e984b83678413b9bec6f4f790453a98ad0aeddb539b265523b0b161c8`.

`examples/morph_publication_check.rs` (`7855fe7`) rejects a stale descriptor in
any recorded collected input, even if a later frame repairs it. It requires
complete probe history and visible final GPU morph inputs, and ignores pending
uncollected instances and pre-collection stages. Four tests cover sentinels,
mismatched indices, pending/preparation states and incomplete evidence. Run the
saved checker with `oslo make run-headless-benchmark --binary CHECKER --args='CAPTURE.txt ...'`.

The checker rejects all three unpatched GPU reports and accepts all three patched
reports. Each patched report verifies four history inputs and two final inputs,
covering parent and subset. First collection is still at frames 66/67/68, but now
contains matching descriptor indices immediately. All source geometry, morph
targets/weights and joint world/inverse matrices match their corresponding
unpatched reports exactly; `analyze-fixed.mjs` and `fixed/analysis.json` retain
that comparison. The GPU renderer remains active.

Hardening passes 864 no-default and 991 all-feature/all-target executions,
formatting and strict Clippy (`harden-ready-fixed.log`). `make test-pbr` passes
the published crate's three unit tests; these are not GPU acceptance tests.
The hardening replay exposed an unrelated single-update assumption in
`independent_clocks_sample_light_intensity`. `12dbc0a` makes that test wait for
both roots to become `Ready`, explicitly exercises default and zero projection
budgets, and preserves all intensity/identity assertions. The original failure
and the first repair's compile error remain in `harden-checker.log` and
`harden-ready.log`; no production scheduling budget changed.

After the captures finished, `make test-native-deformation` passed all four
native OpenUSD CPU-oracle comparisons, and `make check-web` passed the wasm32
compile check. Logs are `native-deformation.log` and `check-web.log`. Neither
replaces actual browser rendering or the broader native GPU matrix.

All six patched native processes pass pipeline/readback checks. The eager GPU
image is pixel-exact with the CPU reference, as are eager probe-off, deferred
probe-off and deferred redo. The former 39,764-pixel/max-182 wrong-shape failure
is absent. Both unpatched and patched eager screenshots were visually inspected.

The strict image gate **still fails**. Probed cold deferred reveal differs from
the common reference at `(625, 538)`, inside the deformed mesh: RGBA
`[208, 200, 191, 255]` versus `[208, 200, 190, 255]`. This single blue-channel
difference causes four of the six exact comparisons to fail. Its cause is not
established; do not label it grid noise or loosen the threshold. The other two
comparisons are exact. `fixed/pixel-locations.json` records all five comparisons
against the eager reference. Four isolated-host warnings remain per capture.
The patched driver therefore still exits 1, and those failures are retained.

This establishes the stale-index repair for the tested first-collected inputs
and final rendered shape, not all first-visible-frame images or broad GPU/platform
acceptance. The follow-up matrix and diagnostic captures below retain the
low-limit adapter, downstream Cargo patch, earlier visual, host diagnostic,
60 Hz and platform gates.

## Full command-prefix replay

`/tmp/bevy-optimization-morph-matrix.E51V0L/` records the patched 16-case matrix:
CPU/GPU deformation, eager/deferred policy and select/reveal/undo/redo command
prefixes. It uses the same saved `64a754f0…` viewer and `41ef4e9e…` lock above,
unchanged shaders/settings and no probe. No owned builds overlap captures;
observed one-minute host load ranges from 7.39 to 15.49. These are correctness
captures, not performance measurements. Input and asset hash checks pass.

All 16 processes pass pipeline/readback checks and command-state assertions,
including identity, scene time, camera, history, selection and residency.
The substantial 39,764-pixel/max-182 shape failure does not recur. Nevertheless,
only 15 of 24 exact image comparisons pass. All nine failures change exactly
215 pixels by at most two RGB levels; all nine use the same pixel coordinates,
including pairs where the deformed mesh is hidden. The changed positions span
the background/grid region rather than the former large shape discrepancy.
`metadata-analysis.mjs` verifies equal camera/grid metadata across all 16 reports
and writes the coordinates to `metadata-analysis.json`. This is main-world
metadata, not proof of equal uploaded GPU inputs or an established shader cause.

Each capture still reports four broader isolated-host warnings. The driver exits
1; the 24 comparison logs, raw images and failures remain intact. No image
tolerance or quality setting changes.

## Extracted lights and staged palettes

Probe extension `e5d2780` records bounded extracted directional-light parameters
and ordering after light preparation, plus CPU-side skin staging palettes for
the parent/subset. It does not read actual GPU buffers or alter rendering order.
The normal viewer does not install it. Unit tests cover palette bounds and the
distinction between main/render entity ordering. `Entity::Ord` follows entity
bits, not the numeric index shown by entity formatting; the original test's
incorrect ordering assumption and subsequent repair are retained in the logs.

Evidence: `/tmp/bevy-optimization-light-witness.aPKcBd/`.

- Source: `e5d2780`; viewer SHA-256
  `be22bb39debf5daf6337edc0a82eed0e5897398702a61f6ce6f9eaf59ab3e843`.
- Cargo.lock remains `41ef4e9e…`; hardening passes 866 no-default and 993
  all-feature/all-target executions, formatting and strict Clippy
  (`harden-fixed.log`).
- Six same-binary native captures cover probed eager reveal, deferred reveal,
  deferred redo, CPU reveal and two probe-off controls. All process/pipeline/
  readback checks pass; all retain four host warnings and the driver exits 1.
- `analyze.mjs` verifies identical extracted light order (rim, fill, key) and
  parameters across all four probed cases, identical staged parent/subset
  joint matrices across the three GPU cases, and equal camera/grid metadata.
  Skin allocation offsets differ, but matrix values do not. No probe history
  or text is omitted/truncated. These observations do not justify a light-order
  or skin-matrix runtime patch.
- The saved publication checker accepts all three new GPU reports, each with
  four history inputs and two final inputs (`publication-check.log`).
- One of six strict comparisons is exact. Other failures are 215 pixels/max 2
  for deferred probe on/off, eager/deferred and reveal/redo; CPU/eager differs
  by one mesh pixel/max 1, and CPU/deferred by 216 pixels/max 2.
- At `(625, 538)` CPU is `[208, 200, 191, 255]` and all three probed GPU images
  are `[208, 200, 190, 255]`. The previous batch assigned that one-pixel variant
  differently. It is not established as a reproducible cold-deferral defect.

The remaining next step is correlated prepared/GPU render-input evidence, not
another unchanged matrix, an unproven light-order fix or threshold relaxation.
Full deformation, lifecycle, native performance and platform acceptance remain
open; library and viewer stay eager by default.
