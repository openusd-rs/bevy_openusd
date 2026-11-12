# Scoped animation authored-value flags

**Status: withdrawn.** Local candidate `09af584` passed correctness checks but
missed the predeclared cow performance target. The implementation below is
historical, not the current production path. Its source and tests remain in
Git history; `skel.rs` is restored byte-for-byte to parent `3928a11`.

## Candidate

The [CPU profile](preparation-cpu-profile.md) identifies substantial crate
decoding/copying. At the pinned OpenUSD revision, `Attribute::has_authored_value`
calls proximal `resolve_info`. The composition walker reads the time-sample
field through `AbstractData::try_field`; the crate backend materializes every
value in that map. `InfoResolver` then inspects whether the map supplies a value.
This is not merely a cheap presence-bit query.

Sibling meshes repeatedly classify the same animation through
`skin_is_time_varying`, `blend_is_time_varying` and
`deformation_is_time_varying`. The existing `SkelSampleScope` already retains
one query per animation during a built-in read-only pass. The candidate adds
two optional boolean answers to that entry: authored joint transforms and
authored blend weights. It preserves the existing meaning of "authored",
including static defaults, blocks, schema fallbacks and short-circuit order.
Any visited query error prevents memoization; that call still treats the error
as false and checks later attributes, matching the original behavior.

No array, decoded time-sample map or new clock-keyed history is retained. Pose
and influence values are still sampled at each requested clock. Scope nesting,
stage identity and cleanup are unchanged; later edits and custom routes use
fresh reads. This introduces no new dependency patch and keeps upstream
`900ea31591348e3fbf416d4661d703aa8700771b` unchanged.

## Acceptance declared before measurement

- Preserve fresh-reader answers for both categories, mixed answers, absent and
  blocked attributes, sibling meshes, independent/nested stages and later edits.
  Keep query errors uncached and preserve short-circuit behavior. Custom routes
  must observe edits made by preceding callbacks.
- Pass repository hardening, native CPU deformation and wasm compilation;
  retain the existing value, normal, tangent, lifecycle and mutation regressions.
- Compare saved release binaries in alternating before/after/after/before
  order for bird, cow, CPU/GPU morphs and unchanged subsets. Use 1,000 unique
  authored-range clocks and four retained warmups. No task-owned builds overlap.
  Targets: at least 10% lower bird and 5% lower cow CPU-update medians, no
  repeatable greater-than-5% control median regression, unchanged geometry/image
  payload and no greater-than-5% peak RSS increase. Record all tails, opening,
  idle and failed observations; never rerun unchanged merely to get a pass.
- Compare native before/after final poses after forward/reverse/repeated seeks
  with strict RGBA equality and unchanged readiness/camera/residency metadata.
  Previous visual and host diagnostic failures remain open.
- CPU update measurements are not native FPS. Full `OPTIMIZATION.md` acceptance,
  including the native 60 Hz target, remains separate.

Evidence directory: `/tmp/bevy-optimization-animation-authored.vrRsJn/`.
The baseline is the saved query-sharing editor executable, SHA-256
`e18898e1dc35afd923f6edcd1934ede521fdffbc15f1f3733f459437d8673547`, matching
the current library source before this candidate.

## Correctness checkpoint

Workspace hardening passes formatting, strict Clippy, 887 no-default and
1,014 all-feature/all-target test executions; the configurations overlap.
The three new tests cover short-circuit/error handling, shared and isolated
authored answers, later sampled/default/block edits, fresh numeric-time reads
and mutation from custom route callbacks. Existing query lifetime, deformation,
independent-root and reload tests remain enabled. Four native OpenUSD CPU
deformation-oracle tests and wasm compilation also pass.

Logs are `harden.log`, `native.log` and `wasm.log`. The earlier failures remain:
`harden-first-compile.log` caught an old test assertion matching the replaced
cache entry type; `harden-fixture-failure.log` caught the new fixture's missing
applied `SkelBindingAPI`. Both were repaired in tests without weakening their
assertions or changing production schema recognition.

Both release builds completed. `build.log` records the editor benchmark and
`viewer-build.log` the capture-feature viewer. The candidate editor SHA-256 is
`a7323cd8f9bcbf55375f8815bd59be061c75e90cce743860f94cced71a1f617f`;
the baseline editor hash is recorded above. The native baseline viewer is the
saved `190f4b8` INFO-default build with unchanged pre-candidate library code.

## Release CPU comparison

`driver.sh` completed all 20 saved-binary processes in
before/after/after/before order, four per workload. Each process uses four
warmups followed by 1,000 distinct authored-range clocks in a fresh headless
editor. Default eager residency, CPU affinity and four-job tangent preparation
are unchanged. No task-owned builds or captures overlap the measurements;
unrelated host work was not stopped. Input, full cow-directory and executable
manifests were checked before and after. These are CPU update measurements,
not native frames or isolated `has_authored_value` timings.

`analyze.mjs` validates all 20,000 measured updates and 80 warmups, clock ranges,
nearest-rank summaries, zero process failures/diagnostics and equal final
geometry/image/assets/cache payload. Measured mesh preparation, mesh-read and
GPU-skin work counters also match across each workload's four runs.

Values below average the two per-process summaries for each binary. Negative
percentages mean lower cost; the comparison is not a pooled median.

| Workload | Before median ms | Candidate median ms | Median change | p95 change | Maximum change | Peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Bird GPU-prepared | 23.107335 | 18.336169 | -20.65% | -15.39% | +38.54% | +0.08% |
| Cow GPU-prepared | 54.733541 | 53.691372 | -1.90% | -9.50% | +5.11% | +0.45% |
| Morph CPU | 0.337324 | 0.336492 | -0.25% | -6.05% | -33.07% | +3.34% |
| Morph GPU-prepared | 0.368786 | 0.373387 | +1.25% | +4.46% | -21.12% | +3.02% |
| Material subsets | 0.166659 | 0.172730 | +3.64% | +17.89% | +84.16% | +3.18% |

| Workload | Opening change | Idle change | Nearby pair 1 median change | Nearby pair 2 median change |
| --- | ---: | ---: | ---: | ---: |
| Bird GPU-prepared | -0.66% | -2.84% | -10.59% | -28.51% |
| Cow GPU-prepared | -2.92% | -11.30% | -0.19% | -3.58% |
| Morph CPU | +3.23% | -15.89% | +0.52% | -1.01% |
| Morph GPU-prepared | +3.75% | +3.40% | +1.00% | +1.50% |
| Material subsets | +12.03% | +334.57% | +2.07% | +5.23% |

The 10% bird median target passes, but the 5% cow target fails in both pairings
and the mean-of-medians comparison. No control has a greater-than-5% median
regression in both pairings; mean peak RSS increases stay below 5%. Those
passing checks do not erase the missed cow target, the bird maximum or the
subset opening/idle/tail regressions. The available measurements do not
establish why the cow benefit is small or what causes the regressing tails.
The source-level decoding path is not proof that authored-value queries
dominate either asset's total decoding cost.

Raw logs, `results.tsv`, `summary.tsv`, `analysis.json`, host snapshots and
manifests remain in the evidence directory. `benchmark-status.txt` records
`driver=0` and `analysis=1`; the analyzer explicitly reports
`cow: median target -5% missed`. This is a failed performance gate, not an
application crash. No unchanged rerun was used to seek a passing result.

## Withdrawal and remaining scope

The cache and its three implementation-specific tests are removed together,
restoring `crates/usd_bevy/src/read/skel.rs` exactly to `3928a11`. Existing
query sharing, viewer logging, deformation arithmetic, dependency pins and
all pre-candidate tests remain unchanged. The candidate's original tests,
passing correctness logs and failed performance evidence remain available.
This withdrawal is not a claim that the candidate is functionally incorrect;
it avoids retaining a change that missed its declared acceptance contract.

Post-withdrawal `oslo make harden` passes formatting, strict Clippy, 884
no-default and 1,011 all-feature/all-target test executions; the configurations
overlap. `oslo make build-debugger-capture` also succeeds, rebuilding the
release viewer from the restored source. See `withdrawal-harden.log`,
`withdrawal-viewer-build.log`, `withdrawal-status.txt` and
`withdrawal-source.sha256`. The production tree, lockfile and build recipes
match `3928a11`; only this evidence document and `OPTIMIZATION.md` differ from
that revision. Native CPU-oracle and wasm checks were not rerun after the
exact-source restoration; the earlier candidate results remain historical.

`native-driver.sh` and its analyzer were prepared but deliberately not run
after the CPU gate failed. Therefore no rendered equality, native frame-rate
gain or release acceptance is asserted for this candidate. The saved viewer
binaries remain diagnostic artifacts, not accepted release artifacts.
Earlier visual discrepancies, host warnings, 60 Hz, memory/lifecycle,
downstream integration and platform checks remain open. Further optimization
requires a distinct measured hypothesis rather than lowering these targets
or repeating this unchanged experiment.
