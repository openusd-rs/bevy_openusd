# bevy_openusd

> This project was supported by **[Wageningen University and Research (WUR)](https://www.wur.nl/)**.
> A lot of the code was carved out of an internal repo to be open-sourced. Special thanks
> to the team for letting it ship.

A live [OpenUSD](https://openusd.org) editor on [Bevy](https://bevy.org) 0.19.1.

The composed USD stage is the source of truth: it's held live (not baked), projected
into Bevy entities, and kept in sync off openusd's change notifications. Edits flow
both ways — author back to the stage, undo/redo, and save.

## Crates

- **`usd_bevy`** — the editor library:
  - `live` — `LiveStage` (stage + change sink), the `SdfPath ↔ Entity` bimap, the
    project/reproject loop, and `LiveStagePlugin`.
  - `authoring` — namespace ops (define/remove/rename/reparent/move), attribute
    authoring, undo/redo, and persistence (export/save).
  - `read` — decode geometry/transforms/visibility off the composed stage via
    [`mxpv/openusd`](https://github.com/mxpv/openusd).
  - `mesh` — `UsdGeom.Mesh` → `bevy::mesh::Mesh`.
- **`usdview`** (`src/main.rs`) — a minimal viewport host: opens a USD file, projects
  it with `LiveStagePlugin`, and renders with studio key/fill/rim lighting.
  `src/environment.rs` configures the slate background and anti-aliased infinite
  X/Z grid: one-stage-unit minor lines, major lines every ten units, colored axes,
  and a fading horizon. The grid remains transparent to geometry below it.

## Usage

```sh
make run ARGS="path/to/stage.usda"
```

Requires Rust 1.96+ and the sibling `../mara` checkout.
OpenUSD core and generated schemas are pinned to upstream commit
`b7df5add628cbb791103a7da842dbd82810da5d0` (0.7.0).
The sibling `../openusd` checkout is not used or modified.
See `OPENUSD_UPGRADE.md` for the migration and revised capability assessment.
See `PLAN.md` for the architecture and remaining integration work.

Run `make check-all` and `make test-all` to check and test the full workspace.

Inline `usd!` snippets open directly from memory. Use
`snippet.open_stage_at("path/to/source.usda")` to anchor relative references at
an explicit filename without creating that file. `UsdSource` also opens root
bytes (including USDZ) as independent stages. Ongoing Bevy integration work is
tracked in `BEVY_WORK.md`. The AssetServer loader now uses dependency snapshots
and exposes `UsdSceneState` (Loading/Ready/Failed). Snapshot PNG/JPEG textures are
tracked labeled assets with color/data color spaces. Time-sampled material
`inputs:file` paths are supported: texture samples reachable through each
material's connections are decoded into the snapshot, and each instance selects the image at its own
clock. Explicit `inputs:sourceColorSpace` raw/sRGB samples also follow that clock;
snapshot discovery includes their sample times and loads each required image
interpretation. This supports explicit asset-path samples, not filename sequence patterns.
Dependency-change events
reload their owning USD asset. Each root retains an independent live stage;
matching prim entities and runtime-only components survive source reloads.
For native filesystem events, enable the optional `usd_bevy/file_watcher`
feature and set `AssetPlugin::watch_for_changes_override` to `Some(true)`.
Without that feature, dependency tracking alone does not install an OS watcher.
This applies to AssetServer-loaded scenes, not the viewer's direct editor Open
path. Run the explicit OS-event regression with:

```sh
make --eval='test-file-watcher:; @$(CARGO) test -p usd_bevy --features file_watcher native_file_watcher -- --ignored --nocapture' test-file-watcher
```

Bevy 0.19.1's default source reloads modified/replaced dependencies but does not
invalidate them when a file is deleted. To report missing dependencies as Failed
and recover when they reappear, register the read-only removal-aware source
**before** adding `AssetPlugin` (or `DefaultPlugins`):

```rust,ignore
app.register_asset_source(
    bevy::asset::io::AssetSourceId::Default,
    usd_bevy::watcher::file_source("assets"),
);
```

The adapter retains Bevy's file reader and debounced watcher, forwarding removal
events and both paths of file renames as dependency invalidations as well.
Untyped non-metadata removals also invalidate their reported path. The reader
records requested file paths, including failed reads, so folder additions,
removals and renames invalidate requested descendants without scanning the disk.
Paths are retained until the source is dropped; this is not an active-handle-only
index. On Unix, the adapter checks the source root's device/inode once per second,
invalidates requested paths when it disappears or changes, and re-arms watching
when a replacement directory exists. Event forwarding polls at 250 ms intervals
in addition to Bevy's 300 ms debounce. Failed initial setup retries on Unix once
per second when the root is a directory, including roots created after startup.
Processed assets and non-Unix source-root replacement remain unsupported. Registration
does not override the AssetPlugin runtime watch setting. Its event worker is
closed and joined when the source is dropped.

Before publishing a source or override revision, Bevy traverses the active
composition, reads default attribute values and asset time samples, and checks
reported composition errors. Failure retains any previous projection and reports
`UsdSceneState::Failed`. Unselected variant branches are not eagerly validated.
`UsdSource::open_stage` itself retains upstream's permissive partial-stage behavior.
The viewer's Open command applies the same composition validation before replacing
the current document. Rejected opens retain selection and edit history; long
failure messages wrap in the outliner's Status section.

For undoable payload authoring, use `EditorEdit::Payloads { prim, payloads }`
with canonical `openusd::sdf::Payload` entries, or `EditorEdit::ClearPayloads`.
An empty replacement list blocks weaker payloads; clearing removes the local
opinion and restores weaker composition. Entries allow empty defaultPrim targets
or absolute prim paths, with finite offsets and positive finite time scales.
The headless equivalents are `authoring::set_payloads` and `clear_payloads`.
These author layer opinions; `EditorCommand::Payload` separately changes runtime
load rules. The inspector's Payload authoring group accepts up to 64 draft
entries with asset, prim, offset and scale fields. Replace authors the whole list;
an empty draft offers an explicit Block action. Clear removes the local opinion.
Drafts reset when the document, selected prim or edit layer changes. They start
empty rather than copying composed relative paths into a different authoring
layer. Existing list-op provenance and in-place composed-list editing are not
displayed by this form.
`assets/payload_authoring.usda` is a visible authoring fixture: the weaker layer
loads a blue cube from `payload_authoring_content.usda` at `/Box`. Replacing the
payload with that same asset at `/Ball` loads an orange sphere; clearing the
root-layer opinion restores the cube. Matching inspector replays live in
`scripts/replays/payload_{replace,clear,invalid,scroll}.replay`.
The viewer warns before opening the file picker when a document is already open.
Cancel keeps the current document and lets you save the required layers first.
The warning is conservative: it appears even for a saved document, without
claiming to track every layer's unsaved state. The original document/revision
token remains checked after selection, so edits during the warning or picker
invalidate the replacement. This does not add a window-close guard or change
the explicitly unchecked programmatic `EditorCommand::Open` API.
Flattened editor exports also validate the active composition before writing.
Reported composition errors preserve the existing destination. Root/edit-layer
exports retain authored unresolved references for repair rather than requiring
a complete composed scene; USDZ packaging still requires its dependencies.

For reusable in-memory assemblies, `UsdSource::snapshot(path, bytes)` disables
filesystem fallback. `root.with_dependency(&model)` returns a new source containing
the model and its captured dependencies, retaining each filename as its relative
asset anchor. Identical bytes deduplicate; conflicting bytes at the same identifier
return an error without changing either source. The root source controls filesystem
fallback; merging a filesystem-backed dependency does not enable it on a snapshot.
Supply opaque assets such as textures as dependency snapshots too. A merge captures
only supplied bytes, not files reachable on disk. Rebuild an assembly from its root
when replacing a dependency revision; this merge API never silently overwrites one.

`with_reference(destination, &model, target)` also authors the reference through
the typed USD API and validates the composed result, returning a new snapshot:

```rust
let assembly = root.with_references([
    ("/First", &model, "/Model"),
    ("/Second", &model, "/Model"),
])?;
```

The receiver must have a `.usda` identifier. Destinations and explicit targets
must be absolute non-root prim paths; targets must exist and destinations must be
new. Pass `openusd::sdf::Path::default()` as the target to use the model's
`defaultPrim`. The emitted reference retains its empty target rather than baking
in the resolved name; a missing or unresolvable default fails. An empty typed
path is distinct from an empty string, which the upstream path parser rejects.
Existing prim
patches remain explicit editor/authoring operations. Conflicting dependencies or
composition errors fail without changing either input. References use the source
identifier as an absolute asset anchor. For a self-contained assembly with
captured dependencies, export USDZ through the editor:

```rust
usd_bevy::editor::EditorSession::new(assembly.open_stage()?)
    .save("assembly.usdz", usd_bevy::editor::SaveMode::RootLayer)?;
```

The native export test lane verifies a relocated diskless batch assembly with
default and explicit targets, including its captured opaque asset bytes, through
OpenUSD `usdcat --flatten`.

Ordinary USDA/USDC root-layer export retains references but does not write their
in-memory dependencies to disk. Such an export cannot reopen independently when
the referenced files do not exist. Flattened export removes composition arcs,
but does not bundle external textures; it is not a substitute for USDZ packaging.
`with_references` mounts entries in iterator order and publishes a new snapshot
only if the whole batch succeeds. It opens one assembly stage, exports its root
once, and reuses a validation stage for repeated mounts of the same source
snapshot. An empty batch preserves the original revision and bytes. The
single-reference method delegates to this batch path. Dependency merges and USD
composition still have costs, and this is not a complete typed scene DSL.

`reference_benchmark` compares sequential and batch assembly of a captured shared
Cube model. It warms both modes, alternates their order, and verifies equivalent
exported roots and composed values outside the timed region:

```sh
make run RUN_WITH= APP_TARGET='--example reference_benchmark' ARGS='32 3'
```

It reports assembly time only, not Bevy projection, rendering or disk I/O.
For release measurements, append `--release` to `APP_TARGET`.

`with_instanceable_references` accepts the same four-field entries as
`with_offset_references` and marks every mount instanceable. OpenUSD determines
prototype sharing; descendant prims become instance proxies, not independently
editable copies. The ordinary reference APIs do not mark mounts instanceable.
Native relocated-USDZ tests retain shared instances in root/edit-layer and
flattened editor saves. The editor rebuilds shared prototype subtrees as internal
references after upstream flattening, retaining instance-root opinions and
processing nested instances before their parents. Generated prototype names avoid
existing prims. Retimed instances keep their baked stage-time samples. The
intermediate upstream layer still expands instances, so this does not reduce peak
flattening memory; direct upstream `Stage::flatten` still expands instances.
Flattened editor saves bake clip-resolved values before rebuilding instances.
They sample stage-time keys and their immediately preceding representable times
to retain boundary behavior, including value blocks. Asset values are anchored;
USDZ saves package their dependencies. Clip-sourced `timecode`/`timecode[]` values,
unresolved clip assets and more than one million attempted baked samples are
rejected before publishing output. Root/edit-layer saves retain authored clips.
Native export tests cover scalar switching, value blocks, packaged asset values
and nested retimed instances; they do not establish every clip schedule or type.
The headless example verifies two same-timing mounts sharing a mesh and a third
retimed mount using the correct sampled geometry. It checks forward/backward
clock changes, stable entities/runtime names and cleanup after root despawn:

```sh
make run RUN_WITH= APP_TARGET='--example instanceable_sources'
```

`assets/retimed_instances.usda` places three native instances side by side;
the third uses offset 10 and scale 2. Its independently authored baked reference
is `assets/retimed_instances_reference.usda`. The GPU regression checks times
10/20/30 at a fixed camera, three visible meshes and zero RGB pixel differences:

```sh
make --eval='check-retimed-images:; @/bin/bash scripts/check_retimed_instances.sh target/NEW-retimed-images' check-retimed-images
```

The output directory must be new. Logs, metadata, raw pixels and diff images are
retained. The three fixed-time cases establish sampled-frame equivalence, not performance.
The script also swaps two assemblies' clocks from 10/20 to 20/10 after 30 ready
frames and compares against the baked reference at the final times. That case
uses `USD_CAPTURE_INSTANCE_SPACING=14` and a wider camera to expose all six cubes.
The capture tool accepts a positive finite spacing safe for 16 instances; its
default remains 2.5, and the selected spacing is recorded in capture metadata.

`with_offset_references` accepts `(destination, source, target, LayerOffset)`
entries using `openusd::sdf::LayerOffset`. It preserves reference arcs while
retiming samples; offsets must be finite and scales finite and positive.
The existing `with_references` API uses identity offsets. Invalid entries reject
the whole batch without changing inputs. The lower-level
`authoring::set_references` also requires positive finite scales. Native OpenUSD
supports negative scales with a deprecation warning, but the pinned Rust reader
failed a reversed-sample probe; newly authored negative mappings are rejected
rather than silently interpreted incorrectly. Source validation also rejects
invalid time offsets in composed reference and payload lists on traversed prims, including
the internal-reference case that did not surface a composition error. Rejected
reloads retain the last valid projection. Authored sublayer offsets in loaded,
unmuted layers are checked too; absent offsets retain identity timing. This does
not eagerly inspect unselected branches or layers that composition has not loaded.
`examples/retimed_sources.rs` demonstrates
one animated source mounted twice with different timing and checks projected mesh
radii and stable entities across clock changes:

```sh
make run RUN_WITH= APP_TARGET='--example retimed_sources'
```

`examples/clipped_sources.rs` captures a value-clip dependency in memory and
checks two independent Bevy clocks against projected sphere radii. It exercises
retimed endpoints, interpolation, backward clock changes, stable entity IDs,
runtime names and root cleanup. This single-clip example does not cover clip
switching or flattened clip baking.

```sh
make run RUN_WITH= APP_TARGET='--example clipped_sources'
```

The AssetServer loader queries sample times on numeric attributes as well as
asset-valued attributes so their value-clip layers enter captured dependencies.
`assets/clipped_sphere.usda` exercises this file-loading path. Its independent
reference authors sphere radius samples directly at stage times 0 and 20.
The GPU regression compares times 0/5/10/20 and two live roots whose clocks reverse
from 10/20 to 20/10, requiring zero RGB differences and the expected visible-mesh
counts. It retains captures, metadata and diffs in a new output directory:

```sh
make --eval='check-clips:; @/bin/bash scripts/check_clipped_sphere.sh target/NEW-clip-images' check-clips
```

This checks sampled Bevy-frame equivalence, not native-renderer parity, clip
switching or flattened clip baking.
The `switching_clips` fixture additionally checks two active clips against
native-verified direct samples. It uses wider cameras and root spacing to keep
the larger spheres separated:

```sh
make --eval='check-switches:; @/bin/bash scripts/check_clipped_sphere.sh target/NEW-switch-images switching_clips' check-switches
```

Add `flattened` to export a new USDZ first and compare its rendered animation
against the same direct-sample reference, including reversed independent clocks:

```sh
make --eval='check-baked:; @/bin/bash scripts/check_clipped_sphere.sh target/NEW-baked-images switching_clips flattened' check-baked
```

The standalone exporter accepts an input scene and a new destination path:

```sh
make run RUN_WITH= APP_TARGET='--example flatten_scene' ARGS='assets/switching_clips.usda target/NEW-baked.usdz'
```

It refuses an already-existing output path at its initial check. This is not
exclusive publication against another process creating that path concurrently.

The local upstream patch corrects interpolation toward a subsequent clip's
activation value and preserves value-block boundaries. These sampled cases do
not establish all clip schedules, asset-valued clips or timecode-value parity.
Clip dependency reload regressions also check independent clocks, malformed-layer
failure with retained mesh handles, recovery, stable entities and runtime names.
The native filesystem event lane runs explicitly:

```sh
make test APP_TARGET='-p usd_bevy --features file_watcher native_numeric_clip_dependency_reloads -- --ignored --nocapture'
```

`examples/composed_sources.rs` combines inline `usd!` root metadata with a model
authored through the upstream typed Sphere schema. It verifies projection under
two independent Bevy roots, typed reference composition, shared meshes and isolated edits without source files.
It also exercises captured dependency replacement, persistent overrides,
runtime-only component preservation, malformed-root failure/recovery and cleanup:

```sh
make run RUN_WITH= APP_TARGET='--example composed_sources'
```

`source_benchmark` measures the actual UsdSceneRoot lifecycle for one and four
roots, each containing the requested number of native USD instances. Three
alternating-order samples report typed instanceable-batch assembly and replacement
assembly separately from initial load/validation/projection, captured
dependency reload, idle App updates and asset sharing. It checks geometry updates,
entity identity, shared native prototypes and runtime-only components. Inputs are already captured in memory;
disk I/O, plugin startup, GPU upload and rendering are excluded.

```sh
make run RUN_WITH= APP_TARGET='--example source_benchmark' ARGS=128
make run RUN_WITH= APP_TARGET='--release --example source_benchmark' ARGS=128
```

Set `USD_PROFILE_SOURCES=1` to report opening, override application, composition
validation and projection/reconciliation separately. Add `USD_PROFILE_ROUTES=1`
for route matching/application counters per load/reload phase. Profiling output
goes to stderr; CSV timings remain on stdout. Both instruments add timing overhead.
Apps can opt in directly with `usd_bevy::asset::UsdSceneTimings`; its cumulative
attempt/failure counters and durations can be read or reset. Failures counted here
occur during stage opening, overrides or validation, not earlier AssetServer I/O.
No clock reads are added to source publication when this resource is absent.

The separate `projection_benchmark` measures direct LiveStage projection and edits,
not the source-root publication path. Its timings are not interchangeable with
the lifecycle benchmark. Debug timings are diagnostic, not production guarantees.
The [release CPU baseline](benchmarks/source-lifecycle.md) records raw samples up
to 4,096 projected shapes, source-phase profiles and a separate cache comparison.
It demonstrates asset sharing, but not CPU acceleration or rendered performance.
The subsequent [reload comparison](benchmarks/reload-animation.md) measures reduced
CPU reload cost from skipping an unused animation-index scan, with clock-isolation
regressions. It does not establish GPU or frame-rate improvements.

`editor_benchmark ASSET [SAMPLES] [cpu|gpu-prepared] [seek|seek-unique]` measures the actual editor Open path and retained
mesh/image payload after 100 idle updates with Bevy asset tracking enabled. It
includes file reading and texture decoding, but excludes GPU work and the UI:

```sh
make run RUN_WITH= APP_TARGET='--release --example editor_benchmark' ARGS='assets/material_subsets.usda'
make run RUN_WITH= APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_animation.usda 3 gpu-prepared'
make run RUN_WITH= APP_TARGET='--example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 gpu-prepared seek'
```

Optional `seek` cycles clocks 0/5/10/5 for four warmup updates and 100 measured
updates. It checks document identity, Ready status and the requested clock after
each update, then reports nearest-rank median/p95/max `App::update` duration and
post-seek retained payloads. Command enqueue and result validation are outside the
timer. GPU morph entity counts distinguish prepared morphs from CPU fallback.
These are headless editor update timings, not GPU frame latency or VRAM use.
`seek-unique` instead measures 1,000 distinct clocks from 0.01 through 10 after
the same four warmup updates. Distinct measured clocks do not guarantee cache
misses. Additional columns record peak mesh/material/image asset counts sampled
after updates, final mesh-cache entries/payload, and process RSS before/after
seeking. RSS is Linux `/proc/self/status` VmRSS in bytes or `NA` when unavailable;
it includes the whole process and is not an allocator count, peak RSS or VRAM.
Asset-count sampling and RSS reads are outside the update timer.

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='assets/morph_tangent_normals.usda 3 gpu-prepared seek-unique'
```

The initial [morph-tangent seek measurements](benchmarks/morph-tangent-seek.md)
record debug and release CPU/GPU-prepared runs with explicit limitations.
The [distinct-clock retention baseline](benchmarks/unique-clock-retention.md)
records cache growth that the repeated-clock workload does not expose.
`UsdPlugin` now prunes mesh-cache entries in `Last` when the cache is their sole
strong owner. Entity-owned and externally held meshes remain shared; revisiting
an unused historical state can allocate a new handle. Bevy's asset tracking
performs subsequent asset removal. Bare Worlds using the cache without the
plugin retain the existing entry/byte-budget behavior. Cache payload accounting
keeps insertion-time sizes; material-cache retention is unchanged.

The default CPU mode evaluates deformation on the CPU. `gpu-prepared` enables
GPU deformation routing and measures its retained CPU-side mesh/morph payloads;
it does not create a renderer or measure GPU execution. Unsupported deformation
can still use the route's CPU fallback. Morph bytes count inline Bevy 0.19
attributes, separately from image bytes; asset handles and allocator overhead
are excluded.

The [real-asset baseline](benchmarks/editor-assets.md) records ANYmal and Spot
measurements. Payload bytes are not resident-memory or VRAM measurements.

GPU flat-normal materials share converted assets when entities use the same base
material and its values match. The conversion cache retains at most 1024 handles
and rejects missing or modified cached assets. As with other shared Bevy assets,
clone a material before making entity-local runtime changes.

`usd_bevy::instance::UsdInstanceTime` controls each root's position in USD time
codes. `UsdPlayback` adds pause/play, signed speed, looping and an optional
time-code range; otherwise it uses the stage's authored start/end and rate.
Playback is paused by default. Non-looping playback stops at its endpoint;
looping uses a half-open range. `UsdInstanceOverrides` stores
variant selections and typed attribute opinions reapplied on reload. Removing
an override restores the source opinion. Access stages and prim entities through
the `UsdInstances` non-send resource; direct stage edits are transient across
source reloads. Failed reloads preserve the last good live projection.

Broader editor, material fidelity and performance work remains in progress.
See [SUPPORT.md](SUPPORT.md) for the integration support/limitation matrix and a
runnable, self-checking independent-instance example.

`UsdPlugin` enables mesh and material sharing. Treat shared material handles as
shared Bevy assets: clone the material into a new asset and replace the entity's
handle before making an entity-local runtime modification. USD-authored material
changes are reprojected into matching/new handles rather than mutating a shared
material in place. Remove `route::cache::MaterialCache` to disable material sharing.
Standard and flat-normal material caches release cache-only handles in `Last`;
entity-owned and externally held materials remain eligible for sharing within
the 1024-entry limits. Bevy asset tracking performs subsequent reclamation.
Revisiting an unowned historical material can allocate a new handle.
Converted RGB/normal, scalar-packed and alpha-packed texture caches likewise
release cache-only images in `Last`, retaining their existing 64 MiB budgets.
Source snapshot and AssetServer texture ownership is unchanged.

The viewer's outliner and Properties pane operate on the rendered live document.
Properties supports scalar, string/token/asset-path and three-vector edits, relationship
targets, namespace moves, layer targets, variant selection and provenance readouts.
Provenance defaults to the source kind, composition arc and source prim path;
“Show source details” expands the complete wrapped record. Each attribute lists
up to six composed scene-time sample keys, with the total count for longer lists.
Attribute controls can block values or clear local defaults/time samples, including
when the composed value is blocked or absent. These operations are undoable and
preserve property metadata; they differ from `authoring::clear_attribute`, which
removes the local property spec. Clearing values reveals weaker-layer opinions.
Supported attributes without resolved values use their declared USD type to prepare
an editable draft; nothing is authored until an apply button is pressed.
“Apply default value” edits the default opinion, not an animation time sample.
“Apply sample at time” uses the explicit scene-time field; “Use timeline time”
copies the current timeline position into that field. “Clear sample at time”
removes only the local sample at that time, including for read-only value types.
For explicit samples, use `EditorEdit::AttributeSample` or
`EditorEdit::ClearAttributeSample`. Their finite `time` is in scene time and is
mapped through the active edit target; defaults are left intact. These operations
participate in editor undo/redo. The lower-level authoring functions are
`set_attribute_sample` and `clear_attribute_sample`.
The ribbon provides undo/redo and
separate root-layer, edit-layer and flattened export dialogs. Open selections
carry the document id and authored revision from the picker request; changes
made while choosing a file reject the delayed result and retain the document.
This works with undo history disabled, but is not an unsaved-work confirmation
for edits predating the request. Programmatic `EditorCommand::Open` remains
unconditional; integrations can use `OpenChecked` with an `EditorSnapshot` token.
Unsupported value
types are displayed read-only; see `BEVY_WORK.md` for remaining editor work and
visual acceptance limits.

The editor loads PNG/JPEG material images from filesystem and USDZ documents,
refreshes them after edits, and preserves the previous document if opening fails.
Nested USDZ reads support inner-package default layers, explicit bracket paths
and relative inner-layer assets without extraction, through both filesystem and
snapshot resolvers. Each entry traversal permits at most 16 package levels and
256 MiB of cumulative decompressed entry bytes; this is not a total-stage memory
limit. Export bundles referenced nested layers/assets into ordinary unique entries
while preserving composition; it does not reproduce the source archive layout.
`nested_package_fixture` generates implicit/explicit nested USDA and USDC cases
with embedded PNG materials, plus an independent constant-color reference;
run it through `make run` with a new output directory.
Add `--export` after the output directory to generate and reopen portable exports.
The viewer can automatically watch requested external textures on native builds:

```sh
USD_WATCH_TEXTURES=1 make run APP_TARGET='--bin usdview --features file_watcher' ARGS='path/to/scene.usda'
```

Watching is off by default (`USD_WATCH_TEXTURES=0` also disables it). Requesting
it without the build feature, or providing a value other than 0/1, fails before
opening the viewer. The viewer logs active file counts and watcher setup errors.
Library applications can opt in by enabling `usd_bevy/file_watcher` and adding
`usd_bevy::editor::texture_watch::EditorTextureWatchPlugin` alongside
`EditorPlugin`. It watches requested external images and sends RefreshTextures
on native file events. `EditorTextureWatchStatus` reports active file count and
setup errors. Watched paths update with the document/image set; idle frames do
not reread images or rescan USD materials. On Unix, directory device/inode
identities are checked at most once per second; removed or replaced directories
re-arm only their own watchers and queue a texture refresh. Package entries and
USD layers are not watched; directory replacement recovery is not implemented
on non-Unix platforms. Failed watcher setups retry once per second while
the app updates, without restarting successful watchers. Recovery queues a texture
refresh to catch edits made while unavailable. Document/path changes reset the
retry state. When no document is open, a texture read/decode failure registers
the requested image paths and retries the initial Open after watcher events or
successful watcher setup. A newer explicit Open takes precedence over a pending
retry. Failed replacement opens never enable retries over an existing document,
so later image repairs cannot replace its edits or undo history. Composition or
root-file failures do not enable this texture retry path.
An edit that requests a missing or undecodable image keeps the previous image
handles while registering the requested paths. Unresolved relative paths are
watched conservatively under loaded filesystem layer directories; these candidate
paths can trigger extra refreshes and are not custom-resolver/search-path support.
Creating a missing parent directory is handled by the watcher setup retry.
Successful loading replaces candidate paths with resolved image paths.

```sh
make --eval='test-editor-watch:; @$(CARGO) test -p usd_bevy --features file_watcher native_editor_texture_watch -- --ignored --nocapture' test-editor-watch
```

For a same-window recovery capture, put Weston and weston-screenshooter on PATH:

```sh
make --eval='capture-recovery:; @/bin/bash scripts/check_initial_texture_recovery.sh target/initial-recovery' capture-recovery
```

The output directory must be new. The script generates a fixture, captures the
missing-texture state, repairs the PNG, and captures the recovered window plus
Bevy viewport without restarting the viewer. Inspect `initial.png`,
`recovered.png`, `recovered.viewport.png` and the logs; automated checks confirm
capture completion and reject a near-black viewport, not visual correctness or
warning-free operation. It uses a private Vulkan Weston compositor on Linux.

Applications can send `EditorCommand::RefreshTextures` through `EditorBridge`
to reread source-backed images without reopening the USD document. Successful
refreshes preserve selection, unsaved edits, runtime entities and undo/redo;
decode failures retain the previous image set and report an error. This command
does not reload USD layers or changed package-root bytes. The viewer exposes
it as the image-icon **Refresh textures** action in the left ribbon. Errors
appear in the outliner's Status section while the last good textures remain.
Read and decode failures identify the texture path (resolved when available),
including the package entry when applicable.
Unresolved relative texture identifiers are rejected at the byte-reading
boundary instead of being opened relative to the process working directory.

Undoable authoring uses `editor::EditorSession` and `editor::EditorEdit`:

```rust,ignore
use usd_bevy::editor::{EditorEdit, EditorSession};

let mut editor = EditorSession::new(stage);
editor.edit(EditorEdit::Rename {
    path: "/World/Box".into(),
    name: "Crate".into(),
})?;
editor.undo()?;
editor.redo()?;
```

Editor sessions retain at most 128 undo/redo commands by default. Use
`set_history_limit(n)` to change this; zero disables retained history. Lowering
the limit keeps newest undo commands first, then nearest redo commands. Eviction
discards complete commands and their inverse transactions only after successful
edits, so a failed multi-transaction batch still rolls back completely. Evicted
edits remain in the document. This is a command-count limit, not a byte budget
or a bound on temporary capture during a command; direct stage edits establish
a new history baseline when the editor next synchronizes them.

The old `authoring::EditHistory` API has been removed. Migrate `define`,
`set_attr`, `rename`, `reparent` and `set_variant` calls to the corresponding
`EditorEdit` variants; `undo`/`redo` no longer take a stage argument. Low-level
`authoring` functions remain available for edits without editor history. In the
viewer, send `EditorCommand::Edit` through `EditorBridge` to retain live namespace
entity identity as well as authored history.

`usd!` escapes `${value}` inside quoted strings. Unquoted interpolation accepts
only built-in numeric/boolean scalars; arbitrary strings cannot inject USD
structure. Asset/prim-path interpolation is rejected. Dynamic identifiers and
field-specific value validity are checked when parsing/opening the result.

For reusable assets, `authoring::set_references` accepts upstream typed
`sdf::Reference` values (relative paths, default prims and time offsets).
`EditorEdit::References` adds undo/redo grouping. Empty reference lists block
weaker references; `clear_references` removes the edit layer's local opinion.

Native instance proxies are projected into the Bevy hierarchy. Identical static
geometry shares mesh handles, with prototype identity available through
`route::native::{UsdNativeInstance, UsdInstanceProxy}`. This is not yet a claim
of prototype-level CPU build reuse or measured rendering performance.

The viewer uses Bevy GPU skinning for supported classic-linear bindings and
falls back to CPU deformation with a `UsdCpuSkinFallback` reason otherwise.
Library users opt in with `route::gpu_skin::UsdGpuSkinningPlugin`.
Set `USD_CPU_SKINNING=1` for the CPU baseline. Optional `USD_SCREENSHOT` and
`USD_CAPTURE_TIME` capture a fixed-time viewport frame; UI overlays are excluded.
Current binding restrictions and fidelity evidence are in `BEVY_WORK.md`.

Dome projection now emits sampled `UsdDomeLight` data without overwriting
`GlobalAmbientLight`. For an explicit ambient approximation, add
`route::dome::UsdDomeAmbientPlugin` and attach
`route::dome::UsdDomeAmbientSource(dome_entity)` to a camera. Different cameras
can select domes from different USD roots. Removing the selection, hiding or
despawning the dome restores the camera's prior ambient value, provided it has
not been overwritten externally. This adapter owns camera ambient while selected;
it is not image-based lighting. The viewer keeps its existing studio ambient.

### Animation showcase

Open the Timeline ribbon pane to play/pause, step one USD time code, return to
the authored start time, or enter a time code and seek. Seeking pauses playback;
playing loops over the authored start/end range using the stage's time-code rate.
Timeline operations change viewer time, not USD opinions or undo history.
The editor refreshes its sampled snapshot when playback changes the clock, so
matrix sample labels and outliner local-visibility indicators follow scene time.
Paused unchanged clocks do not trigger another snapshot. Visibility indicators
show the prim's local token, not inherited/effective visibility. The existing
visibility toggle writes a sample at the current clock when the composed attribute
has time samples, otherwise a default opinion. Successful toggles pause playback
and participate in undo/redo; they do not change other authored sample keys in the
same edit layer. Normal USD strength rules still apply across edit layers.
`assets/visibility_animation.usda` hides Animated at time 10 while Visible remains
shown; it is a control for sampled outliner and rendered visibility.

`assets/animation_showcase.usda` combines a growing Cube, three shared animated
mesh prototypes and an orange-to-blue material transition over time codes 0–10.

```sh
make run ARGS='assets/animation_showcase.usda'
USD_SCREENSHOT=target/showcase-t10.png USD_CAPTURE_TIME=10 make run ARGS='assets/animation_showcase.usda'
```

`USD_SCREENSHOT` reads the actual embedded viewport GPU texture, without the UI.
The **Frame visible scene** ribbon action refits the current visible document
geometry, camera clipping range and grid. Use it after revealing objects or
changing the scene; ordinary visibility changes do not reset camera navigation.
Framing is a viewer-only operation and does not author USD or add undo entries.
When capture is enabled, the viewer rejects non-PNG destination names and
non-finite or malformed `USD_CAPTURE_TIME` values before constructing the window.
Negative and fractional finite time codes are accepted. Without USD_SCREENSHOT,
USD_CAPTURE_TIME does not enable capture or set the editor clock.
It writes the PNG plus tightly packed `.rgba` pixels and `.capture.txt` containing
dimensions, source texture format, row length and capture-request timing/clock
metadata. Effective camera eye, forward, up and column-major world-from-view /
clip-from-view matrices use the same fields as the offscreen capture tool.
Both snapshot the camera in `Last`, after camera/transform updates; this is
request-time evidence, not a GPU completion timestamp. `VIEWPORT_CAPTURE_OK` in stderr
means all three files were written; `VIEWPORT_CAPTURE_ERROR` reports a failure.
The viewer stays open. Capture is requested once after at least 120 consecutive updates
with the same open editor document and an active camera. Losing the camera or
replacing the document resets that count and the ready-duration timer.
`USD_CAPTURE_DELAY_MS=0..45000` adds a minimum ready duration (default 0), useful
for captures after a timed UI replay. Invalid values are rejected before window
creation. A 60-second deadline, checked on each
update, reports an error if those conditions were not met; an invalid initial
open no longer produces a grid-only success. This does not guarantee GPU
pipeline readiness, completed uploads or correct projection. The deadline ends
when readback is requested; the paired script separately bounds its wait for
the callback.
The standalone `viewer_capture` example below has explicit readiness checks and
process exit status. Use `scripts/capture_viewer_ui.sh` to capture UI composition.

Scalar UsdUVTexture inputs now apply the selected channel's `scale` and `bias`
when packing roughness, metallic, occlusion and opacity. Values are transformed
after color-space decoding, then clamped/quantized into the existing 8-bit packed
images. Sampled transforms participate in material time updates; packed content
keys distinguish changed results. Connected Material/NodeGraph scale/bias
interfaces resolve through upstream UsdShade value-producing attributes,
including sampled values. Multiple producers, invalid direct sources and
connections with no readable value reject the material read. Authored fallback
values follow the upstream resolver's rules; arbitrary shader computation is
not evaluated for these float4 inputs.
For scalar maps, out-of-range values are clamped before GPU filtering, so this is not exact
shader parity for arbitrary filtered float textures.
Diffuse and emissive RGB scale/bias use a separate linear `Rgba16Float` path.
Negative and HDR values remain unclamped in the transformed image; nonfinite
coefficients and float16 overflow return material warnings. Alpha is preserved,
and subsequent opacity packing retains transformed HDR RGB. Identity RGB
transforms reuse original images. Transformed content uses a 64 MiB accounted
cache and a 16M-pixel input limit; those are not total process/VRAM limits.
Float16 rounding remains an approximation, and normal-map scale/bias is not
implemented by this path.
Material graph evaluation has a 256-input traversal budget and a separate
32-level recursion limit for arithmetic/constant nodes; exceeding either reports
an error rather than continuing unbounded recursion.
The operation follows the
[USD texture-reader specification](https://openusd.org/release/spec_usdpreviewsurface.html#texture-reader).

`scalar_texture_fixture` creates a mapped sphere, an independently precomputed
texture reference and an untransformed negative control in a new directory:

```sh
make run RUN_WITH= APP_TARGET='--example scalar_texture_fixture' ARGS='target/NEW-scalar-fixture'
```

It also writes `animated.usda` with sampled scale values and
`animated_reference.usda` with precomputed texture files at times 0 and 10.
The live-clock regression includes these two scenes, reversing the mapped roots
from 0/10 to 10/0 and comparing against fresh reference roots at 10/0. The
file-switched reference establishes endpoint equivalence, not intermediate-time
GPU interpolation or arbitrary texture-filtering parity.
`interface_animated.usda` supplies the same sampled coefficients through a
Material input and a NodeGraph output and runs as a separate live-clock case.
`file_interface_animated.usda` instead supplies sampled texture filenames through
those interfaces. File reads use the canonical value-producing attribute, and
texture discovery gathers its sample times even when the interface input is not
named `file`. Invalid/ambiguous connections and unreadable producer values fail
explicitly; sampled-only filenames remain unset during default-time discovery.
Connected `sourceColorSpace` inputs use the same producer resolution and sample
discovery. `colorspace_interface_animated.usda` switches raw/sRGB interpretation
through a Material/NodeGraph interface; `colorspace_reference.usda` switches
between precomputed raw linear bytes instead. This is an explicit color-space
test, not implementation of metadata-based `auto` color-space detection.

The emissive fixture compares a texture-only
emissive input against a constant linear-color reference and a non-emissive control:

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example emissive_texture_fixture' ARGS='target/NEW-emission'
USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off make run CARGO='cargo --offline' APP_TARGET='--example viewer_capture' ARGS='target/NEW-emission/textured.usda target/NEW-emission/textured.png 0 0 2 6 0 0 0'
USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off make run CARGO='cargo --offline' APP_TARGET='--example viewer_capture' ARGS='target/NEW-emission/reference.usda target/NEW-emission/reference.png 0 0 2 6 0 0 0'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example capture_compare' ARGS='target/NEW-emission/textured.rgba target/NEW-emission/reference.rgba 0 1280 target/NEW-emission/diff.png'
```

Texture-only emission uses a white Bevy emissive multiplier when an image handle
is bound. Explicit emissive colors retain their multiplier, including black;
untextured defaults remain non-emissive.
The same generator writes `animated.usda` (emissive filename samples) and
`animated_reference.usda` (constant-color samples). The live-clock regression
compares their endpoints after reversing two existing instance clocks. Filename
selection is held, while the constant-color reference interpolates, so this
comparison does not claim agreement at intermediate times.

`color_texture_fixture` generates diffuse, emissive and opacity-combined RGB
transform cases with HDR constant-color references and untransformed controls:

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example color_texture_fixture' ARGS='target/NEW-rgb'
```

Capture `<semantic>_mapped.usda` and `<semantic>_reference.usda` with the same
camera (`0 2 6` looking at `0 0 0`), forward renderer and shadows disabled, then
compare their RGBA files at zero RGB tolerance. Semantics are `diffuse`,
`emissive` and `alpha`. The alpha reference includes the existing scalar
8-bit quantization followed by float16 rounding; it is not ideal float opacity.
The generator also writes `<semantic>_interface_animated.usda`, with sampled
RGB gain supplied through a Material input, and `<semantic>_animated_reference.usda`,
with independent constant-color samples. The live-clock suite tests all three
semantics after reversing two existing instance clocks. Fixture source tests also
check interpolated midpoint values and backward reads.

For a deterministic textured UV comparison, generate a new fixture directory
(existing directories are refused), then capture both indexed samples:

```bash
make run RUN_WITH= APP_TARGET='--example uv_fixture' ARGS='target/uv-check'
USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='target/uv-check/scene.usda target/uv-check-0.png 0 0 0 7 0 0 0'
USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='target/uv-check/scene.usda target/uv-check-10.png 10 0 0 7 0 0 0'
```

The left panel inherits constant indexed UVs, the center instances that mesh,
and the right uses explicit local UVs. All three should be blue at time 0 and
green at time 10. The generated quadrant texture lives beside the USD source.
This checks constant UV inheritance, indices and V orientation, not normal maps
or arbitrary shader primvar networks.

`assets/point_curve_animation.usda` exercises sampled Points and BasisCurves
without default positions. From time 0 to 10, four points and a line move upward;
the line splits into two disjoint segments at time 10. Both routes follow each
scene root's clock. Default unbound geometry uses an unlit point/line preview.
Points remain one-pixel marks; curves optionally project width-aware surfaces.

In the Rendering pane, choose **Width surfaces** with 8, 16 or 32 tube sides,
or **Use lines** to restore the default. Sampling and surface mode can change
without reopening the scene. Startup accepts `USD_CURVE_SURFACE_SIDES=3..32`:

```bash
USD_CURVE_STEPS=32 USD_CURVE_SURFACE_SIDES=16 make run ARGS='assets/curve_widths.usda'
```

Widths are object-space diameters. Composed `primvars:widths` override built-in
widths; indexed constant, uniform, vertex and varying values follow curve
sampling. Authored normals select oriented ribbons; otherwise curves form tubes.
Missing widths fall back to lines. Surface mode rejects invalid cardinality,
nonfinite/negative widths, zero ribbon normals, parallel normal/tangent pairs,
coincident adjacent samples and cusps. Errors clear stale geometry and appear in
the Rendering pane. Corrected source data can recover on the same entity.
Surfaces are fixed-sampling, open-ended polygonal approximations without caps,
UVs or native curve-renderer parity. Low sampling visibly facets curved shapes.
`assets/curve_surface_loops.usda` exercises two periodic B-spline tubes in one
prim with indexed uniform widths and separate colors: a thick blue loop and a
thin orange loop. The loops close independently without connecting triangles.
Surface materials honor `doubleSided`: single-sided ribbon backs are culled,
while double-sided backs remain visible with flipped lighting normals.
`assets/curve_surface_sidedness.usda` shows a front-facing ribbon above a hidden
single-sided back and a visible double-sided back below it.

Point clouds project constant/inherited and indexed vertex/varying display color
and opacity through the same RGBA expansion as mesh vertices. The unbound
material selects alpha blending when needed without multiplying opacity twice.
`assets/point_colors.usda` exercises independent color/opacity sources: the left
pair changes from translucent red to opaque green; the right pair swaps yellow
and blue while retaining its per-point opacity. Points remain one-pixel marks.

Curves project constant/inherited and indexed `uniform`, `vertex` and `varying`
display color/opacity. Uniform samples broadcast per curve; cubic vertex values
use the geometric basis, while varying values interpolate linearly between
segment boundaries. Periodic interpolation wraps within each curve; pinned
vertex interpolation expands the endpoint values. Linear curves use per-point
vertex/varying values. Single unindexed values also broadcast.
`assets/curve_colors.usda` compares inherited animated style with uniform styles.
`assets/curve_gradients.usda` places indexed vertex gradients above varying
gradients on identical Bezier geometry, with independent varying opacity.
Unbound curve materials choose blending from generated vertex alpha, including
cubic undershoot, rather than from the authored control values alone.
Curve projection rejects negative/mismatched counts and non-finite input or
tessellated positions. It clears owned geometry and reports `UsdCurveError` in
the inspector; corrected source data recovers on the same entity. The capture
example exits with an error instead of saving a partial scene. Missing counts
still infer one curve. Unknown type/basis/wrap tokens and unsupported segment
layouts report errors; nonperiodic Bezier requires `4 + 3n` control points,
periodic Bezier `3n`, and pinned Bspline/Catmull-Rom at least two. Periodic cubic
counts below three are not supported. Display primvars validate interpolation
cardinality, index bounds and finite authored values; single unindexed values
retain their broadcast behavior. `assets/invalid_curves.usda`,
`assets/invalid_curve_layout.usda` and `assets/invalid_curve_colors.usda` are
negative capture fixtures. Width and normal validation applies in surface mode.
Curve color/opacity interpolation uses f64 intermediates. Generated values that
cannot be represented as finite f32 RGBA report `UsdCurveError` before upload;
the renderer does not receive NaN or infinity from this path.
Curve projection also rejects output above 1,000,000 vertices or 2,000,000
indices per prim before allocating tessellation buffers. These limits include
all curves in the prim and use checked arithmetic. They do not bound source
decoding, the number of prims, asset-cache memory or total GPU memory.

`assets/periodic_curves.usda` compares two closed linear loops in one prim with
an open control. Periodic closure stays within each curve's vertex range:

```bash
USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='assets/periodic_curves.usda target/periodic-curves.png 0 -0.5 0.5 7 -0.5 0.5 0'
```

The two left squares should be closed; the right square intentionally lacks its
left edge. This follows [OpenUSD's periodic segment rules](https://openusd.org/dev/api/class_usd_geom_basis_curves.html).
Pinned Bspline and Catmull-Rom curves add extrapolated phantom control points
per curve so the tessellated output reaches both authored endpoints. Bezier and
linear curves retain their nonperiodic behavior for `pinned` wrap.
`assets/pinned_curves.usda` places pinned curves on the left and explicitly
expanded equivalents on the right, with Bspline above Catmull-Rom:

```bash
USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='assets/pinned_curves.usda target/pinned-curves.png 0 0 0.5 8 0 0.5 0'
```

Each row should have matching shapes and endpoint heights. Tessellation defaults
to eight samples per segment rather than adaptive or exact-limit rendering.

`assets/curve_widths.usda` is a width/ribbon acceptance fixture with an authored
camera. The current line renderer ignores these widths and normals; it is not
yet a tube/ribbon showcase. Capture its current baseline with:

```bash
USD_CAPTURE_CAMERA=/Camera USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='assets/curve_widths.usda target/NEW_CURVE_WIDTHS.png 0'
```

The local native Embree renderer rejects BasisCurves, so its black capture is
not a reference for this fixture.

The reusable `read::curves::read_widths_at` and `read_normals_at` readers preserve
composed interpolation, indexing and time-sampled values with primvar precedence.
They reject malformed data but leave topology-dependent cardinality, basis
sampling and surface construction to the projection path; this does not yet
change line rendering.

Library applications can choose 1–64 samples per cubic segment:

```rust
app.insert_resource(usd_bevy::route::curves::UsdCurveSettings::new(32)?);
```

The setting controls cubic positions and display-primvar interpolation together;
linear curves are unchanged. Existing output-size limits still apply.
`LiveStagePlugin` and `UsdAssetPlugin` reproject existing curve entities when the
effective sample count changes, retaining independent instance clocks. Removing
the resource restores eight samples. This does not add width-aware curve surfaces.

For the viewer and fixed-camera capture tool, set `USD_CURVE_STEPS=1..64` before
launch (default eight). Invalid values fail before renderer startup. Capture
metadata records `curve_steps`.

The Rendering pane's **Cubic curves** section switches live between 1, 8, 32
and 64 samples, shows the effective setting, and lists curve projection errors.

```sh
USD_CURVE_STEPS=32 make run ARGS='assets/curve_gradients.usda'
USD_CURVE_STEPS=32 make run APP_TARGET='--example viewer_capture' \
  ARGS='assets/curve_gradients.usda target/curves-32.png 0 0 0 8 0 0 0'
```

`assets/periodic_bezier.usda` compares a three-control-point periodic Bezier
with an explicitly closed four-point nonperiodic equivalent. Both render the
same cubic loop, not the triangular control hull.

`scene_report` distinguishes referenced vertices, unreferenced vertices, invalid
referenced normals and out-of-range indices. Its total invalid-normal count still
includes unused vertex data; that total alone does not establish a draw defect.
These counts describe source-sampled meshes, not visibility-filtered GPU draws.
`scene_report` also reports exact authored-vertex-normal preservation, degenerate
and exact-duplicate triangles, normal/face alignment and indexed versus
exact-position-welded boundary edges. These are local, undeformed triangle-data
diagnostics across all traversed meshes, not a visibility-filtered reference
render. Geometric boundary counts can include intentional openings; they do not
by themselves establish a rendering defect.
Set `USD_REPORT_EXPORT_LAYER=NEW.usda` to write a diagnostic root-layer text
snapshot; it refuses existing files and does not rebase relative assets. This
is not a validated interchange exporter: see the native compatibility findings
in `OPENUSD_UPGRADE.md`.

For independent reference renders, `make capture-reference ARGS='...'` invokes
an installed native `usdrecord` (`USD_RECORD` overrides the executable):

```bash
make capture-reference ARGS='--renderer Embree --disableGpu assets/subdivision_cube.usda target/native-cube.png'
```

Renderer availability depends on that native installation. Inspect the result;
successful image writing does not prove that the native stage contained meshes.

For a local-space triangle/normal isolation probe:

```bash
make run RUN_WITH= APP_TARGET='--example normal_isolation' ARGS='ASSET.usdc /Mesh NEW_DIRECTORY'
```

It writes `with_normals.usda` and `without_normals.usda` with identical projected
triangle positions/indices and a shared `/Camera`. The first copies projected
normals; the second omits them. Both explicitly use polygon geometry. An optional
fourth argument applies finite subdivision (1..6) before copying the triangles.
This is a time-zero diagnostic, not a scene exporter: original transforms,
deformation, materials, subsets and other primvars are omitted. Render either
through `viewer_capture` with `USD_CAPTURE_CAMERA=/Camera`, or through native
`capture-reference` with `--camera /Camera --imageWidth 1280`.
The probe also reports scale-independent triangle-quality bins and diagnostic
normal recomputations excluding very thin triangles at two thresholds. These
report changed vectors, reversed normal corners and zero referenced normals;
they do not modify the emitted normals or enable a production filtering rule.
Indexed fan diagnostics join corners across exactly two-face edges, report
nonmanifold edges and inconsistent winding, and count disconnected fans without
position welding. Zero-normal witnesses print at most eight referenced vertices
and twelve incident triangles each, including face directions and corner angles.

For an independent one-level position check, the optional native diagnostic
`scripts/compare_subdivision_reference.cpp` uses an installed OpenSubdiv SDK:

```bash
OSD_INCLUDE=/path/to/include OSD_LIB=/path/to/lib make \
  --eval='compare-osd:; @$(CXX) -std=c++17 -O2 -I$${OSD_INCLUDE} scripts/compare_subdivision_reference.cpp -L$${OSD_LIB} -Wl,-rpath,$${OSD_LIB} -losdCPU -o target/compare-osd && target/compare-osd $(ARGS)' \
  compare-osd ARGS='CAGE/with_normals.usda LEVEL1/with_normals.usda 17054'
```

Inputs must be uncreased triangle-cage and level-1 files from `normal_isolation`,
with unchanged point indexing and edge-and-corner boundary interpolation. This
is a reader for that tool's single-line arrays, not a general USDA parser.
The comparison maps OpenSubdiv child vertices to our vertex/face/sorted-edge
ordering, checks all positions with absolute component tolerance `1e-7`, and
optionally prints native limit-tangent cross-products at requested vertex indices.
Exit codes are 0 for matching positions, 1 for mismatch, and 2 for invalid inputs.
It does not compare refined topology, materials, authored normals, creases or
rendered images. Native limit tangents are diagnostic only, not installed into
the viewer. OpenSubdiv is not a new viewer build dependency.

`--compare-normals` additionally compares the probe's normals against native
limit derivatives evaluated on the same refined float positions, at absolute
component tolerance `1e-5`. Using native double-precision refined positions
instead can amplify tiny position differences around nearly cancelling fans.

Append `--normal-probe NEW_DIRECTORY` after any witness indices to write two
controlled polygon probes with the input camera and triangle indices retained:
`with_limit_normals.usda` replaces only normals with normalized native limit
tangent cross-products; `with_limit_surface.usda` also replaces positions with
native limit positions. The directory must not exist. Zero derivatives remain
zero and are counted separately for referenced vertices; no fallback axis is
invented. These files are finite triangles, not native subdivision patches.
Check the compiled tool's planar orientation, output preservation and invalid
inputs through Make:

```bash
make --eval='check-osd:; @/bin/bash scripts/check_subdivision_reference.sh target/compare-osd' check-osd
```

From the repository root, the optional collection regression generates fresh
UR5 shoulder and Spot base probes, records source hashes, then compares both
positions and limit normals against the compiled native tool:

```bash
make --eval='check-osd-assets:; @/bin/bash scripts/check_subdivision_assets.sh target/compare-osd ../usd_collection target/NEW_NATIVE_CHECK' check-osd-assets
```

`assets/subdivision_connectivity.usda` places disconnected and connected triangle
cages side by side with identical local face-corner positions. Under level-one
Catmull-Clark, the disconnected boundary opens while shared vertices stay shared.
The ordinary regression preserves that distinction instead of welding positions.
Compare both cages' refined positions and normals against the native tool with:

```bash
make --eval='check-connectivity:; @/bin/bash scripts/check_subdivision_connectivity.sh target/compare-osd target/NEW_CONNECTIVITY_CHECK' check-connectivity
```

This isolates topology-dependent gaps; it does not identify every seam in a
production asset or authorize automatic welding of intentional boundaries.

To compare every reported mesh under a literal prim-path prefix, retaining
failures rather than stopping at the first mismatch:

```bash
make --eval='check-meshes:; @/bin/bash scripts/check_subdivision_meshes.sh target/compare-osd ../usd_collection/oems/spot_boston_dynamics/spot_base_urdf/spot.usdc target/NEW_SPOT_CHECK /spot/base/' check-meshes
```

This compares local-space, level-one triangle cages under the native tool's
Catmull-Clark edgeAndCorner rules, not visibility, transforms, materials or other
subdivision rules. The prefix is a selection filter, not a visibility test.
Results include per-prim paths, isolation logs, native logs and a summary;
zero selected meshes or any failure returns nonzero. Native normal comparisons
print up to eight mismatching vertex indices and normal vectors.
`assets/subdivision_bowtie.usda` is a known-failing two-triangle normal witness
from Spot: matching positions, but opposite fallback/native normals at the
shared vertex whose two incident triangles share no edge. Both face windings
point +Y, agreeing with Bevy's fallback, while the native corner tangent cross
points -Y. The cyclic-index control reproduces the same difference. The native
tool labels the vertex non-manifold and still reports failure; this discrepancy
is not grounds to flip Bevy's normal. General non-manifold normal parity is not
claimed.

Both native checks require a new output directory and retain logs and generated
assets on failure. The asset check additionally requires the collection; the
connectivity fixture is included in this repository. These OpenSubdiv checks are
separate from the ordinary Rust suite and do not perform image acceptance.

`assets/normal_scale.usda` places three identical world-size panels side by side
using local coordinate scales of `1e-12`, `1`, and `1e12`. It exercises generated
flat normals independently of authored normals. Capture without shadow maps:

```bash
USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='assets/normal_scale.usda target/normal-scale.png 0 0 0 6 0 0 0'
```

The capture command saves the viewport after 120 document/camera-ready updates and leaves the viewer
open. Set `USD_CAPTURE_TIME=0` for the starting state. These fixed-time captures
do not demonstrate interactive playback or inspector controls.

`make run ARGS='assets/material_subsets.usda'` shows two material-bound panels.
Their orange/blue assignments swap at time code 10; use the timeline to seek.
`make run ARGS='assets/point_material_subsets.usda'` instances that layered asset
three times with shared subset meshes and materials.
`make run ARGS='assets/point_deformation.usda'` shows three CPU-skinned prototypes;
seek to time code 30 for the bent pose.

### Typed component authoring

`usd_bevy::sync::author_component_value(&registry, &stage, "/Enemy", &health)`
authors a registered `Component + Reflect` value directly, without an ECS entity
or string type-name argument. Register `ReflectComponent` for the type. All
encodable fields commit atomically into the current edit target; a field error
rolls back the write. Read-only instance proxies/prototypes are rejected.
This is a field patch: supported absent options are omitted and do not clear
previous opinions. Unsupported fields reject typed writes before editing the
stage. `sync::component_field_issues(&value)` reports field paths, Rust types and
whether each omission is unsupported or an absent option. Empty reflected struct
and tuple-struct components author a bare boolean presence opinion. The older entity-based
`author_component` still patches only encodable fields. This is not complete
arbitrary-component serialization.

`custom bool bevy:Marker = true` constructs a registered default component even
without field opinions. `false` suppresses its USD projection, including any
remaining field opinions, and removes it only if the route owned it previously;
unowned runtime components are preserved. Clearing that boolean resumes ordinary
field-driven projection, or removes an owned marker if no fields remain. Typed
empty-component writes set presence to true. Nested empty fields and root enum
components remain unsupported. An invalid presence value reports a projection
issue and retains the previous component.

Use `sync::author_component_presence::<Health>(&registry, &stage, "/Enemy", false)`
to author suppression without a string type name or component value. Passing true
re-enables projection of the retained field opinions. This uses the same checked
type naming, owner validation and atomic current-edit-target writer as typed field
authoring. The returned attribute name can be passed to `authoring::clear_attribute`
to remove the local presence opinion and reveal weaker layers. Field patches do
not implicitly override an existing false presence opinion.

For reload-persistent root-local edits, call
`overrides.set_component(&registry, "/Enemy", &health)` on
`UsdInstanceOverrides` and attach it to the `UsdSceneRoot` entity. The method
encodes and validates the entire typed patch before modifying the override list,
replacing matching prim/property entries without duplicating them. Unrelated
opinions and absent-option overrides remain intact. These opinions are reapplied
to each new source snapshot; direct `UsdInstances::stage` edits are transient.
Stage-dependent validation is deferred until projection and can fail the root's
load state. Removing an override reveals the latest source opinion.

Unique short component names retain `bevy:Health:field` attributes. Colliding
short names use the full reflected Rust path with `::` encoded as `__`; authoring
checks that the segment resolves back to the same registered type before writing.
The entity-based API also accepts full Rust paths and encoded paths. An ambiguous
short-name argument or a lossy qualified encoding is rejected. Register all
component types before authoring: adding a short-name collision later does not
migrate existing short-name opinions. Refactoring Rust paths requires migrating
qualified opinions in saved USD files.

Integer projection uses checked range conversions, including `Option<i32>`.
Negative-to-unsigned, overflow, fractional and nonfinite floating-point opinions
are rejected rather than wrapped, truncated or saturated. Rejected assignments
leave the existing field unchanged and emit the route's unsupported-value warning.
Integral floating-point values remain accepted when representable by the target.

`route::reflect::UsdReflectIssues` exposes current projection failures on the prim
entity: missing registry/type/component reflection/default, unknown fields and
unsupported values. Each issue includes its type segment and optional reflected
field path. Successful reprojection clears resolved issues. The editor snapshot
publishes the selected prim's issues after projection; the inspector lists them
as component issues. These are diagnostics, not atomic component-level rollback.

Short and qualified aliases resolving to the same registered type share field
ownership. Disjoint fields merge; clearing one alias does not remove fields still
authored under another. Multiple effective aliases for the same field produce
`ConflictingAliases`: projection retains the previous component until the conflict
is resolved, rather than inventing a strength order between different attributes.
On first projection a conflicting component is not constructed. Diagnostics use
the canonical Rust path for resolved types and the authored segment otherwise.

```sh
make run RUN_WITH= APP_TARGET='--example typed_authoring'
```

The example authors a sphere through upstream's typed schema API, adds a typed
`Health` value, projects one source twice and verifies isolated subsequent edits.

### Low-level viewport capture

Mesh conversion honors sampled `holeIndices` even when subdivision refinement is
disabled. Hole faces emit no triangles, while source face/corner numbering remains
unchanged for primvars and material subsets. Refinement retains hole-face support
until after tessellation; this does not make the default control cage a subdivision
surface.

Inspect sampled source geometry and the current mesh conversion with:

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example scene_report' ARGS='path/to/scene.usdc 0'
```

The report includes point/face/corner counts, authored normal interpolation and
invalid-normal counts, UV counts, projected vertex/unique-normal counts, and
subdivision settings with authored-versus-fallback status. It traverses meshes
without visibility filtering and does not evaluate skinning or morph deformation.
Subdivision meshes produce a warning: conversion currently renders their control
cage, not the requested subdivision surface. In particular, Spot's collection
asset omits `subdivisionScheme`, resolving to Catmull–Clark despite having vertex
normals. [USD defaults meshes to subdivision surfaces](https://openusd.org/24.08/user_guides/render_user_guide.html);
forcing smooth polygon normals would not establish fidelity for this asset.

`subdivision::catmull_clark` now provides a separate finite-level CPU position and
topology kernel for consistently oriented, edge-manifold cages, with edge-only/edge-and-corner
boundary modes and source-face IDs on refined quads. It validates topology and
caps refinement levels/output sizes. Rendering is opt-in and incomplete:
other subdivision rules and limit-surface evaluation
remain required. Analytic cube/boundary tests
are not OpenSubdiv reference-renderer acceptance.
The returned `RefinedSurface::interpolate_vertex` reuses recorded per-level weights
for finite scalar/vector control-point data. Geometry itself uses those same
weights. Separate methods interpolate linear varying values, copy face-uniform
values by source-face ID, and refine expanded face-corner values with the explicit
all-linear rule while preserving UV seams. Other face-varying rules, normals of
the limit surface and gamma/color-space conversion are not implemented. Weight
storage is bounded across all retained levels.
`interpolate_primvar` validates and expands indexed mesh primvars before applying
their interpolation; face-varying data requires explicit `AllLinear` selection.
`remap_subsets` maps material face membership to refined faces, preserves binding
paths and rejects out-of-range source indices.
`read::subdivision::read_subdivision_at` reads composed rules, creases, corners
and holes at a requested time. `ReadSubdivision::validate` checks sharpness
cardinality and index bounds against sampled mesh dimensions; it does not check
whether crease pairs are actual topology edges. The scene report includes these
rules and counts. Reading a rule does not imply the renderer evaluates it.
`subdivision::refine_mesh` applies supported Catmull-Clark or bilinear rules to sampled mesh
data, including indexed UV/color/opacity primvars and material subsets. It removes
authored normals and generates per-corner normals for permanent sharp features;
conversion handles other finite-mesh normals. These are not limit-surface normals.
Unsupported interpolation modes return errors.
Set `USD_SUBDIVISION_LEVELS=1` (1–6) for the viewer or offscreen
capture, or insert `route::subdivision::UsdSubdivisionSettings` before projection.
The mesh route path refines CPU-deformed control points before material subsets.
It exposes `UsdSubdivisionApplied` or `UsdSubdivisionError`; failures suppress
generated mesh/subset geometry, and offscreen capture fails instead of saving a
partial result. Direct mesh point-instancer prototypes use the same CPU refinement
before prototype transforms and material-subset splitting, retaining shared mesh
handles. Hierarchical prototypes remain unsupported.
Changing or removing the settings resource automatically reprojects existing
meshes and point instancers in live stages and asset instances, preserving each
instance's clock and runtime-owned entities. Catmull-Clark meshes with no
remaining sharpness or holes, using edge-only or edge-and-corner boundaries, use limit-surface
normals evaluated on the refined float positions. Disconnected vertex fans retain
finite-mesh normals. Zero authored sharpness and sharpness fully decayed by the
selected refinement level use the same limit-normal path. Remaining creases,
corners, holes and boundary-none use the existing
finite-normal path. Bilinear surfaces remain flat shaded. GPU subdivision and
full USD limit-surface fidelity remain unsupported.
With subdivision enabled, unknown/malformed scheme values are errors, not silent
polygon fallbacks. The selected prim's inspector shows subdivision, deformation
and point-instancer diagnostics. For UI capture, `USD_VIEWER_PANE=inspector` and
`USD_VIEWER_SELECT=/Prim` open the inspector with an initial selection; pane values
also include `lighting`, `timeline` and the default outliner.
The Rendering pane (`USD_VIEWER_PANE=rendering`) switches between the control cage
and subdivision levels 1–6 at runtime, reports refined mesh prim counts, and lists
subdivision/point-instancer errors. These controls do not author USD opinions.
Its Scene assets section counts USD mesh entities and distinct referenced mesh
and StandardMaterial handles, including hidden entities but excluding viewer
helpers. These are handle counts, not draw calls or GPU memory measurements;
custom material types are not included in the StandardMaterial count.
`assets/subdivision_cube.usda` and `scripts/replays/subdivision_toggle.replay`
exercise level 2 at five seconds and restore the control cage at fourteen seconds;
capture at eleven or twenty seconds respectively to inspect the two states.
`subdivision::bilinear` preserves existing vertices and uses linear edge/face
interpolation. Vertex and varying primvars use the same linear weights, and
face-varying values remain linear across all schema interpolation settings.
Warped quads are sampled as bilinear patches; finite triangles still approximate
the surface. Bilinear positions are unaffected by sharpness.
The Catmull–Clark adapter supports uniform-decay crease/corner position stencils,
including fractional transitions, per-chain/per-edge sharpness, and permanent
sharpness at values of 10 or higher. Crease pairs must name actual topology edges;
duplicate crease edges and duplicate corner entries are rejected. The decay and
transition rules follow [OpenSubdiv's crease implementation](https://raw.githubusercontent.com/PixarAnimationStudios/OpenSubdiv/release/opensubdiv/sdc/crease.cpp).
`assets/subdivision_creases.usda` animates all cube-edge sharpness through 0, 0.5,
2 and 10. Permanent creases and corners split finite-mesh normal averaging into
smooth face fans after hole filtering. Other points retain smooth averaging;
normal data uses face-varying interpolation and preserves subset point mappings.
Chaikin creasing, exact semi-sharp/limit normals and reference-renderer parity
remain unverified/unsupported.
Authored hole faces retain their subdivision influence but omit their refined
descendants from rendering; uniform/face-varying data and subsets are filtered
with the topology. `assets/subdivision_holes.usda` demonstrates a center hole.
Catmull–Clark's `interpolateBoundary = "none"` similarly omits faces touching
boundary vertices while retaining their refinement support. Bilinear refinement
does not implicitly hide boundary faces under this setting.
**Experimental:** scale-independent generated normals fix the large dark patches
in the first level-1 Spot capture. Smaller seams/faceting remain, and comparison
against a reference renderer is still required. The default remains control cages.
An optional third scene-report argument (`ASSET TIME LEVELS`) runs this refinement
check for 1–6 levels and returns failure if any mesh cannot be refined. All 26
traversed Spot meshes pass one-level refinement. Disconnected vertex fans keep
their shared vertex fixed, following OpenSubdiv's infinite-sharpness treatment
for this case; non-manifold edges and inconsistent winding remain unsupported.
The rule is visible in [OpenSubdiv's topology initialization](https://github.com/PixarAnimationStudios/OpenSubdiv/blob/release/opensubdiv/far/topologyRefinerFactory.cpp).
No topology welding or splitting is performed. This is not rendered parity proof.

For the actual Mara viewer window, including panes, use an isolated Weston
compositor. This Linux-only script creates a private runtime directory, captures
one framebuffer and stops only its own viewer/compositor process groups:

```sh
make build CARGO='cargo --offline'
USD_VIEWER_DOME=/Env USD_VIEWER_PANE=lighting nix shell nixpkgs#weston -c /bin/bash scripts/capture_viewer_ui.sh assets/dome_directional.usda target/viewer-ui.png
```

Choose a new output filename. Companion `.viewer.log`, `.weston.log` and
`.capture.log` files are retained. The script waits up to 300 seconds for the
viewer's first completed UI update, then `USD_UI_CAPTURE_WAIT` sets a 1–300 second
delay (default 20). Build-lock waits and synchronous first-update projection do
not consume that delay. `USD_VIEWER_STARTED` marks viewport construction;
`USD_VIEWER_UI_UPDATED` starts the delay. Neither proves GPU pipeline readiness
or final texture upload completion; inspect the screenshot. Before reporting
success, the script runs `capture_inspect` on a fixed interior viewer region of
its 1600×1000 output, excluding desktop wallpaper. Predominantly near-black
regions fail and retain the PNG plus `.inspect.log`; no automatic retry hides
the failure. This catches the observed black-window case, not arbitrary incorrect
rendering or incomplete uploads.

Set `USD_UI_CAPTURE_VIEWPORT=1` to additionally retain the embedded Bevy readback
as `.viewport.png`, `.viewport.rgba` and `.viewport.capture.txt`. Its dimensions
drive a separate full-image near-black check in `.viewport.inspect.log`. The
script waits up to `USD_UI_CAPTURE_TIMEOUT` additional seconds for the readback
after the UI delay; missing/failed readback is recorded in settings and fails
the run, but does not suppress the compositor screenshot attempt. Existing
viewport companions are rejected before launch.

These are two capture paths, not synchronized frames: Bevy requests its readback
after 120 document/camera-ready updates and its optional ready delay, whereas
Weston captures after the UI delay. For the 10-second visibility-click replay,
`USD_CAPTURE_DELAY_MS=15000 USD_UI_CAPTURE_WAIT=20` targets the post-click state
on both paths. Timing metadata identifies the request, not the exact rendered
frame or callback time. A healthy viewport
with a black host image narrows the failure to host presentation/composition or
compositor readback, not necessarily to Weston itself. Neither near-black check
proves scene fidelity or completed asset uploads.

`examples/host_capture_probe.rs` draws colored panels and text with the same
Mara native runner, without constructing a Bevy app or loading USD. Use it to
isolate host/compositor failures (the script's required asset argument is ignored
by this example):

```sh
USD_UI_CAPTURE_VIEWPORT=0 make APP_TARGET='--example host_capture_probe' --eval='capture-host:; @/bin/bash scripts/capture_viewer_ui.sh assets/retimed_instances.usda target/NEW-host-only.png' capture-host
```

Use the Weston-enabled environment described above. This is a diagnostic, not
the viewer or proof of its rendering fidelity.
Repeated local Vulkan Weston captures have reproduced a wholly black host-only
window too: neither Bevy nor USD is required. Inspect and preserve failed runs;
do not interpret a few successful probe launches as a reliable fallback.

`examples/bridge_capture_probe.rs` adds only the Bevy viewport bridge, a blue
clear color and an orange unlit cube, with a white egui label over the viewport.
It has no USD plugins, stage, lights or editor. Compare it with the host-only
probe to isolate failures that require the embedded Bevy renderer:

```sh
USD_UI_CAPTURE_VIEWPORT=0 make APP_TARGET='--example bridge_capture_probe' --eval='capture-bridge:; @/bin/bash scripts/capture_viewer_ui.sh assets/retimed_instances.usda target/NEW-bridge-only.png' capture-bridge
```

The asset argument is ignored. Inspect both the cube and the egui label; a
nonblack background alone does not prove that the bridge rendered correctly.
The probe defaults to `USD_BRIDGE_PROBE_TRANSFER=native`, sharing the host wgpu
device and registering the Bevy texture directly. Set
`USD_BRIDGE_PROBE_TRANSFER=cpu` to construct an independent Bevy device and use
Mara's existing CPU readback/egui texture upload path. Both construction and
per-frame drawing omit the host render state in CPU mode. This diagnostic does
not change the viewer's transfer path or establish a production fallback.
Set `USD_BRIDGE_PROBE_DELAY_MS=33` to deliberately block each probe UI update
for 33 milliseconds while retaining the selected transfer path. The default is
zero; accepted values are integer milliseconds from 0 to 1000. This bounds the
probe's update rate independently of egui repaint requests, but blocks input
handling too: it is a timing diagnostic, not production frame pacing.

`examples/eframe_capture_probe.rs` removes the Mara runner as well, drawing only
colored panels and text with eframe's wgpu backend (`AutoNoVsync` by default,
matching Mara). Set `USD_HOST_PROBE_PRESENT_MODE=auto-vsync` for the explicitly
vsynced comparison, or `auto-no-vsync` for the default. Set
`USD_HOST_PROBE_SCREENSHOT` to a new PNG path for eframe's direct GPU screenshot:

```sh
USD_HOST_PROBE_SCREENSHOT="$PWD/target/NEW-eframe.host.png" USD_UI_CAPTURE_VIEWPORT=0 make APP_TARGET='--example eframe_capture_probe' --eval='capture-eframe:; @/bin/bash scripts/capture_viewer_ui.sh assets/retimed_instances.usda target/NEW-eframe.png' capture-eframe
```

The example requests readback in its first painted UI pass and logs `HOST_READBACK_OK`
or `HOST_READBACK_ERROR`; callback timeout is checked on UI updates after thirty
seconds. The compositor script does not validate this additional readback: check
the log and inspect both PNGs. The readback and compositor capture are not the
same frame. Existing direct-readback files are never overwritten. This isolates
host paths; it is not a replacement viewer or proof of USD rendering fidelity.

The `.settings.txt` companion records compositor backend, output/inspection
dimensions and wait time. Set `USD_UI_CAPTURE_SCENE_GRAPH=1` to collect Weston's
one-shot surface/buffer dump in `.scene-graph.log` immediately before capture.
This opt-in diagnostic requires `weston-debug` and `timeout`; it has a five-second
timeout followed by a one-second kill grace. Failure is recorded in settings and
does not suppress the screenshot. The dump describes submitted surfaces, not
their pixel correctness or rendering readiness. See
[Weston's debug scopes](https://wayland.pages.freedesktop.org/weston/toc/libweston/log.html).

The `.settings.txt` companion also records whether the diagnostic was requested.
`USD_UI_CAPTURE_TIMEOUT` bounds the screenshot subprocess to 1..120 seconds
(default 15), followed by a one-second kill grace. A failed or timed-out command
records its exit status in settings, retains its log and triggers normal cleanup;
it does not emit `UI_CAPTURE_OK` or retry automatically.
Vulkan remains the default. `USD_UI_COMPOSITOR_RENDERER`
accepts `vulkan`, `gl` or `pixman` for diagnostic experiments; the latter two are
not validated alternatives. Three local GL captures were vertically inverted
despite passing the near-black check, so GL emits an orientation warning. Three
paired Vulkan captures were correctly oriented, but this small comparison did
not reproduce or resolve the intermittent black-window failure.
Pixman also emits a warning: a native playback attempt failed during window
creation with wgpu reporting incompatible Vulkan and GL surface backends. No
frame or PNG was produced; software compositing is not a working fallback here.

The low-level inspector also accepts explicit PNG regions:

```sh
make run RUN_WITH= APP_TARGET='--example capture_inspect' \
  ARGS='target/viewer-ui.png 110 110 1400 800'
```

Optional final arguments set the black-channel threshold (default 8) and minimum
nonblack percentage (default 1). The executable exits 0 for a passing region,
1 for predominantly near-black, and 2 for invalid input; make wraps nonzero exits.
Input is limited to 32 MiB and decoded output/decoder allocations to 64 MiB each.
Animated PNGs are rejected. Alpha is ignored; passing is not visual acceptance.
At render startup the viewer caps mesh allocator slabs at half the shared device's
maximum buffer size, retaining smaller configured limits. This leaves allocation
rounding headroom and avoids growing pooled buffers beyond the host's limit.
It is not a total-memory budget or support for individual meshes exceeding the
device limit. Large scenes can still spend substantial time projecting before
the first UI update.
Reported Bevy renderer failures appear in the Outliner status and Rendering pane,
taking precedence over document Ready/save-dialog status. Rendering stays stopped;
the handler does not request application exit or attempt automatic recovery. Save
edits before restarting. The first diagnostic is retained, with long descriptions
limited to 2,048 characters plus an ellipsis; full errors remain in the log.
Use real `/bin/bash`, not the local `bash` wrapper. Weston uses its Vulkan renderer
by default (`USD_UI_COMPOSITOR_RENDERER` overrides it); on this machine the GL
capture was vertically inverted and Pixman could not host the viewer GPU surface.
Weston's debug/capture interface is enabled only inside the private runtime, not
on the user's desktop. See [Weston's headless backend documentation](https://wayland.pages.freedesktop.org/weston/toc/running-weston.html).

Native-picker captures can also isolate D-Bus and XDG config/data/cache directories:

```sh
portal=$(nix build --no-link --print-out-paths nixpkgs#xdg-desktop-portal)
gtk=$(nix build --no-link --print-out-paths nixpkgs#xdg-desktop-portal-gtk)
USD_UI_CAPTURE_PRIVATE_BUS=1 XDG_CURRENT_DESKTOP=gnome \
XDG_DATA_DIRS="$portal/share:$gtk/share" \
USD_UI_REPLAY=scripts/replays/open_dialog.replay \
nix shell nixpkgs#weston -c /bin/bash scripts/capture_viewer_ui.sh assets/editor_samples.usda target/native-picker.png
```

`save_dialog.replay` opens the root-layer save picker instead;
`save_edit_dialog.replay` and `save_flattened_dialog.replay` open the other save modes.
The filename extension selects `.usda`, `.usdc`, `.usd`, or `.usdz` output;
the defaults are `scene.usda`, `edit-layer.usda`, and `flattened.usda`, respectively.
Unsupported or missing extensions are rejected without writing a file. These replays only
open the chooser; they neither select a file nor save one. Their coordinates are
specific to the 1600x1000 capture layout. Private-bus cleanup stops the viewer's
session and removes its temporary XDG directories. Native dialogs currently lack
a parent-window handle from Mara; GTK reports that missing association.

Set `USD_UI_REPLAY=path/to/input.replay` to replay input into this viewer's egui
input pipeline. Each line has an elapsed millisecond timestamp followed by
`move X Y`, `down X Y`, `up X Y`, `scroll DX DY`, or `text TEXT`. Coordinates and
scroll deltas are window-local logical points; timestamps must be nondecreasing.
Separate pointer movement, press and release into successive frames. Text retains
spaces after its single separator. Blank lines and `#` comment lines are ignored.
The tool is disabled unless explicitly configured, logs event times without text
contents, and does not inject OS input or issue editor commands directly.
Replays use elapsed time rather than widget readiness; inspect captures and logs
before treating them as acceptance evidence. For example:

```text
5000 move 600 450
6000 scroll 0 -700
```

The repeatable sample-authoring replay uses `assets/editor_samples.usda` and
`scripts/replays/sample_history.replay`. Build first, retain the private
compositor's default 1600x1000 output and default UI scale, and select `/Model`:

```sh
make build CARGO='cargo --offline'
USD_UI_CAPTURE_WAIT=24 USD_UI_REPLAY=scripts/replays/sample_history.replay USD_VIEWER_PANE=inspector USD_VIEWER_SELECT=/Model nix shell nixpkgs#weston -c /bin/bash scripts/capture_viewer_ui.sh assets/editor_samples.usda target/sample-clear.png
```

Use a new output name for each run. Capture at 16 seconds to inspect Undo
(`0, 10`), 20 for Redo (`0, 10, 25`), 24 for Clear Sample (`0, 10`), or 28 for
undoing that clear (`0, 10, 25`). These are expected scene-sample keys; the default
value stays `5`. The replay does not save the fixture. Coordinates are layout
dependent, so a successful capture alone does not establish the expected result.

The current Mara host creates a GPU device with four storage textures per shader
stage. Bevy's dome filtering requires at least six plus compute support, so the
embedded viewer cannot currently filter dome maps on that device. The adapter
reports `Unavailable` instead of waiting indefinitely. Standalone captures request
a different device and have passed IBL checks; they do not prove embedded-host
capability. The Mara GPU configuration hook remains a separate integration change.

For opt-in runtime dome IBL, add
`usd_bevy::route::dome_environment::UsdDomeEnvironmentPlugin` and attach
`UsdDomeEnvironmentSource::new(dome_entity)` from that module to a camera.
The selected dome must have a loaded `UsdDomeTexture`; AssetServer snapshots
provide it. The adapter converts latitude-longitude textures and lets Bevy filter
the cubemap on the GPU. It preserves/restores the camera's previous environment
on deselection and leaves ambient lighting and skybox selection unchanged.
`UsdDomeEnvironmentState::Attached` reports map attachment, not GPU completion.
Runtime filtering requires a compute-capable PBR renderer.

An isolated dome capture disables directional lights and camera ambient:

```sh
USD_CAPTURE_DOME=/Env make run APP_TARGET='--example viewer_capture' ARGS='assets/dome_environment.usda /dev/shm/dome.png 0 0 3 7 0 1 0'
```

Time `10` is the zero-intensity control. The fixture's constant warm HDR tests
diffuse/specular illumination, not directional detail or reflection orientation.
The viewer's **Lighting** pane lists projected domes and offers studio-only,
dome-only and combined lighting. These are viewport settings and do not author
USD opinions. The studio toggle affects only the viewer's own lights and camera
ambient; authored USD lights remain unchanged. A missing selected dome is reported
and detached, rather than silently replaced by another dome.
For startup selection, use `USD_VIEWER_DOME=/Env`; `USD_VIEWER_PANE=lighting`
opens that pane initially. Without a selection, studio lighting remains the default.
EXR and non-HDR dome color-space
metadata remain unsupported; automatic layout currently accepts only 2:1 latlong.
`assets/dome_directional.usda` uses a red/blue HDR and rotates the dome 180 degrees
between times 0 and 10. Use the same camera arguments above for both captures;
the blue contribution moves from right to left on the spheres. Capture metadata
records the applied environment intensity and quaternion. Owned generators stop
after their filtering commands have been recorded. Image/tint/resolution changes
start another generation; rotation/intensity changes reuse the filtered maps.
`UsdDomeEnvironmentDiagnostics` counts recorded generations, not GPU time or
completion fences. Dome captures wait for generator retirement and record both
the generation count and active-generator count.

```sh
make run APP_TARGET='--example viewer_capture' ARGS='assets/point_deformation.usda /dev/shm/capture.png 30 6 4 8 0 1 0'
```

This windowless tool renders through Bevy into a fixed 1280x720 GPU texture and
reads it back through Bevy's screenshot pipeline. It shares the viewer's studio
environment, USD routes and skinning selection, but does not capture Mara UI.
Arguments are asset, PNG output, USD time code, then optional eye XYZ and target
XYZ. The default eye is `(6,4,8)` looking at `(0,1,0)`. No orbit input or automatic
camera framing is used. The grid fits below visible mesh bounds using the viewer's
shared height/scale/fade calculation; this does not move the USD scene or camera.
Both grid fitting and initial viewer framing sample current morph and skin
deformation through `usd_bevy::mesh::bounds::MeshBounds`. This on-demand CPU
bounds calculation leaves vertex rendering on the GPU and does not replace
Bevy's culling bounds. Viewer framing still runs only once per opened document.

For a repeatable deformation sweep, use a fresh output directory and an explicit
RGB tolerance (0 requests pixel equality):

```sh
make --eval='compare-deformation:; @/bin/bash scripts/compare_deformation.sh assets/skel_morph_subsets.usda target/deformation-check 0 0 15 30 45 60' compare-deformation
```

The script saves both PNG/RGBA captures, metadata, renderer logs, difference
images and `results.tsv` for every time, continuing after a failed pair. It
returns nonzero for capture/comparison failure, renderer warnings/errors, missing
GPU-deformation components in the GPU run, or GPU-deformation components in the
CPU control. Component counts are coverage checks, not proof of every draw or
GPU performance. Existing output directories are rejected. Inspect the images;
a chosen nonzero tolerance is not automatic evidence of visual fidelity.

Set `USD_CAPTURE_CAMERA=/Scene/Camera` to copy a projected USD camera's world
transform and projection instead of using eye/target arguments. Metadata records
the selected path; missing cameras wait until the capture timeout. The tool does
not activate the source camera itself. Camera transforms must be representable as
Bevy TRS; sheared camera transforms are not fidelity-validated.

The offscreen tool records `camera_eye`, `camera_forward`, `camera_up`, and
column-major `camera_world_from_view_cols` / `camera_clip_from_view_cols` after
camera/transform updates, in `Last` when requesting readback. These describe the
effective Bevy camera, including authored aperture offsets. `requested_eye` and
`requested_target` are only the CLI fallback settings (formerly the misleading
`eye` / `target` fields); they are not the active view when a USD camera is chosen.
This metadata is a request-time snapshot, not a GPU completion timestamp.

Perspective aperture offsets use `route::camera::UsdPerspectiveProjection` inside
`Projection::Custom`; unshifted cameras retain `Projection::Perspective`.
Viewport resizing preserves vertical FOV and physical aperture-offset/focal-length
ratios, adjusting horizontal coverage. Orthographic cameras retain their fixed
filmback policy. This follows the aperture-offset units described by
[OpenUSD GfCamera](https://openusd.org/dev/api/class_gf_camera.html).
`assets/camera_offsets.usda` animates horizontal/vertical offsets from zero:

```bash
USD_CAPTURE_CAMERA=/Scene/Camera make run APP_TARGET='--example viewer_capture' ARGS='assets/camera_offsets.usda target/camera-offset.png 10'
```

Set `USD_CPU_SKINNING=1` to compare the ordinary CPU skin path.
Set `USD_CAPTURE_INSTANCE_TIMES=0,10` to mount the same asset at independent
time codes in one capture. One to sixteen finite values are accepted; roots are
centered along X with 2.5-unit spacing. Use the fixed camera arguments to frame
the group; authored-camera selection is rejected for multiple roots. The default
remains one root at the positional TIME argument. Instance times and spacing
are written to capture metadata, and capture waits for every root to be Ready.
Set `USD_CAPTURE_SWAP_CLOCKS=1` with multiple instances to reverse their clocks
after 30 render-ready frames, then wait another capture settling period. It
mutates the existing roots without reopening the source. The default is 0; other
values and single-root reversal requests are rejected. Metadata records the
final clock order and whether the live reversal happened.

Run the live-clock GPU regression suite into a new directory:

```bash
make --eval='check-live-clocks:; @/bin/bash scripts/check_live_clocks.sh target/live-clock-check' check-live-clocks
```

It generates fresh UV/texture/scalar/color/normal fixtures and compares live clock
reversals against independent references. Seventeen comparisons use zero RGB
tolerance; normal-interface and constant-normal cases use RGB tolerance 1 for
independently animated geometry normals, matching the static precision bound.
Metadata must confirm the reversal and expected visible mesh counts. The flat
morph case verifies shared GPU material use; the morph-tangent case verifies four
GPU-morphed ordinary/subset meshes against CPU deformation without flat materials.
The subdivision-crease case uses level 1 and reverses clocks 1/3 to 3/1,
switching both roots between decayed-sharpness vertex normals and permanent-crease
face-varying normals. Its final image must match freshly loaded roots at 3/1.
The constant-coordinate case samples animated Material interface coordinates,
ignoring deliberately different vertex UVs, against independent mesh-UV samples.
Renderer warnings/errors
fail the case. PNG/RGBA captures, metadata, comparison logs and results.tsv remain
in the output directory, including on failure. This requires native GPU access
and uses offline Cargo through Make.
For the generated UV-transform fixture, use eye `(0,1,8)` and focus `(0,1,0)`:

```bash
USD_CAPTURE_INSTANCE_TIMES=0,10 USD_CAPTURE_SHADOWS=off make run APP_TARGET='--example viewer_capture' ARGS='target/uv_transform_probe/mapped.usda target/two-clocks.png 0 0 1 8 0 1 0'
```

Set `USD_CAPTURE_SHADOWS=off` to disable directional, point and spot shadow maps
for an isolated rendering comparison. The default `scene` preserves each light's
shadow setting. This diagnostic is recorded in metadata and does not edit USD.
Set `USD_CAPTURE_RENDERER=forward|prepass|deferred` to select the render path
(default `forward`). The latter two enable depth, normal and motion-vector
prepasses and disable MSAA; deferred also selects Bevy's deferred opaque renderer.
The selected renderer and subdivision level (`0` means disabled) are recorded in
capture metadata. Set `USD_SUBDIVISION_LEVELS=1..6` to refine the scene. Match renderer settings
as well as camera/time when comparing CPU and GPU captures.

It uses synchronous pipeline compilation and waits for scene readiness plus
60 consecutive frames with no pending/failed GPU pipelines, then writes `.png`, tightly
packed `.rgba` (RGBA8 sRGB, 5120 bytes per row), and `.capture.txt` settings.
`CAPTURE_OK` and exit status 0 indicate completed readback/file output, not scene
fidelity. Load, readback timeout and write failures produce a nonzero exit.
Use identical camera/time arguments and `cmp` on the raw files for repeat checks.
`/dev/shm` avoids the current `/tmp` quota issue; choose persistent storage for
long-term baselines. Repeated captures were byte-identical on the tested Vulkan
adapter; this is not a cross-GPU determinism guarantee.

Capture metadata also records hierarchy-visible mesh/GPU-mesh counts and CPU
fallback reasons. Visibility here does not prove a mesh was inside the camera
frustum. Compare raw captures with matching camera/time/resolution settings:

```sh
make run RUN_WITH= APP_TARGET='--example capture_compare' ARGS='/dev/shm/before.rgba /dev/shm/after.rgba'
```

The comparison reports changed RGB pixels, maximum channel error and mean
absolute RGB error. It ignores alpha (the PNG preview does too), exits nonzero
when pixels differ, and accepts an optional per-channel tolerance from 0 to 255.
Background pixels contribute to the mean; a small whole-image mean does not
establish mesh fidelity.

To locate differences, append a tolerance, image width and diagnostic PNG path:

```sh
make run RUN_WITH= APP_TARGET='--example capture_compare' ARGS='/dev/shm/before.rgba /dev/shm/after.rgba 0 1280 /dev/shm/difference.png'
```

Changed pixels are marked red/yellow over a dim grayscale reference; green
intensity encodes maximum RGB channel error. The command also reports inclusive
changed-pixel bounds. A successfully written diagnostic still exits nonzero when
the captures differ. Width must match the source image.

`assets/skel_double_sided.usda` and `assets/skel_backface_reference.usda` compare
backface illumination with an equivalent reversed-winding front face. Capture
both at time `30`, eye `(2,3,4)` and target `(0,0,0)` in forward mode.

`assets/skel_influences.usda` animates weights with a static joint pose. Capture
times `0`, `5` and `10` using eye `(4,3,6)` and target `(1,1,0)` to inspect
influence-only motion; set `USD_CPU_SKINNING=1` for the CPU comparison.
`assets/skel_constant_influences.usda` expresses the same motion with one
constant influence set shared by the entire mesh instead of per-vertex arrays.
`assets/skel_material_subsets.usda` exercises shared-palette GPU skinning with
orange/blue face materials. Capture time `30`, eye `(4,3,6)`, target `(0,1,0)`
and compare against `USD_CPU_SKINNING=1` using the same camera.
`assets/morph_tangent_normals.usda` exercises normal-mapped morphs with authored
normal offsets and material subsets. GPU morph preparation regenerates the
tangent frame on the CPU from sampled morphed positions/normals and authored UVs,
while position/normal offsets remain GPU morph targets. Rest-pose triangulation
is retained. This adds CPU work and time-specific mesh data; it is not a measured
performance improvement or proof of tangent parity after arbitrary skinning.
Set `USD_CPU_SKINNING=1` for the CPU reference and **unset** it for GPU rendering;
the override is presence-based, so setting it to `0` still selects CPU mode.

`assets/morph_animation.usda` exercises standalone GPU morph targets. Capture
time `10`, eye `(2,2,4)`, target `(0.5,0.5,0)`; `USD_CPU_SKINNING=1` also disables
the GPU morph route for comparison. Metadata reports GPU morph meshes separately
from GPU-skinned meshes.
`assets/morph_subsets.usda` adds a time-varying orange face subset to that morph
fixture. Use the same camera and time for GPU/CPU comparisons.
`assets/skel_morph_subsets.usda` combines a rotated skeleton, tip morph and
orange/blue subsets. Capture time `30`, eye `(4,3,6)`, target `(0,1,0)`.
`assets/morph_normals.usda` adds indexed authored vertex normals and sparse
normal offsets to the standalone morph-subset fixture. Capture time `10`, eye
`(2,2,4)`, target `(0.5,0.5,0)` for CPU/GPU normal-delta comparison.
`assets/skel_morph_normals.usda` adds constant skin influences, joint rotation
and nonuniform scale to the same normal-delta fixture; use the same capture settings.
`assets/skel_singular_normals.usda` has a zero-scale pose at time 10 and valid
poses at 0/20. An unevaluable deformation carries `UsdDeformationError` and omits
generated geometry/subsets until a valid sample or edit recovers it. The capture
tool treats this component as a failure and reports the prim path rather than
publishing a partial-scene screenshot.
`assets/skel_corner_normals.usda` adds an indexed face-varying normal seam to the
combined skin/morph fixture. Capture time 10 with the same camera arguments.
Authored constant, uniform, vertex, varying and face-varying normals are retained
through deformation. Constant normals become per-point; uniform normals become
per-corner when deformation can vary within the authored interpolation domain.
`assets/skel_corner_influences.usda` exercises corner normals with different
sampled source-point skin influences. Generated smooth normals and animated
normal-map tangent fidelity remain separate limitations.

### Saving files

The viewer polls native open/save dialogs asynchronously and requests a repaint
when a selection completes. Only one dialog can be pending. Paths are accepted
without lossy Unicode conversion. Save selections carry a process-local document
ID and edit-layer identity; `EditorCommand::SaveChecked` revalidates both when the
command executes. A changed document/target requires choosing the destination
again. `EditorCommand::Save` remains an unconditional programmatic operation.
This keeps file selection off the blocking UI path; USD loading/export itself
still runs on the editor thread. Native-dialog platform acceptance remains open.

Editor root-layer, edit-layer and flattened saves, and `authoring::save_stage_as`,
export to a temporary sibling with the destination's format extension. The file
is synced before atomic replacement; Unix saves also sync the parent directory.
Export or publication failures preserve the previous destination and clean up
the temporary file. A directory-sync error explicitly reports that publication
already occurred, but durability could not be confirmed.

Existing file permissions are retained. Final symlinks, directories and read-only
destinations are rejected. Replacement creates a new inode; ownership, ACLs and
hard-link identity are not preserved. Concurrent saves use last-writer-wins,
without conflict detection. Cross-directory ordinary root/edit saves anchor
external sublayers, references, payloads and typed asset values through the
stage's resolver without changing its source identifier or live layers. The
dependencies remain external; use USDZ for dependency bundling. Same-directory
saves retain authored path spelling, including directory aliases. Layers inside
a USDZ are re-anchored even when exported beside it, retaining the archive as an
external dependency. Asset expressions, tile/sequence patterns
and clip templates currently reject cross-directory relocation before replacing
the destination. Flattened export has different composition semantics; nested
asset metadata in anonymous flattened layers remains a separate limitation.

### Hierarchical point prototypes

Direct mesh prototypes bake their full invertible affine matrix into positions,
inverse-transpose normals and tangent frames, including shear. Material subsets
and repeated instances still share the baked assets. `assets/point_affine.usda`
and `assets/point_affine_reference.usda` provide matrix versus explicit-vertex
comparison scenes. The multi-node hierarchy path described below still rejects
per-node shear.

Ordinary prim transforms separately preserve affine residuals after Bevy's
transform propagation. USD reset stacks discard preceding operations and USD
ancestor transforms while retaining scene placement and up-axis conversion.
`assets/xform_affine.usda` and `assets/xform_reset.usda` have corresponding
`_reference.usda` fixtures for fixed-camera capture comparisons. Invalid
projective/non-finite transforms retain the previous pose and attach
`route::xform::UsdTransformError`; capture fails on that diagnostic.
`make run ARGS='assets/xform_animation.usda'` shows a Z-up cube transitioning
from identity through shear to translation at times 0, 5 and 10, beside a blue
reset-stack cube following its own translation without inheriting the shear.

For grouped, matrix-preserving editor undo, use
`EditorSession::edit(EditorEdit::TransformMatrix { prim, matrix, reset })`.
`matrix` is a column-major `[f64; 16]`; this command replaces the local stack
with a static matrix and explicit reset state, clearing local samples on its
matrix/order attributes. Other op attributes remain authored but inactive.
Undo restores the previous authored fields and samples rather than decomposing
the old transform into TRS. The lossy `live::TransformHistory` API has been
removed; use the same `EditorSession` for transform and other document edits.
For Bevy TRS input, convert `transform.to_matrix().to_cols_array().map(f64::from)`
to the command's matrix field and choose the reset state explicitly.

`EditorEdit::Batch(Vec<EditorEdit>)` applies a sequence, including nested batches,
as one undo step. A failed child edit rolls back preceding changes in the batch;
empty batches leave undo/redo intact. Selection follows namespace edits in order,
and the editor bridge remaps existing Bevy entities in reverse order during undo.
For example, group a `Define`, its `Attribute` edits and a `TransformMatrix` to
create a configured assembly with a single history entry. All children use the
session's current edit target. This groups history and rollback, not stage-sink
notifications: upstream still emits each constituent transaction.

Run the assembly walkthrough with:

```sh
make run RUN_WITH= APP_TARGET='--example editor_assembly' ARGS='target/editor_assembly.usda'
make run ARGS='target/editor_assembly.usda'
```

It builds a typed Sphere source in memory, composes two colored reference sites
through a reusable batch function, verifies one-step undo/redo and a live affine
edit, then saves and reopens a flattened scene. The optional output also supports
USDC and USDZ. Reference sites stay typeless so they inherit the model's schema.
This is an executable composition recipe, not a new typed scene DSL.

TRS-only reads (`read_transform` / `read_transform_at`) reject shear, projective
matrices and degenerate decompositions instead of returning misleading TRS.
Use `read_transform_stack_at` or `read_transform_stack_f64_at` for full matrices.
The renderer still preserves finite affine shear/singular transforms through its
residual path. `live::current_transform` returns `None` on a TRS read error;
`route::xform::transform_of` retains its documented identity-on-error fallback.

`make run RUN_WITH= APP_TARGET='--example normal_fixture' ARGS='target/normal_probe'`
creates a new directory containing normal-mapped quads, equivalent
explicit-normal references and opposite-Y controls. It refuses overwrites.
The 8-bit map uses raw color space and scale/bias (2,-1), following the
[UsdPreviewSurface normal specification](https://openusd.org/24.08/spec_usdpreviewsurface.html).
Bevy's tangent generation already compensates the relevant handedness change;
adding a second normal-map Y flip would break this fixture. This checks a static
constant map, not arbitrary texture transforms or deformed tangents.
The `scaled_*` variants exercise noncanonical Preview Surface `UsdUVTexture`
RGB scale/bias. Signed normal results are encoded for Bevy as `(value + 1) / 2`
in a linear float16 texture; canonical scale/bias (2,-1) reuses the original.
Sampled scale/bias interfaces are read at the requested clock. Other surface
dialects and `ND_normalmap` wrappers retain their previous handling.
`interface_animated.usda` animates the normal gain through a Material input;
`animated_reference.usda` independently animates the geometry normal. The fixture
test checks directions at times 0/5/10 and backward to 0.
The scaled mapped/reference capture differs at six pixels by at most RGB 1;
the canonical case retains its two-pixel RGB-1 difference. Neither is pixel-exact
reference parity, and general MaterialX normal processing remains unsupported.

Authored constant Preview Surface normals also use the linear float16 path, with
a shared 1x1 image and no external texture dependency. `(0,0,1)` needs no image.
Nonfinite, zero, out-of-range and float16-zero vectors produce material warnings
and retain the geometric-normal fallback. This requires the mesh's tangent frame;
missing UV/tangent data and deformed-tangent fidelity remain separate limitations.
The normal fixture includes `constant.usda` and `constant_animated.usda`; the latter
animates the signed normal directly rather than through a texture scale input.
`constant_no_uv.usda` omits UVs and exercises the missing-tangent diagnostic.
Normal-mapped ordinary meshes, material subsets and point-instancer prototypes
carry `UsdMaterialWarning` when the projected mesh lacks tangents. Bevy then uses
geometric shading normals; the diagnostic does not invent a UV/tangent basis or
validate the numerical quality of existing tangents.
Material warnings on the selected prim's mapped entity appear in the Properties
pane's render issues and clear when the warning or selection disappears. This
does not aggregate warnings from generated subset/prototype descendants.

`UsdTransform2d` scale/rotation/translation is converted through the mesh V-flip
before becoming Bevy's UV transform. Generate the animated comparison fixture:
`make run RUN_WITH= APP_TARGET='--example uv_transform_fixture' ARGS='target/uv_transform_probe'`.
Its mapped and explicit-UV scenes are equivalent at times 0 and 10; intermediate
times are not reference baselines because interpolating coordinates differs from
interpolating rotation. Different per-texture coordinate transforms remain unsupported.
`sampled_reference.usda` separately bakes analytic coordinates at times 0, 2.5,
5, 7.5 and 10 using scalar sine/cosine math. Only those keys are reference
samples; interpolation between its baked keys is not the original rotation curve.
The live-clock suite tests 5/10 reversed to 10/5 against this reference.
`interface_mapped.usda` drives the same transform chain through sampled Material
inputs. Scale, rotation and translation resolve canonical value producers at each
instance's clock; malformed connections, wrong value types and nonfinite results
are errors rather than silently ignored interface values.
The same generator writes `shear_mapped.usda`, `shear_reference.usda` and
`shear_lossy.usda`: a nonuniform-scale/37-degree-rotation chain, independently
baked vertex UVs, and a deliberately lossy SRT control. Capture at time 0 with
eye `(0,1,5)`, focus `(0,1,0)` and `USD_CAPTURE_SHADOWS=off`. On the tested Vulkan
renderer, mapped versus baked differs in 7 of 921,600 pixels (maximum RGB error
2); the lossy control differs in 120,136 pixels (maximum error 91). These are
recorded strict-zero comparison results, not a claim of bit-identical rendering.
The generator also writes `file_samples.usda` (sample-only red/blue texture
files) and `file_reference.usda` (fixed quadrant texture with explicit UV samples).
`color_space_samples.usda` uses one gray image with raw/sRGB token samples.
Capture both with `USD_CAPTURE_INSTANCE_TIMES=0,10`, shadows off, eye `(0,1,8)`
and focus `(0,1,0)`. The tested Vulkan captures match all 921,600 pixels at strict
zero RGB tolerance. This checks simultaneous texture selection, not a live reload.
UV transforms are read from connected texture-coordinate paths, including nodes
outside the material's immediate children; disconnected nodes have no effect.
Explicit constant float2/double2 coordinates at a texture-coordinate terminal
are represented by a zero-scale affine transform. Sampled Material/NodeGraph
interface values and constant `UsdTransform2d.inputs:in` values follow the same
path and compose with downstream transforms. The fixture's
`constant_coordinates.usda` deliberately has varying vertex UVs while its
animated Material coordinates select one texture location across the mesh.
Unauthored coordinate terminals retain the existing implicit mesh-UV behavior;
named/per-texture UV-set selection remains unsupported.
Different transforms across a material's texture channels, non-finite transforms
and over-budget coordinate graphs produce explicit
errors instead of silently choosing the first material child.
Connected `UsdTransform2d` chains compose in order without decomposing the result,
including nonorthogonal affine axes from nonuniform scales and rotations.
`ReadPreviewMaterial::uv_transform` now contains `bevy::math::Affine2` in USD UV
coordinates; the old `UvTransform` struct was removed. Construct single-node
values with `Affine2::from_scale_angle_translation` (angle in radians).

The inspector exposes `matrix4d` attributes in a Matrix attributes section with
four USD rows (translation in row 4), preserving f64 input precision. Apply a
default or a time sample using the ordinary attribute controls; these edits keep
the existing transform-op order. Sample-only attributes start with an explicitly
labelled identity draft, not the evaluated timeline pose. “Load sampled matrix”
copies the matrix at the labelled scene time into the draft and sets the sample
time field; loading alone does not author USD. Use the separate
`TransformMatrix` API only when replacing the complete stack is intended.

`scripts/replays/matrix_sample_{load,edit,undo}.replay` exercise the actual
matrix widgets through `USD_UI_REPLAY` in the fixed 1600x1000 private capture
layout. The edit/undo sequences reload the time sample after applying or undoing
it, checking the authored result rather than only the text draft. See
`BEVY_WORK.md` for capture settings and inspected evidence.

`assets/xform_half.usda` exercises half-precision translation, rotation and scale
against `assets/xform_half_reference.usda` using double-precision attributes.
Unsupported op kinds or value types now produce transform diagnostics instead
of disappearing into identity. Half-quaternion orientation is covered by
`assets/xform_quath.usda` and its native-USD-derived matrix reference. Quaternion
conversion preserves the real-component angle and normalizes the imaginary axis;
it does not normalize the entire quaternion. Scalar-axis translation/scale ops
are exercised by `assets/xform_scalar_axes.usda`. Adjacent op/inverse pairs
cancel before evaluation. For compatibility with native OpenUSD 25.05.01,
an unpaired inverse scalar-axis scale negates its value; vector-scale inverses
instead use reciprocals. These two forms are not interchangeable.

Local op stacks compose in double precision before final conversion for Bevy.
`read::xform::read_transform_stack_f64_at` exposes that result directly;
`assets/xform_precision.usda` demonstrates large offsets cancelling to a one-unit
translation. This does not provide double-precision world/hierarchy transforms.

`assets/point_hierarchy.usda` instances a two-mesh assembly. Its cyan child moves
between times 0 and 10 while the red child stays fixed. Mesh/material/subset
handles are shared across copies. Nested transforms remain on generated entities
rather than being decomposed from an accumulated matrix. Generated nodes carry
`route::instancer::UsdPrototypePart` with the source prototype path; hierarchical
`UsdInstance` entities are transform roots, not necessarily renderable meshes.

Xform, Scope, SkelRoot, untyped grouping nodes, Mesh, Cube, Sphere, Cylinder,
Capsule, Cone and Plane nodes are supported;
material/subset/skeleton definitions are consumed by their mesh adapters.
Hierarchy preparation rejects reset stacks, per-prim shear, perspective/singular
transforms, unsupported node types, depth >=256 and more than 4096 projected
nodes. Failure suppresses generated geometry while retaining existing node
identities and runtime children for recovery. Full hierarchy/deformation fidelity
and performance remain unverified; this is not native scenegraph instancing.

`assets/point_shapes.usda` instances two copies of a six-shape assembly. All
dimensions double between time codes 0 and 10. Shapes use the same tessellation
and preview-material conversion as ordinary prims, with shared handles. Constant
display colors/opacities (including sampled indices) and single unindexed values
are replicated across generated vertices; nonconstant multi-value shape colors
and opacities are not mapped by this adapter. Shape
face subsets are not supported. Shape prims can also be direct prototype targets.
Negative, nonfinite or f32-overflowing dimensions suppress shape geometry and
publish `route::shapes::UsdShapeError`; nonfinite generated positions/normals are
also rejected. The selected prim's inspector shows the error, and the standalone
capture exits unsuccessfully instead of publishing a partial scene. Valid edits
restore geometry without replacing the prim entity or its runtime children.
Shape regeneration invalidates cached bounds before Bevy recomputes them.

`assets/display_opacity.usda` compares an ordinary mesh, cube and point-instanced
cube against a blue backdrop. Their display opacity animates from 0.2 to 1 at
time codes 0 and 10. Unbound fallback materials switch between blend and opaque
mode while opacity remains in vertex alpha, without multiplying it a second time
into material alpha. This does not implement shader primvar-reader networks or
order-independent transparency for intersecting transparent surfaces.

`assets/inherited_display.usda` authors animated display color and opacity on
the parent Xform instead. Meshes, primitives and their point prototypes resolve
the nearest authored, nonblocked constant ancestor value; local values take
precedence. Values and indices are sampled from that same owner. Parent edits
reconcile consumers, and independent instance clocks sample inherited inputs.
This follows [OpenUSD primvar inheritance](https://openusd.org/dev/api/class_usd_geom_primvars_a_p_i.html).
Mesh UVs also inherit constant `primvars:st` (or fallback `primvars:st0`), including
sampled owner-local indices and parent-edit invalidation. The canonical `st`
lookup precedes `st0`, including inherited values. Both flat and indexed mesh
layouts resolve constant UV indices before applying the USD-to-Bevy V flip.
General shader primvar-reader networks and arbitrary per-texture UV sets remain
separate work; constant UVs do not define a usable normal-map tangent basis.

```bash
make run APP_TARGET='--example viewer_capture' ARGS='assets/point_shapes.usda target/point-shapes.png 10 6 4 9 2.5 0.75 0'
```

### Native save interoperability gate

`make test-native` requires Pixar OpenUSD's `usdcat` on PATH, or an executable
path in `USD_CAT`. It exercises the editor's root-layer, edit-layer and flattened
save modes in USDA, USDC, USD and USDZ, then checks hierarchy, attribute values,
selected variant composition and API metadata through native flattening. Missing
tools are errors, not passes.
The test is ignored in the ordinary suite because native OpenUSD is optional.

The fixes in `patches/` are integrated through the root Cargo patch table and
`vendor/openusd`, so this gate runs against the actual viewer dependency without
temporary overrides. Ordinary Rust self-reopen tests are not a replacement.
The fixture covers selected scene data, not arbitrary lossless interchange.
See `vendor/openusd/VENDORED.md` for provenance and `OPENUSD_UPGRADE.md` for the
remaining compatibility findings.

A second native fixture checks sublayers and external references in all three
save modes and four formats. These checks keep exports beside their dependencies;
they do not alone certify cross-directory Save As or self-contained USDZ packaging.
A separate native fixture now checks cross-directory root and nested edit-layer
exports in USDA, USDC and USD, retaining referenced values and external textures.

The native gate also removes the source directory before checking exported
packages. All three save modes now pass that regression, including an asset
payload. USDZ persistence uses the stage-aware dependency packager described in
`PACKAGING.md`. Nested package inputs and package-relative assets can be
re-exported into unique entries while retaining layer composition. Expressions
and tile/sequence patterns still produce explicit errors. This does not certify
arbitrary interchange.
