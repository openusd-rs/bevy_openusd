# Patched OpenUSD dependency

Upstream: https://github.com/mxpv/openusd

Base revision: `b7df5add628cbb791103a7da842dbd82810da5d0` (0.7.0).
Rechecked upstream HEAD on 2026-09-10; it still matches this revision.

Included: all three workspace crates, their schema inputs and licenses, root
workspace manifest, README, license and Rust formatting/lint configuration.
Excluded: upstream's large external fixture packs, repository/CI metadata,
toolchain override, build outputs and dependency lockfile. The containing Bevy
workspace supplies its toolchain and lockfile. No sibling checkout is required.

Changes from the base source are recorded in these patches in the project root:

- `patches/openusd-empty-index.patch`
- `patches/openusd-singleton-listops.patch`
- `patches/openusd-token-vector-fields.patch`
- `patches/openusd-stage-packaging.patch`
- `patches/openusd-package-directory.patch`
- `patches/openusd-prepared-roots.patch`
- `patches/openusd-reference-diagnostics.patch`
- `patches/openusd-layer-reanchoring.patch`
- `patches/openusd-clip-activation-samples.patch`
- `patches/openusd-nested-package-reading.patch`
- `patches/openusd-nested-package-export.patch`
- `patches/openusd-undo-pruning.patch`
- `patches/openusd-payload-identity.patch`
- `patches/openusd-reference-custom-data.patch`

The binary metadata patch includes sublayer StringVector encoding as well as
token-vector child/order fields and variant-set StringListOp encoding.
The stage packaging patch adds resolver-aware dependency traversal without
changing the layer-only ArchiveWriter contract. See the root PACKAGING.md for
its limits and unsupported inputs.
Single-level source archives are cached and their entry reads are bounded;
package-relative dependencies are repackaged rather than retained externally.
The reference diagnostic patch reports missing external root and sub-root targets.
Sub-root targets are checked after composition tasks finish, retaining targets
whose opinions come from variants. Broader ancestral/relocation combinations
remain outside the current acceptance evidence.
The layer-reanchoring patch adds resolver-aware copies for cross-directory
ordinary root/edit exports. It anchors sublayers, reference/payload list buckets
and nested typed asset values without editing the live layer. Dependencies stay
external; asset expressions, tile/sequence patterns and clip templates fail
explicitly rather than being silently relocated.
The clip-activation patch interpolates non-asset values between composed
stage-time sample boundaries across active clips and retains value-block
transitions. Asset-valued clip resolution is unchanged. It does not add clip
baking or establish complete clip parity.

The root Cargo patch table redirects all three Git dependency packages here.
The capture wrapper also exposes scoped recording suspension, recent transaction
layer identifiers, and an oldest-first transaction retention mask. The editor
uses these APIs to publish external layer refreshes without recording them as
user edits, retain unrelated undo/redo commands, and discard commands touching
reloaded layers. Suspension restores its previous state across nested calls and
unwinding; a retention-mask length mismatch leaves history unchanged.

The usd_bevy and usd_macro manifests also use direct relative paths to this
copy, so external path consumers do not depend on inheriting a root patch table.
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
Library path consumers require this repository's vendor directory; crates.io
publication requires an upstream/fork release containing the required API first.
Do not overwrite user-modified ignored vendor or sibling state during upgrades.
