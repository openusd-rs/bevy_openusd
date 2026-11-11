# Remaining preparation CPU costs

## Scope and execution

At `190f4b8`, the saved `26bbf91` release editor benchmark still matches the
current `crates/usd_bevy` sources and lockfile exactly. The intervening viewer
logging change does not install a subscriber in that benchmark. Reuse that
executable without rebuilding or changing production:

- Evidence: `/tmp/bevy-optimization-preparation-cpu.8OvN5a/`.
- Executable: `/tmp/bevy-optimization-shared-queries.cQLdQZ/after-editor`.
- SHA-256: `e18898e1dc35afd923f6edcd1934ede521fdffbc15f1f3733f459437d8673547`.
- Unchanged lockfile: `818d95e2b264ce2bbda3fe8be369204f63b46157e9b0a861ff1c3d3cf35019db`.

`driver.sh` records bird and cow with `perf record -e cpu-clock:u -F 199`.
`stacks.sh` then records cow and bird with `-F 49 --call-graph dwarf,8192
--user-callchains`. All scripts and saved executables run through
`nix develop --command oslo make run-headless-benchmark`. Each fresh process
uses the existing GPU-prepared headless timeline, default eager residency,
default affinity, four warmups and 1,000 unique authored-range clocks. Route,
image and raw-sample profiling are enabled. No task-owned build or native
capture overlaps; unrelated work is unchanged. Starting one-minute host load
ranges from 7.94 to 9.50.

The original stack-recording attempt fails before launching the benchmark:
`cpu-clock:u` cannot open with error 13. A minimal permission probe establishes
that explicit user-only callchains work; the revised command does not change
kernel settings, privileges or security policy. The failed driver, zero-byte
recording and diagnostics remain separate. Thus four application runs pass,
not every attempted command.

## Results

`analyze.mjs` verifies 4,000 measured updates and 16 warmups, their exact clock
sequence, nearest-rank summaries, matching retained payload/asset counts between
the two profiles, and all raw sample-period totals against `perf report`.
The four successful recordings have zero reported lost samples and zero
matched application diagnostics. They contain 5,147 / 1,656 bird and 17,124 /
4,209 cow samples (flat / stack recording). Input hashes verify afterward.
The entry-asset manifest is not a complete dependency-closure audit.

The table groups **self** samples by symbol, including generic functions whose
symbol names contain the listed module. Samples are weighted by their recorded
period. Categories are mutually exclusive; unnamed categories remain in the
denominator. Scope includes opening, idle, warmups, seeks and shutdown across
inherited application threads. It is neither seek-only main-thread wall time
nor native/GPU frame time.

| Self-sample category | Bird flat | Bird stacks | Cow flat | Cow stacks |
| --- | ---: | ---: | ---: | ---: |
| MikkTSpace symbols | 22.32% | 20.11% | 49.98% | 50.32% |
| Allocation/free symbols | 16.24% | 16.55% | 12.00% | 11.40% |
| Memory copy symbols | 8.45% | 9.66% | 9.05% | 8.91% |
| OpenUSD crate decoder symbols | 8.86% | 8.64% | 5.38% | 5.75% |

All 160 bird and 375 cow sampled memory-copy stacks have a resolved first
non-copy caller. Bird's largest such caller is the OpenUSD `Vec3f` array
decoder, accounting for 4.23% of all stack-profile samples. Cow's largest is
the OpenUSD `[f32; 4]` decoder at 2.42%; its `Vec3f` decoder contributes another
0.95%. These are copy samples attributed to those callers, not the complete
inclusive cost of the decoder. MikkTSpace triangle-list construction, vector
growth, `ReadMesh`/vertex-attribute clones and time-sample vector construction
also appear; allocation work cannot all be assigned to skinning or tangents.

Deeper unwinds are incomplete: 633 / 1,656 bird and 1,766 / 4,209 cow samples
include unknown frames; another important limitation is the many single-frame
stacks (479 bird, 1,141 cow). Do not infer a complete call tree or attribute
every copy to a particular stage/attribute from these stacks.

Median update times are 20.875 / 25.537 ms for bird and 56.695 / 56.696 ms for
cow (flat / stacks). The binary is unchanged. These instrumented observations
are not a before/after optimization comparison; the bird timing difference
does not establish a production regression or a sampling-overhead estimate.
Raw tails, opening and RSS remain in `analysis.json` and the individual logs.

## Source findings and next action

Pinned OpenUSD `900ea31591348e3fbf416d4661d703aa8700771b` resolves a crate field
through `CrateData::try_field`, calling `CrateFile::value` for a stored crate
representation. In `usdc/reader.rs`, `Type::TimeSamples` reads the times and
value representations, then calls `nested_value` for **every** sample before
returning the normalized map. A field read is not selective decoding of only
the interpolation endpoints. The profile contains both array decoding/copying
and `Vec<(f64, Value)>` construction. This supports investigating selective
time-sample decoding at the OpenUSD boundary; it does not prove that all decoder
cost belongs to that path or that any proposed implementation is faster.

The next dependency-level investigation should establish the exact composed
read path and a compatible selective-read API before adding library-side
cross-frame caches. Preserve layer overrides/tombstones, interpolation,
malformed-data behavior, value clips, reload/edits and current upstream use.
No new dependency patch, cache or API change is made at this checkpoint.

Bevy MikkTSpace 1.0.0 is built with `default,std`. Its documented
`corrected-edge-sorting` and `corrected-vertex-welding` options can change
generated values relative to the original algorithm. They remain disabled;
enabling them is not a fidelity-preserving performance optimization. The
withdrawn larger tangent queue and rejected correction cache remain rejected.

This identifies where further work belongs; it is not a new speedup. Earlier
visual failures, native 60 Hz, loading, lifecycle, downstream and platform gates
remain open. Nothing is pushed.
