# Scoped animation authored-value flags

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

The release benchmark/viewer build and the saved-binary CPU/native-render
comparisons remain pending at this checkpoint. `driver.sh`, `analyze.mjs` and
`native-driver.sh` are prepared but have not run. The native baseline viewer
is the saved `190f4b8` INFO-default build, with the same capture feature and
unchanged library source before this candidate. No measured speedup, native
60 Hz, rendered equality or complete acceptance is claimed yet.
