# Sharing scoped skeleton query handles

## Source finding

The existing `SkelSampleScope` stores owned `SkeletonQuery` and `AnimQuery`
values. Every cache hit clones a query. In the pinned OpenUSD source
`900ea31591348e3fbf416d4661d703aa8700771b`, `AnimQuery::clone` deep-copies its
`Vec<String>` joint order. `SkeletonQuery::clone` shares the definition but
clones its bound animation and mapper. A hit therefore still allocates and
copies immutable query metadata before classification or deformation.

The candidate shares those two private query handles with `Rc` inside the
existing stage-specific read-only scope. No new cache or persistent lifetime
is introduced. Pose samples, mesh mappings, influence reads, arithmetic,
unsupported-route behavior, custom-registry behavior and public APIs stay the
same. An unscoped lookup still builds a fresh query; failed construction still
returns before cache insertion.

## Acceptance declared before measurement

- Prove pointer sharing across cache hits and independently rebuilt mesh
  bindings; preserve absence, nested scopes, stage isolation and later edits.
  Weak handles must expire after scope cleanup when no consumer retains them.
- Run workspace hardening, native CPU deformation and wasm compilation. Existing
  bit-exact upstream-query tests must still pass for changed clocks, mappings,
  influences and geometry-bind transforms.
- Save release binaries before and after the change and compare the same
  whole-editor CPU workloads in alternating order without task-owned builds.
  Retain raw samples, opening/idle/tail/resource regressions and diagnostics.
  Reject a repeatable greater-than-5% control regression. Seek evidence for
  lower preparation cost, not a fabricated aggregate FPS claim.
- This is not a replacement for the existing native 60 Hz, visual, loading,
  resource, downstream or other-platform gates. Those remain open even if this
  local copy-elimination candidate passes its correctness checks.

The fresh pre-change release example is saved with revision `e6ee48e` under
`/tmp/bevy-optimization-shared-queries.cQLdQZ/` (`before-editor`,
`before-build.log`, `before-inputs.sha256`).

## Correctness checkpoint

Workspace hardening passes 882 no-default and 1,009 all-feature/all-target
executions, formatting and strict Clippy. The configurations overlap. The two
new sharing/lifetime/edit regressions pass, as do the existing bit-exact
upstream skinning comparisons. Four native OpenUSD deformation-oracle tests,
wasm compilation and the release editor benchmark build also pass. Logs are
`harden.log`, `native.log`, `wasm.log` and `after-build.log` in the evidence
directory. `candidate.patch` and `after-inputs.sha256` identify the tested code.

At the correctness checkpoint, the alternating whole-editor CPU comparison was
still pending. Its subsequent results are below.

## Alternating whole-editor CPU comparison

The saved binaries compare `e6ee48e` with the candidate committed as `26bbf91`:

- Before SHA-256: `f297fab23b2382a3df94db6d3b2de6f6f68948a0ad2a5c64116d0ae0b8d0906a`.
- After SHA-256: `e18898e1dc35afd923f6edcd1934ede521fdffbc15f1f3733f459437d8673547`.

`driver.sh` runs before/after/after/before per case with 1,000 distinct authored
clocks and four separately retained warmups per process. All 20 processes and
diagnostic scans pass. `analyze.mjs` verifies all 20,000 measured updates, all 80
warmups and their summaries, exact clock progression, CSV percentiles and
matching retained geometry/image/cache payload and asset counts within each
case. These payload counts are not a rendered-image or geometry-byte comparison.
Entry assets, the cow directory manifest and both executables verify afterward.

The measurements keep eager residency, the default four-job preparation and
default CPU affinity. No task-owned build overlaps; unrelated jobs are not
modified. One-minute host load ranges from 4.66 to 15.13. The table reports
means of the two per-process medians, not displayed FPS or a causal attribution
of every timing difference to query sharing.

| Workload | Before median update, ms | After median update, ms | Change |
| --- | ---: | ---: | ---: |
| Bird, GPU-prepared timeline | 22.235 | 21.002 | -5.54% |
| Cow, GPU-prepared timeline | 56.676 | 55.435 | -2.19% |
| Blended morph, CPU timeline | 0.702 | 0.448 | -36.16% |
| Blended morph, GPU-prepared timeline | 0.398 | 0.500 | +25.49% |
| Material subsets, GPU-prepared timeline | 0.174 | 0.180 | +3.15% |

Bird's two nearby before/after median comparisons improve 4.43% and 6.57%;
cow's improve 0.29% and 4.02%. Bird preparation counters remain identical
(11,000 batches, 40,914 tangent jobs, 82 cache hits, maximum four jobs and
435,792 input bytes). Its aggregate p95 nevertheless increases **3.77%**, and
maximum increases **23.10%**. The available evidence does not establish 60 Hz
or improved native frame pacing.

The GPU morph control's first nearby comparison improves 0.75%, while the
second regresses **50.99%** (0.610 ms candidate versus 0.404 ms baseline).
Neither this nor the unusually large CPU-morph decrease can be treated as a
stable query-copy effect. Subsets' p95 increases **19.87%**. No case regresses
more than 5% in both nearby median comparisons, but the aggregate GPU-morph
failure remains unresolved; it is not waived by that narrower check or by
host load. A follow-up must distinguish added CPU work from scheduling/counter
effects before full performance acceptance, rather than rerunning until green.

`summary.tsv` and `analysis.json` retain opening, idle, p95, maximum and peak
RSS observations alongside all raw update samples. Retained payloads are
unchanged; process RSS variation does not prove memory savings. Native rendered
comparisons, broader lifecycle/platform/downstream checks and the prior
intermittent pixel failures remain outside this CPU-only comparison.

## Counter audit of unstable small controls

`/tmp/bevy-optimization-query-counters.hKg7Hi/` adds hardware-counter evidence
without changing or rebuilding either saved executable. `driver.sh` runs two
ABBA blocks each for GPU morph, CPU morph and the unchanged subset control,
with the same authored timeline, profiling flags, eager residency and default
affinity. All 24 processes and diagnostic scans pass; input and asset hashes
verify. Load ranges from 6.72 to 9.47, with unrelated host work unchanged.

The pinned `perf` executable records **unscaled** user-space instructions and
cycles separately on the `cpu_core` and `cpu_atom` PMUs, plus software
task-clock. Every run's summed hardware running time equals software task-clock
running time, with matching instruction/cycle coverage on each PMU. The
analyzer rejects uncovered intervals or missing counters rather than scaling
each PMU count to the full interval and double-counting migrated work.
Counter scope includes opening, idle, warmups, seeks, shutdown and inherited
threads; it is not seek-only or native frame timing.

| Workload | Before mean instructions | After mean instructions | Instruction change | Mean-of-medians update change |
| --- | ---: | ---: | ---: | ---: |
| GPU morph | 3,430,593,881 | 3,417,003,618 | -0.396% | -15.29% |
| CPU morph | 3,190,234,084 | 3,184,031,131 | -0.194% | +16.17% |
| Material subsets | 1,856,827,320 | 1,854,286,902 | -0.137% | -17.95% |

These averages do not establish equally large positive or negative speedups.
GPU morph's first seven update medians range from 0.389 to 0.408 ms, while its
eighth **baseline** run reaches 0.699 ms. CPU morph has a 0.634 ms candidate
outlier versus 0.349–0.391 ms in the other seven runs. The unchanged subset
baseline likewise reaches 0.357 ms versus 0.179–0.196 ms otherwise. Instruction
counts stay close while cycles and task time vary substantially. Per-PMU
distribution is retained in `counter-runs.tsv`; the slow runs are not explained
by a single consistent core-type share. Reported context-switch/migration
counters are user-filtered zero and are not used to infer a scheduling cause
or an absence of switching.

`analyze.mjs` verifies all 24,000 unique-clock updates and 96 warmups, their
summaries and percentiles, counter coverage and matching retained payload/asset
counts. `analysis.json`, `counter-summary.tsv`, `counter-runs.tsv`, raw `.perf`
files and logs retain every run. The whole-process instruction changes are
small, with some variation even in the unchanged control; do not attribute
their full magnitude to query sharing.

This audit does **not** support a large added-instruction regression from the
query-sharing change. It also does not prove why an individual process was
slow, erase the original +25.49% GPU-morph result, or clear native pacing and
tail gates. No compensating code change or rollback is justified by these
counters alone. Stop unchanged small-control repetitions; retain the timing
limitation and return to distinct measured playback costs and rendered/lifecycle
verification. No production source, dependency or policy changes in this audit.
