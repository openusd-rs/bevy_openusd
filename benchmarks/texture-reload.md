# Initial texture verification

## Scope and diagnosis

This follow-up to [texture finalization](texture-finalization.md) removes an
unnecessary reload after opening separate texture files. It does not change
texture resolution, sampling, image decoding, validation or watcher settings.
The broader objective remains in [`OPTIMIZATION.md`](../OPTIMIZATION.md).

Stage layer reads recorded file hashes, but initial editor texture reads did not.
The first source verification therefore classified unchanged external textures
as changed. Reloading them rebuilt images, materials and mip chains. A 4x2 PNG
test with the production plugins reproduces this deterministically: after Open,
an explicit no-change `ReloadSources` replaces the three-mip image handle.
The red test is recorded in `/tmp/bevy-optimization-reload-red2.log`.

## Repair

`9ded82f` records hashes of the exact encoded bytes passed to image decoding.
Packaged textures use the outer package identity and its captured bytes, not a
later disk read. Previously recorded hashes are retained so changes made during
loading remain detectable. Closed snapshots do not acquire filesystem baselines.
`4eac681` preserves directly supplied package-entry overrides without inventing
a hash for an unrelated outer file.

New texture paths introduced by edits, undo/redo or refresh use the same tracking.
Refresh publishes new hashes only after decoding and image installation succeed.
Initial-open hashes belong to the candidate document and are discarded on failure.
The existing reload transaction still owns replacement publication, dirty-layer
protection and last-good recovery. No watcher is disabled or delayed in production.

Only path/hash entries are added to editor provenance. Tracking does not retain
another copy of each encoded texture in `EditorDisk.snapshots` or source files.

## Regression coverage

- Three-mip textures keep their handles and finalization counts on unchanged
  verification, both initially and after a genuine successful reload.
- Same-size red-to-green PNG changes update pixels and regenerate mips. Invalid
  images retain the last-good handle and hash until a valid file is supplied.
- New texture paths record baselines; undo and redo followed by unchanged checks
  preserve texture handles, resource change ticks and finalization counts.
- Partial decode failures and unavailable image assets do not publish new hashes.
- Source tests cover first-read hashes, replacement snapshots, missing files,
  closed snapshots, nested packages changed after capture and explicit entry
  overrides. Encoded texture buffers are not retained by hash-only tracking.

The explicit-reload fixture disables its timer only to avoid wall-clock races;
native watcher checks separately exercise the unchanged production watcher.

## Alternating release measurements

The first paired run (`9ded82f` production code) is recorded in
`/tmp/bevy-optimization-reload-abba.yhzgMj/`. Order is before, after, after, before;
each process creates one App, runs four warmup seeks and 100 measured seeks.
Binary hashes, raw samples and host load are retained. There is no overlapping
task-owned build/benchmark; unrelated applications remain active. Load averages
are roughly 4.6–5.3. These are headless CPU updates, not displayed FPS.

| Cow measurement | Before range | After range |
| --- | ---: | ---: |
| Worst measured seek | 1.909–2.029 s | 104.4–106.2 ms |
| Reload command, 84 files checked | 407.4–430.8 ms | 25.4 ms |
| Files classified as changed | 5 | 0 |
| Measured texture finalizations | 4 | 0 |
| Measured texture finalization time | 588.9–628.9 ms | 0 |
| Process peak RSS | 816.9–817.0 MB | 552.8–553.1 MB |
| Peak mesh/material/image asset counts | 44 / 5 / 10 | 42 / 3 / 6 |

MB is decimal Linux VmHWM, not VRAM or retained allocation. Final rendered
geometry counts and mesh/image payload match; fewer transient assets are created.
Opening remains around 1.5 seconds. Steady cow updates remain about 79 ms;
bird updates remain around 31–34 ms. No steady-animation improvement is claimed.

Final production-code confirmation (`1eff934` binaries) is recorded in
`/tmp/bevy-optimization-reload-abba.yQGQcC/`. All 12 processes pass. SHA-256
manifests cover the bird package, morph fixture and all 267 files in the cow
directory; post-run checks pass. This directory manifest is not a semantic USD
dependency-closure audit. Entry-file/binary hashes and raw samples are also saved.

During this second run, unrelated builds drove load averages around 18–35 and
latencies varied substantially. The cow's redundant reload happened during the
four excluded warmup seeks in the before runs. Consequently its measured-seek
maximum and finalization counters no longer capture that initial stall; they
must not be used as a second first-seek speedup measurement. Logs still confirm
five false changes before versus zero after, matching final geometry/image
payload, reduced peak asset counts, and process peaks around 814 MB before versus
553–554 MB after. Bird timing is noisy; this is not final performance acceptance.
The follow-up in `e66e070` retains warmup timings and counters separately; see
[skinning preparation and warmup accounting](skinning-preparation.md). It does
not retroactively supply the missing warmup measurements in this older run.

## Verification gates

`test-all` at the initial fix passes 877 tests (20 ignored). `harden` at the final
code and follow-up tests (`1eff934`) passes 791 no-default tests (21 ignored),
884 all-feature/all-target tests (29 ignored), strict Clippy, formatting and
whitespace checks. Logs: `/tmp/bevy-optimization-reload-tests2.log` and
`/tmp/bevy-optimization-reload-harden.log`.

`test-watchers` passes all five native asset-watcher and four editor texture
watcher tests. `check-web` passes wasm32 compilation. Logs:
`/tmp/bevy-optimization-reload-watchers.log` and
`/tmp/bevy-optimization-reload-web.log`. Compilation is not browser rendering or
browser-performance acceptance.

The final default suite passes 878 tests (20 ignored):
`/tmp/bevy-optimization-reload-tests-final2.log`.
Before/after Vulkan captures at 1280x720 and 4x MSAA are pixel-exact at RGB
tolerance zero for bird at 0, cow at 5 and clearcoat at 0. Bird and cow images
were inspected. Evidence: `/tmp/bevy-optimization-reload-captures.N9LYjZ/`.
The RTX 4080 capture path uses `UsdAssetPlugin`, not the editor reload path;
editor behavior is covered by the production-plugin tests above. Both cow
captures retain the unavailable `OmniPBR.mdl` warning, so the combined capture
gate exits 1 for that diagnostic. It is not an all-green render/release gate.

The first final default-suite run exposed a projection-budget race in
`gpu_morph_subsets_follow_independent_clocks_and_cpu_cleanup`: the fixture
asserted both independently timed instances existed after one update, although
the default projection time budget can defer the second. Its fixture now uses
an unlimited projection budget, retaining every deformation/cleanup assertion.
The test-only repair is `bc95517`; the production budget is unchanged. Failure log:
`/tmp/bevy-optimization-reload-tests-final.log`.

## Remaining work

This is not a steady-animation or native-FPS fix. Initial file verification still
has a CPU cost; skin/mesh preparation, native frame pacing, browser rendering and
broader lifecycle acceptance remain open. The previously recorded real-bird
CPU/GPU image mismatch and cow's unavailable MDL diagnostic are not resolved here.

The first paired run isolates the next CPU target: cow `SkinRoute` application
takes 6,341.685 ms across 100 measured seeks, whereas GPU joint updates report
1.692 ms cumulatively. Bird `SkinRoute` takes 2,313.595 ms. These are nested CPU
measurements, not GPU elapsed times. Inspect `gpu_skin::attach`, its skin sample,
normal/tangent correction and intermediate mesh interning before attributing the
remaining cost to GPU joint updates. Evidence:
`/tmp/bevy-optimization-reload-abba.yhzgMj/{cow,bird}-2-after.log`.
