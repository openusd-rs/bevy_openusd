# Skeletal playback costs after skinned rest tangents

Follow-up to [skinned rest tangents](skinned-rest-tangents.md). Each change was
measured with the release `editor_benchmark` (`1 gpu-prepared seek-timeline`,
1,000 unique clocks) in before/after/after/before order against the previous
commit's binary, default affinity. Medians are per headless update, not native
frame rates.

| Commit | Change | Cow median | Bird median |
| --- | --- | ---: | ---: |
| `53faf17` | skinned rest tangents | 54.4 → 20.8 ms | 20.6 → 13.6 ms |
| `71c359b` | GPU-skin unweighted points | 21.0 → 18.3 ms | 12.7 → 12.6 ms |
| `4c0528a` | stop resampling SkelAnimation prims | 18.3 → 15.9 ms | unchanged |
| `9f5ffd5` | deformation-only meshes run only deformation routes | 16.2 → 13.7 ms | 12.7 → 10.1 ms |
| `ed33cde` | viewer defers hidden meshes by default | 13.8 → 6.7 ms | 10.2 → 9.9 ms |

## Changes

- **Unweighted points.** Each cow `LOD0` has 24 points with all-zero weights,
  which forced the 5,688-point mesh onto CPU skinning. UsdSkel linear blending
  sends such a point to the mesh origin. The GPU sample now gives each one its
  own palette entry, a translation by minus its rest position, which lands it
  on the origin with an identity normal and tangent transform. CPU normal
  skinning keeps the input normal for those points instead of failing the mesh.
  Blend-shaped meshes with unweighted points still use the CPU route.
- **SkelAnimation clips.** The cow stage holds 150 SkelAnimation clips; their
  time-sampled `translations`/`rotations`/`scales`/`blendShapeWeights` made
  each one an animated prim that ran every route per frame. Those inputs reach
  the scene only through bound meshes, which remain animated. Other sampled
  attributes, such as `bevy:*`, still mark a clip animated.
- **Deformation-only meshes.** A mesh whose only time-varying input is its
  skeletal or blend-shape deformation carries `DeformationOnlyAnimation`.
  Resampling a resident marked mesh runs only the skin, subdivision and subset
  routes; residency transitions and deferred meshes use the full route pass.
  Markers are re-derived after projection, reconcile and property changes,
  and cached per composition revision for instance roots.
- **Deferral default.** `USD_DEFER_HIDDEN_MESHES` now defaults to `1` in the
  viewer; `0` restores eager residency. The library default is unchanged.
  While verifying it offscreen, a deferred `UsdSceneRoot` revealed by a Bevy
  `Visibility` change never materialized at a paused clock; `e6fab1b` queues
  such meshes for preparation.

## Verification

Every commit passed `make test-library` (both feature sets), `make lint`,
`make test` and `make test-downstream`. Offscreen `viewer_capture` frames of
cow (144) and bird (132), CPU and GPU, are byte-identical to the tangent
commit, and resampled instances (`USD_CAPTURE_SWAP_CLOCKS=1`) match directly
projected ones exactly. `make test-downstream-gpu` now also runs CPU and GPU
deferred modes; all phases match the eager CPU reference within 1/255 and the
GPU first-visible morph descriptors stay valid. Native-window reveal captures
were not rerun.

## Remaining

- The bird `body` mesh uses up to 20 influences per point and stays CPU
  skinned; keeping the four strongest would move 23 points by more than 10%
  of their weight.
- GPU-skinned meshes with non-identity normal corrections still rebuild and
  re-upload a mesh every frame for pose-dependent normals, and preparation
  still re-reads static mesh data per frame (about 6 ms on the eager cow).
- Morph-only meshes still regenerate MikkTSpace tangents every frame.
