# Skinned rest tangents

## Change

Deforming skinned meshes no longer rebuild MikkTSpace tangents from the
deformed mesh every frame. Tangents are now generated on the pre-skin mesh
(the rest pose, or the rest pose plus blend shapes) and carried into the pose
by each vertex's blended linear skin matrix, then normalized. Handedness is
unchanged. This is the usual real-time approach and what Bevy already did for
GPU-skinned meshes whose normal corrections were identity.

- GPU skinning: the rest mesh always uses the cached `StandardTangents`
  assembly. Per-frame CPU deformation, deformed MikkTSpace and the inverse-skin
  tangent correction are removed. Pose-dependent normal corrections are kept.
- CPU skinning: the route generates tangents on the pre-skin mesh, which hits
  the tangent cache when it is the unchanged rest pose, and applies the same
  blended matrices. Both routes therefore still match each other.
- Morph-only meshes are unchanged.

## Editor timeline (release, gpu-prepared, 1,000 unique clocks)

Fresh processes in before/after/after/before order, default affinity. Before is
the saved `8297f078…` editor (no runtime source difference from this change's
parent); after is `999afc02…`.

| Asset | Median before | Median after | p95 before | p95 after | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cow | 54.14 / 54.60 ms | 20.82 / 20.82 ms | 58.40 / 70.74 ms | 22.36 / 22.66 ms | 529 → 532 MiB |
| Bird | 20.99 / 20.30 ms | 13.81 / 13.38 ms | 39.56 / 23.06 ms | 26.90 / 24.78 ms | 934 → 937 MiB |

Cow medians fall about 62% (≈2.6× faster) and bird medians about 34%. Earlier
identical-binary controls varied bird and cow medians by roughly 1%. These are
headless CPU update times, not native frame rates.

## Shading change

Tangent directions against per-frame deformed MikkTSpace, across all skinned
UV meshes at five clocks:

| Asset | Median | p90 | p99 | Handedness flips |
| --- | ---: | ---: | ---: | ---: |
| Cow | 0.06–0.15° | 3.5–5.3° | 17–36° | 0 |
| Bird | 0.11–0.17° | 11.9–12.9° | 34–37° | 0 |

The new tangents stay perpendicular to the deformed normal (cow p99 skew at
most 2.4°, bird 16°); the difference is a rotation within the surface plane
near joints and seams. Rendered with `viewer_capture` (1280×720), old versus
new GPU frames differ by more than 8/255 in 81 cow pixels (0.009%, cow at 144)
and 241 bird pixels (0.026%, bird at 132), with a maximum of 38/255. CPU and GPU
frames of the new build match within 1/255 for both assets.

## Verification

`make test-library` (both feature sets), `make lint`, the renders above and
the new GPU-mode assertions that supported skinned meshes submit no CPU
preparation jobs. Blend-shaped meshes still regenerate tangents on the morphed
pre-skin mesh each frame; that remains the next tangent cost.
