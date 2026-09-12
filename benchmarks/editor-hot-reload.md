# Editor hot reload

The core `EditorPlugin` now polls loaded layer files on native targets by default.
It checks metadata every 250 ms and waits for a stable observation before reading
changed files. No `file_watcher` Cargo feature or viewer-only plugin is required.

`reload::LayerReload` parses replacement layers, validates them in an isolated
composition, and publishes field-level differences through `Stage::batch_edit`.
The existing stage and Bevy entities remain in place. USD change notices drive
projection; this is not `EditorCommand::Open` or a root-wide forced resync.

Initial regression evidence (`/tmp/core-reload-test.log`): eight tests pass,
including a real filesystem save affecting two references, unchanged unrelated
entity/mesh/runtime-component identity, malformed-save retention, atomic-rename
recovery, dirty-layer conflict rejection, repeated package updates, stale-plan
rejection, selective texture refresh, and new-reference repair without rewriting
the referring file. The library gate passes 523 tests with 19 ignored;
workspace check passes (`/tmp/reload-library.log`, `/tmp/reload-check.log`).

The viewer enables this native core service by default. `USD_HOT_RELOAD=0`
disables it; it replaces the viewer's optional `USD_WATCH_TEXTURES` setup.
Library hosts can configure `EditorReloadSettings`. Packages reload their
existing layer entries and supply updated bytes to the live resolver. Texture
updates decode only changed/new requests, retain other image handles, and
enqueue projection for their current material/dome consumers.

Live viewer evidence: `target/hot-reload-live.png` and
`target/hot-reload-live.second.png` were captured from the same running viewer.
Editing only `target/hot-reload-demo/model.usda` changed the referenced blue cube
into a larger orange cube without reopening the root document or moving the
camera. Both screenshots were inspected. The independent green cube's
160x190 ROI at (1000,480) is pixel-exact (mean/RMS/max error zero;
`/tmp/reload-unchanged-roi.log`). Capture and renderer-log checks passed in
`/tmp/reload-ui.log`. This is a controlled fixture, not machinery-scale evidence.

Current limitations, still being implemented:

- Renaming a package's default layer entry is not qualified; existing entry
  identity is required. Nested-package and large machinery refresh timing still
  need dedicated coverage.
- External layer updates retain unrelated undo/redo commands. Commands touching
  a reloaded layer are discarded, including redo commands whose replay would
  overwrite the new source. A changed dirty layer still blocks publication.
- Candidate validation traverses a document copy even though publication applies
  only changed layer fields. Large-document timing is not qualified.
- Structural projection is scoped, but shared-schema removals and some material/skeleton/prototype
  changes still use conservative full reconciliation. Entity identity is retained;
  strict minimum-consumer processing is not yet universal.
- Metadata polling cannot detect a writer that preserves both mtime and size.
- A Blender-driven export itself has not been tested; tests write the same disk
  files directly, including atomic rename saves.

## History isolation

The follow-up history gate passes 527 library tests (19 ignored) and workspace
check. Nine focused editor reload tests pass, including unrelated undo/redo
survival, selective removal of conflicting commands from mixed-layer history,
rejection of redo that would overwrite a reloaded source, nested/panic-safe
capture suspension, and retention-mask length validation. Evidence:
`/tmp/reload-history-library.log`, `/tmp/reload-history-tests-final.log`, and
`/tmp/reload-history-check-final.log`. These checks do not establish the remaining
minimum-consumer or large-document performance requirements.

## Shared material value updates

Changes confined to Material, Shader, and NodeGraph prims now collect bound
consumers through shader connections and expand dependent point instancers,
instead of automatically reconciling the entire projection. Binding/collection
edits, graph traversal errors, mixed structural changes, and removed graph prims
retain the conservative path.

The regression changes a shared shader outside its Material namespace and adds
a roughness input. Both bound cubes and a point-instancer prototype update;
the separately bound cube retains its material handle and transform change tick,
and the unrelated instancer's transform tick remains unchanged. This verifies
projection isolation, not new screenshot or large-scene timing evidence.
Logs: `/tmp/material-scope-tests.log`, `/tmp/material-scope-library-final.log`,
`/tmp/material-scope-check-final.log`.

## Geometry removal isolation

The prim/entity map retains the last projected schema type and removes that
metadata with its mappings. Removed ordinary geometry subtrees can therefore be
reconciled locally; deleted shared schemas and unknown prior types still trigger
dependency-safe conservative handling.

The regression removes an Xform and its Cube child while retaining a separate
animated cube's entity, mesh handle, transform change tick, and animation-index
membership. A second test verifies that a deleted Material still requests
dependency reconciliation. The library gate passes 530 tests with 19 ignored,
and workspace check passes. Logs: `/tmp/removal-scope-tests.log`,
`/tmp/removal-scope-library.log`, `/tmp/removal-scope-check.log`.
