# Pending upstream fixes

These patches are review artifacts, not automatically applied dependencies.
The viewer still uses the revision in Cargo.toml and Cargo.lock.

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
interpretation is correct. That discrepancy and integration of this patch into
the actual dependency remain open.
