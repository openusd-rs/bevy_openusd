# Posed sorting centers

## Problem and constraints

The labelled hummingbird phase diagnostic records different left-wing/grid draw
order between CPU and GPU deformation at time zero. Both grid settings are
identical; the generated wing subsets use posed mesh positions on CPU but rest
mesh positions for the GPU render asset's sorting center. See
[the observed phase keys](hummingbird-playback.md#observed-transparent-phase-divergence).

The correction keeps the existing grid, material depth bias, sorting mode and
geometry. It does not change comparison tolerances, shared mesh AABBs or position
compression metadata. Independent roots and material-subset children require
independent centers even when they share a mesh asset.

## Implementation under validation

`route/deformed_sort.rs` caches one optional world-space center per owned GPU
deformation draw. It follows the CPU renderer's convention: enclose all posed
vertices in mesh-local space, then transform that AABB center to world space.
This is not generally the same as the center of a world-space AABB. Morphs are
applied before skinning, including referenced morph weights and inverse bind
poses. Existing camera/grid bounds continue enclosing drawn vertices only.

The main-world Last system runs after palette/global propagation and asset-event
publication. It rebuilds on mesh/transform/skin/morph changes, referenced weight
or joint changes, and mesh/inverse-bind asset events. Idle frames reuse the
cached center. Invalid or unavailable data yields no override. Cache ownership
follows the existing skin/morph claims and is released with the last claim;
cache entries retain no asset handles. Extraction replaces the per-entity map
each frame and shrinks excess capacity after scene removal.

Before render queuing, original retained phase keys from the previous frame are
restored. After queuing and before sorting, only marked USD draws receive their
posed center in transparent/transmissive phases. Restoring the original key
prevents a removed or invalid override from leaving stale posed data in retained
phase items. Material bias, always-on-top mode and unrelated draws remain intact.
The systems are registered by the GPU skinning plugin only with a RenderApp;
headless scene-update comparisons do not measure this render-path cost.

`DeformedSortTiming` optionally records main-world dependency-check/update time,
center rebuilds and vertices visited. Capture metadata includes these totals;
they are not a native frame-cadence measurement.

## Required evidence

- Unit coverage for shared meshes with independent weights, local-vs-world AABB
  centers, joint and inverse-bind edits, missing inputs, idle reuse, last-owner
  cleanup, and retained transparent/transmissive key restoration.
- Original hummingbird CPU/GPU captures at 0/5/10 with RGB tolerance one and the
  grid visible; retain any residual discrepancy rather than weakening the gate.
- Independent timed roots with reversed clocks, including subset geometry.
- CPU controls unchanged, production plugin registration, strict hardening,
  native deformation/watcher/export checks and wasm compilation.
- Render-path cache/rebuild cost and native playback cadence. Existing headless
  paired timing improvements do not prove this new path's runtime cost.

## Initial checkpoint

Default workspace/all-target tests pass: 900 passed, 20 ignored. The time-zero
bird capture in `/tmp/bevy-optimization-posed-sort-captures.rK9qoF/` passes the
original RGB tolerance of one: zero failing pixels, maximum RGB error one,
versus 2,053 failing pixels before the correction. The CPU control remains
pixel-identical to its pre-change capture at tolerance zero. Both images have
compiled-pipeline/readback proof and clean renderer log checks; the source hash
verifies. The bird image and difference image were inspected with the grid and
transparent wing blur still visible.

Actual GPU phase order now matches CPU: right wing, left wing, grid. Left-wing
distance is -11.429891 and the grid remains -10.7276535; the grid transform,
spacing, fade and sort key are unchanged. No material or vertex data is altered
by the sorting override. Metadata records 40 center rebuilds / 5,292 vertices
across 798 main-world updates, with 10.103 ms accumulated update work. Many
updates precede loading, so dividing that total into an apparent frame rate
would be misleading. The idle-cache regression separately verifies no rebuilds
without dependency changes.

Other clocks, independently timed renders, strict hardening, native gates and
render-path cadence remain to be validated. This checkpoint fixes the observed
time-zero discrepancy, not every outstanding rendering/performance requirement.
