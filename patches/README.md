# Local upstream fixes

These review patches are applied to `vendor/openusd`. The root Cargo patch table
uses that repository-local copy for all three OpenUSD packages. The Git revision
declarations record its upstream baseline; Cargo.lock records path packages.
See `vendor/openusd/VENDORED.md` for provenance and removal instructions.

## Nested package export

`openusd-nested-package-export.patch` applies after the nested-reading patch and
the earlier packaging changes. It caches nested source containers through the
stage resolver, accounts their bytes within the existing aggregate input budget,
and identifies inner default layers and terminal entry extensions correctly.
Dependencies become ordinary unique output entries while preserving layer
composition, rather than flattening the scene. Original nested archive layout
is not reproduced. Traversal is limited to 16 bracket levels; existing input,
output and entry-count budgets remain. Bevy's nested fixture tests cover portable
snapshot-only export with live edits and destination preservation on failure.

## Nested package reading

`openusd-nested-package-reading.patch` applies after the earlier local patches.
It shares bounded in-memory ZIP entry traversal between the default resolver
and Bevy's snapshot resolver, and anchors a referenced inner USDZ to its default
layer using nested bracket syntax. Each entry traversal is limited to 16 package
levels and 256 MiB of cumulative decompressed entry bytes. Root archive storage,
central-directory metadata and total stage work are not covered by this limit.
Raw Archive::read still decodes native USD entries, not packages; nested package
layers load through the resolver-backed file format. Packaging/export limitations
are unchanged. The patch includes depth, byte-budget, missing-entry and corrupt
inner-package tests; the Bevy project tests snapshot/filesystem composition and
relative asset bytes.

## Clip activation samples

`openusd-clip-activation-samples.patch` fixes interpolation across active-clip
boundaries in the same upstream baseline. Native OpenUSD 25.05.01 returns 3.5
at stage time 5 for clips with samples 0:1/20:3 and 0:5/20:7 switching at 10;
the unpatched Rust resolver returns 1.5. Interpolation now uses composed
stage-time sample boundaries for non-asset values, retaining value blocks rather
than blending them as values. Asset resolution remains on its existing path.
The upstream regression accompanies the patch; the Bevy regression and GPU
fixtures live in the containing project. This does not implement clip baking.

## Singleton metadata list operations

`openusd-singleton-listops.patch` applies to mxpv/openusd revision
`b7df5add628cbb791103a7da842dbd82810da5d0`. It fixes the USDA writer at its
list-operation formatting boundary: metadata arrays retain brackets even when
they contain one item. Regression coverage checks explicit, prepend, append,
delete, add and reorder operations and preservation through Rust re-parsing.

Apply only to a separate checkout of that revision, not a modified sibling or
Cargo's cached source:

```sh
git -C "$UPSTREAM" apply --check "$PWD/patches/openusd-singleton-listops.patch"
git -C "$UPSTREAM" apply "$PWD/patches/openusd-singleton-listops.patch"
CARGO_WORKSPACE_DIR="$UPSTREAM/" make test \
  APP_TARGET="--manifest-path $UPSTREAM/Cargo.toml -p openusd --lib"
make run RUN_WITH= \
  APP_TARGET="--manifest-path $UPSTREAM/Cargo.toml -p openusd --example convert" \
  ARGS='assets/single_api_schema.usda target/single-api-patched.usda'
usdcat target/single-api-patched.usda
```

Use a writable Cargo cache and temporary directory if the environment requires
them. The upstream tests require the checkout's vendor fixtures.

Validation in an isolated copy: all 1,582 core library tests passed. Native
OpenUSD 25.05.01 accepted the patched minimal fixture and the Spot USDC-to-USDA
conversion. Native Embree rendered the converted Spot visibly, with faceting.
It warns that Material prims and GPU-disabled color correction are unsupported;
this capture is a geometry diagnostic, not material or pixel-parity acceptance.

The original Spot binary still produces only root metadata in native usdcat.
Converting through the Rust reader does not establish that reader's binary
interpretation is correct. That discrepancy remains open.

## Binary structural metadata

`openusd-token-vector-fields.patch` applies to the same revision and is independent
of the text patch. It encodes the eight native token-vector fields (child lists
and ordering) as non-array TokenVector values, while ordinary token arrays keep
their array representation. Sublayer paths use non-array StringVector values,
while ordinary string arrays retain their array representation. It also emits variantSetNames token list operations
as native StringListOp values with string-table indices. These types follow the
[native schema definitions](https://github.com/PixarAnimationStudios/OpenUSD/blob/v25.05/pxr/usd/sdf/schema.cpp).

Wire-format regressions cover empty, singleton and multi-item token/string
vectors, ordinary arrays, and all six variant-set list operations. Both patches
together pass 1,585 upstream core tests and 56 binary fixture roundtrips.

With both patches temporarily supplied to the actual Bevy workspace, the native
editor gate passes all 12 save-mode/format combinations, including selected
variant composition from `assets/native_save.usda`. The text patch fixes both
root/edit and flattened singleton API metadata; those native errors did not
require separate text fixes. These results do not establish arbitrary layer,
reference, payload, animation or material interchange.

After integration, the native gate also covers sublayers and external references
in the same directory across all 12 save-mode/format combinations. Combined with
the variant fixture, this is 24 native save checks. It does not prove that USDZ
contains its external dependencies or remains portable when moved away from them.

The normal `make test-native` now uses both fixes without command-line overrides.
To review them independently, apply both to a separate checkout of the recorded
revision. Never apply them again to the already-patched repository-local copy.

## Stage-aware USDZ packaging

`openusd-stage-packaging.patch` adds `Stage::write_usdz_package` and a bounded
registry asset-read operation. The Bevy persistence integration calls this API
for USDZ saves; ArchiveWriter itself remains a byte-oriented archive sink.
The patch is applied in `vendor/openusd` alongside the two writer fixes.
See `PACKAGING.md` for resolver/snapshot behavior, bounds, unsupported inputs
and the full acceptance requirements that remain open.
The packaging patch also handles single-level input archives, bounded cached
entry extraction and bare-package references; nested packages remain unsupported.

## Missing external reference target diagnostics

`openusd-reference-diagnostics.patch` adds UnresolvedPrimPath diagnostics for
external references to absent root prims and for absent external sub-root targets.
The baseline only diagnosed missing root targets for payloads and missing sub-root
targets associated with cycles. Native usdcat reports missing reference targets
at both depths while still producing a partial stage
(`/tmp/native-missing-reference.log`, `/tmp/native-missing-subroot.log`). Core
coverage checks reference and payload arcs at both depths. Sub-root diagnostics
are deferred until composition tasks finish, preserving variant-supplied targets.
Node culling and reference composition are unchanged; broader ancestral and
relocation combinations still need acceptance coverage.
