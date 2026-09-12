# Editor hot reload

The core `EditorPlugin` now polls loaded layer files on native targets by default.
It checks metadata every 250 ms and waits for a stable observation before reading
changed files. No `file_watcher` Cargo feature or viewer-only plugin is required.

`reload::LayerReload` parses replacement layers, validates them in an isolated
composition, and publishes field-level differences through `Stage::batch_edit`.
The existing stage and Bevy entities remain in place. USD change notices drive
projection; this is not `EditorCommand::Open` or a root-wide forced resync.

Initial regression evidence (`/tmp/core-reload-test.log`): four tests pass,
including a real filesystem save affecting two references, unchanged unrelated
entity/mesh/runtime-component identity, malformed-save retention, atomic-rename
recovery, and dirty-layer conflict rejection. Workspace check passes.

Current limitations, still being implemented:

- Package-contained layers are explicitly rejected, not silently reopened.
- This layer watcher does not yet handle texture-only changes.
- External layer updates reset undo history; current unsaved opinions in other
  layers remain, but a changed dirty layer blocks publication.
- Candidate validation traverses a document copy even though publication applies
  only changed layer fields. Large-document timing is not qualified.
- Metadata polling cannot detect a writer that preserves both mtime and size.
- No live Blender or GUI screenshot validation has been performed for this path.
