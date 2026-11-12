# Standalone library consumer

This crate is its own Cargo workspace. It depends on `usd_bevy` through the
public library API and explicitly repeats the two required Bevy patches; it
does not inherit the viewer's workspace overrides or feature unification.
The local lockfile starts from the root's resolved dependency versions and is
then maintained independently. It is not a published-package test.

From the repository root:

```sh
oslo make prepare-downstream
oslo make test-downstream
```

`test-downstream` tests the consumer with no optional library features and
with all three optional features, then runs strict consumer Clippy. The
tests cover public plugin setup, two independently timed roots, shared mesh
assets, seek, identity-preserving reload, last-good failure/recovery, removal
and cleanup, and source-load ordering without blocking unrelated sources.
The intentional invalid-source/older-loader failures are test inputs.

`graph.mjs EVIDENCE_DIRECTORY`, run through `make run-headless-benchmark`,
records actual Cargo metadata for the patched consumer and two disposable
consumers each missing one patch. It requires one Bevy 0.20.1 graph, both
local patches and the pinned official OpenUSD revision, and rejects the
unpatched registry resolutions. The positive graph must not contain the
viewer or Mara. This is dependency-resolution evidence, not a GPU test.

Both overrides belong in the **consuming workspace root**. Cargo ignores
patch tables in dependencies; see the [Cargo reference](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section).
The asset override preserves source-load ordering. The PBR override repairs
first-visible morph descriptor publication. Use the same local checkout or
verified revision for the library and both overrides. A Mara override is only
needed by the viewer, not this standalone library consumer.

`oslo make test-downstream-gpu --output NEW_DIRECTORY` needs a GPU. It renders
`assets/morph_subsets.usda` offscreen through the public plugins, first with
CPU morphs as the reference, then with `UsdGpuSkinningPlugin`. Each run
captures five phases: hidden, first reveal, hidden again, revealed again and a
replacement root. Hidden frames must be black. Visible frames must show both
the orange subset and the parent material, and GPU frames must match the CPU
reference within RGB 1. The GPU run also checks that every visible morph
instance gets a current descriptor with the expected vertex count and weight.
Without the PBR override the first reveal draws the wrong shape and the
comparison fails; the subset/parent colour check alone does not catch it.
The recipe repeats both runs with `DeferHiddenMeshes`: the initially hidden
root has no meshes until its Bevy `Visibility` reveals it, and every phase
must still match the eager CPU reference.
Acceptance was recorded on local Linux NVIDIA Vulkan only.

Downstream application workloads, other browsers and platforms, and
package/release acceptance remain separate gates. The fixture does not turn
root-only overrides into transitive patches or remove the consumer
configuration requirement.
