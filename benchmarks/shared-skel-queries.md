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

## Native rendered before/after checks

Evidence: `/tmp/bevy-optimization-query-render.COOryT/`, including `reframed/`.
The freshly built candidate viewer at `c0724f2` contains `26bbf91` and matches
the saved `2b4c718` pre-change viewer's `debugger_capture` feature selection.
The optional debugger, morph and independent-target probes are disabled in
both; there is no RenderDoc injection. The source delta is retained, with no
other production change between these viewer checkpoints.

- Before viewer SHA-256: `cc2b573b49545100a212be98e4ed079cfd6dd7b915c7b45cd68bc537d7e20d4e`.
- After viewer SHA-256: `e02bc26c3b07457e59a39f3f464e74e134ff249a37904f19ccbfff93b9dfd254`.

Each capture uses the native viewer in its own private Weston/DBus session,
validation/debug checks enabled, eager residency, default quality, 1440x920
viewport readback and at least 120 settled updates / 1,000 ms after the seek
sequence. No task-owned build overlaps. The bird seeks 0,264,132,132; the cow
0,288,144,144; the combined morph/skin fixture 0,10,7,7. These are live editor
seeks, including reversal and repetition, not an animation-frame-rate test.

All twelve capture processes, readbacks, input hashes and entry-asset checks
pass; the full cow directory verifies before/after. Camera, grid, environment
and retained-mesh metadata match within every before/after pair. Tracked fixture
assets have no diff between the recorded revisions; their dependency hashes
are also retained at the end, not presented as a pre-capture closure manifest.

The initial four before/after comparisons are exact in **all RGBA bytes**.
Visual inspection shows a fully visible textured bird and cow, but the
fixture's camera fitted at time zero crops its later pose. That cropped result
is retained rather than treated as full-object proof. Four further fixture
captures change only the initial stage time to seven, allowing the opening
camera to fit that pose, then perform the same 0,10,7,7 seeks. Both CPU and GPU
fixtures are fully visible on inspection and both additional before/after
comparisons are exact. No geometry, lighting, quality or tolerance was changed.

| Fully visible comparison | Final time | Changed RGB pixels | Changed alpha pixels |
| --- | ---: | ---: | ---: |
| Bird, GPU-prepared | 132 | 0 | 0 |
| Cow, GPU-prepared | 144 | 0 | 0 |
| Combined morph/skin, GPU | 7 | 0 | 0 |
| Combined morph/skin, CPU | 7 | 0 | 0 |

This yields six exact before/after comparisons in total, four with inspected
full-object framing. `analysis.json` and `reframed/analysis.json` retain the
metadata and image hashes. This proves unchanged output for these sampled
poses, not every clock, asset, device or lifecycle path.

Separate CPU-versus-GPU comparisons **fail zero tolerance** in both executables:
two mesh pixels differ by one RGB level, with identical coordinates and channel
values before and after query sharing. Initial framing differs at (627,691)
and (479,896); full framing differs at (591,302) and (871,307). Alpha is exact
and camera metadata matches across modes. `mode-analysis.json` retains all
values. The existing mode discrepancy is not fixed or waived, and these pixels
are not evidence that the earlier intermittent grid discrepancy was repaired.

Both capture drivers exit 1 because diagnostics remain: the initial matrix
records 32 matching Vulkan-loader/clipboard warning lines, the reframed matrix
17 matching diagnostics including a 100 ms XDG color-scheme portal timeout.
Full logs also retain portal/PipeWire messages. Image/state analysis passes,
but the combined diagnostic gate is not all-green. No production code changes
were needed for these checks. Native pacing, loading, resource, downstream and
other-platform acceptance remain open.
