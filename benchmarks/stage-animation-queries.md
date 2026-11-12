# Stage-owned animation queries

Status: a separate viewer checkout connects bounded animation-source reuse to
built-in projection. Its library feature-matrix tests, strict library Clippy
and workspace compilation pass. Production runtime and dependency pins are
unchanged. Whole-editor performance and rendered acceptance have not been
measured for this candidate.

Evidence: `/tmp/bevy-optimization-query-owner.tZmTqT/`. The viewer checkout is
its `viewer/` directory, based on root `c97e4c7`, committed as `08b2ed4`.
Upstream metadata accounting is implemented in `926aacca`, with documentation
clarified in `c517437f`. Root `adb022a` retains the
[metadata patch](patches/openusd-anim-metadata.patch); `2cabc76` retains the
[viewer integration](patches/viewer-animation-query-owner.patch).

The metadata patch follows the four patches listed in
[the schema report](budgeted-animation-query.md). Fresh upstream and viewer
replays match all twelve upstream and five viewer files byte-for-byte
(`verify-source.log`, exit zero). Production manifest/lock hashes verify. The
isolated build uses command-line path overrides for the three OpenUSD crates;
its resolved lockfile is archived as `viewer-tested.Cargo.lock`. It omits the
three git source entries and has no other changes. The viewer commit retains
the original dependency pin and lockfile.

## Ownership and accounting

`LiveStage` owns an optional `Rc<AnimationQueries>`. Built-in projection passes
lend it to the existing `SkelSampleScope`; the scope still owns per-pass poses
and query lookups. Custom registries disable the scope, including when called
inside a built-in scope. Unwinding restores the preceding scope.

The owner defaults to 128 animation queries, 1 MiB of retained query metadata,
and a shared source budget of 8 MiB/256 attribute sources. Applications can
replace `AnimationQueryLimits` in the world. A stage-identity or settings change
replaces the owner. A live-stage revision change clears its retained queries.

Slot records occupy a fixed boxed slice. Admission counts that slice plus each
query's record, attribute handles, full shared path allocations, joint-vector
capacity and ordering-string capacities. `AnimQuery::metadata_bytes` exposes
these capacities; `sdf::Path::heap_bytes` exposes its shared text allocation.
Checked arithmetic and a fallible slot allocation precede admission. The source
budget independently counts retained decoded values. Fixed owner bookkeeping,
allocator/reference-count headers, the stage, transient reads and query handles
held outside the owner are not charged to the metadata limit. Caller-held
queries continue sharing the source limit until they are dropped. Inline source
records can be counted by both limits; their sum is conservative, not exact RSS.

Full slots, oversized metadata, disabled sources or allocation refusal return a
direct query. Queries are admitted on demand; there is no scan of unused clips
and no replacement policy for otherwise valid entries when the owner is full.

## Freshness and reentry

Each first lookup in a projection pass constructs fresh ordering metadata and
compares both joint and blend-shape orders before returning a retained query.
Missing or differently typed animations remove their entry. Changed orderings
replace it, with admission checked again. Ordinary component/weight reads use
the upstream attribute queries' private source stamps.

This preserves fresh ordering reads across direct layer edits, load rules,
muting and reload, without depending solely on public index revisions or the
delivery order of stage sinks. Ordering construction is still performed on a
cache hit; a miss reads it again when creating the budgeted query. Query reuse
uses `Rc` clones, not `AnimQuery` clones, which start with cold source memos.

No owner or scope `RefCell` borrow spans USD evaluation. A construction that
reenters lookup cannot insert duplicate records for the same path. Closing a
live stage releases its owner; the budget alone does not retain the stage.

## Verification

Two new upstream tests exercise spare ordering capacities and separate source
charges. On Rust 1.96.1, 406 schema/geometry library tests and 75 Skel integration
tests pass. Strict core/schema all-target/all-feature Clippy and both rustdocs
pass. Existing upstream core Clippy failures on Rust 1.98 remain a separate
unresolved issue.

Nine new consumer tests cover:

- Reuse across clocks, component edits, joint and blend-shape ordering edits,
  revision clearing and source-charge release.
- Disabled/undersized limits, exact-fit versus one-byte-short admission, and
  an impossible slot allocation.
- Entry saturation, ordering growth beyond the limit, shrink/re-admission and
  a changed animation type.
- Payload unload/load, layer mute/unmute and file reload.
- Reentrant reads from a stage sink after an ordering edit.
- Independent stages at equal source labels and stage release after owner drop.
- Actual built-in projection scopes, edits, close, settings replacement,
  nested read scopes and nested custom-registry bypass.

The first consumer compile found a missing `String`-to-`Token` conversion in a
test. The second found the reload fixture passing `PathBuf` to the upstream
string-only `StageBuilder::open`. Both failed logs are retained. The corrected
all-feature library run passes 760 tests with 34 existing ignored cases, and
strict all-target/all-feature library Clippy passes (`viewer-reload-checks`).
These checks do not execute the ignored native/performance probes.

The final `viewer-feature-checks` run also passes the no-default-feature library
suite (752 passed, 25 ignored), strict all-target/all-feature library Clippy and
workspace/all-target compilation, including the editor. Both configurations
include all nine new tests; their counts overlap. This is Rust 1.98.1 native
compilation, including the web crate as a host target, not a wasm/browser run.
The final upstream schema checks also pass after the documentation correction
(`upstream-final-checks`). All recorded driver exits are terminal.

To check a fresh viewer replay against the patched upstream checkout:

```sh
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  nix develop --command oslo make test-animation-query-owner \
  --upstream /absolute/path/to/patched/openusd
```

The recipe formats workspace code, runs both library configurations, lints the
library and checks workspace targets. It resolves a local lockfile through the
explicit path overrides; preserve that lockfile with the test evidence.

## Remaining acceptance

Measure the integrated editor against the unmodified production dependency,
not just against the other policy in the patched upstream executable. Preserve
default eager mesh residency, authored quality, payload/work equivalence,
unique/forward/reverse clocks, unrelated host load and all failed evidence.
Include actual cache-use/resource diagnostics, original-default regression,
borrowed oversized-source refusal, representative edits/reload/close, native
rendered output, frame pacing and browser checks. No whole-application speedup,
60 Hz result or completion of the broader optimization workstreams is claimed.
