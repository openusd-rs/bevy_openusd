# Budgeted upstream query prototype

Status: isolated implementation and focused correctness checks pass. The
production dependency is unchanged. No application speedup is measured or
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
is refused; this prototype does not bound decoding peaks or yet avoid that
extra work on a rejected read. A production owner must separately bound query
handles/keys and release them on close, replacement and eviction.

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

## Next integration gate

First compare the budgeted API with direct reads on the actually requested
three bird/six cow component channels from the saved operation trace. Use an
8 MiB/eight-entry shared source budget per stage, 1,000 unique clocks plus four
warmups, balanced first-read order and reversed fresh-process order. Require
exact option/type/numeric-bit answers throughout, no exceeded byte/entry limit,
and at least 25% lower query batch median in both order strata for each asset.
Preserve raw samples, identities, manifests and all failures. This new
used-channel comparison supplements the earlier all-authored probe; its cow
512-attribute failure remains a failure.

Before production adoption, also validate refused-read cost, error/reentrant
settlement paths, clips, bounded query-owner lifetime, schema integration and
real edit/reload workloads. Then run the full-editor performance and rendered
regression gates. Attribute-kernel success cannot establish 60 Hz, whole-process
memory bounds or completion of the broader optimization roadmap.
