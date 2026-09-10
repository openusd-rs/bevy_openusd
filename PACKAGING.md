# Portable USDZ export

## Acceptance gap

`persistence::tests::native_export_package_survives_removal_of_layer_dependencies`
saves into a separate directory, deletes the owned source directory, then opens
each archive with a fresh native usdcat process. RootLayer and EditLayer lose
both the sublayer and referenced prim. Flattened preserves these two values;
this fixture has no textures and does not certify flattened-package portability.

The existing 24 native checks still pass because their exports remain beside
their dependencies. `make test-native` now includes the portability regression
and fails until dependency packaging is implemented.

## Implementation boundary

Keep `usdz::ArchiveWriter` as the aligned, stored ZIP sink. Its current API is
explicitly bytes-in/bytes-out, and the file-format writer serializes only one
layer. Packaging needs a separate dependency traversal above that layer writer,
not post-processing of USDA strings or implicit flattening of root/edit saves.

Resolve through the stage's existing LayerRegistry/resolver context. A new
DefaultResolver in the viewer would lose snapshot-only assets and custom
resolver identities. Prefer live layer data already in the stage graph so edits
are exported rather than re-reading stale files. Unvisited external layers must
be loaded through that same registry, including inactive variants and unloaded
payload targets needed to preserve authored composition.

## Required packaging work

1. Build an isolated dependency graph before publishing anything. Give each
   canonical asset identity a deterministic package-relative name. Reserve names
   before traversal so cycles terminate; bound entry count and total bytes.
2. Clone authored layer data, preserving root/edit-layer semantics and all list
   operation buckets. Rewrite only asset-bearing locations: subLayers,
   references, payloads, assets in attributes/dictionaries/time samples, and
   schema-defined asset inputs such as clips. Keep USD prim paths unchanged.
3. Resolve every path relative to its owning layer's real location. Handle
   anonymous/snapshot layers, package-relative assets, resolver expressions and
   tile/sequence patterns explicitly; unsupported inputs must report errors,
   not produce a successful but incomplete package.
4. Include reachable layers and non-layer assets. Preserve selected output root
   as the first archive entry and use ArchiveWriter's path/alignment checks.
   Identical basenames from different directories must not collide.
5. Publish through the existing same-directory atomic persistence path only
   after graph resolution and serialization succeed. Missing dependencies,
   resolution failures and budget failures must retain the old destination.

## Verification

- Native portability regression above passes for every save mode.
- Move packages away from sources and delete sources before checking assets;
  a successful native parse without the expected prims/values is a failure.
- Cover texture bytes, nested dependencies, repeated paths, cycles, basename
  collisions, unsaved layer edits and snapshot-only dependencies.
- Verify source exports, undo state and prim identity are unchanged by packaging.
- Inspect archive entries for stored compression, 64-byte alignment, safe names
  and the correct first layer; exercise failure cleanup and destination retention.
- Retain all existing native and ordinary checks. This work does not by itself
  fix relative references in non-package cross-directory Save As.
