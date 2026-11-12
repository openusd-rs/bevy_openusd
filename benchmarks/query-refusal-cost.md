# Budgeted query refusal cost

The zero-byte, one-byte and zero-entry refusal gates pass with the no-room
shortcut. The 8 MiB admitted-read regression gate also passes. This is an
isolated upstream prototype; production dependencies and runtime are unchanged.

Evidence: `/tmp/bevy-optimization-query-refusal.3MSgW5/`. The probe and refusal
runner are recorded in local commit `3a1fb1d`. The upstream implementations are:

- Before: `49bd693b6530a910be069ac670b98b5cee62c30b`, the original
  [budgeted-query prototype](bounded-query-prototype.md).
- After: `7d91508d412113d3a87fe6ba6a60fe3df84cdc72`. Apply
  [the original patch](patches/openusd-query-budget.patch), then
  [the no-room patch](patches/openusd-query-no-room.patch), to upstream
  `900ea31591348e3fbf416d4661d703aa8700771b`.

The saved release executables have these SHA-256 identities:

```text
before/probe
5e3699c2c3d9df90a907e79b8a77b8907c27a1c3015794354018afceea38853f
after/probe and admit-after/probe
4ec18bc0d58e8a1d2c444e75b52a44cbf11801cfd206670408c80b01ab54157f
```

## Protocol and result

The refusal gate was declared before measurement in `0216612`: no more than
5% median overhead versus direct upstream reads in either first-read stratum,
in both fresh-process orders, for each asset and refusal policy. Each process
uses 1,000 unique clocks, four warmups and balanced ABBA first-read order on
the traced three bird/six cow channels. Refusal must retain zero source bytes
and entries at every observed batch and return exactly the direct answer.

All 12 processes in each implementation exit zero with empty diagnostic
streams. Across both implementations, 24,000 measured updates and 96 warmups
validate 108,432 exact attribute-answer pairs. The independent cross-check
matches all non-timing output, including clocks, order, fingerprints, inventory
and charges, plus all recorded inputs except candidate source and binary identity.

| Asset | Original overhead range | No-room overhead range | Original / fixed failed strata |
| --- | ---: | ---: | ---: |
| Bird | +88.33% to +148.16% | -1.40% to +2.30% | 12 / 0 |
| Cow | +83.33% to +88.15% | -0.04% to +3.51% | 12 / 0 |

The original matrix remains a failed gate: `before-run.exit` and
`before/analysis.exit` are both 1. The fixed driver, analyzer and cross-check
exit zero. Each range covers all three policies and both orders, with 500
samples per first-read stratum. Raw warmups, p95/max and individual strata are
preserved in the phase directories and their `analysis.json` files.

The original implementation resolves and clones a source before refusing its
charge. The shortcut checks whether the fixed cache record and one entry can
fit before source construction. When they cannot, it uses upstream's ordinary
value resolution. Settlement and valid-memo reuse precede this check; a stale
memo releases its charge before admission is tested. Thus an already admitted
source remains reusable when the shared budget is full.

This shortcut does not cover every refusal. If the fixed record fits but its
decoded sample arrays exceed the available bytes, source construction still
occurs before rejection. Temporary decoding storage is outside the retained
source budget. Oversized-source cost and its allocation peaks remain open.

## Admitted-read regression

Four fresh processes on the changed candidate repeat the declared 8 MiB/
eight-entry comparison. They validate 4,000 measured updates, 16 warmups and
18,072 exact attribute-answer pairs. All eight strata pass the unchanged
25% reduction requirement:

| Asset | Direct median range | Query median range | Median reduction range |
| --- | ---: | ---: | ---: |
| Bird | 118.373–163.867 µs | 3.061–4.904 µs | 97.01–97.42% |
| Cow | 310.349–315.653 µs | 2.162–2.525 µs | 99.20–99.31% |

Bird retains three sources charged at 1,575,975 bytes; cow retains six charged
at 2,771,274 bytes. Every sample is within budget, and query drop returns both
counters to zero. This is a changed-code regression check, not a replacement
for the original admitted-read results or earlier failed experiments.

## Verification and measurement limits

- Eleven upstream budget tests and 163 existing attribute tests pass on both
  Rust 1.96.1 and 1.98.1. The additional case checks ordinary answers through
  three successive array-growth edits under the three no-room policies.
- Upstream formatting, strict core all-target/all-feature Clippy and core
  rustdoc pass on 1.96.1. The previously reproduced, unchanged upstream 1.98
  Clippy failures remain unresolved.
- Four probe tests, probe formatting, strict probe Clippy and the release
  build pass on 1.98.1. The probe uses the real library `UsdSource`; only its
  standalone manifest receives the recorded OpenUSD path overrides.
- Budget and clock negative controls reject deliberately invalid copies of
  each matrix. Separate performance controls double query times and correctly
  reject all 24 refusal strata and all eight admitted strata.
- Both patches replay onto a fresh archive. All nine changed files compare
  byte-for-byte with the tested upstream checkout. Saved binary/source and all
  268 asset-manifest entries verify after measurement. Production manifests
  and lockfile hashes remain unchanged.

The fixed refusal runs overlap unrelated Pasq and Gearbox builds. Those jobs
were left alone; no task-owned build or capture overlaps. Starting one-minute
host load ranges from 23.90 to 36.56. Direct-read medians vary substantially:
original/fixed bird ranges are 224–336 / 299–655 µs, and cow ranges are
631–660 / 676–1,679 µs. The table reports paired query/direct comparisons within
each process, not an absolute cross-build elapsed speedup. No affinity or
host configuration is changed.

Fixed refusal peak RSS ranges are 47,840–49,080 KiB for bird and
31,460–32,444 KiB for cow. Admitted runs use 49,860–49,940 / 34,112–34,428 KiB.
Each process includes both read modes, the stage and inventory. These figures
are neither isolated cache-memory measurements nor a process-memory bound.

Reproduce with the recorded source checkout and fresh output directories:

```sh
nix develop --command oslo make test-query-budget-probe --upstream "$UPSTREAM"
nix develop --command oslo make build-query-budget-probe --upstream "$UPSTREAM"
nix develop --command /run/current-system/sw/bin/bash \
  benchmarks/query-budget-probe/run-refusal.sh "$REFUSAL_OUT" "$UPSTREAM_HEAD"
nix develop --command /run/current-system/sw/bin/bash \
  benchmarks/query-budget-probe/run.sh "$ADMIT_OUT" "$UPSTREAM_HEAD"
```

The scripts fix the trace and upstream checkout locations; inspect those paths
before reuse. `cross-check.mjs`, `verify-results.mjs` and `verify-source.sh` in
the evidence directory record the additional comparison, negative controls and
patch replay. Their logs and terminal exits are retained.

Before production adoption, validate oversized-source refusal, error/reentrant
settlement and clips, then bounded query ownership, schema integration and
real edit/close/reload behavior. Full-editor timing, rendering and native 60 Hz
acceptance remain open, as do the broader loading, memory and platform tasks.
These attribute-read results do not establish whole-application performance.
