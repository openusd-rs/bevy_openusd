# Local Bevy PBR patch

Source: crates.io `bevy_pbr` 0.20.1, MIT OR Apache-2.0, upstream commit
`a0577b5c6a7b208719dd761c5c6a7a539e472a97`. The original source, manifests,
licenses and VCS metadata were imported unchanged before the local patch.
Only `src/render/morph.rs` differs from the published Rust source.

## Extract the mesh identity with morph weights

`MorphWeightsInfo` retains the `Mesh3d` asset ID alongside the weights extracted
from the same entity and frame. Descriptor preparation uses that ID rather than
looking up an instance that GPU mesh collection has not yet published.

Bevy prepares morph descriptors before collecting GPU mesh instances. Previously,
a newly visible mesh could be collected with a cached `u32::MAX` morph descriptor
index. A valid descriptor appeared on the next frame, but unchanged mesh input
uniforms retained the sentinel indefinitely. Material readiness could incidentally
change this outcome, so cold first-open success did not prove correctness.

This patch preserves descriptor index allocation/reuse, current and previous
weights, GPU deformation, uniform-buffer fallback and the existing render
schedule. It does not force CPU mesh preprocessing, refresh every mesh each frame,
change authored weights, or delay the first visible frame.

The root Cargo patch selects this copy for this workspace. Cargo patches do not
propagate to downstream consumers of `usd_bevy`; they need this patch in their
own workspace until an upstream Bevy release includes an equivalent repair.
This remains a downstream release requirement, not an upstream fix or published
package claim. Do not edit the shared Cargo registry copy.

Regression evidence and remaining acceptance work are recorded in
`benchmarks/morph-publication.md`. Remove the patch only after the replacement
passes first-visible-frame, parent/subset, reveal/undo/redo and retained mesh
descriptor checks as well as the broader native deformation gates.
