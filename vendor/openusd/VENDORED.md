# Patched OpenUSD dependency

Upstream: https://github.com/mxpv/openusd

Base revision: `b7df5add628cbb791103a7da842dbd82810da5d0` (0.7.0).
Rechecked upstream HEAD on 2026-09-10; it still matches this revision.

Included: all three workspace crates, their schema inputs and licenses, root
workspace manifest, README, license and Rust formatting/lint configuration.
Excluded: upstream's large external fixture packs, repository/CI metadata,
toolchain override, build outputs and dependency lockfile. The containing Bevy
workspace supplies its toolchain and lockfile. No sibling checkout is required.

Changes from the base source are exactly the two patches in the project root:

- `patches/openusd-singleton-listops.patch`
- `patches/openusd-token-vector-fields.patch`

The binary metadata patch includes sublayer StringVector encoding as well as
token-vector child/order fields and variant-set StringListOp encoding.

The root Cargo patch table redirects all three Git dependency packages here.
Keep the Git revision declarations: they record the baseline for removing this
override when the fixes become available upstream. The patches are applied to
these files already; they are not applied at build time.

Use the containing project's `make test-native`, `make test-all`, `make check-all`
and `make build`. Upstream's own tests additionally require its external fixture
packs and CARGO_WORKSPACE_DIR; their results were obtained in a separate full
checkout, not by treating omitted fixture tests as passing here.

To return to upstream, verify a revision containing equivalent fixes, remove
the root patch table, update all Git revision declarations and regenerate the
lockfile. Run the native and ordinary gates before removing this directory.
Do not overwrite user-modified ignored vendor or sibling state during upgrades.
