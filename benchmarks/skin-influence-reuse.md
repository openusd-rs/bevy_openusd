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

## Initial wrap-up checkpoint

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

At this checkpoint it was a prepared diagnostic, not another completed
optimization. The original playback, visual, loading, resource and downstream
acceptance gaps remained unchanged.

## Real-asset inventory: candidate rejected

The subsequent goal continuation runs the saved `d2362b3` release diagnostic
without rebuilding or changing production. Evidence is under
`/tmp/bevy-optimization-skin-inventory.LsPmtX/`: `bird.log`, `analysis.json`,
`analyze.mjs`, the command and revision, source/binary verification and the
asset manifest. The saved executable SHA-256 is
`80cba009240b4397366fc7d5d581bc20818f2e2547e353c50a9ab63211bbdd18`.
There is no overlapping task-owned compilation or native capture.

All five authored clocks (0, 66, 132, 198 and 264) yield the same counts:

| Modeled entries | Points | Existing adjacent hits | Additional hits | Remaining misses | Additional evaluations avoided |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 1,238 | 184 | 0 | 1,054 | 0% |
| 16 | 1,238 | 184 | 59 | 995 | 5.60% |
| 64 | 1,238 | 184 | 63 | 991 | 5.98% |
| 256 | 1,238 | 184 | 63 | 991 | 5.98% |

The denominator excludes the existing adjacent fast path: 63 / 1,054, not
63 / 1,238. The 64-entry model avoids only 315 of 5,270 remaining evaluations
across the five clocks, **failing the predeclared 25% opportunity gate**.
Even 256 entries provide no additional hits. Do not implement this cache or
reinterpret 5.98% as a sampler or application speedup.

The release diagnostic passes its bit-exact modeled-reuse checks for 190
accepted samples (38 mesh paths per clock). No accepted sample is flat-normal
or rigid. Fifteen rejected samples are retained: `body`, `head_feathers` and
`tail_transition` at each clock exceed the existing four-influence GPU limit.
The inventory neither measures their CPU fallback cost nor changes that route.
Its analyzer verifies all 760 capacity rows, unique sample/capacity pairs,
clock coverage, accounting, capacity bounds and the completion summary.
Input and asset hashes verify before/after the run.

This result closes only the proposed non-adjacent correction-cache experiment.
It does not clear prior timing or rendering failures. Further skinning work
must target another measured cost, such as binding/eligibility reads or
CPU-fallback/tangent preparation, rather than enlarging or retrying this cache.
No production behavior, fidelity, dependencies or default policies change.
