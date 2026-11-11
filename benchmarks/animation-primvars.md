# Animation primvar ownership sampling

## Candidate and acceptance

At `0b4203a`, initial projection shares ancestor primvar authoring/interpolation
answers through `PrimvarScope`, but animation does not. Animated sibling meshes
repeat the same ancestor resolution for absent normals/UV/color/opacity and
inherited constant attributes. Values and time samples must still be read fresh.

The candidate bounds that existing scope to 4,096 answers and 1 MiB of key text,
then enables it for one built-in animation pass, including serial fallback.
Over-budget reads remain uncached. No scope crosses updates, clocks, stages or
custom-route callbacks; no geometry, tangent arithmetic or cache policy changes.
The text cap excludes hash-table metadata, allocator overhead and stage storage.

Declared before implementation:

- Compare ownership with independent uncached walks in both traversal orders,
  including absent, blocked, local and nonconstant-ancestor attributes.
- Exercise nested/independent stages, edits between passes, zero/entry/text caps,
  cleanup and custom callbacks that mutate a later sibling's inherited owner.
- Compare complete assembled mesh payloads across serial/batched animation,
  changing/repeated/reversed clocks and inherited attribute edits.
- Run repository hardening, native deformation/watchers/export and wasm checks.
- Compare saved release executables in fresh-process ABBA order for bird, cow,
  arm and subsets using 1,000 authored-range clocks; preserve raw samples,
  warmups, complete update intervals, payloads, asset counts and RSS. Do not
  overlap task-owned builds or alter unrelated work/host settings.
- Incremental target: at least 10% lower bird mesh-read time and 5% lower bird
  whole-update median, with no greater-than-5% aggregate timing regression in
  controls. Retain failed observations rather than weakening these thresholds.
- Compare native rendered poses at exact clocks with RGB tolerance zero.
  CPU timings are not window FPS. Prior render/performance failures and the
  full optimization checklist remain open regardless of this candidate.

Baseline executables and build logs are retained under
`/tmp/bevy-optimization-primvar-pass.SEBLD3/`.

## Correctness checkpoint

Four new regression tests cover the ownership/cache contract, custom-route
mutation visibility and byte-exact CPU/GPU mesh output against uncached direct
patches across forward/reverse/repeated clocks and inherited interpolation edits.
Hardening passes 852 no-default and 971 all-feature/all-target executions,
strict Clippy, formatting and whitespace checks. Configurations overlap.
The authoritative log is `harden-final.log` in the evidence directory.
Two earlier attempts are retained: a moved test attribute builder and a missing
attribute spec in the new fixture. Both were test construction errors; the
fixture now creates its attribute and clones the consuming builder explicitly.
This checkpoint does not establish performance or rendered acceptance.
