# Budgeted upstream query prototype

Status: isolated implementation, focused correctness checks and the used-channel
attribute-kernel gate pass. The subsequent [no-room shortcut](query-refusal-cost.md)
also passes the declared zero-byte/one-byte/zero-entry refusal gate. The
production dependency is unchanged. No full application speedup is measured or
accepted, and nothing is submitted upstream.

The [admission investigation](sample-query-reuse.md#admission-and-lifetime-constraints)
rules out using a one-time payload inventory or public index build revision as
a persistent cache's memory guard. This prototype enforces admission inside
the upstream query, where its source and private value/population stamps are
available together.

## Implementation

The [replayable patch](patches/openusd-query-budget.patch) applies to upstream
`900ea31591348e3fbf416d4661d703aa8700771b`. The isolated checkout's local commit
is `49bd693b6530a910be069ac670b98b5cee62c30b`. It is not a production Cargo patch
or dependency revision.

- `QueryCacheBudget` shares immutable byte/entry limits and current charges.
  Checked arithmetic refuses overflow, zero limits and exhausted budgets.
- `AttributeQuery::with_budget` admits sources under the query's existing
  private invalidation stamp. A stale memo is dropped before resolving its
  replacement. A refused source supplies the current answer and is then dropped.
- The source owns an RAII charge. Cloning a query shares its budget but starts
  with an empty memo; dropping or replacing a source releases its charge.
- Accounting covers the fixed source record, sample-map capacity, numeric array
  and string capacities, source-site buffers and shared path text. Unknown value
  kinds decline caching. Composition, interpolation and typed decoding stay in
  upstream code.

The byte limit bounds **charged retained source storage**, not process memory.
Shared path text is charged in full for each source. Allocator/reference-count
headers, the budget object, stage storage, query handles, temporary decoding and
returned values are outside it. A source can be decoded/cloned before admission
is refused. The subsequent no-room shortcut avoids source construction when
even the fixed cache record cannot fit; larger-source refusal and decoding
peaks remain open. A production owner must separately bound query handles/keys
and release them on close, replacement and eviction.

## Verification

Evidence: `/tmp/bevy-optimization-query-budget.uW9rMA/`.

The final candidate passes ten new budget tests and all 163 upstream attribute
unit tests on both Rust 1.96.1 and 1.98.1. These are 173 distinct tests repeated
on two toolchains, not 346 independent cases. The new cases cover:

- Exact byte boundaries, entry exhaustion, zero limits, overflow and release.
- A cached 24-byte numeric sample payload growing to 98,304 bytes, rejection
  after the edit, later re-admission after shrinking, and value blocking.
- The same growth through an instance proxy with no visible-path index revision.
- Query clones sharing one entry limit and independent stages sharing one budget.
- Stage lifetime observed through a weak sentinel: the query keeps its stage
  alive, but the remaining budget handle does not retain it after query drop.
- Retimed reads under linear/held interpolation and default/forward/reverse/
  repeated/out-of-range clocks, compared with upstream direct reads.
- Unsupported value fallback and spare capacities in numeric arrays, strings,
  string arrays and the sample-map allocation.

Formatting, strict all-target/all-feature core-crate Clippy and core rustdoc pass
with upstream's declared 1.96 toolchain family. On 1.98, Clippy rejects two
unchanged `chunks_exact` calls in upstream `tf/hash.rs`, plus reports a removed
lint. A separately cloned, tracked-clean original reproduces those failures.
They remain recorded; the toolchain change does not turn 1.98 lint into a pass.
The first attempt to select 1.96 used a compiler component without its standard
library; the complete Nix toolchain fixes that setup. A new test's invalid
`Vec3f::ZERO` constant was also corrected. All failed logs remain available.

The patch applies cleanly to a fresh archive of the pinned source. Every changed
file in that replay compares byte-for-byte with the tested candidate. Production
manifest/lock hashes remain unchanged. No Cargo-cache checkout, existing OpenUSD
worktree, renderer source or user desktop is modified.

The patch includes an isolated `Makefile`: `make fmt test-budget test-attributes
lint doc` runs the focused checks under the selected toolchain. Logs retain the
compiler identity, command output and terminal exits. Full upstream/workspace
tests, browser/native rendering and application lifecycle acceptance have not
been run for this prototype.

## Used-channel gate

Compare the budgeted API with direct reads on the actually requested three
bird/six cow component channels from the saved operation trace. Use an
8 MiB/eight-entry shared source budget per stage, 1,000 unique clocks plus four
warmups, balanced first-read order and reversed fresh-process order. Require
exact option/type/numeric-bit answers throughout, no exceeded byte/entry limit,
and at least 25% lower query batch median in both order strata for each asset.
Preserve raw samples, identities, manifests and all failures. This new
used-channel comparison supplements the earlier all-authored probe; its cow
512-attribute failure remains a failure.

## Used-channel result

Evidence: `/tmp/bevy-optimization-query-budget-used.nkmq12/`. The standalone
probe, channel lists, analyzer and runner are in local commit `4d6b375`. Its
release executable is `probe`, SHA-256
`3af2c6c0fdfb347cc6d16f1c98e79e6cf3f2aa6e963115f3b18baa4846930e33`,
built with Rust 1.98.1. The isolated manifest uses the library's real `UsdSource`
and existing Bevy patches; only its three OpenUSD packages receive command-line
path overrides to the recorded candidate. Production manifests/lock are intact.

All four processes exit zero with empty application diagnostic streams. They
validate 4,000 measured clocks plus 16 warmups and 18,072 exact attribute-answer
pairs, including option presence, value kind and numeric bits. The independently
parsed output validates selected paths against the saved projection trace,
sample/default inventories, clocks, ABBA order, fingerprints across fresh
processes, source charges and final release. All eight declared median strata
pass the unchanged 25% gate:

| Run | First batch | Direct median µs | Budgeted median µs | Change |
| --- | --- | ---: | ---: | ---: |
| Bird 0 | Direct | 116.035 | 3.002 | -97.41% |
| Bird 0 | Budgeted | 116.476 | 3.019 | -97.41% |
| Bird 1 | Direct | 115.673 | 3.081 | -97.34% |
| Bird 1 | Budgeted | 116.529 | 3.064 | -97.37% |
| Cow 0 | Direct | 313.887 | 2.356 | -99.25% |
| Cow 0 | Budgeted | 313.677 | 2.110 | -99.33% |
| Cow 1 | Direct | 315.630 | 2.512 | -99.20% |
| Cow 1 | Budgeted | 315.157 | 2.232 | -99.29% |

The absolute saving is roughly 0.113 ms per bird batch and 0.312 ms per cow
batch in this kernel. Geometry, authored-value checks, rendering and editor
updates are outside the timed region. This is not a whole-app percentage or
native frame-rate measurement. Initial query fills take 0.505–0.579 ms for bird
and 1.028–1.179 ms for cow; all warmups, p95/max and raw samples are retained.

| Asset | Numeric sample payload | Charged source bytes | Cached sources |
| --- | ---: | ---: | ---: |
| Bird | 1,484,000 | 1,575,975 | 3 |
| Cow | 2,223,228 | 2,771,274 | 6 |

Charges stay constant within each run, below 8 MiB/eight entries, and return to
zero when queries are dropped. The budget exclusions above still apply. Process
peak RSS is 49,936 / 50,064 KiB for bird and 34,364 / 34,296 KiB for cow; each
process includes both read policies, its stage and inventory, so these are not
before/after memory comparisons.

The probe's three unit tests, formatting and strict probe Clippy pass. Its
initial compile failure used the wrong error type while collecting attribute
handles; the corrected build and its original failed log are preserved. Three
negative controls correctly reject an oversized charge, a changed clock and
eight deliberately slower query strata. The saved binary/source and all 268
asset-manifest entries verify before and after measurement. Host snapshots show
unrelated Pasq build/test work, left untouched. No owned builds or captures
overlap the measurements.

Build/test through `oslo make test-query-budget-probe --upstream PATH` and
`oslo make build-query-budget-probe --upstream PATH` inside `nix develop`.
The recorded runner fixes the candidate/trace locations and saves a binary
before executing it through `run-headless-benchmark`; inspect those locations
before reproducing it. Its commands do not modify the production dependency.

## Remaining integration checks

Before production adoption, also validate refused-read cost, error/reentrant
settlement paths, clips, bounded query-owner lifetime, schema integration and
real edit/reload workloads. Then run the full-editor performance and rendered
regression gates. Attribute-kernel success cannot establish 60 Hz, whole-process
memory bounds or completion of the broader optimization roadmap.

The declared refusal gate uses zero-byte, one-byte and zero-entry budgets on
these same real channels: exact answers, zero retained source charges and no
more than 5% median overhead in either first-read stratum, in both fresh orders
for each asset. The [refusal result](query-refusal-cost.md) retains the original
24 failing strata and the fixed candidate's passing matrix. It also records an
8 MiB regression on the changed candidate. Oversized-source refusal when the
fixed record fits, semantic/lifecycle integration and broader acceptance remain.
