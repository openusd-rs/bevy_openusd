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
