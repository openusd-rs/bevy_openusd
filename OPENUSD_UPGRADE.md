# OpenUSD upgrade and capability reassessment

## Editor clip baking

The Bevy editor now bakes clip-resolved attributes during flattened saves before
rebuilding shared instance subtrees. This is downstream export logic, not a
change to upstream Stage::flatten. It anchors asset samples, preserves blocks
and rejects clip-sourced timecode values, unresolved assets and excessive sample
counts. Root/edit-layer saves remain composition-preserving. See README.md and
BEVY_WORK.md for native export coverage and remaining scope.

## Clip activation interpolation

`patches/openusd-clip-activation-samples.patch` corrects non-asset interpolation
across active-clip boundaries in the vendored baseline. Native OpenUSD probes
and the containing Bevy regression distinguish interpolation toward the next
activation value from interpolation solely inside the active clip. Value-block
boundaries retain the preceding value until the boundary; asset resolution is
unchanged. This is a prerequisite for faithful clip baking, not baking support.

## Integrated writer fixes

The fifth local patch, `patches/openusd-layer-reanchoring.patch`, adds
Stage::anchored_layer for resolver-aware, non-mutating layer copies. Bevy uses
it for cross-directory ordinary root/edit saves; native regression coverage
checks external sublayers, references and textures in USDA, USDC and USD.
Dependencies remain external. Asset expressions, tile/sequence patterns and
clip templates reject relocation; anonymous flattened nested asset metadata
remains outside this fix.

Bevy source publication now checks discovered composition diagnostics rather
than treating a successfully parsed but incomplete stage as Ready. A fourth
local patch adds missing external root/sub-root target diagnostics. Sub-root
checks run after composition tasks finish so variant-supplied targets remain
valid. Native USD reports missing reference targets at both depths.
See `patches/openusd-reference-diagnostics.patch` for the bounded coverage.

Packaging now accepts single-level source USDZ archives and package-relative
layers/assets. Native tests cover snapshot-only inputs, a second re-export after
deleting the first file, and a wrapper referring to one bare package twice.
Containers and extracted bytes share the input budget; entry lengths are checked
before reading. Nested packages remain unsupported. Logs:
`/tmp/repackage-case-native.log`, `/tmp/upstream-repackage-final-tests.log`.

Stage-aware packaging is now integrated as a third local upstream patch.
Root/edit USDZ exports preserve authored composition while bundling reachable
ordinary layers and assets through the same resolver; flattened saves bundle
their remaining asset dependencies too. The moved-package native regression
passes for all modes, including archived asset bytes. This closes the reproduced
dependency-loss case, not arbitrary packaging support. `PACKAGING.md` records
explicitly unsupported inputs, bounds and remaining acceptance work.

The added moved-package test demonstrates a separate remaining gap: root/edit
USDZ writes do not bundle dependencies. Once original files are deleted, native
composition loses sublayer and referenced values. `make test-native` currently
failed that portability regression before the stage-aware packager was added.
See `PACKAGING.md` for the implementation and remaining limitations.

Native layered-scene checks exposed and fixed an additional binary mismatch:
subLayers must use a non-array StringVector, not a string array. Root/edit USDC
and USD exports previously lost the weak layer under native composition. The
updated binary patch includes this correction with 0/1/3-element wire regressions
and checks that ordinary string arrays remain arrays. Native tests now pass
24 save checks: 12 for hierarchy/variants/API metadata and 12 for sublayers and
external references, all saved beside their dependencies. USDZ portable bundling
and cross-directory asset-path preservation remain separate acceptance work.
Logs: `/tmp/native-layered-gate.log` (before),
`/tmp/native-layered-packages.log` (after), `/tmp/upstream-sublayer-tests.log`.

The root Cargo patch table now uses the additive `vendor/openusd` snapshot for
all three OpenUSD packages. Its upstream baseline remains b7df5ad, rechecked as
upstream HEAD on 2026-09-10. Local source differences are recorded in the two
writer patches and the stage-packaging patch. Existing xtra directories and sibling checkouts are
untouched. `vendor/openusd/VENDORED.md` records included files, licenses and the
switch-back procedure. No temporary source path or build-time patching is needed.

## Native interoperability audit — history and remaining scope

Before integration, both patches passed the editor's 12-combination native gate
when supplied as temporary Cargo overrides. The gate additionally
uses native flattening to verify the selected variant's authored value. Fixes:
USDA singleton list brackets; non-array TokenVector encoding for the eight
structural/order fields; StringListOp encoding for variantSetNames. The last
case is required by native schema.cpp and is distinct from the token-vector
array bit. Upstream's parser currently stores variantSetNames as TokenListOp,
so the binary writer translates it at the format boundary. Both patches pass
1,584 core tests and 56 binary fixture roundtrips. Logs:
`/tmp/upstream-vector-tests.log`, `/tmp/upstream-vector-roundtrip-final.log`,
`/tmp/bevy-native-patched-variants.log`. These are temporary-override results,
not dependency integration or comprehensive interchange acceptance.

`make test-native` now checks actual editor saves through native `usdcat` in all
three save modes and four supported extensions. It fails in all 12 combinations
at the unpatched upstream pin: root/edit USDA reject singleton list-op syntax, flattened
USDA rejects a non-shaped value with an array type, and all USDC/USD/USDZ cases
decode without the expected parent/child, attributes or API metadata. The
original USDA control parses successfully. Log: `/tmp/bevy-native-export-tests.log`.
The test is explicitly ignored in the ordinary suite because it requires native
OpenUSD, not because self-reopen constitutes equivalent coverage.

An isolated binary-writer experiment encodes `primChildren` and `properties` as
non-array `TokenVector` representations instead of token arrays. This makes a
minimal Rust-written USDC prim visible to native usdcat. The Rust reader merges
both wire types into the same TokenVec value, masking this distinction in
self-reopen tests. Native [crate data loading](https://github.com/PixarAnimationStudios/OpenUSD/blob/v25.05/pxr/usd/usd/crateData.cpp)
also treats TokenVector specially. This initial experiment was subsequently
expanded to all eight token-vector fields and integrated as recorded above.

On 2026-09-10, the installed native OpenUSD 25.05.01 tools exposed compatibility
gaps not covered by the Rust reader/writer self-reopen tests:

- Native `usdcat` reads only layer metadata from the collection's original
  `spot_base_urdf/spot.usdc`, while the pinned Rust reader traverses 26 meshes.
  CPU `usdrecord`/Embree produces a black Spot image, including with all purposes
  enabled. The same native renderer produces visible geometry from our
  `assets/subdivision_cube.usda`. The Spot image is not a valid visual reference.
- Exporting the Spot root layer through the current Rust writer produces
  `prepend apiSchemas = "MaterialBindingAPI"`; native `usdcat` rejects that
  singleton list-op representation. Minimal reproduction:
  `assets/single_api_schema.usda` is accepted natively, but its Rust root-layer
  text export is rejected with `Expected None or [` and status 1.
  Logs: `/tmp/single-api-export.log`, `/tmp/single-api-native-error.log`.
- A native Storm offscreen attempt segfaulted in this environment; it provides
  no acceptance evidence. CPU Embree emits a color-correction limitation warning.

The original Spot binary compatibility issue is not yet attributed to a writer
version or specific binary field. The minimal text-export failure is reproduced
against the unpatched upstream writer and is now fixed locally. Broad native
interchange and reference-render acceptance remain open; the selected native
fixture and self-reopen tests do not prove arbitrary interchange.
The diagnostic `scene_report` export writes a new file only and does not modify
the original asset or rebase its relative references.

Reviewed on 2026-09-09 against upstream
[`b7df5add628cbb791103a7da842dbd82810da5d0`](https://github.com/mxpv/openusd/commit/b7df5add628cbb791103a7da842dbd82810da5d0).
Rechecked with `git ls-remote` on 2026-09-09: this is upstream HEAD,
not a floating dependency.

## Dependency and integration

- Core `openusd`, `openusd-schemas`, and its transitive generator
  `openusd-build` are version 0.7.0 from the same pinned Git revision.
- Minimum Rust version is 1.96. Bevy remains 0.19.1.
- Removed the local Cargo patch; `../openusd` and its uncommitted skinning
  changes remain untouched. The remote `bresilla/openusd` HEAD was behind
  the existing local checkout, so it was not an upgrade target.
- Migrated generated schema names, accessor traits, applied-API constructors,
  fallible path access, and typed errors.
- All viewer/asset/snippet stage constructors now install the generated schema
  registry. Callers constructing their own stage for `LiveStage` must also use
  `Stage::builder().schema_registry(openusd_schemas::schema_registry())`.
  Bare `Stage::open` knows only the upstream core USD schema family.

## Revised roadmap assessment

These are separate questions: what upstream can represent/resolve, and what
our Bevy runtime and editor expose. Upgrading the former does not finish the latter.

| Earlier recommendation or blocker | Latest upstream evidence | Revised conclusion |
|---|---|---|
| In-memory text-to-layer opening is blocked | `crates/openusd/src/sdf/layer.rs`: `Layer::from_bytes`; `usd/stage.rs`: `Stage::insert_layer` | **Blocker removed.** A local integration test composes byte-backed USDA and flattens it with no files. The current asset and snippet entry points now use `UsdSource` snapshots without temporary files. |
| Layer-relative asset resolution is missing | `ar.rs`: resolver and package-path APIs; `usd/attribute.rs`: asset-value resolution through the winning composition site | **Earlier blanket claim withdrawn.** Core resolution exists. Our snapshot loader now preserves source identity, tracks external layer/texture bytes through Bevy and tests automatic dependency reloads. Anonymous `Layer::from_bytes` alone still provides no relative-path anchor. Nested packages remain unsupported. |
| Reference authoring needs upstream work | `usd/prim.rs`: metadata authoring; upstream stage/editor tests author `ReferenceListOp` | References can already be authored through the core metadata APIs. A typed reusable USD scene-component API remains our work; no claim that upstream ships a BSN-style helper. |
| Native instances/prototypes need support | `usd/prim.rs`: `is_instance`, `prototype`, `instances`, instance-proxy queries; `usd/stage.rs`: `prototypes` | Core capability exists. Sharing Bevy assets and managing projected prototype/instance entities still need an explicit runtime design. PointInstancer support is not the same as native instanceable prims. |
| Multiple independent live instances | Core stage handles remain `Rc<StageInner>`; stages, variants, time queries and edit targets exist | Now integrated through the non-send `UsdInstances` registry, per-root sampling and reload-persistent variant/attribute overrides. Editor integration and performance validation remain. |
| Composition-aware inspector | `usd/attribute.rs`: `resolve_info`; `usd/stage.rs`: layer stack, layer identifiers, node-layer stacks, edit targets | More is available than the old roadmap implied. Build the UI and authoring contracts on these APIs instead of implementing another composition engine. |
| Prim asset metadata was unavailable | `usd/prim.rs`: `Prim::get_metadata` returns composed dictionary fields | Removed the obsolete `read_asset_info` stub. The reader and inspector now expose `assetInfo`, with nested layered-dictionary composition verified. |
| Save/reopen and export | `usd/flatten.rs`: `Stage::flatten` | Composed flattening is now available and tested locally. Existing save helpers still export the root layer, which is different from flattening or saving a selected override layer. Keep those operations explicit. |
| Better developer authoring API | Generated schema views provide read and create accessors, registered defaults and inheritance | Upstream provides much of the typed USD foundation. Our `usd!` still produces interpolated text, not a typed Bevy scene; runtime escaping, component diagnostics and optional BSN integration remain separate. |
| Rendering and performance | Upstream schemas and CPU deformation helpers describe/evaluate content, not Bevy GPU resources | IBL, GPU skinning/morphs, measured native-instance projection and render fidelity remain renderer work. No evidence here that the dependency upgrade implements them. |
| Regression corpus and releases | Our tests and build remain owned here | Real AssetServer dependency/reload, package and material-assignment tests now exist. Multi-instance lifecycle tests, rendered image baselines, benchmarks and a support matrix remain. This upgrade is not a production-readiness certification. |

## Regressions caught during migration

1. Schema fallback `purpose = default` masked an authored ancestor purpose.
   Inheritance now checks whether the value was authored before accepting it.
2. Property creation can produce resync notices. Echo suppression now accounts
   for self-authored property resyncs, without suppressing structural prim resyncs.
3. Asset-typed fields reject string values. The asset-handle test now authors an
   actual `Value::AssetPath`.
4. Generated light schemas expose common inputs through `LightAPI`.
5. The parser rejects numeric prim identifiers. Macro validation now uses an
   identifier-safe placeholder in quoted interpolation sites.
6. Upstream binding discovery walks to the pseudo-root and attempts to append
   `skel:animationSource`, which now fails path validation. Our reader resolves
   the selected mesh's binding directly and stops inherited-target walks before
   the pseudo-root. Existing animated-skeleton and blend-shape tests cover it.
7. The sibling's uncommitted fallible skinning helpers are not in upstream.
   Our reader validates influence lengths, stride and joint bounds before calling
   upstream LBS. Added negative-index and mismatched-stride regression coverage.

## Verification

- `make check-all CARGO='cargo --offline'`
- `make test-all CARGO='cargo --offline'`: 116 passed, zero failures on recheck,
  including the subsequent snapshot and dependency-reload integration tests
  (106 passed at the original migration checkpoint).
- `make build CARGO='cargo --offline'`
- `git diff --check`
- Timed `make run` desktop smoke tests: Spot projected 96 prims; the skinning
  fixture projected four prims, two animated. Both runs were intentionally
  terminated after 25 seconds. No screenshot/fidelity comparison was performed.

The capability table combines source inspection with the explicit byte-backed
composition/flattening test. It does not claim every upstream resolver, authoring
or instancing API was exhaustively tested.

## Integration progress after migration

Further inspection found upstream `usd::UndoStage`, which captures layer-level
transaction inverses. The new composition editor model uses it to restore the
absence of authored opinions correctly; the older composed-value inverse model
is not the editor's foundation. Bevy command grouping and redo remain local.

The current checkout adds source-preserving snapshots, tracked layer and texture
dependencies, loading/failure states, static subtree reloads and byte-backed
snippets. PNG/JPEG snapshot images are labeled Bevy assets with separate color
and linear handles. See `BEVY_WORK.md` for the acceptance checklist and remaining
material fidelity limitations.

Per-instance live state now reconciles reloads without replacing matching
entities and supports independent sampling and persistent overrides. Next:
complete playback/lifecycle acceptance, then build the composition inspector and
typed developer API on the now-available upstream primitives.
