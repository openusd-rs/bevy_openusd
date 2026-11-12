# Owned sample-map transfer

Status: owned-map transfer passes core tests and the mixed/admitted/no-room
kernel gates. Production dependencies and runtime are unchanged.

`CrateData::try_field` returns an owned decoded `TimeSamples` value. The shared
opinion walk in the original prototype borrows its map for `SourceResolver`,
which clones it into the query source. The candidate preserves ownership
through `SampleField` and `OpinionResolver`, letting the source take the
decoded map directly. Borrowed layer maps still need a clone. Accounting uses
the retained capacities.
This changes data ownership, not sample selection or invalidation.

The predeclared gate compares the `7d91508d` prototype with the candidate under
a 64 KiB/eight-entry budget on the same three bird/six cow channels. Only small
scale sources may be admitted (one bird/two cow). Larger rotation and
translation maps must be refused on every batch. The requirements are exact
answers, bounded charges, zero charges after query drop, and at most 5% query
overhead versus direct reads in both first-read strata and both fresh-process
orders. Each process uses 1,000 clocks plus four warmups. This is the `mixed`
protocol of `query-budget-probe/run.sh`; all raw evidence is retained.

The changed candidate runs the same protocol and the admitted/no-room
regressions. Original failures remain recorded. Kernel results do not establish
whole-editor or rendered performance.

## Implementation and evidence

Evidence: `/tmp/bevy-optimization-query-owned.HNc2v3/`. Commit `945d516` records
the probe extension and the gate before measurement. The isolated upstream
candidate is `f4b1d0bbcf6ab78727136f37e90538194e7c7b1a`; its parent is
`7d91508d412113d3a87fe6ba6a60fe3df84cdc72`. Apply the
[owned-samples patch](patches/openusd-owned-samples.patch) after the original
budget and no-room patches. All ten changed files in a fresh three-patch replay
match the tested checkout byte-for-byte.

`SampleField::Map` and the sample hook carry `Cow<TimeSampleMap>`.
`SourceResolver` takes an owned map with `into_owned`; borrowed maps clone.
Other resolvers read the same samples, and sample-time collection keeps its
existing map-copy behavior. The patch adds no retained cache or invalidation
state. Two regressions verify outer/inner buffer identity and capacity for
owned maps, independent storage and unchanged values for borrowed maps, and
unchanged empty/blocked/unusable field classification for both ownership modes.

Saved release executable SHA-256 identities:

```text
before/probe
924af3f9c88bca5c6e2e23d170cf1a08f493e26aa7e6f2a458e1a52d14b8adb9
after/probe, admit-after/probe and refusal-after/probe
8639d5ebc7ec091569f62f8692802633775467ada135d08339a03fae96551ad9
```

## Mixed-budget result

The four original processes validate exact answers and budget bounds but fail
all eight median strata. The four candidate processes pass the same 5% gate:

| Asset | Original overhead | Candidate overhead | Candidate direct / query median ranges |
| --- | ---: | ---: | ---: |
| Bird | +89.58–152.16% | +1.20–2.19% | 294.223–295.714 / 299.259–300.659 µs |
| Cow | +81.83–84.07% | +3.07–4.39% | 659.217–662.710 / 683.041–688.177 µs |

Together these processes validate 8,000 updates, 32 warmups and 36,144 exact
attribute-answer pairs. The independent comparison requires equal paths,
clocks, first-read order, inventories, answer fingerprints and entry counts.
Only timings and retained capacity charges may differ in those outputs. All
recorded non-candidate inputs match. Each process exits zero with empty
diagnostics; the original driver/analyzer still exit 1 on performance.

Only the small scale sources are retained: one bird source charges 1,548 bytes
in both implementations. Two cow sources charge 2,048 bytes before and 3,056
after. Rotations/translations remain unadmitted under 64 KiB. Every observed
batch is within budget, and all charges return to zero after query drop.

## Regression matrices and capacity trade-off

Four admitted-budget processes pass all eight unchanged 25% reduction gates.
Twelve no-room processes pass all 24 unchanged 5% overhead gates, with zero
retained sources throughout:

| Protocol | Bird query/direct median change | Cow query/direct median change |
| --- | ---: | ---: |
| 8 MiB/eight entries | -98.56% to -98.53% | -99.64% to -99.58% |
| Zero bytes, one byte, zero entries | -0.30% to +0.51% | -0.94% to -0.06% |

The admitted bird retains three sources charged at 1,658,967 bytes; cow retains
six at 2,916,090 bytes. Those are 82,992 / 144,816 bytes above the original
cloned-source charges. Taking the decoded vectors preserves their spare
capacity, which the existing accounting charges in full. Both remain below
8 MiB, retain the same entries and release all charges on drop. Avoiding copies
is not a claim of lower retained storage or lower whole-process RSS.

Across the original mixed matrix and all three candidate matrices, 24
processes validate 24,000 measured updates, 96 warmups and 108,432 exact
attribute-answer pairs. Raw warmups, per-stratum p95/max, opening costs,
process time/RSS and host snapshots remain in the evidence directory. The
source/binary and 268-entry asset manifests verify after measurement.

## Checks and limits

All 2,494 upstream core library tests with all features pass on Rust 1.96.1
and 1.98.1. These are repeated executions of the same suite, not 4,988 distinct
cases. Core strict all-target/all-feature Clippy and rustdoc pass on 1.96.1;
the unchanged upstream 1.98 lint failures recorded earlier remain unresolved.
Probe formatting, four unit tests, strict probe Clippy and release build pass
on 1.98.1.

The initial test build failed because a test used an owned enum accessor where
a reference accessor was needed. The first full core run then passed 2,480
tests and failed 14 because the isolated checkout lacked vendor fixtures.
The exact gitlink pin `c3b0e81377b08e54658cb714ee54fb0fde63a03f` was cloned
locally into that checkout from the existing fixture repository; the original
was not modified. The complete suite then passed. Both failed logs remain.

Every matrix's budget and clock negative controls reject invalid copies.
Performance controls with doubled query times reject eight mixed, eight
admitted and 24 no-room strata. `compare.mjs`, `verify-negative.mjs` and
`verify-source.sh` retain these checks and the fresh source replay, with logs
and terminal exits. Production manifest/lock hashes are unchanged.

No owned build or capture overlaps timing. Host load is variable: original
mixed runs start at 9.97–10.78, candidate mixed at 5.36–5.70, admitted at
5.07–5.14 and no-room at 4.62–4.88. Affinity and unrelated work are untouched.
The reported gates compare query/direct reads within each process; absolute
cross-build timing and whole-editor performance are not established.

Oversized borrowed maps still clone, and decoding peaks are outside the
retained-storage budget. Budgeted-query error/reentrant settlement, clips,
bounded query ownership, schema integration and real edit/close/reload
acceptance remain. The schema's current `AnimQuery` still performs direct
attribute reads; this prototype does not yet accelerate production playback.
