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

## Same-frame GPU view and light readback

`5241231` adds `src/capture_uniforms.rs` to the opt-in morph capture diagnostic.
It attaches the selected camera to the screenshot entity, extracts that request,
and copies the camera's actual prepared `ViewUniform` and `GpuLights` buffer
ranges after preparation and before rendering that frame. The snapshot decodes
every field, preserves the uploaded frame counter and requires it to match the
extracted frame. The screenshot observer rejects missing, failed or mismatched
readbacks. This is no longer an inference from main-world camera settings.

The diagnostic adds `COPY_SRC` only when the probe is installed, bounds the
combined readback to 64 KiB and GPU polling to five seconds, and runs once per
capture. It can perturb timing and is not a performance measurement. No normal
viewer/library setting, rendering quality, light order or shader changes.
Tests cover invalid/overflowing ranges, screenshot identity, missing/failed
results, incomplete data, frame mismatches and one-ULP view changes.

Evidence: `/tmp/bevy-optimization-gpu-uniforms.IQkPUy/`, including `hidden/`.

- Saved viewer SHA-256:
  `2d5c82c7024cfda8228cc63faa1198903c6efa9a0e36a2b763369afd0126efe1`.
- Cargo.lock remains `41ef4e9e…`; hardening passes 870 no-default and 997
  all-feature/all-target executions, formatting and strict Clippy
  (`harden-fixed.log`). The initial formatting and mapped-range API errors
  remain in `harden.log` and `harden-formatted.log`.
- Twelve native captures use that one saved binary, unchanged assets/settings,
  no owned build overlap, and the previous isolated compositor/capture protocol.
  First-batch load is 3.96–4.48; hidden-batch load is 5.51–8.11. These are not
  native frame-rate measurements. All process/pipeline/readback and input/asset
  hash checks pass. Four broader host warnings remain in every capture.
- The first six-case reveal/redo/probe-control batch passes all six exact image
  comparisons. Extracted lights, staged palettes and main camera/grid metadata
  remain equal. The publication checker accepts all three visible GPU histories
  (4/2/4 recorded inputs, two final inputs each). This batch alone does not fix
  or invalidate the earlier intermittent failures.
- The next six cases cover eager/deferred GPU selection and undo, and both CPU
  selection controls. Only four of seven exact comparisons pass. GPU eager
  selection differs from deferred selection, eager undo and the CPU control by
  220 pixels, max RGB error two. All three failures share their coordinates:
  the earlier 215 positions plus five additional pixels. Main camera/grid
  metadata and the final hidden state agree. `hidden/pixel-analysis.json`
  preserves the coordinates and colors; the eager hidden image was inspected.
- Across all ten probed captures, 5,360 bytes of view/light uniforms are read
  for the matching screenshot frame. All decoded view fields and the complete
  decoded light data are equal within each batch. Frame counters are explicitly
  retained: reveal/CPU 185, redo 187, hidden selection 184, undo 186. Importantly,
  the failing eager/deferred selection pair has **the same counter, 184**, as
  well as equal uploaded view/light values. `uniform-analysis.json` and
  `hidden/uniform-analysis.json` retain the full decoded data and comparisons.

Both drivers exit 1 with the original host diagnostics retained; the hidden
batch also retains its three image failures. The initial analysis invocation
failed because the outer shell had no `node` in PATH; the successful analyses
run the explicit Nix-store Node executable through the make recipe. Their
initial logs are preserved, and the staging script reports actual batch results
rather than carrying forward the previous batch's hard-coded failure message.

This rules out a difference in the captured view/light uniform values as the
explanation for these particular hidden-state failures. It does not establish
equality of grid/material buffers, compiled shaders, draw bindings, shadow/depth
textures or intermediate render targets. Inspect those remaining render inputs
and passes next, rather than changing camera/light math or repeating an unchanged
capture matrix. The strict visual gate, earlier one-pixel mesh observation,
downstream patch distribution, low-limit adapters, lifecycle/platform checks and
full performance acceptance remain open.

## Release GPU capture and shader replacement

`906918b` adds the optional viewer-only `debugger_capture` feature and
`make build-debugger-capture`. `USD_CAPTURE_RENDERDOC=1` requires the morph
capture probe and an already injected RenderDoc library. The API starts after
the same-frame view/light readback and finishes after rendering, requires a
successful capture and exactly one new nonempty capture file, and rejects a
missing injection. Normal viewer/library defaults and upstream versions do not
change. The initial attempt using wgpu's debugger hooks produced no capture:
those hooks are unavailable without debug assertions. Its process exit zero is
not accepted as a successful GPU capture. Global debug assertions remain off.

Evidence: `/tmp/bevy-optimization-renderdoc.E1TYys/`, particularly `api/`.

- Final API hardening passes 872 no-default and 999 all-feature/all-target
  executions, formatting and strict Clippy (`harden-api.log`); the release build
  passes (`build-api.log`). Saved viewer SHA-256 is
  `0e841f64990aae1f89c7d940c49fbe4e2bfe39603c3d07c71baa844116a5dfe3`.
  Cargo.lock is `818d95e2…`, adding existing optional diagnostic dependencies,
  not changing upstream OpenUSD or Bevy versions.
- Eager and deferred hidden-selection captures each save one actual RDC file,
  with matching uploaded frame counter 184, successful native process and
  screenshot readback. The same executable also supplies an uninjected control
  and a negative control that explicitly rejects a requested but absent debugger.
  Four host warnings remain; injected runs additionally report four EGL backend
  errors. Process success does not waive these broader diagnostics.
- RenderDoc 1.44 and its matching source are transient Nix inputs. Layer lookup
  is confined to the child environment; no global layer registration, desktop
  automation or replay window is used. Native C++ replay tools export
  `renderdoc__replay__marker`, avoiding self-injection. The failed first replay,
  helper compilation attempts and accidentally reused old helper are retained;
  only `smoke-repeat-verified` and `smoke-simple-verified` establish the repeated
  full/simple replay result.
- Both captures contain the grid at event 407 and shader-visible frame 184.
  Explicit API captures have no meaningful RenderDoc frame number; the uploaded
  counter establishes correlation. Grid transform and settings buffers are
  byte-exact. Every typed view field matches. The 14 changed bytes in the raw
  784-byte view block lie in padding identified by its reflected shader layout,
  not in camera, grading or frame values (`analysis.json`).
- Grid vertex SPIR-V is byte-exact. Fragment disassembly and executable SPIR-V
  instructions also match. Only one `OpSource` instruction differs: generated
  WGSL moves the fullscreen vertex-output struct before/after the view structs.
  Removing debug instructions yields identical modules with SHA-256
  `4aa1fa29fbe7ca56aca509de60aa985a165513c4e3b26ba00588e021b66de3ec`.
- The native eager screenshot differs from deferred, uninjected control, and
  **replay of its own capture** by the same historical 215 pixels, max RGB two.
  Six snapshots from full/simple repeated replay agree exactly with deferred
  and control, not native eager. Replay is therefore not a pixel-exact oracle
  for that native image; its mismatch is retained, not waived.
- A controlled replacement experiment on each fixed capture builds and installs
  the eager fragment shader, deferred fragment shader and common debug-stripped
  shader in turn. Each variant is replayed twice and the original restored.
  All 22 saved images (including baselines and initial-target reads) are byte-exact
  with the control. Build/replace/replay/restore checks pass in both processes;
  `replacement-analysis.json` records all hashes. The reported rasterization,
  four-sample MSAA, depth and basic blend settings match; this is not a claim
  that every Vulkan pipeline state has been compared.

These results reject different executable grid shaders and different typed
grid/view values as explanations for this captured pair. Shader debug-source
replacement does not reproduce the native mismatch. They do not identify a GPU
driver defect, prove all render inputs equal, or establish native performance.
Next compare the native same-frame main render target with its screenshot and
GPU replay to locate the difference before changing rendering math. The strict
visual gate, earlier mesh-pixel observation, downstream patch distribution,
low-limit adapters and full performance/lifecycle/platform acceptance stay open.

## Native final-target witness

`08fded6` adds a readback of the captured camera's final resolved main texture
after rendering and before cleanup, while the explicitly requested GPU capture
is still active. `src/capture_target.rs` saves `.gpu-main.bin` plus format,
dimensions, row size and matching frame metadata. It copies native texel bytes
without color conversion, strips only transfer-row padding, bounds staging to
64 MiB and polling to five seconds, and rejects unsupported formats, dimensions,
multisampling, missing copy usage, incomplete reads and failed writes. Failures
also propagate into the screenshot report rather than allowing a success marker.
The diagnostic remains opt-in; normal rendering and screenshot tolerances do not
change. Three tests cover checked layouts, incompatible targets and row padding.

Evidence: `/tmp/bevy-optimization-renderdoc.E1TYys/target/`.

- Hardening passes 875 no-default and 1,002 all-feature/all-target executions,
  formatting and strict Clippy (`target-harden-fixed.log` in the parent folder).
  The first compile exposed a missing new state-field initializer; its failed
  log is retained separately. Release build passes (`target-build.log`).
- Saved viewer SHA-256 is
  `dbd3bdee45eecba7ebdf7a1b86a6a783bfcaa79263b68f4963929bfb47c960aa`.
  `inputs.sha256` records the lockfile and exact source hashes; captures preceded
  the focused local commit, so their initial environment files record the parent
  revision and dirty source. The saved source hashes match the committed code.
- Four alternating eager/deferred hidden-selection runs pass process, debugger,
  native target readback and screenshot checks. Each records frame 184, a
  1440x920 `Rgba8UnormSrgb` main texture and 5,299,200 tightly packed bytes.
  Eight structured host diagnostics per injected run remain. No owned build
  overlaps captures or replay; recorded system load is 4.03–4.30. No FPS claim.
- All four native main targets equal their screenshot bytes, including alpha.
  All six additional cross-run main/screenshot comparisons are exact
  (`analysis.json`). Each image has SHA-256 `091585c7…`, matching the earlier
  control, not the earlier failing eager image. This batch **does not reproduce
  or fix** the intermittent mismatch.
- Full replays of the first eager/deferred captures complete with 40 draws and
  last event 459. Their actual grid view counters remain 184. Replayed screenshot
  bytes equal both corresponding native targets and native screenshots. The
  added post-render copy is captured without changing the frame's draw count.
  Grid event IDs are 407 and 408 respectively; the final validator locates the
  pipeline by name instead of assuming event IDs remain stable between captures.

The native-target diagnostic is verified on this passing batch, but has not yet
localized a failing frame. It currently runs with RenderDoc injection, so an
independent native-target readback without that injection is the next useful
control. Do not infer that screenshot conversion, shader debug source, grid math
or the GPU driver caused the retained failure. Avoid another unchanged matrix;
keep the 215/220-pixel and earlier mesh-pixel failures, and the full performance,
downstream, resource-lifecycle and platform gates open.

## Independent native readback and publication race repair

`2b4c718` separates `USD_CAPTURE_TARGET_PROBE=0|1` from debugger injection.
Explicit one requires the existing morph probe but no RenderDoc injection or
optional debugger feature. When omitted it follows `USD_CAPTURE_RENDERDOC`,
preserving the previous diagnostic default; explicit zero also permits an
injected capture without the additional final-target readback. Neither flag
changes normal viewer/library rendering.

The first uninjected attempt exposed an actual capture-publication race. The
deferred run produced valid final-target files, but its screenshot observer had
already cloned the earlier uniform report, so the emitted screenshot metadata
lacked all final-target fields despite `VIEWPORT_CAPTURE_OK`. The controller
rejected this incomplete evidence. A successful process and an eventually
present target file were not accepted as a complete correlated capture.

The repair marks the report pending until requested target readback **and**
debugger finalization finish. Screenshot delivery retains one image and its
original metadata in a main-world writer, which polls readiness without blocking
the render thread. It writes only after completion, or records an explicit error
after a ten-second deadline. Screenshot identity checks, target/readback and
debugger errors propagate; a wrong screenshot cannot complete the pending one.
Tests cover independent flag combinations, pending/wrong/failed completion and
the writer deadline. Normal captures without those diagnostics do not acquire
GPU readback waits.

Evidence: `/tmp/bevy-optimization-native-target.BlmySb/`, with final captures in
`fixed/`. Initial binaries, source hashes, incomplete reports and failed case log
are retained separately from the repaired run.

- Initial independent-probe hardening passed 877 / 1,004 executions but did not
  establish correct asynchronous publication. After the repair, hardening passes
  878 no-default and 1,005 all-feature/all-target executions, formatting and
  strict Clippy (`harden-race-fix.log`), followed by a successful release build.
- Final saved viewer SHA-256 is
  `cc2b573b49545100a212be98e4ed079cfd6dd7b915c7b45cd68bc537d7e20d4e`.
  Cargo.lock remains `818d95e2…`. `fixed/inputs.sha256` and `source.patch` identify
  the tested source, including captures performed before the focused local
  commit. No owned compilation overlaps the native captures; load is 4.09–6.79.
  Unrelated application work is left alone. These are not performance samples.
- Eight controls cover eager/deferred policy, debugger injection on/off and
  final-target readback on/off, using the same saved executable. All native
  processes, requested outputs, screenshot readbacks and configuration assertions
  pass. All view counters are 184. Four requested target readbacks carry complete
  matching metadata; four disabled controls create neither target files nor
  target-report fields. Four injected cases produce verified RDC files, and four
  uninjected cases produce none. Host diagnostics remain four without injection
  and eight with injection; they are not waived.
- All eight screenshot images and four native main-target images are byte-exact,
  SHA-256 `091585c7…`. Nineteen comparisons cover same-frame main/screenshot,
  eager/deferred, readback on/off, injection on/off and cross-run target controls.
  RGB and alpha are exact. Uploaded typed view/light values also match
  (`fixed/analysis.json`). This batch does not reproduce the original 215/220-pixel
  mismatch and therefore does not establish a renderer fix.
- The earlier failing capture pair also has byte-identical blit vertex and
  fragment SPIR-V. Its fragment shader only samples the input texture; the
  upstream blit layout is non-filtering with a default nearest sampler. There
  is no evidence here for a speculative linear-filter or shader-math change.

The capture-publication race is repaired and the independent probe is verified.
The intermittent visual issue remains open. Use this probe on previously failing
undo/reveal command prefixes or inspect remaining recorded render-target/binding
state, rather than rerunning the unchanged hidden-selection controls or claiming
these passing images invalidate earlier failures. Full playback performance,
downstream patch distribution, resource-lifecycle and platform acceptance remain
unproven.
