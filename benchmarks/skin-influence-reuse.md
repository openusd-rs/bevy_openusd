# GPU skin influence reuse inventory

The existing preparation profile identifies GPU skin sampling as part of the
bird's remaining eligibility cost. Production currently reuses normal corrections
only for consecutive identical influences and caches each joint's normal matrix.
This inventory asks whether a small per-sample cache could avoid additional
normal-correction evaluations without changing arithmetic or rendering.

Before measurement, require a modeled cache of at most 64 entries to eliminate
at least 25% of the remaining non-adjacent evaluations across the representative
bird clocks. This is an opportunity filter, not a speedup target or acceptance.
If it fails, do not implement that cache. If it passes, a separate implementation
still needs bit-exact correctness, error/order/lifecycle bounds and a tightly
paired timing comparison before a whole-viewer performance experiment. No
previous timing or visual failure is cleared by an inventory.

`make build-skin-probe` builds a release library test executable without running.
Save that executable, then run its ignored
`read::skel::influence_probe::real_asset_gpu_influence_reuse_inventory` test through
`make run-headless-benchmark`, with `USD_SKIN_ASSET` set to an absolute asset path
and `USD_SKIN_SAMPLES=5`. Do not compile while collecting evidence.

The inventory samples sorted, time-varying skinned mesh paths at the authored
endpoints and three intermediate clocks, using the existing stage-specific
read-only scope. It records current GPU skin-reader time separately from mesh
decoding, classification and simulated cache work. Bounded
models use capacities 0, 16, 64 and 256, distinguish the existing adjacent fast
path, and check correction bits for every reused key. Weight bits include signed
zero. Flat-normal meshes and rigid bindings are reported separately; they must
not be counted as weighted-normal-correction opportunities. Unsupported GPU
bindings remain explicit rejected rows. No production cache, default, scheduler,
geometry, dependency or quality setting changes.

## Wrap-up checkpoint

The user requested wrapping up before this real-asset inventory was run. No
inventory data, modeled opportunity percentage or performance gain is claimed,
and no production reuse cache was implemented.

The two synthetic regressions pass inside final workspace hardening: 880
no-default and 1,007 all-feature/all-target executions, formatting and strict
Clippy. The scoped release test executable builds successfully. Logs are under
`/tmp/bevy-optimization-skin-reuse.wzRawc/` (`harden-scoped.log` and
`build-scoped.log`). The earlier draft's checks are retained separately; the
final probe filters time-varying skin paths and includes binding resolution in
its reader timer. The optional real-asset inventory remains explicitly ignored.

This is a prepared diagnostic, not another completed optimization. Resume it
only if further performance work is requested; the original playback, visual,
loading, resource and downstream acceptance gaps remain unchanged.
