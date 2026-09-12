# Bevy integration support

This describes the current checkout, not every capability of upstream OpenUSD.
Bevy is pinned to 0.19.1. OpenUSD uses the Git baseline in Cargo.toml plus the
review patches recorded in `vendor/openusd/VENDORED.md`.

## Current release blockers

- Kubota's reproduced GPU-memory failure is resolved by generated-image sharing:
  retained image payload drops from 12.43 GB to 557 MB and the viewer renders it.
  Ropa's before/after host captures are pixel-exact. Packed-output reuse reduces
  the sampled Kubota headless open from 12.60 to 7.22 seconds, preserving its host
  pixels. General performance and native material parity remain unqualified.
  Evidence: `benchmarks/machine-texture-memory.md`.
- A matched-camera/dome Kubota comparison still fails material fidelity. Adding
  omitted scalar clearcoat inputs halves the opaque hood ROI's mean RGB error,
  but its maximum remains 25/255. Glass/decals and remaining coat differences are
  unresolved. Evidence: `benchmarks/kubota-native-materials.md`.

The viewer supports experimental order-independent transparency with
the Rendering pane's Enable OIT button, or at startup with
`USD_VIEWER_OIT=1 make run ARGS='/path/to/scene.usdz'`. Disabling it restores
the viewer camera's previous MSAA and FXAA settings. It removes the observed
Kubota grille triangle-ordering patches, but does not fix its roof speckling.
This is opt-in, replaces MSAA with FXAA and uses additional fragment-buffer memory;
Bounded RTX 4080 GPU-pass timing comparisons are recorded in BEVY_WORK.md;
full-frame performance, fragment overflow and cross-device acceptance remain open.
If the shared OIT buffers exceed the device's buffer/binding-size limit, the
viewer disables its OIT and restores prior AA settings with a Rendering-panel
explanation. This does not measure free VRAM or prevent fragment-capacity overflow.

- Intermittent black frames remain a native-capture acceptance blocker. The
  standalone Khronos Vulkan cube reproduced the same visible-to-black failure
  under headless Weston Vulkan on NVIDIA, without Mara, egui, Bevy or USD.
  This establishes an independent graphics-stack reproduction, not a viewer
  rendering fix or identification of the faulty driver/compositor component.
- Spot/UR5 fidelity and broader native-render comparisons remain incomplete.
  Do not infer full-scene parity from bounded numerical or image fixtures.
- Per-layer content-based save state and guarded in-app close are implemented
  and tested. The local Mara develop-based host at 15a2e9d restores OS close
  routing through the guard, direct screenshots and dome-device configuration.
  It lives in the required sibling worktree `../mara-bevy-host`, not unmodified
  upstream develop. An isolated Xwayland WM_DELETE_WINDOW request now has
  rendered confirmation-dialog evidence, Cancel dismissal with the viewer
  remaining alive, and Discard exiting with status zero;
  this is not a cross-desktop/backend acceptance matrix.
  Source-aware opens detect external content conflicts; atomic conflict detection
  against concurrent existing-file writers remains open;
  Clean means matching a loaded or in-place-saved baseline, not unchanged disk.
- General typed scene-building, broader backend coverage and interactive/GPU
  performance acceptance remain incomplete, as qualified in the matrix below.

The full integration goal remains active. The [work log](BEVY_WORK.md) records
revision-specific tests, inspected images and failed probes; historical benchmark
numbers are not measurements of a later checkout. At 7e67bc4, the full workspace
gate passes 654 tests with 15 ignored across 33 suites. The separate Make
test-native gate executes and passes 14 native-export tests; check-all and viewer
build pass. Logs: /tmp/single-pass-{workspace-tests,native-tests,check,build}.log.
The older 23-test native run has a different scope; these counts are not a
release certification.

## Capability matrix

USDZ export bundles ordinary layer and asset dependencies through the stage's
resolver, preserving root/edit semantics and live edits. Moved-package native
checks pass for the covered scene and asset payload, including snapshot-only
package re-export and nested input packages. Expressions and tile/sequence patterns remain unsupported; failures retain the
old destination. Limits and remaining acceptance work are in `PACKAGING.md`.
Passing data tests do not establish rendered fidelity or production performance.

Nested USDZ reading supports implicit inner default layers, explicit bracket
paths and inner relative references/assets through filesystem and snapshot
resolvers without extraction. Each entry traversal is bounded to 16 package
levels and 256 MiB of decompressed entry bytes. This does not bound root archive
storage or total stage work. Export bundles nested dependencies as ordinary unique
entries, preserving layer composition and live edits rather than archive layout.

| Area | Implemented integration | Limits / outstanding acceptance |
| --- | --- | --- |
| Asset loading | Source-backed USD and in-memory USDZ, including bounded nested reads; relative layer and image dependencies through AssetServer | Dependency discovery follows composed variant selections; total stage memory is not bounded by entry-read limits |
| Reloads | Tracked AssetServer dependencies; last-good projection on failure; explicit editor texture refresh and opt-in native texture watching; failed watcher setups retry at one-second intervals; initial image read/decode failures retry on texture recovery only when no document is open | Native watching requires file_watcher; AssetServer removal handling uses the explicit file_source adapter. Editor watching excludes USD layers and package members; directory replacement recovery is Unix-only. Failed replacement opens never defer replacement of an existing document |
| Instances | Independent stages, clocks, playback, variants and attribute overrides | Direct stage edits are transient across asset reload |
| Identity | Same-asset reconciliation preserves matching prim entities; editor namespace commands remap entities | External namespace edits do not infer identity; deletion loses runtime state |
| Ordinary transforms | Affine residual propagation, USD reset-stack prefix/ancestor exclusion, preserved scene placement/up-axis; runtime descendants follow corrected globals; malformed matrix edits retain last-good globals and recover through the stage change sink; half scalar/vector/quaternion decoding; scalar-axis ops and adjacent inverse cancellation | Affine and Z-up reset fixtures captured against explicit geometry; quaternion conversion checked against native axis-angle cases. Scalar-axis inverse scale follows native 25.05.01 negation, not vector-scale reciprocal behavior. Singular lighting, broader animated/native coverage and propagation performance remain unverified. Non-finite orientations, overflowing axis lengths and uncancelled singular matrix/vector-scale inverses produce diagnostics. Point-prototype hierarchy limits below still apply |
| Schema lifecycle | Geometry, point-instancer, camera/light/dome, audio/volume, render-settings/procedural/backdrop and physics markers clean up after schema removal | Backend teardown still requires acceptance; runtime replacement of the same owned component type is not separately tracked |
| Materials | Preview surface channels including sampled scalar clearcoat/roughness; scalar/opacity packing; explicit raw/sRGB; constant-valued MaterialX multiply/add/subtract/mix; graph fallback diagnostics | Clearcoat textures explicitly unsupported; textured graph arithmetic still approximated; color management, auto inference and per-texture UV semantics incomplete; scalar coat mapping is not native BRDF parity |
| Material binding | Upstream inherited/direct/collection resolution, binding strength, preview purpose with all-purpose fallback | Binding-resolution performance and broader rendered corpus remain unverified |
| Material animation | Sampled numeric/color inputs, supported constant graph arithmetic, composed affine UV chains, sampled texture filenames and explicit raw/sRGB sourceColorSpace tokens; per-material reachable file/color-space sample discovery; independent live-clock UV/file captures against baked references | Animation detection retains a 256-node bound; texture-time discovery has a separate 4096-node bound. Filename-pattern expansion, general color management, different per-texture UV transforms and arbitrary shader networks remain unsupported or incomplete; fixture fidelity is not general renderer equivalence |
| Display primvars | Meshes/primitives, curves, point clouds and point prototypes inherit authored nonblocked constant display color/opacity; curves project indexed uniform/vertex/varying color/opacity with cubic vertex versus linear varying interpolation; point clouds project indexed vertex/varying color/opacity; meshes and mesh point prototypes inherit constant st/st0 UVs; local overrides, owner-local sampled indices and parent-edit invalidation | General shader primvar-reader networks and arbitrary per-texture UV sets remain incomplete; face-varying curve primvars and arbitrary nonconstant shape primvars are not projected |
| Texture packing | PNG/JPEG; content-cached scalar and opacity output | Matching resolutions and default samplers required; CPU cost not profiled |
| Geometry sharing | Content-checked mesh cache; native instance-proxy projection; bounded equality-checked standard and flat-normal material sharing; eight independent morph stages use one converted material versus eight with conversion caching disabled | CPU geometry is reconstructed before interning; flat conversions share by base-material asset ID, not across distinct equal base IDs; shared assets require cloning for entity-local runtime mutation; asset counts do not prove GPU speedups or production performance |
| Points and curves | Sampled point-cloud positions and BasisCurves positions/counts with independent clocks; periodic linear closure within each curve; CPU cubic tessellation with periodic Bezier/Bspline/Catmull-Rom closure tests; pinned Bspline/Catmull-Rom phantom endpoints including two-point curves; UsdCurveSettings selects 1–64 samples per cubic segment (default eight), with live-stage/independent-instance reprojection on changes; viewer launch setting USD_CURVE_STEPS; unbound point/line preview materials are unlit | One-pixel point/line rasterization, not width-aware USD surfaces; tessellation is fixed-rate rather than adaptive. Strict topology validation, curve display primvars and bound-material shading fidelity remain incomplete |
| Mesh time sampling | Points, topology, normals, UVs/indices, display color/opacity and extent; local or inherited constant `primvars:normals` takes precedence over `normals`, including sampled values/indices and parent edit refresh; independent normal clocks/edits validated on ordinary meshes, direct point prototypes and subsets; sampled base points and joint indices/weights feed CPU and GPU skinning | CPU mesh reconstruction; inherited-normal grazing-angle striping/reference parity and influence-animation rendered/multi-instance acceptance remain open |
| Generated mesh normals | Flat triangle normals for missing normals on `none`/`bilinear` meshes; GPU skinning uses geometric fragment normals; authored normals preserved; subdivision fallback remains smooth; triangle-corner layout shared with GPU influence mapping | More vertices for flat shading; subdivision limit normals, normal-mapped/deferred/double-sided GPU cases remain unverified; CPU/GPU captures are not pixel-identical |
| Subdivision surfaces | Control cages by default; opt-in finite CPU Catmull–Clark/bilinear refinement for ordinary meshes and direct mesh point-instancer prototypes, after control-point deformation; uniform-decay crease/corner position stencils and permanent-sharp normal splitting; authored holes retain refinement support but omit descendant faces; validated primvars and subset remapping; selected-prim diagnostics; limit-tangent normals on eligible smooth refined cages, with native numerical comparisons | No exact limit-position evaluation, Loop, Chaikin creasing or GPU subdivision. Limit normals require no remaining sharp features, no authored holes and a boundary mode other than `none`; unsupported rings fall back to finite normals. Catmull–Clark supports edge-only/edge-and-corner boundaries and boundary-face holes for `none`; face-varying input data supports all-linear mode only. Bilinear data stays linear. Non-manifold edges and duplicate sharpness entries remain rejected. Spot captures expose remaining seams; full-scene reference-renderer parity is not established |
| Material subsets | Ordinary meshes and direct point prototypes, per-subset preview materials, unassigned-face remainder, sampled membership/materials, stable generated children; CPU deformation and ordinary GPU skinning with a shared joint palette; compacted attributes and morph targets, with ANYmal retained-payload measurements in benchmarks/editor-assets.md | Overlapping/invalid faces fall back to the whole mesh with a diagnostic; GPU point-prototype deformation remains incomplete; peak RSS/VRAM and general performance remain unverified |
| Shapes and visibility | Sampled Cube/Sphere/Cylinder/Capsule/Cone/Plane dimensions and local visibility; constant display colors/opacities with sampled indices and single unindexed values; unbound fallback opacity selects blend/opaque mode; parent hiding propagates through Bevy | Shape mesh regeneration is CPU-side; multi-value nonconstant display colors/opacities and shape face subsets are not mapped; full animated rendering corpus remains unverified |
| Cameras | Sampled lens/aperture/clipping/projection mode; orthographic filmback dimensions and aperture offsets; perspective offsets through UsdPerspectiveProjection with frustum/sub-view support and vertical-FOV-preserving resize; no automatic Camera3d activation | Lens effects, arbitrary film-fit policies and full reference-render parity remain incomplete |
| Lights | Sampled UsdLux color/intensity/exposure, supported light dimensions and shaping cone angle; independent root clocks | Rect/Cylinder lights remain point-light approximations; photometric conversion and rendered lighting fidelity remain incomplete |
| Domes | Sampled data and HDR/EXR textures; explicit per-camera ambient or runtime-filtered IBL with ownership restoration; standalone directional IBL captures; embedded warm-HDR direct GPU capture; single-part half-float RGB EXR preserves HDR values and matches equivalent HDR GPU output | No implicit global mutation; filtering requires six device storage textures and compute support. Mara's Bevy host requests up to six on adapters supporting adapter-specific texture formats; lower-capability devices remain unavailable. Deep/multipart EXR, general color management and broader fidelity remain unvalidated |
| Point instancers | Direct mesh and bounded Xform/Scope/SkelRoot/mesh/primitive hierarchies with shared preview materials/subsets and CPU skin/morph geometry; Cube, Sphere, Cylinder, Capsule, Cone and Plane use the ordinary shape tessellation path, including sampled dimensions; sampled descendant transforms and inputs; stable instance IDs and prototype-path nodes; independent clocks, malformed-update suppression/recovery, sampled transforms/IDs/masks, composed typed inactiveIds | Hierarchies reject reset stacks, per-prim shear/perspective/singular transforms, unsupported node types, depth >=256 and >4096 projected nodes. Primitive tessellation is approximate; primitive face subsets are not projected. GPU prototype deformation, full hierarchy/deformation/reference parity and velocity/angular-velocity motion remain incomplete. Dependency edits reconcile the full stage; upstream USDA parsing rejects prepend/delete inactiveIds |
| Skinning | Optional classic-linear GPU skinning, CPU fallback; vertex and constant influence sets with time-sampled indices/weights, including samples without defaults; material subsets share a palette; compatible morph targets combine before skinning | GPU limit: four normalized influences, 256 joints; unsupported morph normal modes use fallback; broader fidelity/performance remain open |
| Morph targets | GPU position targets with generated flat normals or indexed vertex/varying authored normal deltas; standalone and combined classic-linear skinning; sparse shapes/inbetweens, cached buffers and material subsets; live independent-clock reversal captured against a CPU reference | Face-varying normals and generated smooth-normal deltas remain incomplete; target count capped at 256 and texture-backend vertex capacity; normal-map tangents and broader fidelity/performance unverified. The covered live 10,0 endpoint is pixel-identical; the separate 0,10 comparison differs by one RGB level at one pixel |
| Instancer validation | Array lengths, prototype indices, finite transforms and nonzero quaternion checks; half/float/double quaternion decoding; last-valid projection on malformed updates | Missing prototype lists retain placeholders; unsupported prototype content still needs broader diagnostics |
| Editor | Selection, typed default/sample edits, attribute filtering, matrices, relationships, variants, reference/payload list operations, namespace edits and undo/redo; configurable retained command limit (default 128) | Numeric-array text editing is bounded to 4096 scalar components / 256 KiB; unsupported types remain read-only. General interaction acceptance is incomplete; command count does not bound diff bytes or in-flight capture |
| Runtime layer participation | Checked edit-target, payload-load and mute/unmute actions; effective load-rule/muting changes advance revision without authored undo entries; root/active edit layers are protected from muting | Muting is session-local. Undo/redo of a command targeting a muted layer requires unmuting first; runtime state on excluded/despawned entities is not restored |
| Inspector diagnostics | Selected projection errors, subset and CPU-skin fallback reasons, missing UV/tangent inputs and generated-child material warnings with source labels | Child collection stops at 4096 visited entities / 64 warnings with an explicit truncation message; unrelated runtime subtrees and nonmaterial child issues are excluded |
| Capture validation | Viewport readback, native host capture, replays and same-process two-frame capture; pixel and runtime-panic checks; existing companion/symlink refusal | Fixed waits and brightness are not readiness/fidelity proof. Known panic markers do not detect arbitrary hangs or unlogged failures; artifact preflight is not an atomic reservation against concurrent writers |
| Viewer timeline | EditorBridge seek/play commands; Timeline pane with play/pause, stepping, start and typed seek; shared playback math with independent instances; inspected captures for typed seek, nonfinite rejection, play/pause, next/previous and Go to start | Interactions use injected egui input in an isolated native viewer; OS-level mouse/keyboard, long-running playback performance and broader overlay acceptance remain unverified |
| Persistence | Explicit root-layer, edit-layer and flattened export through same-directory staging, file sync, atomic replacement and Unix directory sync; existing file permissions retained; resolver-byte baselines on viewer/source-aware opens, optimistic content-change check during export and no-clobber publication of new files | Final symlinks, directories and read-only targets rejected; inode identity/ownership/ACL preservation and atomic conflict checks against concurrent existing-file writers are not provided. Arbitrary `EditorSession::new` stages lack original disk provenance; use `from_source`. Root/edit exports retain authored muted sublayers; flattened exports omit muted opinions. Muted sets are not persisted. No universal lossless flattened round trip claimed |
| Viewer | Studio directional lighting, infinite presentation grid, automatic scene framing | No full IBL acceptance; animated bounds and broader asset corpus need validation |
| Animation showcase | Bundled animation_showcase.usda; inspected Vulkan captures show shape/prototype growth and material changes; Timeline replay captures exercise live playback, pause, stepping and return to start | Bounded egui replay acceptance, not OS input or a complete flagship showcase; no reference-renderer comparison for the assembled showcase |
| Authoring | UsdSource::build with canonical generated schemas and registered Bevy components; escaped snippets; immutable typed reference batches and with_edits with captured dependencies; canonical transaction-backed EditorSession; reusable assembly and source-free USDZ examples | Typed construction requires one USDA root; reference composition requires a USDA receiver and new destination prims. Each batch opens an assembly stage and exports its root once, with validation stages per distinct source snapshot. Measurements cover captured fixtures, not production scene performance. Snippets are not a dependency-access sandbox, and these APIs do not implement BSN syntax or full BSN equivalence |

## Runnable evidence

The self-contained [composed showcase](SHOWCASE.md) combines retimed animation,
shared prototypes, skeletal/morph deformation and embedded dome lighting.
Inspected host captures at 0/30/60 and a composition contract cover this fixture;
they do not establish native-render parity or performance certification.

From the repository root, without a display or sibling asset collection:

```sh
make run APP_TARGET='--example independent_instances' RUN_WITH=env CARGO='cargo --offline'
```

The example loads `assets/animated_spinner.usda` through AssetServer. It verifies
separate entity identities, separate animation times, an instance-local size
override, an isolated clock update, and removal of the override without replacing
the surviving entity. It exits nonzero if an assertion or asset load fails, and
is also exercised by `make test-all`.

`BEVY_WORK.md` records the larger acceptance checklist and visual evidence limits.

## Native watching and live rendering evidence

The AssetServer `file_source` adapter recovers watched source-root replacement on
Unix using one-second device/inode checks and watcher re-arming. Requested paths
are invalidated on loss and recovery; event forwarding adds up to 250 ms polling
latency to the native debounce. Failed initial setup retries on Unix when a root
directory becomes available, including directories created after startup.
Processed sources and non-Unix source-root replacement are not covered.

The direct viewer/editor and AssetServer are separate loading paths. Enable
viewer texture watching with:

```sh
USD_WATCH_TEXTURES=1 make run APP_TARGET='--bin usdview --features file_watcher' ARGS='path/to/scene.usda'
```

The viewer logs watcher setup errors and active file counts. Native tests cover
atomic image replacement, deletion/recreation, unrelated-file filtering, document
switching, failed Open, setup-retry refresh of missed image edits and cleanup.
The inspected viewer capture confirms an
automatic red-to-blue update with selection retained. Package image refresh
still reads the opened package snapshot, not a replaced package on disk.

For repeatable native GPU evidence using bundled/generated fixtures:

```sh
make --eval='check-live-clocks:; @bash scripts/check_live_clocks.sh target/live-clock-check' check-live-clocks
```

The output directory must be new. This checks live UV-chain animation, sampled
texture filenames and GPU morph clock reversal against independent endpoint
references. The color-space case compares live clock reversal with freshly loaded
instances of the same sampled source, not an independent color-management reference.
The verified 0,10 -> 10,0 cases pass strict zero-RGB-tolerance
comparisons; metadata and renderer diagnostics are also checked. The script
retains PNG/RGBA output, logs and results.tsv. Other times, cameras, renderers,
normal maps and sustained playback performance remain outside this suite.

## Projection benchmark

```sh
make run APP_TARGET='--example projection_benchmark' RUN_WITH=env CARGO='cargo --offline' ARGS='128'
```

This reports three headless samples of native-instance stage opening, projection,
source editing/reconciliation, empty-notice processing, and initial asset counts.
It also reports cache-accounted mesh payload bytes after the edit (not total
process or GPU memory).
Cached mode enables both mesh and material caches; cached/uncached order alternates.
Each run checks proxy geometry after a prototype
edit and stable entity IDs. Counts from 1 to 4096 are accepted.

The default command measures a debug build, not GPU rendering or production
frame time. No warm-up is excluded, so initial schema/cache setup can affect the
first stage-open measurement. The idle figure measures only `apply_changes`
without notices, not a whole Bevy frame. Use identical build/hardware conditions
for comparisons; this is a diagnostic harness, not a pass/fail latency gate.

Set `USD_PROFILE_ROUTES=1` on that command to print cumulative per-route matching
and application timings for projection plus the source edit. Applications can
opt in by inserting `route::ProjectionTimings`, and reset counters by clearing
its map. Application time also includes cleanup for routes that no longer match.
Instrumentation adds overhead and excludes traversal, context creation,
animation discovery and work outside route dispatch; do not sum it as total CPU
frame time. Without the resource, route dispatch does not read the clock.

Animation discovery scans composed authored attributes rather than every schema
declaration. Skinning and blend animation discovery share a single binding and
animation-source lookup. Dependency scans still include prototype descendants,
inherited normals, subsets and bound material graphs; this is not a persistent
animation-result cache and does not bypass edit/reload invalidation.

The mesh cache retains at most 8192 entries and 256 MiB of accounted payload by
default. Configure it with `ProjectionCache::with_byte_budget(bytes)`; zero
disables new retained entries. Attributes, indices, morph data and morph-name
bytes are measured at insertion. Allocator capacity, GPU copies, materials,
textures and later external mutation of cached assets are not included. Treat
interned meshes as immutable shared assets and intern replacements when editing.
An oversized mesh bypasses retention; filling either limit clears cached strong
handles without removing live assets. This bounds cache retention, not all scene
memory, and eviction may reduce sharing across subsequent projections.
