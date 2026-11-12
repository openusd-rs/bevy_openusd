# Budgeted animation queries

Status: the opt-in schema API, focused semantic checks and all three declared
schema-API performance gates pass in an isolated upstream candidate.
Production dependencies and runtime remain unchanged.

`AnimQuery::with_budget` retains four attribute-query handles for translations,
rotations, scales and blend-shape weights. Source admission uses the shared
`QueryCacheBudget`; clones start with empty memos under the same budget.
`AnimQuery::new` keeps direct reads. The optional query handles are boxed, so
the direct mode does not embed four unused query objects.

Both modes share typed array conversion and short-circuit reads when a required
component is missing or invalid. Joint and blend-shape orderings remain
construction-time snapshots. A persistent application owner must rebuild
queries on ordering edits, bound handles/keys and ordering snapshots separately
from source storage, and release queries on close/replacement. This API alone
is not that owner.

## Declared schema-API gate

Compare direct and budgeted `AnimQuery::joint_local_transforms` on the one
bird/two cow animations identified by the saved channel trace. Use 1,000 unique
clocks and four warmups per process, balanced first-read order and reversed
fresh-process order. Require exact optional-result presence, joint counts and
all matrix bits; match fingerprints across fresh processes and admission
policies. Preserve raw samples, source/asset identities, host load and failures.

- An 8 MiB/eight-entry budget must retain the traced three/six component
  sources, remain within both limits, release all charges on drop, and lower
  each first-read stratum's median by at least 25%.
- Zero-byte, one-byte and zero-entry budgets must retain no sources and have
  no more than 5% median overhead in any first-read/order/asset stratum.
- A 64 KiB/eight-entry budget must retain only the one/two small scale sources
  and meet the same 5% overhead limit while larger maps are refused.

This includes typed conversion and local-matrix construction. It excludes
skeleton-order remapping, skinning, mesh preparation, scene publication and
rendering. Passing it cannot establish a whole-editor gain or native 60 Hz.
The changed direct path also needs a comparison with the original executable
before production adoption; within-candidate policy comparisons do not prove
that the default constructor is regression-free.

## Implementation and checks

The isolated upstream commit is `66499022` (full identity in the evidence
directory's `upstream-head`). The
[animation-query patch](patches/openusd-anim-query-budget.patch) applies after
the budget, no-room and owned-sample patches to upstream `900ea315`. A fresh
four-patch replay matches all twelve changed files byte-for-byte.

Evidence: `/tmp/bevy-optimization-anim-budget.jEfUWU/`. Root commit `eab6e75`
retains the API patch; `656c7c6` retains the schema benchmark and its gate before
measurement. The standalone manifest adds a direct schema dependency at the
same pin; its lockfile changes only that package's dependency list. Production
manifest/lock hashes are unchanged.

Eight new unit tests cover:

- Linear/held reads, defaults, forward/reverse/repeated/out-of-range clocks,
  time-sample introspection and byte/entry/zero limits.
- Cold query clones sharing a budget, release and later admission.
- Sample growth beyond the budget, shrinking and re-admission, value blocks,
  and the existing construction-time ordering snapshots after an ordering edit.
- Precision conversion, missing and wrong-shaped arrays, and blocked weights.
- Reentrant reads from a stage sink during a layer edit.
- Independent stages at equal source labels and release observed through a
  weak stage-owned sentinel.
- Value-clip switching under linear/held interpolation, including weights.
- Short-circuiting after an invalid component and propagation of a later
  component-read error.

The 404 schema/geometry library tests selected by the `skel` feature and all
75 Skel integration tests pass on Rust 1.96.1 and 1.98.1. These are 479 distinct
tests repeated, not 958 independent cases. Strict schema all-target/all-feature
Clippy and schema rustdoc pass on 1.96.1. The consumer's two matrix-probe and
four attribute-probe tests, formatting, strict probe Clippy and release build
pass on 1.98.1. Earlier upstream 1.98 core-lint failures remain unresolved.

An initial clip assertion expected a held value while linear interpolation was
active. The direct and budgeted answers already matched; the explicit expected
value at stage time 1 between values 2 and 8 at times 0 and 5 is 3.2. The test
now checks that linear result and the held result 2, plus reverse clip reads.
The failed run (`schema-final-checks.exit = 2`) is retained; the final fixture
and strict checks pass in `schema-clip-checks.log`.

## Real-asset schema result

All 20 fresh processes exit zero with empty application diagnostics. They
validate 20,000 measured updates, 80 warmups, 30,120 exact animation answers
and 2,991,920 joint-matrix pairs. Bird returns 200 matrices per update; cow
returns 49 for each of two animations. Every matrix coefficient is compared
bit-for-bit in the executable. The independent parser checks the traced paths,
joint orders/counts, clocks, first-read schedule, source charges and final
release. Cross-protocol comparison matches answer fingerprints and recorded
inputs across every admission policy and fresh order.

| Protocol | Bird median change | Cow median change | Failed strata |
| --- | ---: | ---: | ---: |
| 8 MiB/eight entries | -96.80% to -96.76% | -99.28% to -99.22% | 0 / 8 |
| 64 KiB/eight entries | +0.22% to +0.90% | -0.44% to +0.18% | 0 / 8 |
| Zero bytes, one byte, zero entries | -0.70% to +0.09% | -0.96% to +0.36% | 0 / 24 |

Admitted bird batches take 8.110–8.224 µs versus direct 250.625–256.264 µs;
cow batches take 4.693–5.080 µs versus 647.088–658.742 µs. These are paired
schema-kernel measurements, not whole-application percentages. Admitted source
storage is 1,658,967 bytes/three entries for bird and 2,916,090 bytes/six entries
for cow. Mixed runs retain only scales at 1,548 bytes/one entry and
3,056 bytes/two entries. No-room runs retain zero; query drop releases all
charges in every process. Query handles, ordering snapshots, stage storage,
temporary decoding and returned matrices are outside these source charges.

Every protocol's four negative controls reject an excessive charge, a changed
clock, a changed joint count and deliberately doubled query times. All 40
deliberately slower strata fail. All source/binary and 268-entry asset
manifests verify after measurement. The saved executable in each `perf-*`
directory has SHA-256:

```text
22e547040cf036d961f3403e5500edf50a0765ecf5236b15507f45a74c464017
```

Warmups, opening costs, raw p95/max, peak process RSS and host snapshots are
retained. Starting host load ranges from 9.29 to 12.17 across the matrices.
No owned build/capture overlaps; unrelated work and affinity are unchanged.
Each process contains both policies, so its RSS is not a separate cache-memory
measurement. Timing variability remains visible in the raw direct medians.

Reproduce with fresh output directories and the recorded upstream checkout:

```sh
nix develop --command oslo make test-query-budget-probe --upstream "$UPSTREAM"
nix develop --command oslo make build-query-budget-probe --upstream "$UPSTREAM"
nix develop --command /run/current-system/sw/bin/bash \
  benchmarks/query-budget-probe/run-anim.sh "$OUT" "$UPSTREAM_HEAD" admit
```

Use `mixed` or `refusal` for the other matrices. Inspect the runner's fixed
upstream/trace paths before reuse. `compare-protocols.mjs` and
`verify-source.sh` in the evidence directory perform the independent policy
comparison and source replay; their logs and terminal exits are retained.

Next implement bounded, stage-owned query reuse and ordering invalidation,
then measure real editor work and rendered behavior. Oversized borrowed maps
still clone. Application close/reload, original-default regression, native
frame pacing and the broader optimization workstreams remain open.

The subsequent [stage-owned integration](stage-animation-queries.md) bounds
query metadata and connects reuse to built-in projection in an isolated viewer
checkout. Its correctness checks pass; end-to-end performance is unmeasured.
