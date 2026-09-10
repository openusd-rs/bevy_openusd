# Remaining Bevy integration work

Goal: complete the Bevy-facing work identified in OPENUSD_UPGRADE.md.
The dependency upgrade is a baseline, not completion of this goal.

## Acceptance checklist

- [ ] Source-preserving asset loading without temporary files, including USDZ.
- [ ] Bevy-tracked layer/texture dependencies, reloads and explicit failure states.
- [ ] Root replacement/despawn cleanup and independent live USD instances.
- [ ] Independent clocks, variants and overrides; preserve runtime-only components.
- [ ] Composition-aware editing: selection, editable inspector, layers/edit target,
      variants/payloads, provenance, undo/redo and explicit save operations.
- [ ] Typed/reusable authoring API and safe snippet composition.
- [ ] GPU deformation and environment lighting with fidelity/performance evidence.
- [ ] Native prototype/instance projection with measured asset sharing.
- [ ] Real AssetServer, reload, multi-instance, save/reopen and image regressions.
- [ ] Runnable showcase and documented support/approximation matrix.

## Verification

Use make check-all, make test-all and make build. Validate UI changes through
rendered captures, and use explicit benchmarks for performance claims.
Do not count upstream APIs as implemented Bevy features.

## Current work

### Public transform path semantics

Current upstream inspection disproved the suspected property-path panic:
`Stage::prim` converts an already parsed property path to its owning prim. The
transform reader now propagates the upstream Result instead of asserting it,
without introducing a stricter local path policy. Module documentation states
the owner-resolution behavior. A regression covers all five public transform
entry points, authored and unauthored property names, default and numeric times,
and missing owners returning None. This is a documented compatibility contract,
not a claim that the previous implementation had a reproduced panic.

Validation: 429 ordinary tests pass (seven native export tests ignored), plus
check-all, build and whitespace checks. Logs:
`/tmp/xform-path-{tests,check,build}.log`.

### Atomic saves and deleted dependency recovery

Extended native event acceptance to replace both a sublayer and a PNG via
temporary-file rename, verifying changed geometry/pixels and stable runtime
identity in two mounts. Stock Bevy 0.19.1 passes these atomic-save cases.
Deleting a loaded sublayer exposed a separate failure: its AssetServer handles
RemovedAsset by refreshing folders only, leaving loader dependents marked Ready.
The failing native test is recorded in `/tmp/native-watcher-removal.log`.

Added opt-in `watcher::file_source`, a read-only AssetSourceBuilder retaining
Bevy's native reader and debounced FileWatcher. Its owned event worker forwards
file removal as ModifiedAsset plus the original event, so AssetServer reloads
the dependency owners, reports Failed and retains their last good projections.
Recreating the layer restores Ready without manual reload. The worker closes
its input and joins on source drop; setup errors are logged. Applications must
register this source before AssetPlugin; no default sources are replaced
implicitly. This adapter is for unprocessed native file sources, not folder
deletion, processed pipelines or the direct editor Open path.

Separate native tests preserve stock watcher coverage and exercise the adapter's
deletion/recreation behavior. Both passed three consecutive runs on Linux:
`/tmp/native-watcher-lifecycle-{1,2,3}.log`. These are filesystem and projected
asset checks, not GPU screenshot acceptance during reload.

Validation: 428 ordinary workspace tests pass (seven native export tests ignored),
plus check-all, build and whitespace checks. The watcher-enabled library suite
passes 374 tests (nine explicit native tests ignored). Logs:
`/tmp/watcher-removal-{tests,check,build,feature}.log`.

### Native filesystem watching

Added opt-in `usd_bevy/file_watcher` support, forwarding Bevy's native watcher
feature without enabling it for default consumers. A filesystem-backed test
mounts one composed scene twice, writes its sublayer and PNG dependency directly,
and checks changed projected geometry and decoded pixels in both instances.
It then writes a malformed sublayer, verifies Failed states retain the last good
projection, and restores the layer to recover Ready states. Runtime entity IDs,
names and child parenting survive; projected meshes remain shared. Labeled image
handles remain stable while their pixels update, as Bevy reloads them in place.

The regression uses no injected asset events or explicit reload calls. It is
feature-gated and ignored by default because it requires native filesystem
events; README includes its explicit Make invocation. This does not implement
watching for the viewer's direct editor Open path or prove other OS backends,
atomic-save rename behavior, or GPU image fidelity during reload.

Validation: 428 ordinary workspace tests pass (seven native export tests ignored),
plus check-all, build and whitespace checks. The watcher-enabled library suite
passes 374 tests (eight ignored), and the explicit native watcher test passes
once plus three consecutive repeats on Linux. Logs:
`/tmp/native-watcher-{tests,check,build,feature,test}.log` and
`/tmp/native-watcher-repeat-{1,2,3}.log`.

### AssetServer UV-chain instance clocks

Extended the UV fixture regression through the real file-backed AssetServer and
USD dependency loader. Two scene roots share the source and texture while taking
independent clocks through 0/10, 10/0, 2.5/7.5, 5/5 and 10/10. The test checks
the resulting StandardMaterial affine matrices against independent expected
values and verifies actual decoded quadrant image bytes, shared mesh handles,
distinct material handles at different times and shared handles at equal times.
Projected entities, runtime names and runtime-child parenting survive sampling;
both root layers retain identical authored text throughout.

This exercises loaded textures and instance resampling in a headless Bevy
schedule. The earlier GPU endpoint captures remain separate evidence; this test
does not claim OS watcher reloads or simultaneous multi-root GPU image acceptance.
All 428 ordinary tests pass (seven native export tests ignored), plus check-all,
build and whitespace checks. Logs: `/tmp/uv-instance-{tests,check,build}.log`.

### Affine UV-transform chains

Connected `UsdTransform2d` chains now compose outer-to-inner as affine matrices,
retaining nonorthogonal axes rather than decomposing the result into one SRT.
The material still conjugates the composed matrix through the mesh V-flip.
`ReadPreviewMaterial::uv_transform` now exposes canonical `bevy::math::Affine2`;
removed the old local `UvTransform` struct without a shim and documented the API
migration. Different channel transforms remain explicit errors; the traversal
budget still bounds cycles and overly deep graphs.

Regressions cover chain order and a nonuniform-scale/37-degree-rotation composition
with nonorthogonal axes, using independent point calculations. Updated the UV
fixture to split its animated linear transform and translation across two nodes;
its explicit endpoint coordinates remain unchanged. This supersedes the previous
chain-rejection limitation, not the remaining per-texture UV-set limitations.
All 427 ordinary tests pass (seven native export tests ignored), plus check-all,
build, fixture generation and whitespace checks. Logs:
`/tmp/uv-chain-{tests,check,build,fixture}.log`. The two-node chain was captured
and inspected at time 0/10, eye (0,1,5), focus (0,1,0), shadows off:
`target/uv_chain_{0,10}.png`. Both match the validated single-node/explicit-UV
endpoint renders across all 921,600 pixels at strict zero RGB tolerance, with
CAPTURE_OK and no WARN/ERROR entries. Logs: `/tmp/uv-chain-{capture,compare}-{0,10}.log`.
Nonorthogonal chain composition is unit-tested; this endpoint capture pair is
not a rendered shear or normal-map-transform acceptance claim.

### Connected texture-coordinate discovery

The material reader now retains each resolved texture node and follows its
connected `inputs:st` path instead of scanning for the first transform child.
Disconnected transforms no longer affect shading; connected transforms outside
the material's immediate children are discovered. Texture channels must agree on
the material-wide transform. Conflicting transforms, multiple input connections,
chained transform nodes, non-finite values and graphs beyond 16 connections
return explicit errors rather than silently choosing an unrelated node.

The existing reader regression now demonstrates an ignored disconnected node,
a connected local node, a connected external node and explicit chain rejection.
A new graph regression covers conflicting/common transforms, non-finite inputs
and cycles. The independent-instance material animation test previously expected
an unconnected transform to apply; it now expects identity while still validating
animated color/opacity/roughness and instance isolation. Arbitrary per-texture UV
sets and composed multi-transform chains are not implemented by this change.
All 426 ordinary tests pass (seven native export tests ignored), plus check-all,
build and whitespace checks (`/tmp/connected-uv-{tests,check,build}.log`). Both
animated UV endpoints were recaptured and inspected at the earlier fixed camera,
time 0/10, shadows off: `target/connected_uv_{0,10}.png`. Both remain identical
to their validated pre-change captures across all 921,600 pixels at strict-zero
RGB tolerance, with CAPTURE_OK and no WARN/ERROR entries. Logs:
`/tmp/connected-uv-{capture,compare}-{0,10}.log`.

### Correct UV-transform coordinate conversion

Confirmed a material rendering bug: USD UV transforms were applied directly to
already V-flipped mesh coordinates. StandardMaterial now receives `F * T * F`,
where F maps `(u,v)` to `(u,1-v)`, preserving USD scale/rotation/translation in
the renderer's coordinate basis. The unit regression independently evaluates
the transformed coordinates for identity, translation, nonuniform/negative
scales and rotations at several UV points.

Added `examples/uv_transform_fixture.rs`: a quadrant texture, animated mapped UVs
and explicit endpoint references. Before the fix, time 0 rendered blue instead
of red; 121,104 pixels differed, maximum RGB error 194, mean 15.526238.
Both baseline captures were inspected: `target/uv_transform_before_{mapped,reference}.png`.
Logs: `/tmp/uv-transform-before-{mapped,reference,compare}.log`.
The fixture's source-level test checks both endpoint transforms against the
explicit UVs and refuses directory overwrites. Intermediate-time explicit-UV
interpolation is not a reference for interpolated rotation. Per-texture transform
chains and normal-map tangent behavior under arbitrary UV transforms remain open.
After the fix, both endpoint comparisons are pixel-identical across all 921,600
pixels at strict zero RGB tolerance. Fixed images were inspected:
`target/uv_transform_after_{mapped_0,mapped_10,reference_10}.png`; time 0 uses the
unchanged earlier explicit reference. Captures report CAPTURE_OK without
WARN/ERROR entries. Logs: `/tmp/uv-transform-after-{mapped-0,mapped-10,reference-10,compare-0,compare-10}.log`.
All 425 ordinary tests pass (seven native export tests ignored), plus check-all,
build and whitespace checks (`/tmp/uv-transform-{tests,check,build}.log`).

### Normal-map orientation probe

Added `examples/normal_fixture.rs`, which generates a raw 8-bit tangent-normal
map, its equivalent explicit-normal quad and an opposite-Y control in a new
directory. The source follows the canonical USD PreviewSurface scale/bias rule:
https://openusd.org/24.08/spec_usdpreviewsurface.html . The CPU regression verifies
the actual generated tangent basis maps the sampled vector to the expected world
normal and that an extra Y flip produces a different vector. Bevy 0.19.1's
`bevy_mesh/src/mikktspace.rs` already negates tangent handedness after generation;
no renderer flip or other speculative material change was made.

All three GPU captures were inspected at eye (0,1,5), focus (0,1,0), time 0,
shadows off: `target/normal_before_{mapped,reference}.png` and
`target/normal_opposite.png`. Mapped vs equivalent differs at only two of 921,600
pixels, maximum RGB error 1 (strict-zero comparison exits nonzero). The opposite
control changes 121,104 pixels, maximum 163, mean RGB error 19.492016, visibly
darkening the quad. All captures have CAPTURE_OK without WARN/ERROR entries.
Logs: `/tmp/normal-before-{mapped,reference,compare}.log`,
`/tmp/normal-opposite-{capture,compare}.log`.

All 423 ordinary tests pass (seven native export tests ignored), plus check-all,
build and generator execution (`/tmp/normal-fixture-{tests,check,build,run}.log`).
This verifies a static constant normal map and orientation-sensitive reference,
not arbitrary shader scale/bias, UV transforms, deformed tangent fidelity or
native-renderer image parity.

### Guarded public TRS reads

Moved the renderer's guarded TRS decomposition into one internal reader helper.
Public `read_transform`/`read_transform_at` now use the same checks and return an
explicit matrix-API diagnostic for shear, projective or degenerate matrices,
rather than silently returning a lossy/invalid decomposition. Reflection and
ordinary rotated TRS still round-trip within the existing float tolerance.
The renderer retains its finite-affine residual fallback, including singular and
tiny scales; full-matrix reader behavior is unchanged.

The new regression exercises public reads with shear, zero/tiny scale, projective
and reflected/rotated matrices. Existing runtime propagation and reset/history
tests exercise the shared implementation. `live::current_transform` maps these
errors to None and `transform_of` maps them to identity as documented; neither
helper is an exact full-matrix API.
All 422 ordinary tests pass (seven native export tests ignored), plus clean
check-all, build and whitespace checks (`/tmp/trs-reader-{tests,check,build}.log`).
The assembly was recaptured and inspected at the same fixed camera:
`target/trs_guard_assembly.png`. All 921,600 RGB pixels match the pre-refactor
capture at zero tolerance; CAPTURE_OK without WARN/ERROR entries. Logs:
`/tmp/trs-guard-{capture,compare}.log`.

### Shared captured-asset read handles

Captured root/dependency asset opens now retain an Arc-backed read cursor rather
than cloning the entire byte payload into a new Vec. Handles have independent
seek positions and remain usable after the resolver/source is dropped, including
transfer to another thread. The regression verifies the exact shared-ownership
count for root and dependency handles, cursor independence, EOF/rewind/end-relative
seeks, invalid backward seeks and source-lifetime independence.

This removes one full-buffer copy per captured asset handle, not all parser or
texture copies. `read_all` still allocates its output, and ZIP entry extraction
still allocates bounded decompressed bytes. No end-to-end speedup or peak-RSS
reduction is claimed without a separate measurement.
All 421 ordinary tests and all seven optional native export tests pass, plus
check-all, build and whitespace checks. Logs:
`/tmp/shared-source-{tests,native,check,build}.log`.

### Runnable grouped assembly walkthrough

Added `examples/editor_assembly.rs`: a reusable batch recipe composes two colored
reference sites from a canonical typed Sphere source captured entirely in memory.
It verifies creation, one-step undo/redo, inherited schema/radius, live affine
editing with entity/runtime-name preservation, and optional flattened export.
The regression saves and reopens USDA, USDC and USDZ, checking both transforms,
radii and colors. Root dev-dependency `tempfile` supplies isolated test outputs;
the scene's composition itself does not materialize source files.

All 420 ordinary tests pass (seven native export tests ignored), plus check-all,
build, the runnable example and whitespace checks. Logs:
`/tmp/editor-assembly-{tests,check,build,run}.log`. The generated
`target/editor_assembly.usda` was captured on NVIDIA/Vulkan at time 0 with eye
(6,4,8), focus (0,1,0). `target/editor_assembly.png` was inspected: the warm sphere
and cool sheared/elongated sphere have the expected distinct poses and colors.
Capture reports CAPTURE_OK without WARN/ERROR entries
(`/tmp/editor-assembly-capture.log`). This is a focused executable walkthrough,
not the complete flagship showcase, native-renderer parity or a new typed DSL.

### Grouped editor commands

Added `EditorEdit::Batch(Vec<EditorEdit>)` for ordered, nested command groups with
one canonical history entry. Failure rolls back completed child transactions;
empty groups preserve redo. Selection changes compose in child order. The bridge
now collects all namespace movements and reverses both direction and order for
undo, preserving runtime entities across chained renames/moves.

Tests build a configured assembly through a nested batch, compare exact layer
data after undo/redo, verify failed-batch rollback without losing prior redo,
and exercise nested namespace batches through the real editor/live Bevy schedule
while checking selection, entity identity and runtime children. Groups use one
edit target; stage sinks still observe constituent transactions. This is grouped
document authoring, not a completed typed scene DSL or observer-isolated commit.
All 419 ordinary tests pass (seven native export tests ignored), plus check-all,
build and whitespace checks. Logs: `/tmp/editor-batch-{tests,check,build}.log`.

### One canonical transform history

Removed `live::TransformHistory` and its TRS snapshot/inverse-edit implementation.
Transform undo now has the same public owner as other document edits:
`EditorSession::edit(EditorEdit::TransformMatrix { prim, matrix, reset })`.
This is an API removal, with a migration recipe in README and no compatibility
shim. Low-level `live::author_transform` remains a non-undoable TRS writer;
`live::current_transform` remains a lossy TRS read, not an undo snapshot API.

The former two-edit history regression now uses the canonical session and checks
exact layer text before/after full undo/redo. A new real Bevy propagation test
edits a reset-stack shear into an inherited translation, then undo/redo/undo
restores the correct global matrix, reset override and exact layer data while
retaining the projected entity and its runtime name. Existing animated-stack,
mapped-edit-target and save/reopen history regressions remain in the full suite.
This does not add a gizmo widget or native OS input acceptance.
All 418 ordinary tests pass (seven native export tests ignored), plus check-all,
build and whitespace checks. Logs: `/tmp/canonical-transform-{tests,check,build}.log`.

### Range-safe vector-scale inverses

Inverse vector scale ops now use component reciprocals rather than a generic
matrix determinant/inverse. This accepts finite scales such as 1e-200 and 1e200
whose determinants underflow or overflow while their reciprocals remain finite.
Zero components, non-finite input and overflowing reciprocals remain errors.
Scalar-axis inverse behavior is unchanged. Arbitrary matrix-op inversion still
uses the generic matrix inverse and does not gain this range guarantee.

The regression checks positive/negative components at both extremes and the
invalid cases. Native OpenUSD 25.05.01 returns the same reciprocal matrices for
both extremes (`/tmp/native-inverse-range.log`).
All 417 ordinary tests pass (seven native export tests ignored), together with
check-all, build and whitespace checks. Gate logs:
`/tmp/xform-inverse-range-{tests,check,build}.log`.

### Double-precision local transform composition

The reader now composes local op matrices in f64 and converts only the final
matrix to Bevy's f32 representation. `read_transform_stack_f64_at` exposes the
double result and reset flag; the existing float API diagnoses final values
outside f32 range. This preserves small residuals such as 100000001 - 100000000
and allows individually large/small scale ops whose product fits the renderer.
It does not make Bevy hierarchy/global propagation double precision or eliminate
precision loss between separately projected parent/child transforms.

Tests cover the one-unit residual, a sub-f32 residual retained by the f64 API,
large/small finite scale composition and explicit final-range rejection. Existing
half, quaternion, scalar-axis and inverse tests continue to exercise the reader.
Native OpenUSD 25.05.01 confirms the one-unit local result for
`assets/xform_precision.usda` (`/tmp/native-double-stack.log`). Its reference is
`assets/xform_precision_reference.usda`, which authors a direct translation.
Both fixed-camera captures were inspected and have identical RGB pixels at
strict zero tolerance (921,600 pixels), CAPTURE_OK and no WARN/ERROR entries.
Images: `target/xform_precision{,_reference}.png`; comparison:
`/tmp/xform-precision-compare.log`. The original precision gates passed:
`/tmp/xform-double-{tests,check,build}.log`.

### Scalar-axis transforms and adjacent inverse cancellation

Added translateX/Y/Z and scaleX/Y/Z with half/float/double scalar decoding.
Adjacent op/inverse pairs cancel before evaluating either op, including singular
scales and inverse-first pairs. Non-adjacent pairs are not recursively removed.
Native OpenUSD 25.05.01 probes confirm cancellation and its unusual unpaired
scalar-axis inverse-scale behavior: negate the scalar, unlike reciprocal vector
scale. The reader follows that native behavior explicitly rather than claiming
these forms are interchangeable. Probe logs:
`/tmp/native-axis-inverses-final.log`, `/tmp/native-singular-pair.log`.

Tests compare the mixed-precision `assets/xform_scalar_axes.usda` stack against
the existing double-vector fixture, verify per-axis inverse values, and exercise
singular/direct/inverse-first cancellation. Broader animated/native-version
compatibility of the scalar-axis inverse behavior remains open.
Both scalar-axis and vector reference captures were inspected at time 0, eye
(6,4,8), focus (0,1,0): `target/scalar-xform_{scalar_axes,half_reference}.png`.
All 921,600 pixels match at strict zero tolerance, with CAPTURE_OK and no
WARN/ERROR entries in the capture logs. Logs:
`/tmp/scalar-xform_{scalar_axes,half_reference}-capture.log`,
`/tmp/scalar-axes-compare.log`. All 414 ordinary tests pass (seven native export
tests ignored), with check-all, build and whitespace checks passing. Gate logs:
`/tmp/scalar-axes-{tests,check,build}.log`.

### Native-compatible quaternion conversion and half orientations

The earlier unit-length rejection was too strict: native OpenUSD derives an
axis-angle rotation from the real component and normalized imaginary axis, not
from a normalized whole quaternion. Conversion now preserves this behavior in
double precision before producing the float render matrix and accepts quath,
quatf and quatd. Imaginary-axis lengths at or below 1e-10 produce identity;
non-finite values and overflowing axis lengths remain diagnostic failures.

Native OpenUSD 25.05.01 probes through its Python bindings (validation only,
no file edits) checked zero/non-unit quaternions, rounded half values, small-axis
thresholds and large magnitudes. `UsdGeom.XformOp.GetOpTransform` produced the
reference for `assets/xform_quath_reference.usda`; `assets/xform_quath.usda`
authors the corresponding half quaternion. Probe logs:
`/tmp/native-orient-probe.log`, `/tmp/native-orient-limits.log`,
`/tmp/native-quath-op.log`. Unit tests retain these native cases and check the
fixture's composed matrix and inverse. Very large finite axes that overflow
length computation are rejected rather than reproducing native's identity
fallback; broader malformed-data and animated quaternion parity remain open.
Both fixtures were captured at time 0, eye (6,4,8), focus (0,1,0), and inspected:
`target/xform_quath{,_reference}.png`. All 921,600 pixels match at strict zero
RGB tolerance; capture logs have CAPTURE_OK without WARN/ERROR entries. This
compares Bevy rendering against a native-derived matrix, not native rendering.
Logs: `/tmp/xform_quath{,_reference}-capture.log`, `/tmp/xform-quath-compare.log`.
All 412 ordinary tests, check-all, build and whitespace checks pass (seven native
export tests ignored). Logs: `/tmp/xform-quath-{tests,check,build}.log`.

### Half-precision transform values and explicit unsupported-op errors

Transform reading now decodes half scalars and half3 vectors for the existing
rotation/translation/scale routes. Wrong value types, malformed op-order types
and unsupported authored op kinds return diagnostics rather than silently
substituting identity. Missing op values retain the existing identity fallback.
Ordinary projection surfaces these errors through `UsdTransformError` and keeps
the previous valid pose. Scalar-axis translation/scale and half orientations
were added in the later slices above.

The supported half scalar/vector forms were checked against the official
[OpenUSD xform-op implementation](https://raw.githubusercontent.com/PixarAnimationStudios/OpenUSD/release/pxr/usd/usdGeom/xformOp.cpp).
Reader tests compare a half stack to a double stack and an independently
constructed matrix, check inverse operations, and verify wrong-type/unknown-op
diagnostics. Fixtures `assets/xform_half{,_reference}.usda` render identically at
time 0 with eye (6,4,8), focus (0,1,0): zero changed pixels of 921,600 at strict
zero RGB tolerance. Both `target/xform_half{,_reference}.png` were inspected;
capture logs have CAPTURE_OK without WARN/ERROR entries. This is a Bevy
half-versus-double reference, not native USD renderer parity. Logs:
`/tmp/xform_half{,_reference}-capture.log`, `/tmp/xform-half-compare.log`.
All 410 ordinary tests pass, seven native tests ignored; check-all, build and
whitespace checks pass. Logs: `/tmp/xform-half-{tests,check,build}.log`.

### Matrix widget input replay

The existing `USD_UI_REPLAY` hook now has checked-in matrix load/edit/undo
sequences, plus parser regression coverage. Events enter the real egui widget
input stream in the native viewer; these are not direct editor-command calls,
but they do bypass physical keyboard/mouse and compositor input delivery.

Using `/Affine` in `assets/xform_animation.usda` at time 5:

- `matrix_sample_load.replay` clicks Load sampled matrix; the draft displays
  the authored shear (0.5 and 0.75) instead of identity.
- `matrix_sample_edit.replay` clears/types row 4, scrolls, clicks Apply sample,
  then scrolls back and reloads the sampled matrix. Row 4 remains `2 0 0 1`,
  demonstrating that the value reached the authored sample, not just the draft.
- `matrix_sample_undo.replay` additionally clicks the Undo toolbar control and
  reloads the sample. Row 4 returns to `0 0 0 1`, retaining the shear rows.

All three end states were captured and inspected:
`target/viewer-ui-captures/matrix-widget-load.png`,
`matrix-widget-edit-reload.png` and `matrix-widget-undo-reload.png` in the same
directory. Replay events are recorded in the adjacent viewer logs. The scripts
use fixed coordinates for the 1600x1000 private capture layout, inspector open,
selected `/Affine`, `USD_CAPTURE_TIME=5` with `USD_SCREENSHOT` enabled; edit/undo
captures used `USD_UI_CAPTURE_WAIT=25`. Logs:
`/tmp/matrix-widget-{load,edit-reload,undo-reload}.log`.
Native OS input, redo-button and file-picker acceptance remain separate.
All 408 ordinary tests pass, with seven native export tests ignored; check-all,
build and whitespace checks pass. Logs: `/tmp/matrix-widget-{tests,check,build}.log`.

### Sampled matrix inspection and draft loading

`EditorSession::snapshot_at` optionally resolves scalar `matrix4d` attributes at
a labelled scene time while preserving the separate default-value snapshot.
The editor bridge supplies its timeline time. Other attribute types, including
large matrix arrays, are not additionally sampled by this path.
The matrix inspector's Load sampled matrix action copies that value into the
draft and sets the apply-sample time field without sending an authoring command.
Drafts are not automatically replaced during playback.

Tests cover internal-reference time offset/scale mapping, interpolation, absent
defaults, invalid inspection times, unchanged authored source/history and bridge
publication after Seek. The widget replay above subsequently exercised loading,
editing, applying and undo; the snapshot tests alone do not certify input.
The time-5 inspector state was captured and inspected at
`target/viewer-ui-captures/matrix-sample-load5.png`: the load action displays
time 5, while the untouched identity draft remains separate. The capture-time
environment setting requires `USD_SCREENSHOT`; the initial UI-only run did not
set it and stayed at time 0. Corrected log: `/tmp/matrix-sample-load5-ui.log`.
All 407 ordinary tests pass, seven native tests ignored; check-all, build and
whitespace checks pass. Logs: `/tmp/matrix-sample-inspection-{tests,check,build}.log`.

### Four-row matrix attribute inspector

The inspector now edits `matrix4d` values in four USD rows, with an explicit
translation-row label and lossless f64 text round trips. Matrix attributes have
their own section, retaining the existing provenance, default/sample, block and
clear controls. These are ordinary attribute edits, not whole-stack replacement;
the transform-op order remains unchanged. A missing default is labelled as an
identity draft rather than an evaluated animated pose. Attribute draft keys now
include document and edit-layer identity to avoid reusing drafts across them.

Tests cover exact double-precision row formatting/parsing, wrong-length and
non-finite rejection, matrix templates, and a parsed matrix time-sample edit
with unchanged default/op-order and exact undo. The actual static matrix section
was captured and inspected at
`target/viewer-ui-captures/matrix-inspector.png` using `/Panels` in
`assets/xform_affine.usda`. The four rows and authored shear values are visible.
Native button/keyboard interaction is not yet acceptance-tested; parser/model
coverage and screenshot inspection are not a substitute for that interaction.
The sample-only `/Affine` case was also captured and inspected at
`target/viewer-ui-captures/matrix-inspector-sampled.png`; the identity-draft
warning, four rows and sample times 0/5/10 are visible. Capture logs:
`/tmp/matrix-inspector-ui.log`, `/tmp/matrix-inspector-sampled-ui.log`.
Known desktop/shared-device warnings remain; the private capture sessions exited.
All 406 ordinary tests pass, seven native tests ignored; final check-all, build
and whitespace checks pass. Logs: `/tmp/matrix-inspector-final-tests.log`,
`/tmp/matrix-inspector-final2-check.log`, `/tmp/matrix-inspector-final-build.log`.

### Visible transform diagnostics

The selected prim's `UsdTransformError` now joins the inspector's rendering
issues, rather than being visible only to low-level capture. The publication
regression includes transform errors and checks clearing after component removal
and selection changes. `assets/xform_invalid.usda` supplies a projective matrix
over the material-panel fixture.

The actual inspector was captured and inspected at
`target/viewer-ui-captures/xform-invalid-inspector.png` with `/Panels` selected.
It visibly reports `Transform: non-finite or projective USD transform` while the
fallback pose remains displayed. The private compositor/viewer was cleaned up;
known UI SSAO/shared-device and desktop integration warnings remain.
Low-level capture of the same fixture returned a nonzero exit with the prim path
and transform error, no CAPTURE_OK and no output PNG. Evidence:
`/tmp/xform-invalid-ui.log`, `/tmp/xform-invalid-capture.log` and the UI capture's
adjacent viewer/compositor logs. This does not add a matrix-editing inspector
widget or scene-wide error summary.
All 404 ordinary tests pass (seven optional native tests ignored), with check-all,
build and whitespace checks passing. Logs:
`/tmp/xform-diagnostics-{tests,check,build}.log`.

### Matrix-preserving editor command

`EditorEdit::TransformMatrix` accepts a finite affine column-major f64 matrix
representable by the renderer and an explicit reset flag. It replaces the local
stack with a static matrix, clearing matrix/order local samples and retaining
other op attributes as inactive opinions. `EditorSession` groups its authored
transactions into one undo command and uses the existing layer-diff history,
not TRS decomposition. This is an editor API; no new inspector matrix widget or
gizmo wiring is claimed, and the legacy `live::TransformHistory` remains lossy.

Tests verify exact root-layer restoration of an animated matrix stack, static
replacement at four times, reset preservation, redo and invalid-edit rejection
without losing redo. A layered case verifies that undo removes the stronger
override, reveals the weaker animation and restores every loaded layer's
serialized authored data.
All 403 ordinary tests pass, with six native tests ignored; check-all, build
and whitespace checks pass. Logs: `/tmp/editor-matrix-{tests,check,build}.log`.

A mapped-target regression now authors through an internal reference with a time
offset, verifies the matrix is authored on `/Source` rather than `/M`, switches
back to the root edit target and checks exact undo/redo plus target preservation.
Root/edit/flattened output in USDA/USDC/USD reopens with the same matrix and reset
at four times on both composed paths. These modes share one physical layer in
this mapped fixture; external reference-layer and variant-target matrix-command
coverage remain open.

An optional native export test sends all nine mode/format combinations through
native `usdcat --flatten`, then checks the returned composed data for the static
affine matrix/reset, absence of stale matrix samples and retained children.
This verifies native parse/composition interoperability, not native renderer
fidelity or native file-picker interaction.
All 404 ordinary tests and all seven optional native export tests pass, along
with check-all, build and whitespace checks. Logs:
`/tmp/editor-mapped-{tests,check,build,native}.log`.

### Ordinary affine transform propagation: validation in progress

Ordinary prim projection now retains affine linear residuals and applies USD
stack resets after Bevy transform propagation, before frusta and GPU joint
updates. Resets preserve the nearest USD pseudo-root placement and up-axis
conversion. Runtime Transform edits on an affine prim multiply its residual;
descendants retain their existing hierarchy. Invalid non-finite or projective
samples retain the previous pose with `UsdTransformError`; capture rejects that
diagnostic instead of recording an apparently successful image.

Decomposition is guarded by nonzero finite determinant and orthogonal axes;
singular, tiny and sheared affine matrices use the residual path. Unit tests
cover representation, a runtime descendant under shear, runtime translation
edits and nested reset behavior under a rotated scene root and translated mount.
An authored reset regression also checks prefix exclusion, override removal on
reprojection, runtime-name preservation and unchanged source-layer data.
Live edit coverage now sends projective and NaN matrices through the stage
change sink and `apply_changes`, verifies last-good affine globals, then restores
TRS and checks error/override removal, unchanged entity identity and runtime
name preservation. Broader animated coverage remains pending. This does not
complete the integration acceptance checklist.

All 397 ordinary tests pass (six native export tests ignored), alongside
check-all, build and whitespace checks. Logs:
`/tmp/affine-validation-{tests,check,build}.log`.

Rendered fixtures `assets/xform_affine{,_reference}.usda` and
`assets/xform_reset{,_reference}.usda` were captured at time 0, eye (6,4,8),
focus (0,1,0), 1280x720. All four images in `target/xform_*.png` were inspected.
The affine pair differs at two of 921,600 pixels, maximum RGB error 1, mean
0.000001; strict zero tolerance correctly fails. The Z-up reset pair is pixel
identical, despite the authored parent translation and pre-reset translation.
All four capture logs have CAPTURE_OK and no WARN/ERROR entries. Logs:
`/tmp/xform*-capture.log`, `/tmp/xform_{affine,reset}-compare.log`. These are
Bevy geometry references, not comparison with a native USD renderer.
The fixture regression brings the ordinary test total to 398 passing, with six
native tests ignored. Check-all, build and whitespace checks pass; logs:
`/tmp/xform-fixtures-{tests,check,build}.log`.

The initial transform-op guard rejected non-finite/non-unit quaternions before
matrix construction and singular inverse ops before inversion. Inversion uses
double precision before converting back to finite float matrices, avoiding
determinant underflow for tiny invertible scales. Reader tests check malformed
orientations, identity recovery, singular inverses and a 1e-20 scale inverse.
The later native-compatible quaternion slice above replaces its non-unit
rejection; broader animated orientation parity remains unverified.
All 400 ordinary tests pass, with six native tests ignored; check-all, build
and whitespace checks pass. Logs: `/tmp/xform-recovery-final-{tests,check,build}.log`.

`assets/xform_animation.usda` exercises identity → shear → translated TRS under
Z-up, with one following cube and one independently translating reset cube.
An AssetPlugin/UsdAssetPlugin integration test loads two placed instances of the
same source, scrubs six pairs of independent clocks including interpolated times
2.5 and 7.5, and compares globals against independently interpolated matrices.
It verifies affine override insertion/removal, reset exclusion, stable prim IDs
and preserved runtime children/local transforms/names. The focused and full
workspace gates pass; all 401 ordinary tests, check-all, build and whitespace
checks pass, with six native tests ignored. Logs:
`/tmp/xform-clocks-{tests,check,build}.log`.

Native fixed-camera captures at times 0, 5 and 10 were inspected:
`target/xform-animation-{0,5,10}.png`, eye (8,5,10), focus (2,1,0).
The orange cube shears then returns to a translated cube; the blue reset cube
retains its shape and follows its own translation. Logs
`/tmp/xform-animation-{0,5,10}.log` have CAPTURE_OK without WARN/ERROR entries.
These are visual transition checks, not native-USD reference or performance proof.

### Preserve affine transforms on direct mesh point prototypes

Direct mesh prototype baking now uses the authored composed matrix, not its
lossy TRS decomposition. Positions retain shear; normals use inverse transpose,
tangent directions use the linear matrix and tangent handedness tracks its
determinant. Double-precision directional calculations avoid determinant
underflow at small but valid scales. Non-finite, singular and projective matrices
still reject the prototype rather than publishing invalid geometry.

Unit coverage checks sheared positions, perpendicular unit normal/tangent
frames, reflections, tiny scales and invalid matrices. Integration coverage
compares rendered triangle corners and normal directions against explicit
reference points, preserving the source triangulation, and checks shared subset
handles across three instances plus unchanged source-layer data. The initial
reference test needed the same triangulation basis: triangulating already
sheared quads can select a different diagonal without changing the surface.

Fixtures: `assets/point_affine.usda`, `assets/point_affine_reference.usda`.
The hierarchy path still rejects per-node shear; ordinary transform projection
and GPU point-prototype deformation are not changed by this slice.

All 395 ordinary tests, check-all, build and whitespace checks pass; logs:
`/tmp/prototype-affine-final-tests.log`, `/tmp/prototype-affine-final2-check.log`,
`/tmp/prototype-affine-final-build.log`. Captures `target/prototype-affine.png`
and `target/prototype-affine-reference.png` were inspected at time 0 with eye
(12,7,16), focus (0,1,0). They differ at 6 of 921,600 pixels, maximum RGB error
1 and mean 0.000002; zero tolerance correctly fails. Renderer logs contain no
warnings/errors (`/tmp/prototype-affine-{capture,reference,compare}.log`). This
is an explicit-geometry reference in Bevy, not a native USD renderer comparison.

### Repeatable multi-time CPU/GPU image comparisons

Added `scripts/compare_deformation.sh`, using make for both capture modes and
pixel comparisons. It retains per-time images, raw readbacks, metadata, renderer
logs, difference images and a TSV ledger; failures do not stop later pairs.
Tolerance is explicit, output directories must be new, diagnostics fail the run,
and metadata must show GPU deformation in the GPU case and none in the CPU case.
Component counts do not certify every draw or GPU performance.

Five combined skin/morph/subset samples differ at 2–14 pixels with maximum RGB
error 1, so the zero-tolerance sweep correctly fails. Three authored-normal/
nonuniform-skinning samples match exactly and pass. All sixteen images were
inspected, with no renderer warnings/errors. A static negative control has
identical images but correctly fails for missing GPU deformation. Invalid
tolerance and existing-output preservation checks pass. Commands, raw metrics,
artifact locations and limitations are in `benchmarks/deformation-captures.md`.
No Rust source changed in this slice; validation covers the new script and
actual captures rather than claiming another run of the ordinary Rust suite.

### Distinguish directory aliases from package-layer anchors when saving

Ordinary save relocation now compares canonical directory identities when
available. Saving through `..` or a directory symlink into the source directory
preserves authored asset spelling, including patterns that would reject actual
cross-directory relocation. A regression checks both aliases while retaining
the existing real-relocation failure/publication-preservation check.

Conversely, a layer inside a USDZ must be re-anchored even when its ordinary
export sits beside that archive: the package entry, not the filesystem directory,
is the original dependency anchor. The native fixture now unwraps both package
root and nested edit layers into USDA, USDC and USD, checking referenced values
and exact texture bytes through the retained external archive. The ordinary
output still depends on that archive; it is not independently portable.

All 393 ordinary tests, check-all, build and six optional native export tests
pass (`/tmp/save-anchor-identity-final2-{tests,check,build,native}.log`). The
extended native fixture covers six additional packaged-root/edit ordinary
export combinations. Whitespace checks pass. Vendored code and environment
configuration are unchanged in this slice.

### Anchor dependencies for ordinary cross-directory root/edit saves

Ordinary root/edit exports to another directory now serialize an anchored copy
through the stage's resolver. Sublayers, references, payloads, all reference/
payload list buckets, typed asset arrays, dictionaries and time samples are
rewritten without modifying live layer data or identifiers. Same-directory
exports preserve authored spelling. Dependencies remain external, unlike USDZ
packaging. The implementation is recorded in
`patches/openusd-layer-reanchoring.patch` with the vendored source.

The regression exports both the root and a nested edit layer into another
directory in USDA, USDC and USD. Reopened values and texture bytes survive;
explicit-relative unresolved asset samples retain their source directory in the
exported edit layer. Root and weak live-layer serializations remain unchanged.
Unsupported asset expressions, tile/sequence patterns and clip templates fail
before publication; the test preserves an existing destination on such failure.
Anonymous flattened nested-asset metadata and arbitrary custom-resolver
portability remain unverified. This does not save unsaved edits in other layers
or make ordinary exports self-contained.

All 393 ordinary tests, check-all and build pass
(`/tmp/save-reanchor-final2-{tests,check,build}.log`). All six optional native
export tests pass (`/tmp/save-reanchor-native.log`), including six new
cross-directory mode/format combinations. The temporary upstream validation
checkout passes 1,589 library tests and strict library clippy
(`/tmp/save-reanchor-upstream-final3-tests.log`,
`/tmp/save-reanchor-upstream-clippy.log`). Validation required the existing
writable Cargo cache and a trailing slash in CARGO_WORKSPACE_DIR for fixture
paths; earlier setup failures are not counted as product test results. All five
vendored patches pass reverse-application checks; whitespace checks pass.

### Fit viewer and capture bounds to evaluated deformation

`usd_bevy::mesh::bounds::MeshBounds` samples world-space bounds from current
render vertices, target-major morph data and joint global transforms multiplied
by inverse bind poses. Morph offsets precede skinning. Referenced MorphWeights
components are supported, unreferenced vertices and empty subsets are excluded,
and malformed/non-finite inputs return no bounds. Static meshes use available
vertex data, with transformed Aabb fallback when their CPU asset is unavailable.
The sampler does not change Bevy's culling bounds or skinning implementation.

Viewer opening now uses this sampler once per document instead of undeformed
Aabbs, retaining the existing rule that subsequent camera input is preserved.
The standalone capture fits its grid in Last, after joint globals update, using
the same sampler. GPU deformation remains enabled: CPU evaluation here is for
explicit bounds sampling, not replacement of the rendering path. No per-frame
viewer resampling or automatic camera movement during playback was added.

Regressions check morph-before-skin ordering, ignored undrawn vertices, malformed
inputs, shared morph references, inverse bind poses, missing joints and five
times of combined USD skin/morph deformation against CPU-evaluated points under
a rotated, nonuniformly scaled transform.

All 392 ordinary tests, check-all, build and whitespace checks pass; logs are
`/tmp/deformed-bounds-final3-{tests,check,build}.log`. Private-Weston viewer
captures `target/viewer-ui-captures/deformed-framing-{gpu,cpu}.png` were inspected:
both now frame the deformed bar at matching size and position. Embedded viewport
readbacks differ at 318 of 1,324,800 pixels, maximum RGB error 2, mean 0.000114
(`/tmp/deformed-framing-compare.log`). This is a near-match, not pixel equality.
The UI logs still report the environment's missing Mesa device-select layer,
clipboard connection and shared-device SSAO limit; no claim of warning-free UI
startup or full environment parity is made.
Final fixed-camera captures `target/deformed-bounds-final-{gpu,cpu}.png` were
also inspected. They differ at 6 of 921,600 pixels, maximum RGB error 1, mean
0.000002; the zero-tolerance comparison correctly exits nonzero. Logs:
`/tmp/deformed-bounds-final-{gpu,cpu,compare}.log`. Both standalone renderer logs
contain no warnings/errors. These results cover the named fixture at time 30,
not all assets, times, material modes or native USD visual equivalence.

### Build compact subsets directly from borrowed source data

Subset construction now borrows the original mesh and selected indices rather
than cloning the full source before discarding unreferenced attributes. Only
retained attribute and morph entries are copied into compact outputs. The
all-vertices-selected path still clones the required full data, and malformed
inputs retain the previous conservative full-layout fallback. Source assets
are never mutated; different subsets remain independent.

An additional regression checks two independent selections from an unindexed
source, per-corner positions and both morph targets, source immutability and the
all-selected path. All 389 ordinary tests, check-all and build pass in
`/tmp/subset-borrow-final-{tests,check,build}.log`. CPU and GPU captures at time
30 were inspected and their RGBA payloads match the corresponding pre-change
`subset-empty` captures byte-for-byte. Logs `/tmp/subset-borrow-{gpu,cpu}.log`
contain no warnings/errors; images are `target/subset-borrow-{gpu,cpu}.png`.
Release ANYmal opens measured 476–506 ms with all retained payload/entity counts
unchanged. The earlier compacted-source samples were 616–680 ms; these are not
interleaved controlled timings. Raw samples and limits are recorded in
`benchmarks/editor-assets.md` and `/tmp/subset-borrow-anymal.log`.

### Compact subset vertices and keep empty assets out of GPU uploads

`cffdd62` compacts each material subset and the unassigned remainder, applying
one stable vertex remapping to every attribute and every target-major morph
block. Invalid inputs retain their original layout without partial mutation.
Regression tests compare indexed attributes byte-for-byte and preserve skin,
morph, independent-clock, subdivision and instancer behavior. ANYmal's retained
vertex payload drops to 19,969,120 bytes from 720,548,416–771,589,120, with the
same 296 mesh entities and 231 subset entities. Raw before/after samples and
CPU-only measurement limits are in `benchmarks/editor-assets.md`.

Native fixed-camera captures exposed two allocator errors for the empty parent
remainder in both CPU and GPU deformation modes. Bevy 0.19.1's mesh allocator
skips zero-byte vertex allocation but still attempts the vertex/index uploads.
Empty compacted assets now omit RENDER_WORLD usage while retaining their CPU
data and entity identity. Animated subsets regain renderable assets when faces
return; a regression checks this transition and isolation from another root.
Empty morph data also stays absent, avoiding the image-backed morph allocator's
division by zero for zero vertices.

All 388 ordinary tests, check-all, build and whitespace checks pass. Logs:
`/tmp/subset-empty-final-{tests,check}.log`, `/tmp/subset-empty-build.log`.
Both fixed-camera captures at time 30 complete without logged warnings/errors:
`target/subset-empty-{gpu,cpu}.png` and `/tmp/subset-empty-{gpu,cpu}.log`.
Each raw RGBA file is byte-identical to its corresponding pre-upload-fix
`target/subset-compact-fixed-{gpu,cpu}.rgba`; removing empty uploads does not
change those rendered pixels. Both output images were visually inspected.

Source cache payload and transient full-source clones remain. Automatic grid
fitting and viewer framing differ between CPU- and GPU-deformed bounds; fixed
camera captures isolate geometry from framing but do not prove full visual
parity or matched native USD rendering. The broad acceptance checklist remains
open.

### Measure real editor opens and retained subset payload

Added examples/editor_benchmark.rs using the real EditorCommand::Open path,
Bevy asset tracking, 100 idle updates and settled asset-payload counters. It
reports open/idle time, mesh/subset entity counts, unique retained mesh assets,
stored/unreferenced vertices, attribute/index/morph bytes and decoded image bytes.
Failed opens, invalid projected indices and document replacement while idle fail
the run. The bundled subset regression checks exact payload/count results and
failure handling. An initial untracked-assets prototype was corrected before
recording the final baseline; pending handle cleanup must not be counted as
settled memory.

Release runs of real ANYmal and Spot completed. ANYmal retains 720,548,416–
771,589,120 vertex-attribute bytes, with over 98% of stored vertices absent from
their mesh's indices; Spot retains 451,232 bytes with none unreferenced. This
points to subset vertex compaction as substantive remaining memory work. Joint
and morph data must be remapped with positions, not silently discarded.
Raw samples, commands and measurement boundaries are in
`benchmarks/editor-assets.md`. These are CPU payload counts, not VRAM/RSS,
rendered performance or a before/after speedup claim.

All 386 ordinary tests, check-all, build and whitespace checks pass
(`/tmp/editor-benchmark-final-{tests,check,build}.log`). Final release logs:
`/tmp/editor-benchmark-{anymal,spot}-tracked.log`. No collection or sibling files
were modified.

### Build material-subset indices without discarded vertex attributes

Subset preparation previously called mesh_from_usd_subset for each material part
and the remainder, retained only its indices, and discarded newly computed
positions, normals, UVs, colors and tangents. It now uses an index-only helper
that preserves the full source mesh's render-vertex layout. Indexed and expanded
layouts reuse the existing triangulator; flat geometry shares the canonical
triangle-selection mapping with the full builder. Reference triangulation points,
hole filtering and authored winding remain unchanged. Source meshes are still
cloned for their existing attributes, including deformation data.

A 432-case differential regression compares direct indices against the full
builder over six layouts, both windings, hole configurations, empty/reordered/
duplicate/invalid face selections and deformed positions with reference topology.
Existing fixed flat-index expectations and GPU skin/morph/subset tests also pass.
All 385 ordinary tests, check-all, build and whitespace checks pass
(`/tmp/subset-indices-{tests,check,build}.log`).

Inspected `target/viewer-ui-captures/anymal-subset-indices.png` against the previous
healthy ANYmal capture: the robot and material regions remain visible without an
observed regression; no reproduced GPU validation errors are logged. This is a
qualitative rendered regression check, not pixel-identical or reference-renderer
acceptance. No controlled CPU speedup measurement was made for this change.
Per-subset vertex-buffer duplication remains, so the device-bounded mesh allocator
is still required; this change does not establish reduced GPU memory or draw calls.

### Surface stopped-renderer diagnostics in the viewer

Installed a viewer-owned Bevy RenderErrorHandler that retains the first error in
RenderSettingsBridge and returns StopRendering. It does not resume a broken
renderer, request AppExit, mutate the USD document or attempt recovery. The
Outliner prioritizes the renderer failure over Ready/file-dialog text, and the
Rendering pane shows the same wrapped diagnostic and save-before-restart advice.
Descriptions are UTF-8-safe, bounded to 2,048 characters with an ellipsis; the
original Bevy error log remains available.

Verified against real GPU validation failures, not only injected UI strings:
temporarily restored the oversized slab limit and captured ANYmal in both panes.
Inspected `target/viewer-ui-captures/anymal-renderer-error-status.png` and
`anymal-renderer-error-pane.png`; both show readable Validation/buffer-limit
details. Logs no longer repeat the default handler's quitting message. The
temporary unsafe setting was restored before final builds/tests and is absent
from the committed diff.

Inspected the final safe build's `anymal-renderer-status-healthy.png`: ANYmal
renders normally, Ready is shown, and its log has no reproduced render-validation
errors. Tests verify StopRendering, retained first-error text through publication,
no AppExit message, retained world entities, absent-bridge handling and bounded
multibyte diagnostics. All 384 ordinary tests, check-all, build and whitespace
checks pass (`/tmp/renderer-status-final-{tests,check,build}.log`).

This does not implement renderer/device recovery. The viewport's host-provided
warm-up placeholder can remain behind the explicit error pane; no Mara changes
were made. Saving through a native picker after a GPU failure and continued UI
operation after loss of the shared host device remain unverified.

### Render ANYmal within the embedded device's buffer limit

Actual collection inspection exposed two separate failures. The first capture,
`target/viewer-ui-captures/anymal-texture-acceptance.png`, contained only the
private desktop: the old 20-second delay began at viewport construction while
ANYmal was still projecting. The viewer now emits USD_VIEWER_UI_UPDATED after
its first complete host UI update, and the capture script starts its bounded
delay at that marker. Startup still has a 300-second timeout. This is not a
GPU-readiness handshake; screenshots still require inspection.

The next capture, `anymal-ui-update.png`, showed Ready but only the renderer's
warming-up placeholder. Its log identified pooled mesh buffers of 453,496,320
and 536,870,912 bytes exceeding the shared device's 268,435,456-byte limit.
At RenderStartup, the viewer now bounds MeshAllocatorSettings to half the actual
device max_buffer_size, including compatible minimum and large-allocation
thresholds. Smaller configured limits and the growth factor are preserved.
The half-limit leaves room for allocator slot rounding; individual oversized
meshes and total CPU/GPU memory budgets remain separate concerns. No Mara,
Bevy dependency source, collection assets or environment configuration changed.

Inspected `target/viewer-ui-captures/anymal-bounded-slabs.png`: the actual ANYmal
robot, distinct material regions and studio grid render. Its log contains none
of the reproduced Bevy render-validation errors. The source report traverses 25
meshes, and the viewer projects 424 prims. The source JPEG and material graph
confirm a carbon-fibre diffuse texture, but this camera/capture does not establish
texture orientation, normal fidelity or matched reference-renderer appearance.
Ready currently describes the document, not renderer health; surfacing renderer
failures rather than an indefinite warm-up placeholder remains open.

All 383 ordinary tests, check-all, build, shell syntax and whitespace checks pass.
The new unit test covers the 256MiB device, smaller caller settings and lower
limits. Logs: `/tmp/mesh-slab-limit-{tests,check,build}.log`; native export tests
were not rerun for these viewer-only changes. All captures used a private Weston
compositor and cleaned up only their own process groups.

### Refresh live animation membership after sparse edits

Reproduced time-sample authoring on an already-projected static Xform leaving it
out of AnimatedPrims (`/tmp/sparse-animation-before.log`). USD reports these
edits as changed-info, not resync, so the existing sparse component patch bypassed
animation discovery. Sparse live patches now refresh the affected prim's index
membership when a live animation index exists. Source-root ticks without that
resource do not perform this additional scan; dependency-affecting edits still
use the existing reconciliation path.

The regression authors samples through actual USD attributes, checks sparse
notices, scrubs through the real animation-resampling system and verifies the
interpolated transform. Removing both samples restores the static default and
removes index membership without replacing the entity. All 382 ordinary tests,
check-all, build and whitespace checks pass
(`/tmp/sparse-animation-{tests,check,build}.log`). This is live editing/timeline
state coverage, not a new rendered animation or native export acceptance claim.

### Skip unused source-reload animation discovery

Source-root publication now disables animation-index collection during
reconciliation, rather than building the live editor index and immediately
discarding it. Live editor reconciliation still collects its index; source-root
clock changes retain their existing current-stage discovery. No cross-stage cache
was introduced. Two regressions cover index ownership and static/animated reloads
followed by independent root scrubbing, preserving entity identity and unrelated
live-session resources.

Paired before/after/after/before release runs at 4,096 shapes measured reloads
601.643–785.630ms before versus 432.302–508.268ms after. Smaller instrumented runs
locate the reduction in projection; validation ranges overlap. Raw evidence and
limitations are in `benchmarks/reload-animation.md`. This supports a fixture-local
CPU reload improvement, not rendering acceptance or universal speedup. Instance
edit reconciliation and clock discovery remain separate optimization work.

All 381 ordinary tests, check-all, build and whitespace checks pass; logs are
`/tmp/reload-animation-{tests,check,build,release-build}.log`. Both executables
passed all benchmark lifecycle assertions. The five optional native export tests
were not rerun for this projection-only change.

### Release lifecycle and cache baseline

Ran optimized source-root benchmarks at 128 and 1,024 native instances per root,
one/four roots, plus separate source/route profiling and direct projection cache
comparison. All commands passed their runtime assertions. Committed raw samples,
commands, host/toolchain context and limitations in
`benchmarks/source-lifecycle.md`; local logs are `/tmp/source-release-128.log`,
`/tmp/source-release-1024.log`, `/tmp/source-release-profile.log` and
`/tmp/projection-release-128.log`.

At 4,096 projected shapes, dependency reload takes 596.079–870.955ms despite
one initial mesh/material asset and preserved entities/runtime state. At 512
shapes it takes 67.673–122.524ms. Profiling still puts projection and composition
validation ahead of opening. Cached direct projection was slower in each
same-index sample; asset reduction is proven, CPU acceleration is not. High
uncontrolled host load and three samples preclude stable performance thresholds.
These are release CPU baselines, not rendering/GPU acceptance or a before/after
speedup attributable to the opacity-read change.

The preceding `40a13c9` change passes sampled opacity from shape construction to
its fallback material, avoiding a duplicate USD read without cross-frame caching.
Its existing sampled-alpha regression was extended; 379 ordinary tests,
check-all, build and whitespace checks passed (`/tmp/shape-opacity-*.log`).

### Preserve cull state during material interning

Inspection of Bevy 0.19.1 StandardMaterial found that `cull_mode` is excluded from
reflection. Reproduced a cached material mutated from back-face culling to None
being reused for a later default-material request (`/tmp/material-cull-before.log`).
Cache equality now checks cull_mode explicitly as well as reflected fields. The
regression verifies distinct handles, preservation of the application's mutated
asset and the default culling state of the newly requested material.

A compact material-key experiment removed Debug-string hashing, but its four-root
reload measurements overlapped the previous route profile: ShapesRoute application
163.671–204.919ms versus 175.786–210.888ms, and MaterialRoute 63.759–80.266ms versus
68.281–82.753ms (`/tmp/material-key-profile.log`, `/tmp/source-route-profile.log`).
No end-to-end speedup was established, and a subset key increases collision groups
for materials differing in other fields. The experimental key was reverted; only
the proven cull-state correctness fix remains. Projection optimization stays open.

All 379 ordinary tests, check-all, build and whitespace checks pass
(`/tmp/material-cull-{tests,check,build}.log`). This is material-state regression
coverage, not evidence that Spot's authored faceting or broader render fidelity
has changed.

### Opt-in source publication phase profiling

Added `asset::UsdSceneTimings`, an optional resource accumulating stage-open,
override, validation and projection/reconciliation durations plus attempt/failure
counts. The timing helper bypasses clock reads when disabled; the resource is not
installed by default. Cached/idle roots do not add attempts. Failed opens and
validation failures are counted before returning the existing projection; earlier
AssetServer load failures and texture/resource bookkeeping are outside these
phase counters.

The source benchmark exposes USD_PROFILE_SOURCES and the existing USD_PROFILE_ROUTES,
reporting and resetting each phase's counters outside its wall-clock measurement.
Tests exercise enabled/disabled measurement, expected root counts, no idle attempts
or route callbacks, and failure counts in the existing recovery regression.

Source-only debug profiling (`/tmp/source-phase-profile.log`) at four roots x 128
native instances measured reload validation 357.982–410.243ms versus projection
860.684–912.952ms; opening was 7.837–11.265ms. With route timing also enabled
(`/tmp/source-route-profile.log`), ShapesRoute application was 175.786–210.888ms,
MaterialRoute 68.281–82.753ms and VisibilityRoute 57.579–66.092ms. Projection as a
whole was 886.685–1025.285ms, leaving substantial work outside those callbacks.
These are noisy debug measurements with instrumentation overhead, not independent
CPU speedup evidence or release/rendered performance acceptance.

All 378 ordinary tests, check-all, build and the profiled 128-instance benchmark
pass (`/tmp/source-phase-{tests,check,build}.log`); whitespace checks pass.

### End-to-end source-root performance baseline

Added `examples/source_benchmark.rs` to exercise actual UsdSceneRoot publication,
including composition validation, with captured external model dependencies.
Three alternating-order samples use one/four roots with 128 native instances per
root. It measures first App update, captured dependency replacement plus App
update, and mean idle App update time over 100 iterations. Plugin startup and
input construction are outside the timers; disk I/O and rendering are absent.

Every sample verifies unique projected entities, Ready roots, a single initial
mesh/material asset, retained entity identity/runtime-only components across
reload, shared updated mesh handles, and updated cube geometry. Four roots yield
512 distinct projected meshes sharing one asset; this does not measure GPU draw
calls or CPU geometry work avoided by interning.

Current debug measurements (`/tmp/source-benchmark-128.log`):

| Roots | Load/validate/project ms | Dependency reload ms | Mean idle update us |
| --- | --- | --- | --- |
| 1 | 278.156–450.994 | 341.460–345.210 | 392.325–491.726 |
| 4 | 1120.493–1160.782 | 1238.661–1517.410 | 458.587–573.368 |

Also refreshed the separate 128-instance direct projection benchmark
(`/tmp/projection-current-baseline.log`): cached projection 264.962–273.477ms,
uncached 251.639–264.033ms, with one versus 129 initial mesh/material assets.
No CPU speedup is established. These fixtures/interfaces differ, so subtracting
their timings does not isolate validation overhead. Host load is uncontrolled;
release and rendered performance still require measurement.

Validation: 378 ordinary tests, check-all, build, the runnable 128-instance
benchmark and whitespace checks pass (`/tmp/source-benchmark-{tests,check,build}.log`).

### Reject lossy flattening of incomplete compositions

Reproduced flattened export overwriting a valid destination despite an unresolved
reference (`/tmp/flatten-validation-before.log`). EditorSession now validates the
active composition before flattening, before entering the atomic publication
path. Root/edit-layer export semantics remain authored-layer preservation; those
operations can save unresolved references for repair, while package dependency
requirements remain unchanged.

The regression verifies rejection for USDA, USDC, USD and USDZ, unchanged existing
bytes, no staging leftovers, unchanged authored layer contents and selection, and
working undo history. It also checks that root/edit USDA exports retain the
unresolved reference instead of losing it through implicit flattening. This gate
uses the existing active-composition diagnostics, not exhaustive validation of
every future sample or inactive branch.

Validation: 377 ordinary tests, five optional native tests (44 export checks),
check-all, build and whitespace checks pass. Logs:
`/tmp/flatten-validation-{tests,native,check,build}.log`.

### Viewer Open composition guard and readable failures

Reproduced the viewer command path accepting a document with a missing sublayer
as Ready (`/tmp/editor-composition-before.log`). Open now uses the composition
validator before loading textures or replacing the active session/projection.
The command-bridge regression checks missing sublayers, reference layers, root
and sub-root targets, and payload layers. Every rejected open retains document
identity, selection, the existing entity, its edited visibility and undo history;
undo still succeeds afterward.

The first actual failure capture exposed clipped/overlapping status text. The
outliner now uses the existing wrapped-status helper for multiline messages,
while short statuses retain a one-row readout. No Mara changes were made.
Inspected `target/viewer-ui-captures/wrapped-composition-error.png`: the Failed
prefix, asset path and prim target are readable. Also inspected
`target/viewer-ui-captures/spot-composition-guard.png`: the original collection
Spot opens as Ready and renders. This is a positive launch/render regression,
not a new geometry/material fidelity certification or a native picker interaction.

All 376 ordinary tests, check-all, build and whitespace checks pass; logs:
`/tmp/editor-composition-{tests,check,build}.log`. The first clipped capture is
`target/viewer-ui-captures/invalid-composition-open.png` for comparison.

### Deferred sub-root target diagnostics

Extended the recorded reference-diagnostic patch to absent external sub-root
targets. Candidates retain their grafted node and are checked only after the
composition task queue drains, so a target supplied by variant opinions remains
valid. Internal cyclic-target handling is unchanged. Native usdcat independently
reports the missing sub-root reference (`/tmp/native-missing-subroot.log`).

The core regression now checks references and payloads at both root and sub-root
depths. The actual AssetServer regression checks missing reference targets at
both depths, while a new positive case composes a sub-root supplied entirely by
an ancestor's selected variant, verifying its value and absence of diagnostics.
This extends the previous checkpoint's root-only coverage; it does not establish
all ancestral, relocation or late-variant composition combinations.
Validation: 375 ordinary tests, five native tests (44 export checks), check-all,
build and whitespace checks pass (`/tmp/subroot-validation-{tests,native,check,build}.log`).
Upstream passes 1,588 core tests, 56 binary roundtrips and strict Clippy
(`/tmp/upstream-subroot-diagnostics-{tests,clippy}.log`). All four recorded patches
pass reverse-apply checks against the vendored source.

### Composition diagnostics gate before publication

Reproduced a direct-source reload that reported Ready and replaced valid entities
despite an unresolved reference layer (`/tmp/composition-failure-before.log`).
Bevy now validates the active composition after applying root overrides and before
publishing/reconciling a new stage. The same validation serves AssetServer dependency
probing: prim traversal, default attribute reads, asset time-sample reads and
upstream composition diagnostics. A failed revision retains the previous live
stage/entities; a failed first load publishes no live instance. Recovery is tested.

An existing-layer/missing-prim regression exposed another upstream gap: absent
external root-prim targets emitted diagnostics for payloads but not references.
Native usdcat reports the reference error (`/tmp/native-missing-reference.log`).
The fourth recorded patch, `openusd-reference-diagnostics.patch`, adds the missing
reference diagnostic without changing node culling or composition. Core coverage
checks both arc types, and a real memory AssetServer test verifies visible failure.
An unselected variant's unresolved branch is accepted; selecting it fails validation
and returning to the valid branch clears the error.

Validation runs on source/override publication, not every frame. It is not a full
USD correctness validator: sub-root reference target diagnostics remain incomplete
upstream, arbitrary future numeric samples are not exhaustively evaluated, and
renderer-specific asset/geometry errors are separate. `UsdSource::open_stage`
retains upstream's permissive partial-stage semantics for direct callers.
Validation: 374 ordinary tests and all five optional native tests (44 export
checks) pass, as do check-all, build and the composed_sources example. Logs:
`/tmp/composition-validation-{tests,native,check,build,example}.log`. Upstream
passes 1,588 core tests, 56 binary roundtrips and strict Clippy
(`/tmp/upstream-reference-diagnostics-{tests,clippy}.log`). All four recorded
patches pass reverse-apply checks; whitespace checks pass.

### Composed-source lifecycle showcase

Extended `examples/composed_sources.rs` through captured dependency replacement,
root-local persistent radius overrides, malformed-root failure and recovery,
override removal, root-component removal and root despawn. Matching entities and
a runtime-only component survive replacement and recovery. Unmodified mounts
adopt the new typed source radius and share a mesh; removing the persistent
override restores the new source radius and shared mesh on that mount too.

Both roots report Failed for malformed root bytes while retaining their previous
live projections, then return to Ready when valid captured sources are restored.
Removing one UsdSceneRoot leaves its caller-owned root entity and the second
instance alive; despawning the second root leaves no projected UsdPrimRef entities
or registered live instances. No runtime implementation change was required.

Validation: all 371 ordinary tests, check-all, build, the runnable example and
whitespace checks pass (`/tmp/composed-lifecycle-{tests,check,build,example}.log`).
This is programmatic source replacement, not an AssetServer watcher or GUI test;
the failure case is malformed root data, not every possible composition failure.
It does not prove GPU allocation reclamation or rendered fidelity.

### Public immutable source composition

Exposed `UsdSource::snapshot` with filesystem fallback disabled and added
`with_dependency`, which returns an immutable merged source containing another
source's root and transitive captured bytes. Existing identifiers require identical
bytes; collisions fail without changing either input. Duplicate merges preserve
the source revision, while new captured assets generate a new revision. The root's
filesystem policy is retained, not inherited from dependencies.

Regression coverage composes a nested sublayer and parent-relative opaque asset
at two reference sites with no source files, then packages and reopens the assembly
from bytes after removing its saved archive. It checks transitive byte capture,
deduplication, collisions (including normalized aliases and root collisions),
receiver filesystem policy and immutable inputs. Fixed snapshot construction to
anchor relative paths before normalization so leading `..` is not discarded.

The runnable `composed_sources` example combines an inline assembly with canonical
upstream typed Sphere authoring. Two Bevy roots project distinct entities with
shared meshes; changing one mount's radius preserves its entity and leaves the
other mount/root unchanged. Typeless reference-site definitions preserve the
referenced Sphere type; a local Xform type would instead override it.

This is reusable source composition, not a complete BSN-style typed scene DSL or
automatic capture of filesystem dependencies. The broader checklist stays open.
Validation: 371 ordinary tests, five optional native tests (44 export checks),
check-all, build, the composed_sources runnable example and whitespace checks
pass. Logs: `/tmp/source-composition-{tests,native,check,build,example}.log`.

### Variant payload and cyclic asset package acceptance

Added a fixture with two variant-selected payload layers, distinct binary assets,
and asset-valued backlinks forming a two-layer cycle. An ordinary regression
checks that saving with the selected payload unloaded includes both branches,
exact asset bytes and only five archive entries, without changing the root layer.

Ten new native checks export root/edit layers with payloads loaded and unloaded,
plus the explicitly flattened loaded scene. After deleting the source directory,
fresh native usdcat processes select both variants in each authored-layer output;
the flattened output retains its chosen branch even with a different variant
selection override. Composed scores,
asset bytes, backlink layer values and deduplicated archive counts are verified.
This proves an asset-dependency cycle, not a cyclic composition arc. The original
compact fixture syntax was rejected by native USD and was corrected before the
acceptance run; Rust parsing alone was insufficient fixture validation.

Validation: 366 ordinary tests, five optional native tests (44 export checks),
check-all, build and whitespace checks pass. Logs:
`/tmp/package-variants-{tests,native,check,build}.log`. No runtime implementation
change was needed. Nested packages, patterns, broader composition combinations
and rendered texture fidelity remain open.

### Viewer save-format acceptance

The native save chooser exposes USDA, binary USD and USDZ output, with distinct
root/edit/flattened default filenames. Filename validation retains the selected
path and stale-document checks; uppercase `.USDZ` uses the dependency packager.
All 365 ordinary tests, check-all and build pass
(`/tmp/save-format-{tests,check,build}.log`). The four optional native tests pass
again (`/tmp/save-formats-native.log`), covering 34 export checks.

Inspected isolated GTK captures for all three modes:
`target/viewer-ui-captures/save-formats-root.png`, `save-formats-edit.png`, and
`save-formats.png` (flattened). Each shows the correct default filename and a
readable filter listing all four extensions. The old root-picker replay actually
clicked flattened export; corrected its coordinates and added explicit edit and
flattened replays. These checks prove chooser presentation, not native file
selection, cancellation, overwrite confirmation or save/reopen interaction.
GTK does not display the requested explanatory dialog title in this layout.

### Single-level package re-export checkpoint

Added single-level USDZ input/re-export support to the stage-aware packager.
Package-relative layer and asset paths resolve through the existing stage context;
source containers are cached, while container bytes and extracted entry bytes
both count against the input budget. Entry sizes and bounded reads constrain
extraction. Bare-package references map to their default root, and bare/default
root identities share an output entry. Input extensions are case-insensitive.
Genuinely nested packages still return explicit errors.

Native coverage adds seven save checks: all three modes from a never-written
snapshot-only USDZ, a second re-export for each after deleting the first output,
and a wrapper referring to the same bare package twice. Composed values and asset
bytes survive, and the wrapper has the expected deduplicated four entries.
An uppercase input USDZ extension is covered. A new upstream unit test verifies
that an archive entry exceeding the remaining input budget is refused and nested
entry syntax is rejected. Logs: `/tmp/repackage-case-native.log`,
`/tmp/upstream-repackage-final-tests.log`, `/tmp/upstream-repackage-final-clippy.log`.

Validation: 34 native save checks pass across four optional tests. All 364
ordinary tests pass with those four native tests ignored; check-all, build and
whitespace checks pass (`/tmp/repackage-accepted-{tests,check,build}.log`). Upstream
validation passes 1,587 core tests, 56 binary fixture roundtrips and strict core
Clippy. All three recorded patches pass reverse-apply checks against the vendor
source. The wider packaging and Bevy acceptance checklists remain open.

### Initial packager checkpoint

Implemented the initial stage-aware USDZ dependency packager. The new upstream
Stage::write_usdz_package uses the stage's existing registry and live layer graph,
preserves root/edit semantics, rewrites authored asset locations and includes
ordinary layer and non-layer dependencies. Bevy's USDZ persistence routes it
through existing atomic staging/publication. Missing dependencies and entry/byte
budget failures report errors before replacing the destination. Limits are 4096
entries, 256 MiB of serialized payloads and 256 MiB of newly read input bytes;
these are not total process-memory limits. Unsupported package-relative inputs,
expressions, tile/sequence patterns and clip templates fail explicitly.

Native portability now passes all three save modes after source deletion,
including verifying asset payload bytes reached through native package-relative
paths. Added ordinary tests for snapshot-only dependencies, live unsaved sublayer
edits, deterministic output, repeated paths, a root self asset cycle, same-basename
assets, stored/aligned archive entries, unchanged source/undo behavior, missing
asset cleanup and entry-budget rejection. The limited serializer has an upstream
wire-buffer growth test. PACKAGING.md keeps the full remaining requirements.

The changes are recorded in `patches/openusd-stage-packaging.patch` in addition
to the existing writer patches. usd_bevy and usd_macro now reference the vendor
crates directly: external path consumers do not inherit a workspace root's Cargo
patch table, so a root-only override is insufficient for this new API dependency.
The root viewer Git declarations still record the upstream baseline.

Validation: 27 native save checks across three optional tests pass, including
removal of source files and asset-payload verification. All 364 ordinary tests
pass (three native tests ignored), check-all and build pass. Logs:
`/tmp/package-accepted-native.log`,
`/tmp/package-consumer-final-{tests,check,build}.log`. Upstream full-checkout
validation: 1,586 core tests and 56 binary fixture roundtrips pass; strict core
Clippy passes (`/tmp/upstream-package-tests.log`, `/tmp/upstream-package-clippy.log`).
An independent path consumer compiles and resolves one local OpenUSD package
without a root patch table (`/tmp/package-external-consumer.log`,
`/tmp/package-external-tree.log`). All three recorded patches pass reverse-apply
checks against the integrated source. The full packaging requirements remain
open where PACKAGING.md names unsupported inputs or missing acceptance evidence.

### Earlier packaging checkpoints

Added a native USDZ portability regression: save into a separate directory,
close/delete the owned source directory, then decode with fresh native processes.
Root/edit packages lose sublayer and referenced values; flattened preserves
these geometry-only values. The two prior fixtures still pass. The native gate
is now deliberately failing on this real acceptance gap, not claimed green.
Evidence: `/tmp/native-portable-package.log`. `PACKAGING.md` records the
source-grounded implementation boundary and required coverage: resolver-aware
dependency traversal above ArchiveWriter, live layer snapshots, preserved save
semantics, deterministic collision-free paths, bounds and atomic publication.
No dependency bundling is implemented yet; flattened texture portability and
non-package cross-directory Save As remain open too.

The ordinary suite passes 361 tests with three optional native tests ignored;
check-all, build and git diff --check pass. Logs:
`/tmp/native-portability-{tests,check,build}.log`. The separate native gate fails
the moved-package test as above. Next implementation priority is the dependency
packager in PACKAGING.md, not weakening that regression or flattening root/edit
saves implicitly.

Extended native persistence coverage to sublayers and external references in all
12 root/edit/flattened x usda/usdc/usd/usdz combinations. Before the fix, native
composition lost the sublayer in root/edit binary exports. Corrected the vendored
writer to emit subLayers as non-array StringVector; ordinary string arrays are
unchanged. Updated the binary patch artifact and provenance. Wire regressions
cover 0/1/3 items. Upstream validation passes 1,585 core tests and 56 binary fixture
roundtrips; the expanded native gate passes both fixtures, totaling 24 save checks.
Logs: `/tmp/native-layered-gate.log` (failure before fix),
`/tmp/native-layered-packages.log` (pass after), `/tmp/upstream-sublayer-tests.log`.
These exports remain beside their external dependencies: portable packaging and
cross-directory Save As have not been certified by these tests.

Repository validation: 361 ordinary tests pass with two optional native tests
ignored; check-all, build and whitespace checks pass. Logs:
`/tmp/native-layered-{tests,check,build}.log`. Both recorded patch files pass
reverse-apply checks against the integrated source.

Integrated both writer patches through the root Cargo patch table and the new
`vendor/openusd` snapshot (389 files, approximately 7.5 MiB on disk). The base is
still upstream HEAD b7df5ad, rechecked on 2026-09-10. All three packages resolve
to repository-local paths; no temporary override, sibling checkout modification,
existing xtra replacement or build-time patching is required. The vendored
crates differ from upstream only in the two writer files, and both patch files
pass reverse-apply checks against the integrated snapshot. Licenses, provenance
and switch-back instructions are in `vendor/openusd/VENDORED.md`.

The normal make test-native passes all 12 combinations including native variant
composition. Normal test-all passes 361 tests with one optional native test
ignored; check-all and build pass. Logs:
`/tmp/bevy-integrated-{native,tests,check,build}.log`. The integrated scene_report
exports Spot text accepted by native usdcat:
`target/spot-integrated-review.usda`, `/tmp/spot-integrated-export.log`,
`/tmp/spot-integrated-native.usda`. This resolves integration of the diagnosed
writer fixes, not the full Bevy checklist or arbitrary save/render parity.

Native Embree also rendered that integrated export to
`target/spot-integrated-embree.png`, inspected directly. The robot is visible
and faceted; Material prims and GPU-disabled color correction remain unsupported
by this reference renderer. Log: `/tmp/spot-integrated-embree.log`. No original
collection asset was changed, and this is not a matched Bevy/native image test.

### Earlier interoperability checkpoints

Expanded and preserved the binary fix as
`patches/openusd-token-vector-fields.patch`: all eight native token-vector
structural/order fields use non-array TokenVector values, and variantSetNames
token list operations emit native StringListOp values. Wire tests cover vector
sizes 0/1/3, ordinary arrays and six variant-set operations. Both patches together
pass 1,584 upstream core tests and 56 binary fixture roundtrips. The native editor
gate now uses `assets/native_save.usda` and native flattening to check variant
composition as well as hierarchy/values/API metadata. All 12 combinations pass
with temporary Cargo overrides pointing at the patched isolated copy. Logs:
`/tmp/upstream-vector-tests.log`, `/tmp/upstream-vector-roundtrip-final.log`,
`/tmp/bevy-native-patched-variants.log`. Permanent integration remains pending;
ordinary pinned dependencies are not fixed by committing patch artifacts.

The patched Bevy workspace also passes all 361 ordinary tests (one optional
native test ignored), check-all and build; logs:
`/tmp/bevy-patched-{all-tests,check,build}.log`. Restored the normal Git dependency
resolution and verified Cargo.lock has no diff. The unchanged pin fails the
expanded native gate as expected; the normal viewer build passes
(`/tmp/bevy-native-restored-pin.log`, `/tmp/bevy-restored-build.log`). Both patch
files pass git apply --check against the pinned upstream checkout.

Added optional `make test-native` exercising the real editor persistence path
for RootLayer/EditLayer/Flattened and usda/usdc/usd/usdz. All 12 combinations
currently fail native interchange checks: USDA metadata syntax errors (a distinct
array-shape error for flattened output) and missing hierarchy/values/API metadata
for binary/package outputs. Source control parses natively. Log:
`/tmp/bevy-native-export-tests.log`. Normal tests explicitly ignore this native
tool requirement; they do not certify interchange.

Binary failure now reproduced independently of Spot with single_api_schema.usda.
An experimental change in `/tmp/openusd-native-patch-b7df/crates/openusd/src/usdc/writer.rs`
emits primChildren/properties as non-array TokenVector rather than Token arrays;
native usdcat then sees the minimal USDC prim and its API metadata. Evidence:
`/tmp/upstream-vector-convert.log`, `/tmp/single-api-vector-native.usda`.
This temporary change is not yet a reviewed patch artifact, does not cover all
token-vector metadata, and is not integrated. Next: expand binary regression
coverage, correct flattened metadata, and validate the actual dependency with
the native gate before claiming save interoperability.

Validation for the native-gate addition: all 361 ordinary tests pass, with one
explicitly ignored native test; check-all, build and git diff --check pass.
Logs: `/tmp/bevy-native-gate-{tests,check,build}.log`. The separate native gate
fails as described above, and that failure remains an acceptance blocker.

Prepared `patches/openusd-singleton-listops.patch` against the pinned upstream
revision in an isolated temporary copy; no sibling or Cargo cache sources were
modified. All 1,582 upstream core library tests pass, including a new six-operation
singleton metadata regression. Native usdcat now accepts patched conversions of
the minimal fixture and Spot (919 lines of native text output). Native Embree
renders the converted Spot visibly, with faceting, in
`target/spot-patched-embree.png`, inspected directly. Logs:
`/tmp/upstream-listop-all-tests.log`, `/tmp/spot-patched-convert.log`,
`/tmp/spot-patched-embree.log`. Embree does not support the scene's Material prims;
camera/lighting are unmatched. This is not pixel/material parity or proof that
the Rust reader interprets the original binary correctly. Patch integration and
the original binary's native-read discrepancy remain open. The active viewer
dependency is still unpatched; see `patches/README.md` for reproduction.

Returned to Spot seam diagnosis. Extended scene_report with authored-normal
preservation, face/normal alignment, duplicate/degenerate triangles and indexed
versus exact-position-welded boundary/nonmanifold edges. Tests cover reversed
normals, duplicates, degeneracy and a split-index quad. Current Spot data:
26 traversed meshes preserve authored vertex normals exactly; none have reversed
normal corners or exact duplicate/degenerate triangles. Minimum face-normal dot
is at least 0.999785, supporting source-authored flat faceting. Body geometry has
55 exact-position boundary edges and one nonmanifold edge after welding versus
3,368 indexed boundaries. These are diagnostic counts, not proof of defective
source geometry or attribution of every visible seam. Log:
`/tmp/spot-surface-edges.log`. Source assets and runtime normals were not modified.

Found native usdrecord/usdcat (OpenUSD 25.05.01) and added optional Makefile
capture-reference; default make still builds usdview. CPU Embree renders the
cube fixture, but the original Spot USDC renders black because native usdcat
sees only 11 lines of root metadata, unlike the Rust reader. All-purpose capture
also stays black. Offscreen Storm segfaulted. Diagnostic root-layer text export
then exposed invalid singleton apiSchemas serialization in the pinned Rust
writer. Minimal valid source `assets/single_api_schema.usda` passes native usdcat;
its Rust text export fails native parsing with status 1. Detailed evidence and
limitations are in OPENUSD_UPGRADE.md. Native interoperability is not fixed and
self-reopen tests do not prove it. Next priority: isolate/correct the native
export compatibility failure before claiming save/interchange acceptance.

All 361 repository tests, check-all, build and whitespace checks pass
(`/tmp/usd-native-audit-{tests,check,build}.log`), separately from the expected
native interoperability failures. Spot visual fidelity remains unresolved.

Curve color/opacity interpolation now uses f64 scalar/vector intermediates and
f64 basis weights. Generated RGBA is checked after conversion to f32; overflow
reports UsdCurveError and suppresses geometry before GPU upload. A regression
keeps pinned all-f32::MAX color controls finite, rejects true Catmull-Rom
overshoot beyond f32 range and verifies recovery after correction. Geometry
positions remain f32 with their existing finite-output guard.

All 360 tests, check-all, build and whitespace checks pass
(`/tmp/usd-curve-color-finite-{tests,check,build}.log`). Inspected the repeated GPU
gradient capture `target/curve-gradients-finite.png`; raw RGB comparison against
the earlier capture finds zero changed pixels out of 921,600 at tolerance zero
(`/tmp/usd-curve-color-finite-compare.log`). This verifies preservation of that
fixture, not general reference parity or performance of widened interpolation.
Widths, adaptive tessellation and broader acceptance remain open.

Curve validation now preflights total tessellated output across each prim's
curves, with checked arithmetic and inclusive limits of 1,000,000 vertices and
2,000,000 line indices. It derives linear/cubic, periodic/pinned segment counts
before allocating generated geometry; over-budget input reports UsdCurveError
through the existing cleanup/recovery and capture failure paths.

Tests cover exact limits, one-over-limit cases, usize arithmetic overflow, and
a 125,003-control-point Bspline whose output would exceed one million vertices.
That projection creates no mesh asset and recovers after replacing the source
with four points, preserving a runtime child. All 359 tests, check-all, build
and whitespace checks pass (`/tmp/usd-curve-budget-{tests,check,build}.log`).
This bounds generated curve buffers, not input decoding, total prim count,
asset-cache overhead or process/GPU memory. No new GPU acceptance or performance
benchmark was performed; width-aware surfaces and broader fidelity remain open.

Curve display primvars now validate sample cardinality by interpolation, negative
and out-of-range indices, unsupported faceVarying interpolation and non-finite
authored color/opacity values. Constant/uniform/vertex/varying use their distinct
sample counts, including cubic segment-boundary varying counts and periodic
wrap. Single unindexed values retain the existing broadcast behavior. Invalid
data reports UsdCurveError instead of silently substituting fallback colors.

Tests cover interpolation cardinalities, valid indexed palettes, malformed index
arrays, broadcast, non-finite opacity and live recovery preserving entity/runtime
children. All 357 tests, check-all, build and whitespace checks pass
(`/tmp/usd-curve-primvar-validation-{tests,check,build}.log`). Negative GPU fixture
`assets/invalid_curve_colors.usda` exits with `displayColor has 2 samples; expected
4` and writes no PNG (`/tmp/usd-invalid-curve-colors-capture.log`). Width/normal
primvars, output budgets, finite interpolated color arithmetic and broader
reference fidelity remain open. This supersedes the display-cardinality gap.

Curve validation now rejects unknown type/wrap/cubic-basis tokens and unsupported
segment layouts instead of dropping unused control points. Nonperiodic Bezier
requires 4+3n controls; periodic Bezier requires 3n with at least three controls;
nonperiodic Bspline/Catmull-Rom require four, pinned require two. Periodic cubic
counts below three remain explicitly unsupported; zero-count entries are empty.
Linear basis tokens are ignored. The reader fallback now names the schema's
Bezier default rather than Bspline. Older tests were corrected to author linear
type/Bspline basis explicitly instead of depending on malformed cubic fallback.

The layout matrix covers valid/invalid strides, pinned minimum counts, periodic
three-point layouts and unknown tokens. All 355 tests, check-all, build and
whitespace checks pass (`/tmp/usd-curve-layout-{tests,check,build}.log`). Negative
GPU fixture `assets/invalid_curve_layout.usda` exits nonzero with a five-control
Bezier layout error and writes no PNG (`/tmp/usd-curve-layout-capture.log`).
Primvar cardinality, output budgets, width-aware surfaces and full reference
fidelity remain open; this supersedes the preceding basis/stride limitation.

Added UsdCurveError validation for negative/overflowing counts, authored count
totals that differ from the point count, invalid count value types and non-finite
input/tessellated positions. Invalid projection clears owned geometry instead of
silently clamping the shape; corrected data recovers, type changes clear errors,
and entity/runtime children remain intact. Missing counts still infer one curve;
full cubic stride/token and primvar-cardinality validation remain open.

The editor bridge publishes/clears curve diagnostics with other rendering
issues. Standalone capture rejects these errors. Adding a separate curve query
exceeded the capture system parameter limit; combined shape/curve diagnostic
queries without changing the existing shape failure behavior. Regression tests
cover malformed counts, non-finite points, recovery and editor issue lifecycle.
All 354 tests, check-all, build and whitespace checks pass
(`/tmp/usd-curve-validation-final-{tests,check,build}.log`).

Negative GPU fixture `assets/invalid_curves.usda` fails with
`curve /Broken: curveVertexCounts sum 4 differs from 2 points`; verified nonzero
capture exit and no `target/invalid-curves.png` output. Log:
`/tmp/usd-invalid-curves-final-capture.log`. New inspector visual acceptance was
not performed; its bridge state is tested. Validation currently rereads point
arrays before tessellation; performance has not been benchmarked.

Curve fallback materials now choose alpha mode from generated vertex colors,
not only authored opacity controls. Added a defensive Catmull-Rom regression:
controls [2,1,1,2] produce midpoint alpha 0.875 and require blending; resampling
to [1,1,1,1] restores opaque mode and scrubbing back restores blending. The test
uses out-of-range authored opacity deliberately; this is not a claim that such
inputs conform to USD's opacity range or a new input-clamping policy.
The change applies to unbound fallback materials, not bound shader behavior.
All 353 tests, check-all, build and whitespace checks pass. Validation logs:
`/tmp/usd-curve-alpha-{tests,check,build}.log`. No new rendered
acceptance was performed for this defensive case; broader fidelity remains open.

Extended curve display color/opacity to indexed vertex and varying interpolation.
CurveSampling tracks independent control-point and varying offsets per batch:
cubic vertex values use basis weights and pinned phantom endpoint expansion;
varying values interpolate between segment samples with per-curve periodic wrap.
Linear curves address per-point values. Constant/uniform/single-value handling
remains supported. Missing/invalid indices use the channel fallback.

Tests distinguish indexed vertex/varying gradients across two Bezier curves,
assert analytic midpoint colors and independent varying opacity, compare vertex
sampling against geometric tessellation across all cubic bases and wraps, and
verify indexed periodic varying closure with nonzero batch offsets. Inspected
`target/curve-gradients.png`: upper vertex curves pass through green/yellow,
lower varying curves interpolate red/purple/blue, with matching geometry and
opacity fades. All 352 tests pass; verification logs are
`/tmp/usd-curve-gradients-{tests,check,build}.log`.

Widths, adaptive tessellation, malformed topology diagnostics, broader animated
multi-instance gradient image acceptance and full reference-render parity remain
open. This supersedes the preceding vertex/varying display-primvar limitation.

Curves now project inherited/local constant and indexed uniform display color
and opacity, broadcasting each curve's sample over its generated vertices.
Single unindexed values also broadcast. The geometry builder returns optional
RGBA alongside positions/indices; curve-index lookup stays separate from the
tessellated vertex index. `assets/curve_colors.usda` covers inherited style on
two lines and sampled uniform indices on two cubic curves. Plugin tests verify
time scrubbing, alpha modes, parent index edits, local override/clear and stable
entities/runtime children. Multi-value vertex/varying interpolation is still
unimplemented and is not approximated as constant.

Inspected `target/curve-colors-{0,10}.png`: inherited lines change translucent
red to opaque green, and cubic curves exchange yellow/blue while retaining their
independent opacity. All 349 tests, check-all, build and whitespace checks pass
(`/tmp/usd-curve-colors-{tests,check,build}.log`). Width-aware rendering, remaining
primvar interpolation, strict topology validation and broader fidelity remain
open; this is not full curve-renderer acceptance.

Fixed the three-control-point periodic cubic path: wrapping supplies the fourth
Bezier control point instead of falling back to a triangular polyline. Added
`assets/periodic_bezier.usda` and a regression comparing its sampled positions
and indices exactly against an explicitly closed four-point nonperiodic curve,
including endpoint closure and the analytic midpoint.

Inspected `target/periodic-bezier.png`: the periodic and explicit curves produce
matching closed eight-segment loops. Their visible faceting is the existing
fixed tessellation limit, not the discarded triangular control hull. Full tests
pass (348); gate logs are `/tmp/usd-short-periodic-{tests,check,build}.log`.
Adaptive tessellation, strict malformed-topology handling and the broader
acceptance checklist remain open.

Completed point-cloud display-primvar validation after the fixture correction in
ba65438. All 347 tests, check-all, build and whitespace checks pass
(`/tmp/usd-point-colors-final-{tests,check,build}.log`). The regression checks
independent clocks, inherited constant and local indexed vertex/varying RGBA,
parent edits, local override/clear, stable identity/runtime child and alpha mode.

Inspected `target/point-colors-fixed-{0,10}.png` and tightly packed GPU pixels.
At y=360, x=391/515 change from RGB(88,40,47) to (50,144,47), matching translucent
red to opaque green. x=764 changes (81,82,53) to (29,41,92); x=888 changes
(27,43,119) to (108,108,56), matching the independently indexed yellow/blue swap.
These are antialiased, composited one-pixel points, not direct material values.
README/SUPPORT now describe point-color support. Curve display primvars, widths,
bound-material fidelity and the full project acceptance checklist remain open.

User-requested checkpoint committed as 788e3e5
(`feat(bevy):expand USD integration and viewer`), unsigned and title-only.
The pending point-color regression and GPU capture exposed invalid USDA syntax:
interpolation metadata followed a timeSamples assignment. Split it into an
attribute declaration plus timeSamples assignment. Point-color validation is
still in progress; the failed capture did not establish visual acceptance.

User inspection handoff: launched `assets/animation_showcase.usda` through make
on the active desktop `wayland-0`; the inherited WAYLAND_DISPLAY referenced a
stale waypipe socket and failed with NoCompositor. Successful launch log:
`/tmp/usd-user-viewer-desktop.log`, exec session 47655 (left running). No commits
were made during this work; existing HEAD is 743989c. Embedded SSAO device-limit
warning persists.

In-progress point-cloud display primvars: generalized the canonical mesh helper
to `mesh::build_vertex_colors` taking color/opacity primvars directly and wired
PointsRoute to its sampled inherited readers. Added `assets/point_colors.usda`
with inherited constant and local indexed vertex/varying samples. The current
viewer build succeeds, but regression tests, GPU color acceptance and support
documentation for this change remain to be completed. User requested the launch
before that validation was added; do not count the point-color change complete.

Implemented pinned cubic Bspline/Catmull-Rom endpoint expansion using per-curve
phantom points `2*first-second` and `2*last-penultimate`. The existing tessellator
then evaluates the expanded nonperiodic curve; Bezier/linear pinned behavior
remains nonperiodic. Intermediate extrapolation uses f64 before storing f32.
Tests compare decoded pinned fixtures against explicitly authored phantom-point
controls, assert endpoint positions and segment counts, change wrap, and check
two-point interpolation plus disjoint batches for both bases.

Inspected `target/pinned-curves.png` from `assets/pinned_curves.usda`: pinned
curves on the left match expanded controls on the right in each row (Bspline
above Catmull-Rom), including endpoint heights. All 346 tests, check-all, build
and whitespace checks pass (`/tmp/usd-pinned-curves-{tests,check,build}.log`).
The endpoint rule is documented at
https://openusd.org/dev/api/class_usd_geom_basis_curves.html . Fixed eight-step
tessellation, missing widths/display primvars, malformed topology handling and
full reference-render fidelity remain open. This supersedes the pinned-endpoint
limitation in the preceding checkpoint.

Fixed periodic linear BasisCurves dropping the last-to-first edge. Each curve
now closes within its own vertex range, including the short-cubic polyline
fallback. Added `assets/periodic_curves.usda` with two loops in one prim and an
open control. Regression tests assert exact closed/open batch indices and wrap
edits; additional tests verify periodic Bezier/Bspline/Catmull-Rom endpoint
closure and disjoint indices across two tessellated batches.

Inspected `target/periodic-curves.png`: both left squares close and the right
control lacks its left edge. Raw center-row probes find the five expected
vertical sides near x=329,454,578,702,950, with no control edge at x=826.
All 344 tests, check-all, build and whitespace checks pass
(`/tmp/usd-periodic-curves-{tests,check,build}.log`). The implementation follows
https://openusd.org/dev/api/class_usd_geom_basis_curves.html periodic segment
rules. Pinned cubic phantom endpoints, strict topology validation and width-aware
surface rendering remain unimplemented; the support matrix now states these gaps.

Fixed Points/BasisCurves projection reading default geometry despite animation
discovery. Positions, curve counts and curve tokens now resolve at RouteCtx.time.
`assets/point_curve_animation.usda` contains sample-only positions and changing
curve topology. A plugin test verifies two independent root clocks at 0/5/10,
interpolated positions, held integer topology, stable entities/runtime children
and the unlit fallback material. All 342 tests, check-all, build and whitespace
checks pass (`/tmp/usd-point-curve-final-{tests,check,build}.log`).

Initial GPU captures exposed barely visible normal-less geometry under the lit
fallback. Points/curves now use unlit unbound preview materials; bound materials
still follow MaterialRoute. Inspected `target/point-curve-unlit-{0,10}.png`:
four point marks rise, and one continuous line becomes two raised segments.
Raw RGB >150 probes find 496 bright line pixels at row 484 (time 0), versus
246 at row 235 (time 10). Grid height also follows sampled bounds. This remains
one-pixel point/line preview, not width-aware USD surfaces or full curve shading;
display primvar projection and bound-material fidelity remain open.

Added a deterministic textured GPU fixture: `examples/uv_fixture.rs` writes
`assets/inherited_uv.usda` and a generated 2x2 quadrant PNG into a new directory,
refusing to overwrite an existing directory. Its source-preserving load test
checks inherited indexed UVs at time 10. All 341 tests, check-all, build and
whitespace checks pass (`/tmp/usd-uv-fixture-{tests,check,build}.log`).

Inspected `target/inherited-uv-{0,10}.png` after successful GPU readback with
shadows disabled. Left inherited mesh, center point-instanced prototype and right
explicit-UV control all change from blue at time 0 to green at time 10. Raw RGBA
probes at (454,360), (640,360), (826,360) are respectively (37,67,217,255),
(38,67,218,255), (38,67,218,255) at 0 and (76,201,61,255), (77,201,60,255),
(77,201,60,255) at 10. This confirms this constant-indexed texture sampling case
and V orientation, not whole-image parity, general shader networks or normal maps.

Extended constant primvar inheritance to mesh st/st0 UVs, including owner-local
indices, animation discovery and parent-edit invalidation. Canonical st wins over
st0 even when inherited. Fixed constant indexed UV lookup in both corner/flat
sampling and indexed mesh construction (the latter previously emitted zero UVs
for constant interpolation). Single-value corner UVs no longer bypass indices.

Regression tests cover both names, independent roots, direct mesh prototypes,
sampled parent indices, live parent edits, local override/clear, stable entities
and runtime children, plus flat/indexed output and material subsets. They check
the actual Mesh UV_0 values after the V flip. All 340 tests, check-all, build and
whitespace checks pass (`/tmp/usd-inherited-uv-{tests,check,build}.log`). The
textured GPU follow-up is recorded above; broader texture/normal-map fidelity
remains unproven. Constant UVs lack a usable normal-map tangent basis.
Arbitrary per-texture UV sets and shader primvar-reader networks remain open.

Extended the inherited primvar owner resolver from normals to mesh/primitive
display color and opacity. Values/interpolation/indices come from the same
nearest authored nonblocked constant ancestor, unless a local value wins.
Animation discovery follows those owners; parent value/indices/interpolation
edits invalidate consumers. Normal handling uses the same canonical resolver.
Independent-root tests now exercise both local and inherited style sources for
ordinary mesh/cube and point-instanced cubes.

An edit regression exposed ignored indices on constant mesh colors/opacities.
Fixed both flat/corner sampling and indexed output to resolve index slot zero;
single-value broadcasting no longer skips an authored index in those paths.
Tests cover parent indexed edits, alpha-mode updates, local override/clear,
nonconstant ancestor rejection, stable entities/runtime children and both mesh
vertex layouts/material subsets. All 338 tests, check-all, build and whitespace
checks pass (`/tmp/usd-inherited-display-complete-{tests,check,build}.log`).

Added inherited_display.usda and inspected successful GPU captures
`target/inherited-display-{0,10}.png`: inherited translucent red changes to opaque
blue across mesh, cube and point prototype while the backdrop's local style wins.
OpenUSD PrimvarsAPI documentation confirms inherited values must be authored,
nonblocked and constant. UV inheritance, curve/point display inheritance and
general shader primvar-reader networks remain open; these checks are not full
reference-render parity or a new performance claim.

Implemented perspective aperture offsets via public UsdPerspectiveProjection
(Projection::Custom). Matrix, sub-view projection and frustum corners share the
same aperture-offset/focal-length slopes. Resizing preserves vertical FOV;
unshifted cameras retain ordinary PerspectiveProjection. Tests cover shifted
center rays, near/far corners, landscape/portrait resize, center crop and sampled
USD lens/projection changes. General film-fit, oblique planes and lens effects
are not claimed. OpenUSD GfCamera docs supplied aperture-unit semantics.

Standalone capture now accepts USD_CAPTURE_CAMERA, waits for that projected
camera, and copies its transform/projection into the offscreen camera. The path
is recorded in metadata; eye/target arguments are inactive in that mode.
Source camera activation remains application-owned. Added camera_offsets.usda;
both GPU captures completed and were inspected:
`target/camera-offset-{0,10}.png`. A cyan-pixel threshold found 65536 pixels in
both images, bounds (512,232)-(767,487) moving to (152,340)-(407,595): exactly
360px left and 108px down, with unchanged size. Tests also cover missing-camera
readiness and custom-projection selection. All 336 tests, check-all, build and
whitespace checks pass (`/tmp/usd-camera-offset-final-{tests,check,build}.log`).
Sheared camera transforms and full reference-render parity remain open.

Reduced animation-discovery work using upstream composed authored-attribute
enumeration instead of schema-inclusive attributes, and a combined skeleton
binding/animation query for skin and blend variability. No persistent memoization
was added; dependency discovery and edit/reload invalidation remain active.
Regression tests compare old/new scans for schema-only static prims, sampled
schema/custom attributes, native proxies and source edits, plus combined versus
individual deformation queries over skin/morph/static fixtures.

All 334 tests, check-all, build and whitespace checks pass
(`/tmp/usd-animation-scan-final-{tests,check,build}.log`). The final unprofiled
128-native-instance debug benchmark measured cached projection 150.3-150.7ms and
edit 124.0-129.3ms, versus the preceding cache-budget run's 195.0-239.8ms and
165.5-167.5ms. Asset counts/edited geometry/entity checks still pass. This is
local debug evidence, not a release/GPU latency guarantee. Before/after profiled
runs also completed; profiling and concurrent compilation can perturb timings.
Logs: `/tmp/usd-projection-current-profile.log`,
`/tmp/usd-animation-scan-final-{benchmark,profile}.log`.

Added a byte budget to ProjectionCache alongside its 8192-entry cap. Default is
256 MiB of insertion-time attribute/index/morph/name payload. Applications can
use `ProjectionCache::with_byte_budget`; zero disables retention, oversized
meshes bypass it, and either limit releases cached handles without removing live
assets. `retained_payload_bytes` exposes accounting. This excludes allocator/GPU
overhead and external mutations of shared cached assets; it is not a total scene
memory guarantee. Interned assets should remain immutable; edits intern new meshes.

Tests cover exact-budget hits, sharing, eviction/reset, live asset preservation,
oversized bypass, zero budget and index/attribute/morph/name accounting. All 332
tests, check-all, build and whitespace checks pass
(`/tmp/usd-cache-budget-{all-tests,check,build}.log`). The 128-native-instance
headless debug benchmark retained one initial mesh/material versus 129 uncached,
and reported 1824 accounted cached payload bytes after editing, in all 3 samples.
Projection timings overlap (cached about 195-240ms, uncached 190-201ms): this is
retention/sharing evidence, not a speedup claim. Benchmark now reports that byte
counter (`/tmp/usd-cache-budget-benchmark.log`); broader performance work remains.

Connected unbound display-opacity fallback to Bevy alpha modes. Default materials
now select Blend when the sampled referenced opacity values contain translucency,
otherwise Opaque; alpha remains in mesh vertex colors, not multiplied into
material alpha again. Primitive meshes now carry constant/single display opacity
alongside display color, including sampled indices via the shared primvar reader.
Nonconstant multi-value shape opacity remains unsupported. Bound shader networks
and order-independent transparency are not implemented by this change.

Added `assets/display_opacity.usda` and an independent-root clock regression
covering ordinary mesh/cube and a point-instanced cube, alpha mode transitions,
sampled vertex alpha and unchanged material alpha. Extended all-six-shape
constant-color tests with indexed opacity sampling. Inspected matched GPU
captures `target/display-opacity-{0,10}.png`: the blue backdrop shows through at
opacity 0.2, and all three foreground objects become opaque at time 10.
All 330 tests, check-all, build and whitespace checks pass; logs are
`/tmp/usd-opacity-{all-tests,check,build}.log`. Broader rendering parity remains open.

Hardened primitive dimensions before Bevy mesh construction: negative, nonfinite
and f32-overflowing values now fail instead of reaching constructors; generated
positions/normals must also be finite. Ordinary shapes publish UsdShapeError,
clear owned geometry, and recover on valid input while preserving runtime
children. ShapeRoute itself now invalidates Aabb on successful regeneration,
including direct route calls. Existing independent-clock bounds tests still pass.
PointInstancer invalid-dimension sampling suppresses the affected prototype and
recovers stable nodes without affecting the other root's clock.

Editor render-issue publication includes shape errors. Visually verified the
selected invalid sphere's diagnostic in
`target/viewer-ui-captures/invalid-shape-inspector.png`. The standalone GPU tool
now treats UsdShapeError as a capture failure; a negative-radius fixture exited
unsuccessfully with the expected /Broken diagnostic and wrote no PNG
(`/tmp/usd-invalid-shape-capture.log`). All 329 tests, check-all, build and
whitespace checks pass (`/tmp/usd-shape-safety-final-{tests,check,build}.log`).
Regression cases cover every supported dimension, negative/NaN/infinite/overflow
values, recovery, bounds invalidation, runtime children and inspector clearing.

PointInstancer prototype hierarchies now project Cube, Sphere, Cylinder, Capsule,
Cone and Plane through the canonical shape mesh builder. Direct shape targets
use the same path. Geometry and preview-material handles share the ordinary-prim
cache; prototype material resolution is shared with mesh prototypes. Tests cover
all six types at independent times, exact handle parity, runtime-child/entity
preservation, type replacement and retargeting a subtree to a direct shape.

The first GPU captures exposed ignored primitive display colors despite ordinary
and instanced handle parity. Fixed the common shape builder to broadcast constant
colors (including sampled indices) and single unindexed colors. Added explicit
color assertions plus all-six-shape sampled-index coverage. Other interpolation
layouts and shape opacity/subsets remain unsupported rather than claimed complete.
`target/point-shapes-{0,10}.png` show dimensions doubling in both copies;
`target/point-shapes-color-10.png` verifies the corrected cube color visually.
All captures completed GPU readback. Full final suite: 328 passed, zero failures
(`/tmp/usd-shape-final-tests.log`); check-all/build passed
(`/tmp/usd-shape-prototype-{check,build}.log`). No sibling modifications.

Fixed generated flat normals at extreme coordinate scales in `mesh.rs`:
cross products and normalization now use f64 before emitting f32 unit normals.
The previous f32 calculation underflowed/overflowed and substituted +Y for valid
faces. Regression coverage spans 1e-30 through 1e30, polygonal/bilinear meshes,
both winding conventions and material-subset layouts. All 326 tests, check-all,
build and whitespace checks pass (`/tmp/usd-flat-normal-{all-tests,check,build}.log`).

Added `assets/normal_scale.usda`: equal world-size panels with local coordinate
scales 1e-12, 1 and 1e12. GPU capture `target/normal-scale-review.png` completed
and was visually inspected: all three panels have matching apparent shading.
Center RGBA samples were (116,182,191,255), (117,182,191,255), (117,182,191,255);
no exact pixel-equality claim. Shadows were disabled for this normal diagnostic.
This does not fix Spot: the fresh source report (`/tmp/spot-current-geometry.log`)
shows authored vertex normals and fallback Catmull-Clark schemes. Source versus
renderer seam attribution and reference-render subdivision parity remain open.

Extended the embedded viewer's `USD_SCREENSHOT` hook to write tightly packed
RGBA8 pixels and readback metadata beside its PNG. It logs explicit success or
failure and retries camera availability after frame 120 rather than silently
missing a late camera. This is a one-shot diagnostic delay, not a readiness
guarantee. Two focused tests cover pixel layout, dimensions and output failures;
check-all and build pass. The standalone capture retains readiness/exit checks.
Final full-suite validation passed 325 tests with zero failures
(`/tmp/usd-capture-review-all-tests.log`). Matched Spot shadow-on/off raw
comparison changed 3836 of 921600 pixels (0.416233%); comparison exit 1 denotes
different images, not a readback failure.

Visually inspected fresh Spot captures from the real GPU and private Weston UI:
`target/spot-embedded-review.png` (1440x920, 5299200 raw bytes),
`target/viewer-ui-captures/spot-embedded-review.png`, and matched standalone
`target/spot-review-{current,no-shadows}.png`. Embedded capture logged
VIEWPORT_CAPTURE_OK. Faceting and body seams remain visible without shadow maps;
they are not merely UI/compositor artifacts. No geometry-fidelity fix is claimed.
The embedded renderer still logs its SSAO storage-texture-limit failure.
Capture artifacts and logs are local ignored files, not committed baselines.

The preceding hierarchical PointInstancer changes passed all 323 tests,
check-all and build (`/tmp/usd-hierarchy-{tests,check,build}.log`). Prototype
Xform/Scope/Mesh descendants now retain path-keyed entities and local transforms;
independent clocks traverse prototype descendants. Empty grouping children do
not displace the existing direct-mesh/subset layout. Reset-stack operations,
per-local-matrix shear and unsupported geometry fail explicitly. Finite TRS
chains are supported; full USD transform/geometry parity remains open.

Added private-session native-picker capture support. With
`USD_UI_CAPTURE_PRIVATE_BUS=1`, the viewer runs under `dbus-run-session` after
the private Wayland runtime is configured; config/data/cache directories are
also temporary. GTK and XDG portal packages were fetched to the Nix store without
changing project or desktop configuration. Added `open_dialog.replay` and
`save_dialog.replay` and verified real GTK choosers visually:
`target/viewer-ui-captures/native-open-picker.png` and `native-save-picker.png`.
The first replay at y=777 opened Save, not Open; the inspected initial artifact
is `native-open-dialog.png`, copied to the correctly named save-picker artifact.
Correct Open coordinates are y=897. No selection or save was performed. The
completed first capture's D-Bus/portal/GTK processes were verified absent.

The native portal warns that no parent window was supplied. Mara's current
CreationContext exposes egui/GPU state, but no native handle for RFD set_parent.
Requested explicit approval for a small ../mara accessor; no sibling edits made.
Native selection/cancel/save/reopen acceptance remains open. Added a delayed
future test verifying waker registration, notification and one-shot delivery;
all 321 tests, check-all, build, shell syntax and whitespace checks pass. Logs:
`/tmp/usd-native-dialog-{tests,check}.log` and `/tmp/usd-dialog-wake-build.log`.
Tests used workspace TMPDIR; capture documentation includes the isolated recipe.

Replaced synchronous viewer open/save picker calls with `src/file_dialog.rs`.
RFD async dialogs are constructed on the UI thread, polled without blocking, and
wake egui through a repaint waker. One pending dialog is retained; duplicate
requests are ignored before another native dialog is constructed. Completion or
cancellation is consumed once; invalid UTF-8 paths report an error instead of
silently changing the filename. Pending status is shown in the outliner.

Added process-local editor document IDs and `EditorCommand::SaveChecked` so an
async save result cannot write a newly opened document or changed edit target.
The backend validates context when executing the command, not only when the UI
receives the selection. Unconditional `Save` remains for explicit programmatic
use. Tests cover pending/duplicate/cancel/one-shot selection handling, save-context
retention, non-UTF-8 paths, stale document/layer rejection preserving the output,
and successful current-context save/reopen. All 320 tests, check-all, build and
whitespace checks pass (`/tmp/usd-async-dialog-{tests,check,build}.log`); tests use
workspace TMPDIR. Native picker interaction/platform and pending-status rendered
acceptance are not claimed; actual USD load/export remains on the editor thread.
RFD threading behavior was checked against the locally pinned 0.15 source.

Hardened editor/root-layer persistence against partial-export truncation.
Upstream `sdf::Layer::export` creates/truncates its output before format writing;
new private `persistence::export_layer` stages in a unique same-directory file
with the destination extension, syncs the completed file, atomically replaces the
destination and syncs the directory on Unix. Editor root/edit/flattened saves and
`authoring::save_stage_as` use this canonical path. Existing file permissions are
copied; final symlinks, directories and read-only destinations are rejected.
Directory-sync failure explicitly reports that publication already happened.
Moved the already-used `tempfile` crate from dev-only to runtime dependencies.

Tests inject a partial writer failure and a publication failure, verify staging
cleanup and retained prior contents, round-trip USDA/USDC/USD/USDZ with format
magic checks, preserve bytes for an unsupported format, retain Unix permissions,
and reject symlink/read-only targets. Existing editor composition/save-mode
tests also pass. All 315 workspace tests, check-all, build and whitespace checks
pass (`/tmp/usd-atomic-save-{tests,check,build}.log`), using workspace test TMPDIR.
README/SUPPORT describe atomic replacement's inode/ownership/ACL/hard-link limits,
last-writer-wins behavior and unchanged Save As relative-path semantics.
This is failure-injection/round-trip acceptance, not power-loss simulation or
native save-dialog UI acceptance. Broader checklist and rendering gaps remain.

Added `USD_CAPTURE_SHADOWS=scene|off` to the low-level capture tool. Default
`scene` leaves every light's authored/studio shadow state unchanged; `off`
disables directional, point and spot shadow maps without changing USD. Captures
record the choice. A regression checks all three light types and preservation of
disabled scene lights. All 311 workspace tests, check-all, build and whitespace
checks pass (`/tmp/usd-shadow-diagnostic-{tests,check,build}.log`), with test TMPDIR
in `target/test-tmp`.

Matched `target/inherited-normal-no-shadows.png` against
`target/inherited-normal-10.png` at time 10 and camera (3,3,6)->(0,1,0): the fine
striping disappears with shadow maps off. This isolates the visible artifact to
shadow rendering, not normal inheritance. Local Bevy 0.19.1 `pbr_functions.wgsl`
passes `in.world_normal` into `fetch_directional_shadow`; `shadows.wgsl` uses
that normal for texel-scaled receiver bias. Bias is a plausible mechanism, not
yet a proven/fixed root cause. Do not disable production shadows as a fidelity
fix. Matched Spot captures `target/spot-shadow-close-{scene,off}.png`, at camera
(0.95,0.4,1.05)->(0,0,0), retain body/hip faceting and seams without shadow maps.
These closer body inspections crop the feet; wider captures are
`target/spot-shadow-{scene,off}.png`. Next: separate shadow-bias robustness from
Spot topology/normal/subdivision fidelity and establish reference comparisons.

The sandbox mount quota initially prevented both reads and built-in patches.
Approved unsandboxed reads and context-matched `git apply` applied the diagnostic
control hunks; normal built-in patching became available again for the remaining
edits. No dependency, environment configuration or sibling repository was changed.

Validated inherited normals across independent asset roots, direct point
prototypes and material subsets. The new instance regression alternates clocks,
checks normal arrays and prototype handle sharing, edits one ancestor at the
same time, adds/removes a mesh-local override, then blocks the ancestor back to
generated normals. The other root's mesh handle, ordinary mesh identities and a
runtime child remain unchanged. Subsets own the rendered face; the base mesh has
zero indices rather than duplicate geometry.

Fixed actual-window capture startup timing: Cargo build-lock waits previously
consumed the fixed delay and produced compositor-only screenshots (confirmed in
`target/viewer-ui-captures/inherited-normal-0.png`). The script now waits up to
300 seconds for `USD_VIEWER_STARTED`, emitted after viewport construction, before
starting its capture delay. This is not a scene/pipeline-readiness handshake.
Fresh `target/viewer-ui-captures/inherited-normal-ready-{0,10}.png` and embedded
texture captures `target/embedded-normal-ready-{0,10}.png` render the fixture;
time 10 retains the grazing-normal striping also seen offscreen. The first fixed
capture survived a 22.28-second compilation before its 20-second capture delay.

All 310 tests, check-all, build, shell syntax and whitespace checks pass; logs:
`/tmp/usd-inherited-instance-{tests,check,build}.log`. The final test run used
`TMPDIR="$PWD/target/test-tmp"`: the inherited Nix-shell temporary directory hit
a disk quota in the RGBA file-output test, reproduced in
`/tmp/usd-capture-quota-retry.log`; the same test passed with a workspace temp
directory. No test was skipped. Remaining: grazing-normal/Spot fidelity,
reference comparisons, embedded device limits and the broader checklist above.

Implemented inherited constant normal primvar lookup using composed authored-value
provenance. Local authored values win; declarations, blocked values and
nonconstant ancestor primvars do not supply inherited normals. Values,
interpolation and sampled indices are read from the same resolved owner.
Animation detection includes inherited inputs for meshes and direct point
prototypes; normal-primvar edits reconcile consumers. Reader tests cover nearest
owner, local precedence, blocked/declaration/nonconstant ancestors and sampled
indices. A LiveStage test verifies scrubbing, same-time parent edits, blocking
back to generated normals, stable entity identity and a runtime-only child.
All 309 workspace tests, check-all, build and whitespace checks pass; logs:
`/tmp/usd-inherited-normals-{tests,check,build}.log`.
Added `assets/inherited_normals.usda` and inspected matched GPU captures
`target/inherited-normal-{0,10}.png`. Shading changes with the inherited normal;
the deliberately tangent normal at time 10 exhibits fine striping, so this is
normal-routing evidence, not artifact-free shading or reference parity.
Independent-root/prototype runtime acceptance, actual embedded-viewer acceptance,
and diagnosis of Spot seams remain open. Reference semantics:
https://openusd.org/release/api/class_usd_geom_primvars_a_p_i.html

Rechecked screenshot tooling with fresh GPU and actual-window captures. The
private Weston capture `target/viewer-ui-captures/spot-fresh-review.png` shows
visible Spot faceting/seams; its viewer log also reports unavailable SSAO because
the embedded device exposes fewer than five storage textures per shader stage.
Matched offscreen captures `target/capture-review-sharp.png` (control cage) and
`target/capture-review-sharp-level2.png` (level 2), at time 3, show rounded shading
versus distinct flat faces on the permanent-crease cube. This verifies the
existing refined normal path, not a new rendering fix or reference parity.
Capture metadata now records subdivision level (0 disabled); a regression tests
both settings, exact 3,686,400-byte RGBA output, and PNG publication. Focused
workspace-feature test, check-all and diff whitespace checks pass; logs are
`/tmp/usd-capture-review-{test,check}.log`. Next: isolate Spot topology/authored
normal fidelity against a reference renderer and resolve embedded device limits.

Added `animated_sharpness_rebuilds_normals_and_subsets_per_instance` to validate
smooth/hard layout transitions through full asset projection. Two roots alternate
times 0 and 3; ordinary meshes, material subsets and two direct point prototypes
switch between 98 indexed vertices and 384 face-corner vertices. Checks cover unit
normals, axis-aligned hard normals, subset triangle counts, asset sharing, stable
mesh/subset identities and runtime children. Invalid sharpness on one stage
suppresses its mesh/subsets and reports a prototype warning while the other root's
handle stays unchanged. Restoring valid data at the same time clears errors and
rebuilds ordinary/subset/prototype geometry. All 306 workspace tests, check-all,
build and whitespace checks pass (`/tmp/usd-sharp-layout-{tests,check,build}.log`);
the final expanded recovery assertions also pass in the workspace-feature focused
run (`/tmp/usd-sharp-layout-final.log`). A package-only focused run rebuilt a
different Bevy feature combination; prefer workspace/all-target filtering here.
This is headless runtime acceptance, not additional renderer-parity evidence.

Added normal splitting for permanent subdivision creases/corners. Refined topology
retains permanent sharp feature identities; after hole filtering, per-corner
normals average angle-weighted triangle contributions only within connected smooth
face fans. Face-varying output preserves render/source-point and subset mappings.
Tests cover cube face normals, reversed winding, smooth hinge averaging, an
isolated sharp corner, hard-edge separation, subset mapping, and scales 1e-20 to
1e20. All 305 tests, check-all, build and whitespace checks pass
(`/tmp/usd-crease-normals-{tests,check,build}.log`). Matched-camera GPU capture
`target/creases-sharp-normal-fix.png` shows distinct flat faces rather than the
incorrect rounded shading in `target/creases-sharp.png`. This supersedes the
permanent hard-edge normal gap below, but exact semi-sharp/limit normals, other
subdivision modes and full reference-renderer parity remain open.

Implemented uniform-decay Catmull–Clark crease/corner position stencils using
OpenSubdiv's parent/child mask rules and averaged fractional transition weights.
Finite sharpness decrements per level; values >=10 remain permanent. Supports
per-chain/per-edge arrays, validates real topology edges, rejects duplicate
edge/corner entries, propagates child-edge sharpness and leaves varying/bilinear
data linear. Zero/absent sharpness retains the previous fast path. Tests cover
infinite cube corners, fractional positions, finite decay, zero sharpness,
corner pinning, dart masks, per-edge/per-chain expansion and invalid edges.
All 304 tests, check-all, build and whitespace checks pass
(`/tmp/usd-sharpness-{tests,check,build}.log`). Added sampled fixture
`assets/subdivision_creases.usda`; GPU captures `target/creases-smooth.png` and
`target/creases-sharp.png` visibly change from rounded to cube geometry at level 2.
Those captures precede only the no-sharpness fast-path optimization and extra
array-expansion assertions. Hard creases still receive smooth averaged normals;
crease-aware normal splitting is the next concrete rendering gap. This is not
full sharpness rendering, Chaikin support or OpenSubdiv reference parity.
Reference: OpenSubdiv release `sdc/crease.h`, `crease.cpp`, and `scheme.h`.

Added the Rendering pane and `src/render_settings.rs`: control-cage/level 1–6
buttons queue resource changes in PreUpdate; Last publishes refined prim counts
and wrapped subdivision/point-instancer errors. Existing environment-configured
levels are retained until a user request, and no USD opinions are authored.
Added `assets/subdivision_cube.usda` and
`scripts/replays/subdivision_toggle.replay`. Actual captures in
`target/viewer-ui-captures/` show the cube refining through the level-2 button
(`rendering-level2.png`), returning to control cage through its button
(`rendering-disabled.png`), and suppressing an unsupported-scheme mesh with a
readable error (`rendering-error.png`). The disabled capture has a shifted native
window origin, so these are UI/geometry acceptance, not pixel-aligned comparisons.
All 302 tests, check-all, build and whitespace checks pass
(`/tmp/usd-rendering-pane-{tests,check,build}.log`). Exact subdivision fidelity,
unsupported rules, and broader UI/lifecycle acceptance remain open.

Subdivision settings now refresh existing Mesh and PointInstancer projections
automatically in both LiveStagePlugin and independently clocked asset instances.
Each runtime tracks its applied level, including disabled state; initial load
records the current setting without a redundant refresh. Updates use each root's
stage time and texture context. A regression covers level 1 -> 2 -> disabled -> 1,
ordinary/prototype vertex counts 9 -> 25 -> 4 -> 9, different sampled geometry at
times 0 and 10, stable mesh entity identities, runtime components and children,
and clearing/restoring UsdSubdivisionApplied. All 301 tests, check-all, build and
whitespace checks pass (`/tmp/usd-subdivision-settings-{tests,check,build}.log`).
This supersedes the manual-reprojection limitation below. No new GPU visual
acceptance is claimed; subdivision remains opt-in and its unsupported rules and
reference-renderer fidelity work remain open.

Added repeatable source fixtures `assets/editor_samples.usda` and
`scripts/replays/sample_history.replay`, with parser coverage and documented
16/20/24/28-second capture checkpoints. The actual viewer's Clear Sample button
removes scene key 25 while leaving keys 0 and 10 and default 5 visible
(`target/viewer-ui-captures/sample-cleared.png`). Undo restores key 25
(`sample-clear-undone.png`). Both captures used the source fixture, not the earlier
temporary file; fixture contents remain unchanged on disk. All 300 tests,
check-all, build and whitespace checks pass
(`/tmp/usd-sample-clear-{tests,check,build}.log`). This closes basic sample set,
clear, undo and redo UI acceptance for the direct root-layer fixture; mapped
edit-target UI interaction, save/reopen dialogs and broader rendering fidelity
still require verification.

Added compact provenance summaries (source kind, arc, source prim path), with a
per-property Show/Hide source details toggle retaining the full wrapped record.
Snapshots also expose composed scene sample keys; the inspector shows six keys
and a total for longer lists. Reference-arc tests verify source namespace summary
and retiming of source time 2 to scene time 14. All 299 tests, check-all, build and
whitespace checks pass (`/tmp/usd-source-details-{tests,check,build}.log`).
Actual viewer replay captures in `target/viewer-ui-captures/` verify compact and
expanded layouts (`source-summary.png`, `source-expanded.png`), applying a new
sample at scene time 25 (`sample-applied.png`), undo removing key 25
(`sample-undone.png`), and redo restoring it (`sample-redone.png`). Default value
5 remains visible throughout. Inputs pass through real widgets and ribbon
buttons, not editor-command injection. Replay scripts are
`target/{source-expand,sample-apply,sample-history}.replay`; the history script is
captured at 16 seconds for undo and 20 for redo. Successful Clear Sample, save/reopen
through UI, and other fidelity requirements remain open.

Fixed attribute provenance overlap by rendering complete, Unicode-safe wrapped
source lines with matching pod height instead of the single-line readout. Verified
in `target/viewer-ui-captures/provenance-wrapped.png`: the source record is readable
without drawing over the attribute name or action buttons. All 298 tests,
check-all, build and whitespace checks pass
(`/tmp/usd-provenance-ui-{tests,check,build}.log`). The full upstream debug record
is verbose and pushes edit controls below the fold; a compact summary with
expandable details remains a usability improvement. Successful sample edit/undo
UI acceptance is still open; replay coordinates must follow the changed layout.

Added opt-in `USD_UI_REPLAY` input tooling in `src/ui_replay.rs`, registered through
egui's pre-processing input hook without changing Mara. Timed pointer, button,
scroll and literal text events enter the viewer's normal UI input pipeline; no OS
input or editor-command shortcut is used. Parsing rejects unordered timestamps,
unknown actions and nonfinite coordinates, preserves literal text spacing, and
dispatches each event once while preserving host events. All 298 tests, check-all,
build and whitespace checks pass (`/tmp/usd-ui-replay-{tests,check,build}.log`).
Actual captures `target/viewer-ui-captures/sample-score-controls.png` and
`sample-invalid-time.png` verify scrolling to the sample controls, typing `bad`
into time `0`, clicking Clear, and the visible finite-number validation error with
default value 5 retained. Source/provenance readout overlaps nearby text and still
needs layout repair. Successful sample apply/clear and undo through UI remain open.
Replay fixtures are in `target/sample-{scroll,score,invalid-time}.replay`.

Added explicit inspector sample authoring controls: a finite scene-time field,
copy-from-timeline button, sample set and sample clear, distinct from default-value
apply. Time drafts persist while the timeline changes; clear remains available for
unsupported value types. Commands use the existing undoable sample API and active
edit-target mapping. Added finite-time parser coverage; all 296 tests, check-all,
build and whitespace checks pass (`/tmp/usd-sample-ui-{tests,check,build}.log`).
The actual capture `target/viewer-ui-captures/sample-authoring-controls.png`
shows the inspector, but properties fall below its initial scroll viewport.
Therefore it does not establish visual/interaction acceptance of the new controls;
scrolling, sample apply/clear and undo through real UI input remain to verify.

Fixed default-path hole rendering: `ReadMesh` now retains sampled `holeIndices`,
and shared triangulation excludes those faces without renumbering source primvars
or subsets. Indexed, expanded and flat layouts use the same filtering, including
the deformation point map. Refined meshes clear the source hole list after the
existing post-refinement filtering. Regression coverage includes duplicate/all
holes, sampled changes, indexed face-varying UVs, uniform colors, subset layouts,
independent roots and direct point-instancer prototypes. All 295 tests, check-all,
build and whitespace checks pass (`/tmp/usd-default-holes-{tests,check,build}.log`).
Visually verified the hole in both the default actual viewer screenshot
`target/viewer-ui-captures/default-holes-fixed.png` and matched-camera GPU readback
`target/screenshot-default-holes-fixed.png`. This supersedes the default-hole
limitation below; remaining Spot artifacts and broader fidelity work remain open.

Rechecked both screenshot paths against the current checkout. Actual isolated
viewer captures `target/viewer-ui-captures/spot-current-default.png` and
`spot-current-check.png` (subdivision level 1) both show visible faceting and dark
seams; the latter has additional small edge artifacts. These observations do not
establish the cause or reference-renderer parity. Low-level GPU captures
`target/screenshot-lowlevel-check.png` and
`target/screenshot-lowlevel-holes-refined.png` use identical camera/time settings:
the default control-cage path fills the fixture's authored center hole, while
subdivision level 2 displays it. Both captures exit successfully and write PNG,
3,686,400-byte RGBA buffers and metadata. This is direct evidence that capture
success does not establish USD rendering fidelity. No rendering fix was made in
this recheck; default hole handling and remaining Spot artifacts need follow-up.

Added finite scene-time sample set/clear authoring functions and undoable
`EditorEdit::AttributeSample` / `ClearAttributeSample` variants. Sample times are
passed through upstream edit-target mapping rather than treated as raw layer time.
A reference-arc regression with offset 10 and scale 2 verifies scene time 14 maps
to source time 2 and source namespace, retains the default value, and supports
sample set/clear undo/redo. Nonfinite times fail before creating a declaration or
adding undo history. All 294 workspace tests, check-all, viewer build and whitespace
checks pass; logs: `/tmp/usd-sample-edit-{tests,check,build}.log`. Inspector sample
controls and interactive time-authoring acceptance remain open.

The inspector now prepares declared-type drafts for supported attributes with
blocked/absent resolved values instead of making them uneditable. Templates cover
supported scalar and three-vector role types; unknown/array/matrix types remain
read-only. Asset-path text editing preserves raw relative identifiers, Unicode and
spaces. The apply button explicitly authors the default, not a time sample, and
draft creation does not author anything. Tests roundtrip all supported templates,
preserve asset text, reject unsupported templates, and verify authoring over a
value block with undo/redo back to the blocked state. All 293 workspace tests,
check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-inspector-absent-{tests,check,build}.log`. Interactive control/scroll
acceptance and time-sample authoring UI remain open.

Added explicit `clear_attribute_values` and `block_attribute_values` authoring
operations and corresponding undoable EditorEdit variants. They call upstream
attribute value APIs rather than deleting the property spec. Inspector buttons
are available before the absent/unsupported-value early returns and display block
state. A byte-backed layered regression verifies blocking removes local samples,
clearing reveals weaker values, property documentation survives, undo/redo restores
default/sample/block states, and the edit target remains unchanged. All 292
workspace tests, check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-attribute-values-{tests,check,build}.log`. Full interactive button/scroll
acceptance has not been performed for these new controls.

Removed the obsolete `read_asset_info` stub and now read composed prim assetInfo
through the pinned upstream `Prim::get_metadata::<Value>` API. Editor snapshots
and wrapped inspector rows expose this read-only metadata. A byte-backed two-layer
test verifies strong scalar opinions, nested key-by-key merging, retained weak
keys, raw asset identifiers, sorted dictionary entries and clearing on selection
changes. All 291 workspace tests, check-all, viewer build and whitespace checks
pass; logs: `/tmp/usd-asset-info-{tests,check,build}.log`. The actual inspector
capture at `target/viewer-ui-captures/inspector-asset-info.png` was inspected and
shows identifier, name and version. OPENUSD_UPGRADE.md records the corrected
capability assessment. Metadata authoring UI is not implemented by this change.

Inspector edit-target, layer and selected-prim paths now use wrapped rows rather
than clipped single-line readouts. Active layers are labeled explicitly; inactive
layers use a short button while retaining the complete layer identifier for the
edit command. Regression coverage preserves Unicode, repeated spaces, long path
tokens, empty text and explicit line breaks. All 290 workspace tests, check-all,
viewer build and whitespace checks pass; logs:
`/tmp/usd-inspector-paths-{tests,check,build}.log`. The actual two-layer inspector
capture at `target/viewer-ui-captures/inspector-wrapped-layer-paths.png` was inspected:
both full paths, active-target state and inactive-target button fit without the
previous overlap. This is visual layout evidence, not full interaction acceptance.

Catmull-Clark boundary interpolation `none` now marks every control face touching
a boundary vertex as an implicit hole, retains edge-only refinement support, and
uses the same output topology/primvar/subset mask as authored holes. Bilinear
`none` does not hide boundary faces. The regression compares surviving interior
positions against the full edge-only mesh, checks the surviving center-face domain,
combines implicit and authored holes, and distinguishes bilinear output. All 289
workspace tests, check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-boundary-none-{tests,check,build}.log`. Exact limit normals, sharpness,
Loop, other Catmull-Clark face-varying modes and reference parity remain open.

Authored subdivision holes are now evaluated as retained support faces whose
refined descendants are omitted from output topology. Uniform/face-varying data
and subset membership are filtered consistently; vertex data stays in the full
refined point domain. Tests verify unchanged refined positions versus the unholed
mesh, sampled hole selection, indexed input primvars, subset remapping, duplicate
hole IDs and all-hole output. Invalid hole IDs still fail and recover through the
route diagnostic path. All 288 workspace tests, check-all, viewer build and whitespace
checks pass; logs: `/tmp/usd-subdivision-holes-{tests,check,build}.log`.
`assets/subdivision_holes.usda` adds a center-hole fixture. Its actual level-2 viewer
capture was inspected at `target/viewer-ui-captures/subdivision-holes.png`, showing
the grid through the omitted center. This does not establish limit-surface parity.
The image sandbox helper hit a temporary-directory quota error; the existing PNG
was read through an approved base64 read instead, without rerunning the capture.

Bilinear subdivision now uses the shared bounded topology/stencil pipeline with
identity weights at existing vertices, face averages and edge midpoints. The mesh
adapter selects it from matching USD scheme data, retains bilinear output identity,
and interpolates vertex/varying/face-varying data linearly. Ordinary and direct
point-instancer routes reuse this adapter. Sharpness and holes remain explicitly
unsupported. Tests verify a warped z=x*y patch through three levels, retained
control vertices, vertex/varying equality, default face-varying data, boundary
settings, converted vertex counts and malformed input rejection. All 287 workspace
tests, check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-bilinear-{tests,check,build}.log`. This is finite tessellation, not limit
evaluation or reference-renderer acceptance. SUPPORT.md now reflects the opt-in
subdivision implementation rather than its obsolete control-cage-only status.

The subdivision route now bypasses refinement only for an explicit resolved
`none` token; unknown/blocked/unreadable schemes reach validation instead of
silently falling back to polygon rendering. Regression coverage checks unknown
token suppression and recovery to explicit `none`, with another root unaffected.
EditorSnapshot now publishes selected-prim subdivision, deformation and instancer
errors; inspector rows wrap long messages. Tests verify error publication and
clearing on recovery, missing selection and deselection. Startup UI-capture options
now include inspector/timeline panes and `USD_VIEWER_SELECT`. All 285 workspace
tests, check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-render-issues-{tests,check,build}.log`. Actual capture inspected at
`target/viewer-ui-captures/subdivision-inspector-error.png` shows the unknown-token
error readable and mesh suppressed. Existing long layer-path readouts still clip;
this capture is not full inspector interaction/layout acceptance.

Direct mesh point-instancer prototypes now use the same CPU-deformation/refinement
function as ordinary mesh routes, before prototype transforms and material-subset
splitting. Extended integration coverage verifies two copies share mesh handles,
refined vertex/subset counts, independent root clocks, failure suppression and
recovery. A morph regression verifies the animated control corner and refined
face center at three samples while triangulation reference positions stay at rest.
Offscreen capture now rejects point-instancer diagnostics instead of saving omitted
prototype geometry. Hierarchical prototypes, GPU subdivision and rendered reference
parity remain open. All 284 workspace tests, check-all, viewer build and whitespace
checks pass. Validation logs:
`/tmp/usd-subdivision-instancer-{tests,check,build}.log`.

The large fragmented dark patches in the first subdivision capture were traced
to Bevy 0.19.1's fixed epsilon in angle-weighted normal generation: the refined
Spot base had 4,756 zero normals out of 8,292 vertices. Indexed and expanded USD
meshes now share double-precision angle-weighted normal generation over the
actual triangulation, without a world-scale epsilon. Authored normals are retained;
truly degenerate or unreferenced vertices remain diagnosable as zero normals.
A regression covers scales 1e-20 through 1e20, both winding orientations and both
vertex layouts. Scene report now includes projected-invalid-normal counts.
All 283 workspace tests, check-all, viewer build and whitespace checks pass;
logs: `/tmp/usd-normal-scale-{tests,check,build}.log`. Reports:
`target/spot-refinement-normals-{before,after}.txt`; all 26 refined meshes now have
zero invalid normals. Actual full viewer recapture was inspected at
`target/viewer-ui-captures/spot-subdivision-normal-fix.png`: the large fragmented
patches are gone, but smaller seams/faceting remain. This closes the scale-related
normal bug, not reference-renderer acceptance or the full subdivision work.

Finite subdivision is now an opt-in projection route after skinning and before
material subsets. `UsdSubdivisionSettings` (1–6 levels) enables it, with
`USD_SUBDIVISION_LEVELS` accepted by the viewer and offscreen capture. The route
CPU-deforms control points before refinement, clears owned GPU deformation state,
shares refined subset geometry, invalidates stale bounds, and exposes applied/error
components. Failed refinement suppresses mesh/subsets; offscreen capture checks
the error component before saving. Point-instancer prototypes remain unconnected,
and settings changes require reprojection. An integration regression verifies two
independent sampled roots, refined subset indices, failure/recovery, stable prim
entities and preservation of runtime children. All 282 workspace tests, check-all,
viewer build and whitespace checks pass; logs:
`/tmp/usd-subdivision-route-{tests,check,build}.log`.
Actual full-viewer capture was run and inspected:
`target/viewer-ui-captures/spot-subdivision-level1.png`. It shows fragmented dark/
missing-looking patches compared with `spot-grid-refined.png`. This is a FAILED
visual acceptance result, not a subdivision fidelity claim. Investigate refined
geometry/triangulation/normals/culling before enabling the option by default.

Disconnected vertex fans now keep their shared vertex fixed using an identity
stencil, matching the infinite-sharpness rule for this case in OpenSubdiv's
`far/topologyRefinerFactory.cpp`. This does not split or weld topology and does
not enable non-manifold edges or inconsistent winding. Regressions cover open
bow-tie and closed joined-tetrahedron fans, retained shared indices, source-face
lineage and vertex primvars across multiple levels. All 281 workspace tests,
check-all, viewer build and whitespace checks pass; logs:
`/tmp/usd-subdivision-fans-{tests,check,build}.log`. All 26 traversed Spot meshes
now pass one-level mesh refinement; evidence: `target/spot-refinement-fans-report.txt`.
This supersedes the previous disconnected-fan rejection reported below. Runtime
rendering integration and comparison against OpenSubdiv output remain incomplete.

`subdivision::refine_mesh` now applies validated supported rules to a complete
sampled ReadMesh: positions, reference triangulation positions, indexed UV/color/
opacity primvars and material subset lineage. It preserves orientation and sidedness,
clears stale extent and authored normals, and rejects unsupported boundary/triangle/
face-varying rules, sharpness and holes. Generated normals remain finite-mesh
approximations, not limit derivatives. A composed-stage regression checks refinement,
indexed corner UVs, opacity, uniform color, subset bindings, left-handed normals and
failure without modifying inputs. All 280 workspace tests, check-all, viewer build
and whitespace checks pass; logs: `/tmp/usd-subdivision-mesh-{tests,check,build}.log`.
Scene report now accepts optional refinement levels and returns nonzero on any
failed mesh. The one-level Spot probe found 16/26 meshes rejected for disconnected
vertex fans (including library duplicates), while 10 succeeded. The base increases
from 3,368 to 8,292 points and 1,296 to 3,888 faces. Evidence:
`target/spot-refinement-report.txt`. Non-manifold policy/reference checks are now a
confirmed integration requirement; the viewer still renders the original cages.

`read::subdivision::read_subdivision_at` now reads composed subdivision rules,
sampled crease/corner sharpness and hole indices. Unknown tokens, wrong value
types and nonfinite times return errors. Separate dimension validation checks
crease lengths, per-crease/per-edge sharpness cardinality, corner cardinality,
nonnegative finite sharpness and point/face bounds. It does not yet validate that
crease pairs exist as topology edges or evaluate sharpness/holes. Tests cover
schema defaults, interpolated sharpness, both cardinality modes and malformed
inputs. All 279 workspace tests, check-all, viewer build and whitespace checks
pass; logs: `/tmp/usd-subdivision-schema-{tests,check,build}.log`.
The updated scene report ran on Spot: all 26 traversed meshes report
edgeAndCorner, cornersPlus1, standard Catmull-Clark triangle rules, and no creases,
corners or holes. Evidence: `target/spot-subdivision-report.txt`. Rendering still
uses control cages; schema reading alone is not subdivision integration.

Subdivision now accepts indexed `MeshPrimvar` data through
`RefinedSurface::interpolate_primvar`, with exact interpolation cardinality,
finite-value and index validation before expansion. Face-varying data requires
explicit all-linear selection; other modes are not silently approximated.
`remap_subsets` preserves names/material bindings, maps source faces to descendants
and deduplicates membership. Both helpers leave input data unchanged on failure;
invalid public source-face lineage is rejected rather than indexed unchecked.
Tests cover all five primvar layouts, indexed/unindexed equivalence, unsupported
face-varying selection, malformed indices/counts and subset lineage over two
levels. All 277 workspace tests, check-all, viewer build and whitespace checks
pass; logs: `/tmp/usd-subdivision-indexed-{tests,check,build}.log`. Schema-rule
selection, normals, deformation ordering and rendering integration remain open.

The subdivision kernel now also interpolates linear varying values, face-uniform
values and expanded face-varying values under the explicit all-linear rule.
Tests distinguish varying data from smoothed vertex data and preserve independent
UV islands and face labels over three levels. Invalid counts and nonfinite float
inputs are rejected. Both vertex and varying stencils count toward the retained
weight budget. Other face-varying rules and renderer integration remain absent;
the viewer still displays control cages. All 275 workspace tests, check-all,
viewer build and diff whitespace checks pass. Logs:
`/tmp/usd-subdivision-primvar-{tests,check,build}.log`.

The subdivision kernel now records per-level vertex interpolation stencils and
uses them for position evaluation. `RefinedSurface::interpolate_vertex` replays
the same weights on finite scalar/vector control-point data with count validation.
Tests reproduce geometry exactly, preserve constant data and affine UV fields over
three levels, reject malformed values/counts and verify incremental weight-budget
failure without corrupting accumulated data. Stencil storage is bounded across
levels. Vertex-fan connectivity now uses local face adjacency instead of scanning
all incident edges for every visited face. This does not add varying or face-varying
interpolation, sharpness, limit normals, color-space conversion or route integration.
All 273 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-subdivision-stencil-{tests,check,build}.log`.

Subdivision implementation has started with `subdivision::catmull_clark`, a
separate finite-level position/topology kernel. It validates face indices/counts,
repeated vertices, edge winding/incidence and connected vertex fans; supports
edge-only and edge-and-corner boundaries; preserves original face IDs on output
quads; and bounds levels, points and corners. Double-precision intermediate
positions reduce arithmetic overflow before checked f32 output. Analytic tests
cover open-quad boundary modes, two levels of face lineage, closed-cube counts,
the expected 5/9 corner position, winding, affine equivariance and malformed
topology. All 271 workspace tests, check-all, viewer build and diff whitespace
checks pass; logs: `/tmp/usd-subdivision-kernel-{tests,check,build}.log`.
This kernel is not connected to routes yet and is not OpenSubdiv parity. Remaining
integration requires schema rules/sharpness, primvar interpolation, material subset
mapping, deformation ordering, limit-normal handling and rendered reference checks.
The viewer still displays control cages; no new subdivision render claim is made.

`examples/scene_report.rs` now reports source-sampled topology, normal layout and
invalid values, UV/vertex/unique-normal counts and authored/fallback subdivision
state. It explicitly warns that subdivision surfaces are only control cages in
the current conversion, traverses without visibility filtering and does not apply
deformation. Running it on the collection's Spot asset found 26 meshes including
library geometry: all have vertex normals and all omit subdivisionScheme, yielding
the Catmull–Clark schema fallback. The base mesh has 3,368 points, 1,296 faces and
3,368 authored normal values; conversion retains 3,368 vertices. This is a real
unimplemented subdivision-fidelity boundary, not evidence that forcing smooth
polygon normals would be correct. No source normals or asset files were changed.
README and SUPPORT now expose that limitation. Report: `target/spot-mesh-report.txt`.
All 268 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-scene-report-{tests,check,build}.log`.

The shared viewer/capture grid now chooses rounded 1/2/5-decade spacing for roughly
3–8 minor cells across the scene's largest bound, rather than flooring a decade
and creating dense lines. Minor/major contrast is lower, grazing-angle fade is
stronger and fade distance is shorter. Camera transforms, USD geometry, lights
and the red/blue axis palette are unchanged. Scale tests cover 0.001 to 1,000,000
world-unit spans. The full-viewer captures `dome-grid-refined.png` and
`spot-grid-refined.png` under `target/viewer-ui-captures/` were inspected: the grid
no longer dominates the scene, and Spot is framed with visible studio-lit geometry
and Outliner. Dome-only spheres remain black with the explicit capability error;
this presentation change does not resolve the pending Mara device-limit issue.
All 267 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-grid-presentation-{tests,check,build}.log`.

Full Mara UI capture now works through `scripts/capture_viewer_ui.sh`: a private
Weston headless Vulkan compositor, real `/bin/bash`, make-run viewer launch,
framebuffer screenshot, retained logs and owned-process cleanup. Weston GL captures
were inverted including desktop chrome; Pixman could not host the GPU surface.
The local custom `bash` wrapper also misparsed the script. Vulkan plus real Bash
produced an upright screenshot without image postprocessing.

The real Lighting pane exposed an embedded-host failure hidden by standalone
captures: Mara's default shared device has four storage textures per shader stage,
while Bevy environment generation requires six. Dome projection now reports
Unavailable with device limits rather than WaitingForMaps forever. Long status
messages wrap into readable rows. The inspected artifact is
`target/viewer-ui-captures/dome-capability-readable.png`; it clearly shows the
error and black unlit spheres in dome-only mode. This is not successful embedded
IBL acceptance. A sibling Mara GPU-configuration hook needs approval and remains
unimplemented. No sibling files were changed. All 265 workspace tests, check-all,
viewer build, shell syntax and diff whitespace checks pass; GPU-limit gate tests
cover deficient texture/workgroup/compute capabilities. Test logs are
`/tmp/usd-viewer-gpu-limits-{tests,check,build}.log`.

`UsdInstanceOverrides::set_component<T>` now accepts reflected typed patches for
root-local reload persistence. Shared strict encoding validates all fields and
property paths before replacing matching override entries; unrelated opinions and
absent-option overrides remain. Tests cover encoding-failure rollback, deduplicated
updates, two independently projected roots, changed-source snapshot replacement,
stable prim entities, runtime-only component retention and revealing the new source
value after removing overrides. The runnable typed example now also exercises
typed overrides across asset-source replacement. This tests the reload projection
path, not a new filesystem-watcher scenario. Stage-dependent failures remain
deferred to instance projection. All 263 workspace tests, check-all, viewer build,
typed-authoring run and diff whitespace checks pass;
logs: `/tmp/usd-persistent-typed-{tests,check,build,example}.log`.

`sync::author_component_presence<T>` now authors explicit true/false presence
without string type names or source entities. It shares qualified-name checking,
owner validation and the atomic edit-target writer with typed field patches.
A stronger non-root override-layer test suppresses weaker Health fields without
rewriting the root/weak layer, then clears the returned local attribute and
restores those fields on the same entity. The runnable typed-authoring example
now disables/re-enables one instance while preserving the other instance and all
retained field values. Field patches do not implicitly cancel false presence.
All 261 workspace tests, check-all, viewer build, typed-authoring run and diff
whitespace checks pass; logs: `/tmp/usd-typed-presence-{tests,check,build,example}.log`.

Empty reflected struct/tuple-struct components now round-trip using a bare
`custom bool bevy:Type = true` presence opinion. False suppresses USD component
projection even with remaining fields and removes only previously route-owned
components. Clearing presence resumes field-driven behavior or removes an owned
marker with no fields. Invalid presence values report issues and retain the prior
component. The marker regression exports/reopens USDA and exercises live false,
true and clear transitions; another regression verifies field suppression,
recovery and preservation of an unowned runtime marker. Nested empty fields and
root enum components remain unsupported. All 260 workspace tests, check-all,
viewer build and diff whitespace checks pass;
logs: `/tmp/usd-marker-{tests,check,build}.log`.

Reflect ownership now canonicalizes registered type aliases before applying and
clearing fields. Disjoint short/qualified opinions merge onto one component;
clearing an alias resets only fields that no remaining alias authors. Duplicate
effective fields across aliases report `ConflictingAliases` and retain the prior
component, or leave it unconstructed on first projection. No arbitrary strength
order is assigned between different USD property names. The regression exercises
initial conflicts, merged values, live conflict/recovery, per-field default reset
and final component removal. Diagnostics identify resolved types by canonical
Rust path. All 259 workspace tests, check-all, viewer build and diff whitespace
checks pass; logs: `/tmp/usd-alias-{tests,check,build}.log`.

Reflect projection publishes structured `UsdReflectIssues` on each affected prim
entity. Type/registry/reflection/default failures and field/value failures carry
the type segment, optional field path and a classified reason. Corrected or
cleared opinions remove resolved issues. The editor publishes selected-prim
issues in Last, after live projection, and the inspector displays component issue
readouts. Tests verify invalid unsigned and unknown-field diagnostics, live
correction on the same entity, selected-prim publication, cleanup and missing
selection. This is not component-level transactional projection. Inspector layout
has compiled but has not received a new rendered UI acceptance check.
All 258 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-projection-issues-{tests,check,build}.log`.

Reflected integer assignments now check target ranges instead of using wrapping
or saturating casts. Signed/unsigned scalar fields, usize and Option<i32> reject
negative-to-unsigned, overflow, fractional and nonfinite opinions without changing
the existing field. Integral float/half/double opinions remain accepted when in
range. Boundary tests include i64::MIN, u64::MAX, usize::MAX and floating-point
values at the signed/unsigned upper limits. A typed-authoring/projection test
preserves all three integer extrema together. This does not validate every
numeric/vector conversion or provide structured per-field projection diagnostics.
All 256 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-integer-{tests,check,build}.log`.

Typed authoring now falls back to encoded full Rust paths for registered types
with colliding short names. Entity-based authoring resolves short, full and encoded
paths through the projection resolver. The generated segment must round-trip to
the same TypeId before any USD write. Regressions author two `Shared` components
with different values on one prim, reject the ambiguous short argument without
mutation, and project both independently after full/encoded-path entity writes.
A reflected path containing literal double underscores verifies rejection of
lossy encoding before mutation. Registry changes do not migrate old short-name
opinions, and Rust path refactors still require saved-opinion migration.
All 254 workspace tests, check-all, viewer build and diff whitespace checks pass;
logs: `/tmp/usd-qualified-{tests,check,build}.log`.

Capture/viewer grid placement now shares `environment::fit_grid`. The offscreen
tool previously left the grid at world zero, which intersected the Spot model;
it now computes transformed visible mesh bounds and places the grid below them
without changing the fixed camera or USD transforms. The Vulkan recapture at
`target/usd-review-spot-grounded.png` was inspected: the grid no longer cuts
through the body. Grid density remains visually excessive at this camera and
needs a separate presentation pass. This does not establish full viewer UI or
reference-renderer fidelity. All 252 workspace tests, check-all, viewer build and
diff whitespace checks pass at this checkpoint.

Environment-map conversion now exposes `route::environment_map::latlong_cubemap`:
Y-pole USD/OpenEXR longitude orientation, linear-space bilinear filtering,
sRGB decoding, tint and finite HDR RGBA16Float output in cube-face order.
Sources require complete CPU-resident data, a 2:1 layout and at most 16M pixels;
faces are power-of-two up to 1024. Invalid radiance and half-float overflow fail
explicitly. Tests cover axes/poles/seam, HDR, sRGB filtering and malformed input.
Bevy 0.19 already provides runtime cubemap filtering through
`GeneratedEnvironmentMapLight`; the previous dome module description was stale.
At the converter checkpoint, camera ownership/rotation, runtime filtering
captures and DomeLight_1 poleAxis remained unimplemented. Conversion alone is not
environment-lighting acceptance.
All 228 workspace tests, check-all, viewer build and diff whitespace checks pass
after the converter addition; no new environment-map render acceptance yet.

Dome textures now participate in source snapshot image requests and project as
`UsdDomeTexture` handles. Dependency probing reads authored sample times as well
as defaults, including asset paths that have no default value. The Bevy HDR
decoder is enabled; dome textures are loaded as linear radiance. Named-source
tests preserve values above one, switch files at independent instance time and
reload changed HDR bytes into the projected handle. A USDZ test loads both
sampled HDR entries and converts them without external filesystem paths.
Non-HDR dome color-space metadata and EXR decoding remain open.
All 230 workspace tests pass after sampled asset probing was restricted to
asset-valued attributes; check-all, viewer build and diff whitespace checks pass.

`UsdDomeEnvironmentPlugin` now provides explicit camera selection, conversion
and Bevy runtime filtering through an owned child generator. It restores the
previous camera environment on deselection/hide/despawn and preserves external
replacement on cleanup. Changes to source image/tint/resolution rebuild the
generator; intensity/rotation changes reuse maps. Camera despawn owns generator
cleanup. DomeLight_1 pole alignment is projected separately from child transforms.
The capture tool's `USD_CAPTURE_DOME=/Env` mode disables directional/ambient light
and waits for environment attachment plus pipeline readiness. The uniform warm
HDR fixture renders diffuse/metallic spheres; its zero-intensity control renders
black spheres. Both captures were inspected on Vulkan. This verifies illumination,
not directional-map orientation, physical intensity calibration, cross-platform
compute availability or filtering performance. Viewer selection UI remains open.
The pole test exposed that upstream DomeLight::get does not accept DomeLight_1;
the route now selects the corresponding typed schema view rather than silently
skipping versioned domes. Tests verify Y/Z/scene poles on a Y-up stage.
All 234 workspace tests, check-all, viewer build and diff whitespace checks pass
at this IBL checkpoint. Captures: `/dev/shm/usd-dome-ibl.png` and
`/dev/shm/usd-dome-zero.png` (volatile diagnostic storage).

The directional HDR fixture places a blue region toward +X in the unrotated
cubemap, with red elsewhere. Its decoding/cube-face test verifies that axis.
Fixed-camera captures at times 0/10 show the blue diffuse/specular contribution
moving right-to-left under a 180-degree dome rotation; both were inspected.
The two-camera regression verifies independent intensity/world rotation and
stable generator identities across those changes. Capture metadata now includes
applied intensity/rotation. This is not full pole/seam or cross-renderer fidelity.
Upstream Bevy 0.19 filtering_system and downsampling_system dispatch every frame
for active generators; this adapter currently leaves them active. Stable handles
therefore prove allocation reuse, not amortized GPU filtering. One-shot generation
with render-world completion feedback remains the next performance task.
All 236 workspace tests, check-all, viewer build and diff whitespace checks pass.

Owned dome generators now retire after both upstream GPU passes have recorded
their commands with all five compute pipelines available. A render-world marker
suppresses re-extraction while feedback reaches the main world; intermediate
textures/bind groups are released and the output map handles remain owned.
External generators are not retired. Source changes create a fresh generation;
stale/duplicate feedback cannot retire unrelated entities or double-count.
The capture waits for retirement and records one generation/zero active generators
after 60 settled frames. `/dev/shm/usd-dome-once-10.png` was visually inspected and
its raw pixels are byte-identical to the previous continuous-filtering time-10
capture. This proves removal of repeated filtering for the exercised fixture,
not a measured frame-time speedup or a GPU completion fence.
All 237 workspace tests, check-all, viewer build and diff whitespace checks pass.

The viewer now has a Lighting pane backed by a separate viewport-state bridge:
dome selection, attachment/error status, studio-only and studio toggle controls.
Studio lights carry an ownership marker; switching them does not modify authored
USD lights. Selection resolves by prim path across entity replacement, and missing
domes detach without silently selecting another. Startup diagnostics accept
USD_VIEWER_DOME and USD_VIEWER_PANE=lighting. A state test verifies camera ambient
restoration, studio intensity restoration, authored-light preservation and missing
dome cleanup. Full UI rendering/interaction acceptance remains unverified:
native launch failed with Wayland NoCompositor on the session display and the
listed compositor sockets. No viewer process remains running from these attempts.
All 238 workspace tests, check-all, viewer build and diff whitespace checks pass.

GPU skinning now validates the blended normal matrix for non-flat shading, using
the same finite/invertibility checks as CPU authored-normal skinning. Individually
invertible joint matrices can blend to a singular matrix; the regression covers
that case. If CPU deformation also fails, `UsdDeformationError` replaces the
previous silent rest-mesh fallback: generated mesh/subsets are suppressed while
unrelated runtime children remain. Valid live edits recover CPU and GPU geometry.
The capture tool rejects deformation errors with a prim-specific diagnostic.
`assets/skel_singular_normals.usda` has valid/zero-scale/valid poses at 0/10/20.
Time-10 capture failed explicitly without publishing a PNG; time-20 capture
succeeded and was visually inspected. Same-instance live repair is covered by
the CPU/GPU state regression, not by those separate capture processes.
All 240 workspace tests, check-all, viewer build and diff whitespace checks pass.

Deformation now preserves all authored normal interpolations, including indexed
constant/uniform/face-varying values. Normal evaluation carries an explicit
source-point map: per-corner normals receive that point's morph deltas and skin
influences without merging seams. Uniform normals expand to face-varying output;
constant normals expand to per-point output. GPU morph preparation accepts these
authored layouts. Indexed constant base-mesh normals now honor their lookup index.
Tests compare CPU normals with GPU vertex math across five interpolation modes
and three times, plus a two-joint case with different influences at each source
point. The former CPU subset test now checks retained authored corner normals
instead of accepting their loss. Matched time-10 CPU/GPU seam captures differ by
at most one RGB level; the GPU image was inspected. Generated smooth normals,
tangent deformation, broader rendered corpus and UI acceptance remain open.
All 242 workspace tests, check-all, viewer build and diff whitespace checks pass.

`sync::author_component_value<T>` accepts a registered reflected component value
without requiring a source entity or string type name. Both authoring entry
points now write their fields in one upstream batch transaction mapped through
the selected edit target, so a later field failure rolls back earlier writes.
At this checkpoint short-name ambiguity and read-only proxy/prototype owners were rejected.
Tests cover direct-value projection, rollback, non-root targets and instance-proxy
rejection. `examples/typed_authoring.rs` combines typed Sphere schema authoring
with a Health value, then projects two independent instances and edits one.
This API patches encodable fields; unsupported fields/absent options remain
omitted. Complete serialization diagnostics, component replacement semantics and
broader typed scene composition remain open; this is not full BSN equivalence.
All 247 workspace tests, check-all, viewer build, diff whitespace checks and the
`make run RUN_WITH= APP_TARGET='--example typed_authoring'` example pass.

Typed component authoring now rejects unsupported fields before stage mutation.
`component_field_issues` reports nested field paths/types and distinguishes
unsupported values from deliberately omitted supported `Option::None` patches.
Tests cover unsupported arrays/options, unchanged layers after rejection,
legacy encodable-field patch behavior and supported absent options preserving
existing opinions. Unsupported Option::None no longer leaks an enum token.
Unrepresentable empty marker/root-value components report an explicit diagnostic
instead of a misleading successful no-op. Component-presence encoding and full
replacement/serialization support remain open.
All 250 workspace tests, check-all, viewer build and diff whitespace checks pass.

Material subsets now have an actual projection route after CPU deformation:
named generated children carry subset meshes/materials while the original mesh
retains unassigned faces. Subset-only indices and bound shader animation cause
the parent mesh to resample. Live child edits/type changes invalidate consumers.
Tests cover independent clocks, empty subsets, malformed membership and repair,
owned-child cleanup, material sidedness and CPU morph geometry. The bundled
`assets/material_subsets.usda` renders two panels with swapped assignments at
time code 10. Endpoint captures were inspected, but differing camera framing
prevents treating them as a controlled image regression. Vertex buffers are
duplicated per subset; GPU subset deformation remains open. Earlier references
to subset CPU fallback did not prove subset rendering.

Direct point prototypes now reuse the subset route after prototype-local TRS
baking. Tests verify shared subset mesh/material handles across instances,
independent clocks, stable generated identities, subset removal and invalid
prototype cleanup without deleting unrelated runtime children. Subset preparation
is now separate from entity reconciliation and runs once per prototype per
projection. A no-global-cache regression compares 1 versus 64 instances: both
allocate four mesh assets and the same number of material assets. This proves
allocation scaling, not a frame-time improvement. Hierarchical prototypes remain
open. `assets/point_material_subsets.usda` is the layered three-instance example.

Direct mesh prototypes now use the same CPU skin/morph mesh evaluation as the
ordinary mesh route, before prototype TRS and subset preparation. Tests compare
both paths at independent times, including subsets, live blend-shape edits and
malformed skin-influence recovery. Zero-weight morphs retain base geometry rather
than disappearing. Unevaluable prototypes omit geometry with an instancer warning.
`assets/point_deformation.usda` demonstrates three instances of the skinned bar.
GPU prototype deformation and measured deformation throughput remain open.
Fixed-camera GPU readbacks exposed missing/reversed faces and duplicate/internal
caps in the skinned-bar fixture. It now has 14 outward-wound quads; tests check
closed edges and positive signed volume across poses. Rest/bent captures show
closed bars after repair. Polygonal/bilinear meshes without authored normals now
use flat triangle normals instead of smooth averaging, matching Storm's scheme
selection. Tests cover winding, subset layout, authored-normal preservation and
GPU influence mapping for duplicated triangle vertices. This is not broad
rendering acceptance; additional GPU-deformed normal cases remain unverified.

`examples/viewer_capture.rs` provides fixed-camera/time offscreen GPU readback,
PNG/raw RGBA/settings outputs and explicit failure exits. Two repeated time-30
captures on the tested Vulkan adapter have byte-identical raw pixels. This
captures the USD rendering pipeline and shared studio environment, not Mara UI.
Further captures exposed a pipeline-compilation race in the initial 60-frame
delay. The tool now enables synchronous compilation and requires 60 consecutive
frames without pending/failed GPU pipelines. Earlier empty captures alone cannot
prove missing geometry. Pipeline-ready captures show the rest bars without the
previous diagonal smooth-normal gradients.

Matched ordinary CPU/GPU skin captures at time 30 confirmed one hierarchy-visible
GPU mesh versus zero in CPU mode. Their silhouettes/poses agree visually, but the
GPU path interpolates transformed rest normals across the bend while CPU flat
normals remain face-constant. Raw comparison reports 3,195 changed RGB pixels
out of 921,600, maximum error 177 and whole-image mean error 0.092863 on the tested
adapter. Background dominates that mean; it is not fidelity acceptance.
`examples/capture_compare.rs` reports raw RGB errors with explicit tolerance and
nonzero mismatch exits. Identical repeated captures report zero changed pixels.
GPU flat shading now uses an extended StandardMaterial with geometric fragment
normals computed from GPU-deformed positions. The matched time-30 capture reduces
the difference to 437 RGB pixels (maximum error 177, whole-image mean 0.064069);
face lighting is visually flat, but the remaining difference is not resolved.
Authored normals retain the standard material path. Lifecycle tests cover asset
reuse, changed material settings, restoration and externally replaced handles.
The capture tool now selects forward, prepass or deferred rendering explicitly
through `USD_CAPTURE_RENDERER`, records the choice and rejects unknown modes.
Pipeline-ready GPU captures of the bent bar succeed and have been visually
inspected in normal/depth/motion-vector prepass and deferred modes on Vulkan.
Those modes disable MSAA. This verifies the exercised shader variants and final
color output, not the contents of each auxiliary prepass buffer or motion across
frames. Normal-mapped and double-sided render acceptance remain open.
The matched CPU/GPU deferred capture differs at 835 of 921,600 RGB pixels
(maximum channel error 177, whole-image mean 0.064163). It is not pixel parity;
the earlier 437-pixel result used forward rendering with different MSAA settings.
Diagnostic difference images exposed a triangular CPU/GPU mismatch: CPU polygon
triangulation was selected after deformation, while GPU skinning used the sampled
pre-deformation triangulation. `ReadMesh::triangulation_points` now preserves
those reference positions during CPU skin/morph evaluation, including direct
point prototypes and subset layout. Normals still use deformed positions.
Regression tests exercise the bent fixture, per-vertex CPU/GPU source mapping
and every face subset. The corrected deferred capture differs at 472 pixels,
all by at most one RGB level (whole-image mean 0.000172); the triangular region
is gone in the inspected diagnostic. This is one fixture/pose/adapter result,
not broad renderer parity. `capture_compare` can save diagnostic images and
reports changed-pixel bounds with tested tolerance/dimension validation.
The corrected matched forward capture differs at only six pixels, also by at
most one RGB level (whole-image mean 0.000002). Workspace tests pass (211),
and check-all/build/diff whitespace gates pass after this correction.
Backface validation adds `assets/skel_double_sided.usda` and a reversed-winding
front-face reference in `assets/skel_backface_reference.usda`. Initially CPU/GPU
backface captures were byte-identical but both were dark, differing from the
front-face reference at 3,724 pixels (maximum RGB error 162). Tangents generated
from fallback UVs disabled Bevy's untextured backface normal correction. Meshes
without authored UVs now omit tangents; the corrected GPU backface capture is
byte-identical to the reference and has been visually inspected. A regression
test checks that authored UVs still generate tangents. This exercises a flat,
untextured root-joint face in forward mode; textured backfaces remain open.
The corrected CPU backface capture is also byte-identical to the reference.
After this fix, 212 tests pass along with check-all, viewer build and diff checks.

CPU and GPU skin readers now sample joint indices/weights at the instance time,
including bindings with samples but no default influence arrays. The upstream
`SkinningResolver` caches default influences; CPU evaluation now uses its static
joint remap/bind transform with sampled influences passed to upstream LBS math.
Influence-only time variation is detected even with a static skeleton pose.
The new regression checks interpolated weights, held integer indices, matching
CPU/GPU vertex math, missing defaults, malformed sampled lengths and recovery at
a valid time. Negative/nonfinite CPU weights are rejected before deformation.
All 213 workspace tests pass. Rendered animated-influence and independent-clock
acceptance remain open, as do rigid influences and GPU morph/prototype work.

`assets/skel_influences.usda` now isolates influence-only animation: the joint
pose is static and sampled weights without defaults translate a quad. GPU
captures at times 0/5/10 were visually inspected with a fixed camera; midpoint
CPU/GPU captures differ at three pixels by at most one RGB level. Metadata
confirms one hierarchy-visible GPU-skinned mesh. The independent-instance test
checks GPU weight buffers, stable entities/joints and root-local live edits.
The CPU point-prototype/subset test now exercises this fixture with independent
clocks, malformed sampled weights and recovery. All 214 tests and check-all pass.
This covers the simple weight-only fixture, not all influence animation,
normal-mapped deformation or throughput acceptance.

Constant-interpolation skin bindings now deform in CPU and GPU paths instead of
being rejected. CPU evaluation uses upstream's shared weighted matrix without
allocating per-vertex influence arrays. GPU packing repeats the one influence
set over source points, retaining the four-influence limit. The layered
`assets/skel_constant_influences.usda` fixture matches its expanded vertex
equivalent at four sampled times in CPU results and GPU buffers; malformed
constant lengths are rejected and valid samples recover. Its midpoint GPU
capture is byte-identical to the vertex-influence capture, with GPU skinning
confirmed in metadata. All 215 tests pass. Broader constant-binding transforms,
normal fidelity, independent-clock and performance acceptance remain open.

Ordinary GPU-skinned material subsets now share the parent's SkinnedMesh joint
palette and inverse-bind asset. Split meshes retain joint attributes, and each
flat-shaded subset uses its own extended material. The original material handle
remains available for unbound-subset inheritance. CPU transitions remove owned
subset skin/culling/flat-material state; tests verify palette sharing, split
indices, stable subset entities, joint cleanup and CPU-to-GPU recovery.
`assets/skel_material_subsets.usda` renders orange/blue faces on the bent bar.
The inspected time-30 GPU capture differs from matching CPU output at seven
pixels, each by at most one RGB level. Capture metadata now counts actual
SkinnedMesh components, including subset children (three here, including the
empty remainder parent). All 216 tests, workspace checks and viewer build pass.
GPU point-prototype deformation, broader subset animation/material acceptance
and measured throughput remain open.

GPU morph preparation now has `read::skel::morph_sample`: dense source-point
position targets plus independent weights. Sparse duplicate indices accumulate;
invalid counts/indices/nonfinite values fail explicitly. Inbetweens become
fixed linear channels with piecewise weights matching upstream CPU clamping,
instead of rebuilding interpolated position offsets at each weight. Tests
compare seven weights against CPU evaluation and assert unchanged target data;
all 218 workspace tests pass. This is not yet a GPU morph rendering route.
Next integration steps are Bevy's `morph` feature and target-major
`MorphAttributes` buffers, morph-aware mesh cache identity, owned weight/culling
lifecycle, subset weight sharing, normal fidelity and rendered comparisons.
Bevy 0.19 uses mesh-owned morph buffers and supports up to 256 weights; the
current cache hashes only indices/vertex attributes and must include morph data
before those meshes can be safely interned.

Standalone morphs now evaluate authored vertex/varying normal deltas instead
of dropping authored normals on CPU or rejecting them on GPU. MorphSample
contains parallel dense position/normal channels; absent normal offsets are
zero, and authored base-normal indices expand before CPU evaluation. GPU
buffers carry normal deltas and retain StandardMaterial instead of the flat
extension, including subsets. Tests cover indexed normals, sparse offsets and
intermediate weights. `assets/morph_normals.usda` has an inspected time-10 GPU
capture byte-identical to CPU output, with two GPU morph meshes in metadata.
All 223 tests, workspace checks and viewer build pass. Combined skinned
authored normals, face-varying normals, generated smooth-normal updates,
normal-map tangents and broader inbetween-normal acceptance remain open.

CPU skinning now preserves authored vertex/varying normals and transforms them
by the inverse transpose of the blended mesh-local skin matrix after applying
morph normal deltas. This supports vertex and constant influences and matches
Bevy's normal transform, enabling combined GPU skin/morph authored normals.
The numerical regression checks rotation/nonuniform scale at three weights and
rejects a singular CPU normal transform. `assets/skel_morph_normals.usda` has an
inspected combined GPU/subset capture differing from CPU at one pixel by one
RGB level. Metadata confirms two skin and two morph meshes. All 224 tests pass.
Face-varying normals, generated smooth-normal updates, normal-map tangents,
singular GPU-normal handling and broad weighted-joint/animation acceptance
remain open.

Bevy's `morph` feature is now enabled and `gpu_morph` projects standalone flat
polygonal blend shapes into target-major MorphAttributes buffers and per-entity
MeshMorphWeights. The existing GPU plugin enables this route; unsupported
combined skin+morph, subsets and authored/smooth-normal cases keep CPU fallback.
Sparse point offsets expand over render-vertex seams. Morph positions/normals/
tangents and target names now participate in cache equality and hashing.
Tests cover cache separation, handle reuse across weights, sparse expansion,
owned material/weight/culling cleanup and external replacement preservation.
`assets/morph_animation.usda` has a pipeline-ready inspected GPU capture at time
10; matching CPU output differs at 96 pixels by at most one RGB level. Metadata
confirms one GPU morph mesh and no GPU skin meshes. All 220 tests, workspace
checks and viewer build pass. Full GPU morph acceptance remains open, including
inbetween rendered sequences, independent clocks, subset/combined deformation,
normal deltas, normal-mapped tangents and measured throughput.

Standalone GPU morphs now propagate to material-subset children: split meshes
retain the parent target data and receive matching per-entity weights and flat
materials. Owned morph/culling state is cleared before a different deformation
path takes over. Tests cover two independent clocks, sampled face membership,
parent/child target equality, stable generated children and GPU-to-CPU cleanup.
`assets/morph_subsets.usda` has an inspected time-10 orange/white GPU capture;
CPU output differs at 105 pixels by at most one RGB level. Metadata confirms two
GPU morph meshes. All 221 tests, workspace checks and viewer build pass.
Combined skin+morph, authored/smooth normal deltas and full fidelity/performance
acceptance remain open.

Combined GPU skin+morph now uses target and joint attributes on the same mesh,
including material subsets. `deformation` tracks shared culling claims so skin
and morph removal cannot prematurely clear or permanently retain the override.
Tests cover both release orders, prior application culling state, combined
subset GPU/CPU recovery and combined-to-skin-only-to-CPU transitions. The
`assets/skel_morph_subsets.usda` capture was visually inspected; tip offsets
include a direction affected by joint rotation to exercise morph-before-skin
ordering. Matching CPU output differs at 18 pixels by at most one RGB level;
metadata confirms three skin and three morph meshes, including the remainder.
All 222 tests, workspace checks and viewer build pass. Authored/smooth normal
deltas, broader combined-animation/inbetween acceptance and throughput remain
open; this is not completion of the overall deformation requirement.

Implemented `UsdSource`: immutable byte-backed root snapshots, independent stage
opening, source-relative filesystem dependencies and in-memory USDZ entries.
`UsdSnippet::open_stage` no longer writes temporary files; `open_stage_at`
provides an explicit source anchor. Root-layer export retains the source content.

Tests cover independent stages, malformed input, source-relative references with
an absent root file, and relative references inside an in-memory USDZ package.
Package entry reads are bounded to 256 MiB. Nested packages remain unsupported.

The AssetServer loader now discovers dependencies into a snapshot through
`LoadContext::read_asset_bytes`, preserving the Bevy source ID. Snapshot
resolution does not fall back to the filesystem and rejects paths outside the
asset source. Neither the asset loader nor snippets materialize temporary files.

Added Loading/Ready/Failed states, revision-driven static subtree replacement,
and cleanup when UsdSceneRoot is removed. Unowned root children are preserved.
Tests use a named in-memory Bevy AssetSource for relative external layers/assets,
explicit root reload, USDZ, missing references/package members, and cleanup.
Snapshot PNG/JPEG material images now become labeled Bevy assets, with tracked
handles on UsdScene. Color/emissive images are sRGB; normal/scalar images are
linear, with distinct handles when one file serves both usages. Material
projection uses the snapshot handles in an instance-scoped context. USD asset
paths use their resolved source-layer locations.

Real AssetServer tests now cover automatic layer and texture dependency events
from a named source, decoded pixel values, color spaces, material handle binding,
and packaged texture images. 116 tests pass at this checkpoint. These are data
and material-assignment tests, not screenshot/fidelity acceptance.

Each root now retains a separate `LiveStage` and `PrimEntities` map in the
`UsdInstances` non-send resource. Same-asset reloads reconcile matching paths,
preserving their entities and runtime-only components. `UsdInstanceTime` samples
each instance independently. `UsdInstanceOverrides` reapplies root-local variant
and attribute opinions on reload; removing opinions restores source values.
Direct stage edits are transient across reloads. Failed reloads retain the last
good live stage. Root removal/despawn releases retained stages.

Tests cover independent sampling and edits, variant/attribute isolation and
reload/removal, matching-entity identity, runtime-component preservation,
structural reconciliation, global-resource isolation and failed-reload playback.
`UsdPlayback` now advances each root using Bevy time and the stage's time-code
rate, with pause/play, signed speed, looping and explicit or authored ranges.
Tests cover reverse playback, endpoint stopping, looping, invalid/zero ranges
and independent projected animation.

A real AssetServer test reloads two roots through a named source event, verifies
entity/component preservation, then replaces/removes roots while retaining
unowned children. A regression test distinguishes the last successfully loaded
asset from a failed replacement, preventing runtime-component transfer between
different assets after recovery.

Performance tuning and editor integration remain. Snapshot dependency discovery
currently follows the composed selection; variants introducing previously
unloaded external files need additional dependency discovery. Checkboxes above
stay open until their complete acceptance evidence is in place.

Remaining material fidelity work includes automatic color-space inference and
graph operations. Snapshot metallic/roughness packing is described
below; it does not yet cover custom samplers or differing texture resolutions.

## Composition editor foundation

Added `editor::EditorSession` over the authoritative stage, plus a sendable
`EditorBridge` command queue and owned inspection snapshots. `EditorPlugin`
processes commands on the main thread and projects the same stage through
`LiveStagePlugin`. Failed opens preserve the existing document and projection.

Commands cover selection, typed attributes, variants, prim definition/removal,
layer edit targets, undo/redo, and explicit root-layer/edit-layer/flattened saves.
Inspection exposes composed attributes, resolution source, layers and variants.
Undo uses upstream `UndoStage` transaction inverses rather than reauthoring a
previous composed value; grouped attribute creation/value edits undo together.
Failed edits roll back their captured transactions.

Tests cover unauthored fallback restoration, failed-edit rollback, shared-stage
projection, layer-target undo/redo, flattened save/reopen and command transport.
The viewer now uses the bridge instead of a separate read-side stage. Outliner
selection and visibility author to the rendered document. Properties expose
layer edit-target buttons, stage-derived variant choices, scalar/string/token and
three-vector editing, plus resolution-source readouts. Other value types remain
read-only. Ribbon actions provide undo/redo and explicit root/edit-layer/flattened
save dialogs rather than a hard-coded output filename.

History currently retains all transactions; retention limits need hardening
before production acceptance. External layer edits now invalidate the command
history and refresh the snapshot, preventing undo from reverting the wrong edit.
Payload load/unload commands now synthesize projection resyncs; unloaded payload
roots remain selectable. These runtime load-rule changes do not enter layer-edit
undo history. Variant choices are gathered through contributing prim spec sites,
including referenced namespaces and unselected sets, and invalid names are rejected.

Tests cover payload unload/reload projection, referenced variant choices and
selection undo/redo, invalid selections and external-edit history reset.
Richer value editors, viewport selection/highlighting and
rendered interaction acceptance remain. The private Xvfb capture attempt failed
in GLX startup before rendering and is not visual acceptance evidence.

## Authoring API progress

Macro quoted interpolation now escapes USD delimiters, backslashes and whitespace
escapes. Bare interpolation is restricted to sealed built-in numeric/boolean
types; asset/prim-path interpolation and interpolation after a literal escape
are rejected. Runtime identifier/type validity still requires parse/open.
Tests verify malicious quoted content round-trips without creating injected prims.

Typed reference authoring accepts canonical upstream `sdf::Reference` values,
validates target paths/time mappings and exposes replace versus clear semantics.
Editor reference commands are undoable. Tests compose the same relative external
asset at two locations, undo/redo one location, reject a property-path target,
and save/reopen the assembly. A broader reusable typed scene-building API remains.

## Native instance projection

Projection and editor traversal now descend into native instance proxies instead
of stopping at instance roots. `UsdNativeInstance` and `UsdInstanceProxy` record
stage-local prototype identities on entities. Standalone `UsdAssetPlugin` now
installs mesh interning as well as `UsdPlugin`.

An integration test projects 128 native instances of one prototype: all child
geometry is present, correctly parented, and the source plus instances share one
`Assets<Mesh>` entry. Editing the source geometry updates all instance meshes
while preserving proxy entities. This measures asset sharing, not frame rate or
CPU projection throughput; routes still reconstruct geometry before interning.
Material sharing, prototype-aware build reuse, animation coverage and measured
rendering/performance remain before full native-instance acceptance.

Mesh interning now hashes every vertex attribute and compares full geometry on
hash matches. Tests cover secondary UV differences and forced hash collisions;
the previous selected-attribute/hash-only behavior could alias distinct meshes.

## GPU skinning checkpoint

`UsdGpuSkinningPlugin` enables Bevy `SkinnedMesh` projection for classic-linear
bindings with 1–4 normalized vertex influences, up to 256 joints,
no material subsets and no blend shapes. Stage-evaluated palettes
include geometry bind transforms and mesh joint remapping. Joint globals are
updated after Bevy transform propagation, including the mesh's world placement.
Unsupported cases retain CPU deformation with `UsdCpuSkinFallback` diagnostics.

The viewer enables this path; `USD_CPU_SKINNING=1` selects the CPU baseline.
`USD_SCREENSHOT=/tmp/view.png USD_CAPTURE_TIME=30 make run ...` captures the Bevy
viewport at a fixed time code (not the Mara overlays). Tests cover CPU-position
parity at four samples with a nonidentity geometry bind, mesh/joint reuse,
world placement and GPU-to-CPU fallback cleanup.

Wayland captures `/tmp/usd-gpu-t30.png` and `/tmp/usd-cpu-t30.png` were inspected:
matching visible silhouettes, differing shading from the different normal
treatment. Whole-image normalized RMSE was 0.00643075; the large unchanged grid
means this is not a geometry-region fidelity metric. Broader skinning/morph,
subset support, culling bounds and performance
acceptance remain. The GPU route currently disables frustum culling for safety.

GPU influences now follow an explicit render-vertex-to-USD-point map for
face-varying/uniform expansion. A regression test covers UV seam copies and
coincident points belonging to different joints; all emitted vertices match
their CPU-deformed source points. CPU and GPU animation sampling now map joint
names through `AnimMapper`, using per-joint rest transforms for missing animated
joints. A reordered-animation fixture matches the original at four time samples.
This removes the prior count-only CPU check and GPU order restriction.

## Material channel fidelity

Scalar texture reads now retain the connected r/g/b/a output. Snapshot roughness
and metallic images pack into Bevy's green/blue channels in a linear RGBA8 image;
missing channels use neutral factors. Generated handles are cached by sampled
pixel content, with a 64 MiB cache budget and 16M-pixel input limit. Different
resolutions or nondefault samplers produce `UsdMaterialWarning` rather than
silently assigning an incompatible texture. CPU packing still needs profiling.

Tests load a channel-connected material through AssetServer, verify packed bytes
and neutral material factors, then change the source image and verify automatic
repacking. Unit tests cover missing-channel defaults, reuse and invalid channel
indices. Constant diffuse values now use linear color, and positive USD opacity
thresholds select Bevy alpha masking. These are data/assignment tests; rendered
material fidelity remains unverified.

## Source-backed editor images

The standalone editor now resolves PNG/JPEG material images from filesystem and
USDZ sources into the same snapshot texture resource used by material projection.
Open prepares images before replacing the current document; failed opens preserve
the previous stage and entities. Edits, undo/redo, payload changes and detected
external stage edits refresh textures and reconcile material consumers. Unchanged
image content reuses handles; selection alone does not reread images.

Filesystem and package tests verify diffuse and packed scalar pixels, shader-edit
projection, handle reuse and failed-open preservation. Offline `make check-all`,
`make test-all` (145 library plus one viewer test), `make build`, and
`git diff --check` passed. The Spot viewport capture shows the robot and grid,
but the distant framing does not establish material fidelity or UI acceptance.
Editor image refresh remains synchronous and conservatively reconciles the full
stage; automatic filesystem watching for this direct editor path remains open.

## Occlusion channel mapping

Occlusion connections now retain the selected r/g/b/a output and repack it into
the red channel sampled by Bevy 0.19.1. The content-keyed scalar cache includes
the output layout, preventing collisions with metallic/roughness images. Missing
or unsupported inputs produce material warnings instead of sampling the wrong
channel; independent packing failures are retained together in the warning.

Tests cover all four channels, handle reuse, absent/missing inputs, and an actual
AssetServer material whose green-channel occlusion changes after a watched PNG
reload. Offline check-all and test-all pass (146 library plus one viewer test).
These verify bytes and bindings, not rendered occlusion fidelity. Custom sampler,
automatic color-space inference, graph evaluation and per-texture UV semantics remain open.

## Opacity textures

Opacity connections retain the selected r/g/b/a channel. Material projection
packs that scalar into base-color alpha, retaining diffuse RGB with sRGB storage
and linear alpha. With no diffuse image, white RGB leaves the constant color
factor unchanged. The explicit opacity connection replaces incidental diffuse
image alpha. Positive opacity thresholds retain alpha masking; other opacity
textures enable blending. Generated images use a separate 64 MiB content cache.

Unit coverage checks all channels, RGB preservation, replacement rather than
multiplication of diffuse alpha, handle reuse, neutral RGB and resolution errors.
The AssetServer regression also checks opacity pixels and blending before and
after a watched image reload. Different resolutions and custom samplers fail
with material warnings. Rendered transparency fidelity, per-texture coordinates,
graph transforms and color-space metadata still require additional work.

## Explicit texture color spaces

`UsdUVTexture.inputs:sourceColorSpace` values `raw` and `sRGB` now override the
usage-based defaults. These overrides travel through source dependency requests,
image decoding, material handle selection, scalar packing and opacity packing.
Scalar RGB channels are linearized before packing; alpha stays linear. Unknown
explicit values return an error rather than silently choosing a color space.
Absent values and `auto` retain the existing color/emissive-sRGB, scalar/normal-raw
policy; image-metadata-aware USD auto inference is not yet implemented.

A real AssetServer test verifies that raw diffuse images remain linear and that
an sRGB scalar value of 128 becomes 55 in the packed linear image, while raw 128
stays 128. The full test suite passes with 148 library and one viewer test.
General color-management transforms and MaterialX color-space metadata remain open.

## Viewer scene framing

New documents now frame visible projected mesh world bounds automatically. The
fit accounts for rotations, scaling, translation and the narrower perspective
axis, with scale-adjusted near clipping and orbit limits. It runs once per new
document root rather than resetting subsequent user camera movement or edits.
Empty or invalid bounds wait for usable geometry instead of changing the camera.

Three viewer tests cover transformed bounds, aspect/scale fitting, invalid input,
one-shot framing and document replacement. Offline check/build and all 152 tests
pass. `/tmp/usd-framed-spot.png` was visually inspected: Spot is now centered and
large enough to inspect, rather than tiny against the grid. This viewport-only
capture does not validate UI overlays or material fidelity. The grid remains at
the USD origin (which crosses this robot's body), and animated/procedural bounds
and framing generated subset children need broader coverage.

## Grounded presentation and generated-child framing

Framing now includes visible mesh descendants of the document root, including
generated children without prim-map entries, and excludes unrelated viewer
geometry. The presentation grid is placed just below the world-space lower bound
with scene-scaled spacing and fade distance. This moves only the viewer grid;
authored USD transforms and origin remain unchanged. The adjustment remains
one-shot per opened document, not an animated floor-following behavior.

An ECS regression verifies generated child bounds, unrelated-mesh exclusion,
grid placement and unchanged scene transforms. All 153 tests pass; offline check,
build and diff checks pass. `/tmp/usd-grounded-spot.png` was visually inspected:
the grid now sits beneath Spot's feet rather than crossing its body. This is
viewport evidence only; UI acceptance and broader animated bounds remain open.

## Composition-aware namespace commands

`EditorEdit` now supports Rename, Reparent and Move through the upstream namespace
editor and the same layer-transaction undo model as other editor commands.
Selections inside renamed/moved subtrees follow the new path; removal clears the
selection, and undo/redo restores the corresponding selection. Attribute-only
history does not reset a later selection. The Properties pane exposes name and
absolute-destination inputs; a move can combine rename and reparent.

Headless coverage verifies subtree attributes, rename/reparent/move history,
rejected destination collisions, deletion undo, selection and exported-stage
reopening. A command-bridge test verifies that the live mesh projection changes
path and returns on undo. Editor namespace commands now retain affected Bevy
entities as described below. UI controls still need rendered interaction
acceptance. The editor commands use `EditorSession` layer transactions rather
than the removed composed-value inverse model.

## Namespace entity identity

Successful editor rename/reparent/move commands remap existing prim-map entries
and `UsdPrimRef` components before live reconciliation, including undo and redo.
Reparenting updates Bevy hierarchy immediately, so moving a child out of a parent
and then deleting that parent in the same command batch cannot recursively
despawn the moved entity. Reconciliation also restores the composed hierarchy.

The live command regression now verifies stable entity IDs, runtime-only state,
unowned child survival, path components, reparenting, undo/redo and move-then-delete
in one batch. These guarantees cover editor-command namespace changes; direct
external namespace edits still lack an identity mapping. Deleted prims remain
destructive to runtime-only state, and cross-stage identity is not inferred.

## Canonical authoring history

Removed the duplicate `authoring::EditHistory`, its inverse-operation model and
private helpers. Public low-level authoring functions remain; undoable operations
use `EditorSession`/`EditorEdit` directly, with migration instructions in README.
No compatibility wrapper or re-export preserves the obsolete history model.

Migrated variant undo coverage to the canonical session. The authoring regression
now proves that undoing Define on an existing prim preserves its original type
and attributes, and undoing a schema-fallback attribute edit restores exact
authored absence rather than writing the composed fallback. All 155 tests and
offline workspace checking pass. Bounded history retention and failure recovery
remain separate unfinished requirements.

## Runnable independent-instance example

`examples/independent_instances.rs` loads the bundled spinner through AssetServer
and verifies two instances with distinct clocks and an isolated size override.
It checks separate entity IDs, isolated clock advancement, and override removal
without replacing the surviving entity. The example runs headlessly through
the existing Makefile APP_TARGET override and is included in all-target tests.
`SUPPORT.md` documents the command and a capability/approximation matrix.
This is executable integration evidence, not the completed visual flagship
showcase or a performance benchmark; those remain in the full checklist.

## Constant material graph evaluation

MaterialX multiply, add, subtract and mix now evaluate constant scalar/color3
inputs, including chained operations and scalar broadcast. Mix uses
`bg + (fg - bg) * mix`; source reference:
https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/documents/DeveloperGuide/ShaderGeneration.md
Nonfinite arithmetic results fail explicitly. Each material-channel traversal
has a shared 256-input budget and a 16-link forwarding-chain bound; cyclic graphs
return errors instead of recursing indefinitely.

Tests read an actual USD material with chained color arithmetic and scalar mix,
then create a cycle and verify rejection. Additional tests cover overflow and
scalar/color subtraction. All 158 tests pass. Textured arithmetic is not evaluated:
math nodes retain their in1 fallback and mix uses its bg fallback, as prescribed
for unsupported operators by the MaterialX standard. This remains an approximation,
not full shader graph support or rendered material acceptance.

## Material approximation diagnostics

Decoded materials now carry deduplicated node-path warnings when textured or
unresolved arithmetic falls back to in1 or bg. Material projection retains these
in `UsdMaterialWarning` alongside packing errors. Editor snapshots expose graph
warnings for selected materials or bound geometry, and the Properties pane shows
them as readouts. Material read failures are reported in the snapshot rather than
discarding the rest of the selected prim's inspection data.

The graph regression verifies three specific fallback warnings, their presence
in editor snapshots, absence on supported constant graphs, and visible diagnostic
data for a cyclic graph. All 158 tests pass. Actual rendered UI warning acceptance
remains open; these diagnostics do not make textured arithmetic fully supported.

## Typed relationship targets

Added low-level relationship-target authoring and canonical editor commands for
setting an explicit target list and clearing only the local target opinion.
The API validates absolute prim/property targets before authoring, rejects root,
relative and variant-selection paths, and allows unresolved forward references.
Editor snapshots expose composed relationship targets. Properties provides target
editing with separate, explicitly labeled empty-list and clear-opinion actions.

A layered-stage regression proves that an empty list blocks weaker targets,
clearing restores them, undo/redo preserves those distinctions, invalid targets
leave the layer unchanged, and root-layer save/reopen retains the target list.
All 159 tests pass; workspace checking passes. Target forwarding/provenance UI,
relative target authoring and rendered interaction acceptance remain open.

## Native projection measurement harness

Added a runnable cached/uncached projection benchmark with three alternating-order
samples and counts up to 4096 native instances. It reports stage-open, initial
projection, prototype-edit reconciliation and empty-notice processing durations,
plus initial mesh/material asset counts. Assertions check actual proxy vertex
extents after editing, surviving entity IDs and expected mesh sharing. A small
run is also an all-target regression test.

An initial debug/headless 128-instance run produced 129 mesh assets without the
cache versus one with it, but 129 materials in both modes. Projection ranged
130–137 ms uncached and 131–143 ms cached; no CPU speedup is established. The
first stage-open sample also showed setup effects. These measurements prioritize
avoiding repeated geometry construction and sharing material assets; they do not
establish GPU memory use, frame rate or production scalability. Commands and
measurement boundaries are documented in SUPPORT.md.

## Material asset sharing

`UsdPlugin` now enables a material cache used by mesh/shape/curve/point defaults
and the material route. It retains at most 1024 strong handles. A debug-content
hash narrows candidates, then full reflected equality verifies the current asset
before reuse; mutated/missing assets cannot be mistaken for the requested value.
The hash is not trusted as proof of equality. Shared materials use ordinary Bevy
asset semantics: applications must clone before entity-local runtime mutation.

Tests cover default and textured reuse, distinctions between material values,
and rejecting a candidate modified by application code. The benchmark now toggles
both caches. A fresh debug/headless 128-instance run reports one mesh and one
material in cached mode, versus 129 each uncached. Projection remained roughly
133–137 ms in both modes, so this proves allocation sharing, not a CPU speedup.
All 161 tests pass; offline check/build and diff checks pass. Avoiding repeated
geometry reconstruction and profiling the cache's hash/equality cost remain open.

## Plane axis and material sidedness

Fixed plane rotation: Bevy Rectangle is Z-normal, unlike the Y-aligned solid
primitives, so planes now use their own axis transform. Width/length fallbacks
are two units. Plane dimension axes follow the OpenUSD schema:
https://openusd.org/dev/api/class_usd_geom_plane.html
The tests verify positions and normals for X/Y/Z with unequal width and length.

Projected default and bound materials now honor each gprim's composed doubleSided
value, including Plane's true fallback, by setting both normal treatment and
back-face culling. A live edit test checks switching to single-sided; a bound
material test verifies that two prims with different sidedness do not incorrectly
share the same material handle. These are geometry/material-data regressions,
not rendered back-face acceptance. Other primitive semantics still need review.

## Opt-in route CPU diagnostics

Added `ProjectionTimings` with per-route attempt/match counts and separate CPU
matching/application durations. Custom routes report their Rust type name unless
overridden. Instrumentation is absent by default; without the resource, dispatch
does not read the clock. The benchmark enables it with USD_PROFILE_ROUTES=1.
Tests verify opt-in behavior, project/patch accounting and nonmatching routes.

One instrumented debug cached 128-instance sample, cumulative over initial
projection and editing, measured visibility application at 23.7 ms, shape
application at 18.9 ms, native-instance application at 14.3 ms, unsuccessful mesh
matching at 13.4 ms, and unsuccessful material matching at 13.1 ms. These identify
additional candidates beyond mesh construction. Traversal, context creation and
animation discovery are outside these counters; this is not a complete profiler.

## Composed material binding resolution

Replaced direct-only relationship lookup with the pinned upstream
MaterialBindingAPI resolver. Viewer material lookup now requests preview purpose
with all-purpose fallback and respects ancestor bindings, binding strength and
collection membership. A public purpose-specific reader is also available.
Schema views use `new`, not `apply`, so lookup does not author API metadata.

Tests verify inherited versus local bindings, stronger ancestors, restricted
purpose precedence, all-purpose fallback, collection members/nonmembers and
unchanged root-layer text after reads. The material projection test now binds
on an ancestor and verifies child-specific sidedness remains correct. This adds
correctness beyond direct binding, not a speed optimization; ancestor/collection
resolution needs measurement and safe caching before claiming faster dispatch.

## Geometry-only material dispatch

Material matching no longer performs a binding lookup. Application first checks
for Mesh3d and material asset storage, then resolves the binding once. Non-rendered
transform/material prims no longer perform binding resolution or receive unused
material components. Existing bound-geometry and inherited-binding tests pass.
Material read/unsupported-surface failures now produce UsdMaterialWarning rather
than disappearing through Option conversion; removing a binding clears the warning.

The added regression covers non-geometry skipping, zero material allocation on
that path, unsupported bindings and warning recovery. All 167 tests pass. A
debug/headless 128-instance comparison measured cached projection medians of
149.0 ms before and 145.1 ms after, with overlapping/noisy samples. This is a
small local observation, not a production speedup claim; repeated binding work
is structurally removed, while larger costs remain.

## Metadata-only mesh matching

MeshRoute's fallback now checks the composed definitions of points and both
topology attributes instead of decoding ReadMesh during matching. Explicit Mesh
types keep the type-name fast path. Untyped and custom-typed geometry remain
supported; a relationship named points does not count as a geometry attribute.
The attach path still validates/decodes the actual geometry once.

A regression projects untyped and custom-typed referenced triangles and rejects
empty/relationship-only candidates. The debug 128-instance route profile now
measures unsuccessful mesh matching at 7.15–7.47 ms for 516 attempts, compared
with 13.37–13.41 ms in the earlier instrumented run. This is route-local evidence,
not an equivalent whole-frame improvement or release-build claim. All 168 tests
and offline workspace checking pass; larger geometry construction remains open.

## Geometry route lifecycle cleanup

PrimRoute now has a default no-op remove hook invoked when its predicate does
not match, on both projection and patch dispatch. Mesh, primitive shape, points
and curves routes track ownership of their render components and remove obsolete
geometry on type changes. Failed mesh/shape/curve reads and absent/empty point
data clear their owned geometry rather than leave a stale rendered object.
Cleanup removes generated mesh/material handles, bounds, GPU-skin attachment and
material warning, but leaves entity identity, unrelated components and children.

A live Cube→Xform→Sphere regression verifies disappearance/reappearance on the
same entity and survival of runtime-only state/children. An unowned mesh is not
removed by another route's cleanup. Route timing application counters include
cleanup even when match counts are zero. This addresses these geometry routes;
other schema markers and generated point-instancer subtree cleanup still need
their own lifecycle acceptance rather than assuming the new hook handles them.

## Point-instancer IDs and child lifecycle

Point-instancer children now carry UsdInstanceId and reconcile by authored ID,
falling back to array index only when IDs are absent. Reordering preserves entity
identity/runtime components. invisibleIds compares against those IDs and changes
visibility without destroying the child, so unmasking restores the same entity.
Removed IDs and type changes remove generated children, not unrelated children.
Duplicate/mismatched IDs retain the last projection with UsdInstancerWarning.
Prototype mesh/default-material creation now uses the shared caches.

Tests cover authored IDs, reorder, malformed-ID recovery, hide/unhide identity,
type-change cleanup and unrelated child preservation. These changes do not yet
implement inactiveIds, arbitrary prototype hierarchies, full prototype materials
or sampled point-instancer animation; the support matrix records those limits.

## Time-sampled point instancers

Added read_point_instancer_at while preserving the default-time reader. Positions,
orientations, scales and prototype indices now use the route's USD time code;
IDs and invisibleIds use the same time. Existing animation detection and root-local
clocks drive these updates without replacing surviving child entities.

An independent-instance regression opens one sampled source at times zero and ten,
checks distinct positions, scales, endpoint rotation and masking, then advances
only the first root to time five and verifies interpolated position/scale, stable
child IDs and an unchanged second root. All 171 tests pass. This samples authored
arrays through upstream interpolation; velocity/angular-velocity extrapolation,
prototype animation and rendered point-instancer fidelity remain unverified.

## Composed point-instancer inactive IDs

Point-instancer masking now unions composed inactiveIds metadata with sampled
invisibleIds. Hidden children retain IDs, entity identity and runtime state.
Invalid composed inactive metadata retains the previous projection with a warning.
The route consumes upstream's already-folded explicit Int64ListOp rather than
implementing list-op composition locally.

A layered typed-authoring regression verifies weak explicit IDs, stronger
prepend/delete operations, removal of the local opinion, explicit-empty blocking,
union with invisibleIds and unchanged child identities. The pinned upstream USDA
parser rejects prepend/delete inactiveIds as unknown prim metadata; the regression
therefore authors typed metadata through Stage, not a purported working USDA
round trip. File import/export acceptance for this metadata remains open.

Validation: make check-all, make test-all (172 tests), make build with offline
Cargo, and git diff --check pass. Logs: /tmp/usd-inactive-{check,tests,build}.log.

## Point-instancer bound preview materials

Direct mesh prototypes now resolve inherited USD material bindings through the
same material conversion function as ordinary geometry. This includes snapshot
textures, channel packing, opacity, sidedness and shared material-cache handles.
Each generated child carries material diagnostics; missing/unsupported bindings
fall back to the prototype's default material, and reprojection clears repaired
diagnostics without replacing child entities.

The regression checks inherited preview color/metallic, snapshot emissive texture
handles, double-sided culling, shared handles, updates on explicit reprojection,
invalid binding diagnostics and recovery with stable children. This is component
acceptance, not rendered texture fidelity. Arbitrary prototype hierarchies,
material subsets and dependency-triggered prototype/material updates remain open.

Validation: make test-all (173 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-pi-material-{tests,check,build}.log.

## Notice-driven material and prototype invalidation

LiveStage changed-info notices now trigger full reconciliation when they affect
Material/Shader/NodeGraph prims, material bindings, collections or point-instancer
prototype namespaces (including ancestors). Ordinary unrelated edits retain their
sparse patch path; suppressed echo changes do not themselves trigger this branch.
This is conservative dependency invalidation, not a targeted reverse-edge graph:
it scans projected prims for prototype targets and can rebuild unrelated geometry.

The prototype material regression now uses real apply_changes rather than manual
route invocation. Before this fix the shader edit left metallic at 0.8 instead of
0.3; it now updates both generated instances and ordinary prototype geometry.
The same test verifies prototype point edits, inherited-binding failure/recovery,
stable child entities and preservation of runtime-only components. A separate
invalidation test checks shader/nodegraph, binding/collection and prototype paths,
ancestor changes, empty changes and sibling namespace boundaries. Changes that
introduce previously unloaded texture files still require source/texture refresh;
sampled prototype animation remains separate work.

Validation: make test-all (174 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-dependency-{tests,check,build}.log.

## Sampled point-instancer prototype-root transforms

Direct prototype meshes now bake the root's local TRS at the instancer's current
time before applying per-instance placement. Prototype ancestors are excluded.
Animation detection includes direct prototype time samples, so an instancer with
static instance arrays still updates when its prototype transform animates.
The shared mesh cache receives the transformed geometry; this is CPU baking, not
a GPU prototype-transform palette. Singular/nonfinite prototype transforms omit
generated geometry rather than entering Bevy's singular-scale mesh transform.

A two-root regression uses an animated prototype translation, an unrelated 100-unit
prototype-ancestor offset and instance scale/translation. Its vertices land at
12/16 for root times 0/10, then 14/16 after advancing only the first root to time 5;
both generated entity IDs survive. This verifies local transform order and clock
isolation, not arbitrary shear, mirrored-normal fidelity or animated mesh points.
USD's prototype-root-as-most-local transform contract is documented at
https://openusd.org/24.08/api/class_usd_geom_point_instancer.html .

Validation: make test-all (175 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-prototype-xform-{tests,check,build}.log.

## Sampled mesh geometry and primvars

Added read_mesh_at, retaining read_mesh as the default-time API. Mesh points,
topology, normals, UV values/indices, display color/opacity, extent and read subset
indices resolve at the requested time through upstream interpolation. Ordinary
MeshRoute and direct point-prototype baking use this reader. Removed unused
default-only reader helpers rather than retaining parallel plumbing.

The regression covers a sample-only mesh with interpolated floating arrays and
held integer topology/UV indices. It checks reader values at time five and ten,
then projects ordinary geometry and a point-instancer prototype under independent
roots at zero/ten and advances only the first root to five. Both projections
update vertex extents/topology and retain entity IDs; the second root stays fixed.
Topology checks count rendered indices, not unique vertices (the mesh builder
legitimately shares vertices between triangles).

This remains CPU mesh reconstruction. Sampled base geometry combined with
skin/morph deformation, subset-only animation propagation and rendered animation
fidelity are not established by this regression.

Validation: make test-all (176 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-mesh-time-{tests,check,build}.log.

## Sampled base meshes through deformation

CPU skinning, CPU blend-shape evaluation, GPU influence validation and GPU mesh
attachment now read base geometry at the deformation time. SkinRoute rebuilds
with the same sampled topology and primvars rather than default-time geometry.
Existing influence-count validation uses the sampled point count, rejecting a
point-count change that no longer matches the authored influences.

New regressions author animated base-point offsets over the skeletal and morph
fixtures. At three times they verify interpolated base positions, compare Bevy
GPU palette math against CPU-skinned points, check the GPU vertex stream and CPU
route output, and check sparse blend offsets added to the sampled base. A reduced
point-count sample produces a GPU influence-count error and CPU unskinned fallback
rather than indexing mismatched influences. These are CPU-side palette/component
checks, not a new rendered GPU fidelity or performance acceptance. Time-sampled
joint influences and animated material-subset handling remain separate work.

Validation: make test-all (178 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-sampled-skin-{tests,check,build}.log.

## Sampled primitive dimensions and visibility

Shape dimension reads now use the route's USD time code for Cube, Sphere,
Cylinder, Capsule, Cone and Plane. Uniform axis configuration stays default-time.
Added read_visibility_at with the existing default reader preserved; VisibilityRoute
now samples local visibility and lets Bevy propagate hidden parent state.

One regression verifies interpolated dimensions for all six shape types. Another
opens two independent roots containing a sampled Cube under a sampled-visibility
parent, runs Bevy's VisibilityPlugin, and checks different dimensions and inherited
visibility at times zero/ten, followed by times five/twenty. Entity IDs survive,
held token visibility unhides correctly, and Bevy-generated Aabb half-extents
follow the rebuilt shape at every checked time. This is headless propagation and
bounds acceptance, not rendered animation fidelity.

Validation: make test-all (180 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-shape-time-{tests,check,build}.log.

## Camera and light route lifecycle

CameraRoute now removes its UsdCamera marker and Projection when a prim stops
being a camera. LightRoute records the generated light kind and removes that
component plus its area marker on type changes. Reprojection no longer blindly
deletes all three Bevy light component types, preserving unrelated runtime lights.
Neither route despawns the prim or touches unrelated children/components.

A notice-driven lifecycle regression covers RectLight→DistantLight→Camera→Xform,
SphereLight and shaped SpotLight removal, stable prim/child entities and preserved
runtime-only state. It also checks that an unowned Projection and PointLight
survive unrelated route cleanup and DistantLight transitions. Ownership is by
generated component type, not by individual runtime mutations to that same type;
applications should not independently overwrite USD-owned components and expect
the route to distinguish them. Other schema routes still need lifecycle review.

Validation: make test-all (181 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-light-lifecycle-{tests,check,build}.log.

## Sampled camera lenses and orthographic filmback

CameraRoute samples focal length, filmback dimensions/offsets, clipping range and
projection mode at route time. Orthographic cameras now use authored aperture
dimensions converted from tenths of a scene unit, rather than Bevy's default
orthographic extent. Fixed dimensions, viewport origin and initial area preserve
the orthographic aperture offset through viewport resizing. Perspective projection
records the authored filmback aspect; applications activating it still control
their render viewport's film-fit policy. Invalid/nonfinite clip ranges fall back
to a finite default, and scalar lens reads sanitize nonfinite values.

The regression verifies midpoint lens/clipping interpolation, held perspective→
orthographic mode switching, nonzero orthographic offsets, unchanged bounds after
wide/tall viewport updates and LiveStage time-scrub projection changes on the
same entity without attaching Camera3d. Perspective offsets, lens effects and
rendered camera acceptance remain open. Camera unit reference:
https://openusd.org/release/api/class_usd_geom_camera.html .

Validation: make test-all (182 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-camera-time-{tests,check,build}.log.

## Sampled UsdLux inputs

LightRoute now resolves common color/intensity/exposure, radius and area dimensions,
and shaping cone angle at the route's current time. Exposure is interpolated as
an authored value before applying its power-of-two multiplier. Existing approximate
USD-to-Bevy brightness scaling and area-light representation are unchanged.

Tests exercise all five supported light schemas at a midpoint: linear color,
intensity/exposure, radius, Rect/Cylinder markers and transition to sampled cone
shaping. A UsdSceneRoot regression checks separate DistantLight clocks at
zero/ten, advancing only the first to five without replacing either prim entity.
Dome-light animation, photometric normalization and rendered lighting acceptance
remain separate work; green component tests do not establish them.

Validation: make test-all (184 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-light-time-{tests,check,build}.log.

## Sampled material inputs and graph-dependent clocks

Added read_preview_material_at while retaining the default-time API. Numeric/color
inputs, supported constant arithmetic and UV transforms resolve at the root's
current USD time. Ordinary geometry and direct point-instancer prototypes share
this path. Animation detection follows the bound material's descendants and
external shader connections, with cycle detection and a 256-node bound that
conservatively treats oversized graphs as animated. Detection currently rescans
the graph; this is not a cached or profiled dependency index.

A two-root regression checks sampled color/opacity, connected arithmetic
roughness, UV translation and alpha-mode changes for both ordinary and instanced
geometry, including isolated midpoint scrubbing and stable entities. A separate
test proves external-only shader animation detection and termination on cyclic
connections. Texture-node file/color-space inputs remain default-time; animated
texture discovery/loading, global UV limitations and rendered fidelity remain open.

Validation: make test-all (186 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-material-time-{tests,check,build}.log.

## Rendered animation showcase

Added assets/animation_showcase.usda and README launch/capture commands. A plinth,
growing Cube and three shared tetrahedral prototypes demonstrate sampled size,
mesh geometry and material color/roughness over time codes zero to ten. The source
prototype is guide-purpose and does not appear alongside the generated instances.
An initial capture exposed inward-facing fixture triangles; their winding was
corrected and a regression verifies outward orientation at both endpoint shapes.

Launched the viewer on Wayland/Vulkan and inspected corrected captures:
/tmp/usd-showcase-t0.png and /tmp/usd-showcase-t10.png. Both show the plinth, one
Cube and three generated prototypes. The second shows a larger Cube, taller
prototypes and a blue material replacing the first frame's orange. Camera framing
is fitted independently per launch. Logs: /tmp/usd-showcase-t{0,10}.log; timeout
termination was intentional after capture. Existing clipboard/MESA layer/SSAO
warnings remain environmental or renderer capability warnings.

This establishes fixed-time rendered behavior for the bundled fixture only.
Interactive scrubbing, overlay interaction, IBL, normal fidelity against a USD
reference renderer and production performance remain unverified.

Validation: make test-all (187 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-showcase-{tests,check,build}.log.

## Viewer timeline controls

EditorBridge now accepts Seek and Play commands and publishes an owned timeline
snapshot. EditorPlugin advances StageTime before LiveStage's Update resampling,
reusing the existing UsdPlayback arithmetic for stage rate, half-open loops and
invalid-range handling. Seek pauses playback and rejects nonfinite values; opening
a document pauses playback. These operations do not author USD or enter history.

Added src/timeline.rs and a Timeline ribbon pane with time/range readouts,
play/pause, start, previous/next time code and validated typed seek. A headless
command regression verifies seeking updates a live transform, finite validation,
loop wrapping, pause, unchanged entity identity, unchanged USD export and no undo
entries. The pane builds, but clicks and full-host overlay capture have not yet
been verified: the available Bevy screenshot path only captures the viewport.

Validation: make test-all (188 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-timeline-{tests,check,build}.log.

## Metadata and physics lifecycle cleanup

Audio, volume, render-settings, procedural and backdrop routes remove their
projected markers after type changes. PhysicsRoute clears obsolete body/collider,
mass, joint, drive and limit markers both when no longer matching and when a
remaining physics schema is reprojected after another API is removed.

Live-notice tests transition one prim through all five metadata types and Xform,
preserving its identity, runtime-only state and child. Physics coverage removes
RigidBody/Mass APIs while retaining Collision, then removes Collision and changes
a fixed joint back to Xform, checking that no obsolete markers remain. Backend
simulation/audio/volume teardown is not implied: these routes expose data only.
Dome global ambient ownership remains a distinct outstanding lifecycle problem.

Validation: make test-all (190 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-marker-lifecycle-{tests,check,build}.log.

## Explicit dome ambient ownership

DomeLightRoute now emits sampled dome metadata and removes it on type changes;
it no longer writes GlobalAmbientLight as a projection side effect. Added optional
UsdDomeAmbientPlugin and per-camera UsdDomeAmbientSource(Entity). The adapter
updates only explicitly selected cameras, follows projected dome values and
inherited visibility, and restores the previous camera ambient on deselection,
hiding or missing source. Cleanup preserves a value changed externally since the
adapter's last write. While selected, camera ambient is adapter-owned.

Tests verify sampled dome intensity, no global resource creation, marker cleanup,
two cameras selecting different domes, isolated updates, visibility restoration,
source despawn and preservation of an external override at deselection. The
adapter runs after Bevy visibility propagation. README documents the migration
from implicit global ambient. The viewer already has camera-local studio ambient,
which overrides GlobalAmbientLight, and remains unchanged; no automatic dome
selection was added. Real dome texture loading/convolution/IBL remains open.

Validation: make test-all (191 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-dome-owner-{tests,check,build}.log.

## Point-instancer array validation and half quaternions

Point-instancer projection validates transform-array lengths, prototype index
sign/range, finite position/scale values and finite nonzero quaternion norms before
changing children. Malformed updates retain the previous projection and report
UsdInstancerWarning; recovery clears it. Valid quaternion inputs are normalized.
Reader errors and invisibleIds decoding failures now produce diagnostics instead
of silently clearing/reinterpreting the data. Absent required arrays still clear
generated children; an empty prototype target list retains headless placeholders.

Testing exposed that standard orientations are quath[], not quatf[]. Added
QuathVec decoding to the shared quaternion reader and converted the independent
clock rotation fixture to the schema's half-precision type. Regression coverage
checks malformed lengths, negative/out-of-range prototype indices, infinity,
zero quaternion rejection, recovery, preserved child transforms/identity and
normalization of a non-unit half quaternion. This does not add prototype hierarchy
support or guarantee diagnostics for every unsupported prototype shape.

Validation: make test-all (192 tests), make check-all, make build with offline
Cargo and git diff --check pass; /tmp/usd-instancer-validation-{tests,check,build}.log.
