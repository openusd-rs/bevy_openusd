# Library and viewer optimization

## Objective and boundaries

Optimize the whole `bevy_openusd` library/viewer pipeline, not only hummingbird
playback. Preserve the current upstream OpenUSD dependency, public behavior,
authored geometry/material fidelity, edit/reload/save safety, instance isolation,
resource lifetimes and supported platform paths. No reduced-quality defaults or
disabled validation presented as algorithmic speedups.

Work remains local, in focused unsigned conventional commits with title-only
messages. Do not push. Do not control the user's desktop or window manager;
interactive acceptance uses the user's input and application capture controls.
Leave unrelated applications/builds alone. Use `.make.lua` through `oslo make`.

The user accepted the hummingbird freeze repair (animation and window respond).
That does not prove smooth frame pacing. See
[the investigation](benchmarks/hummingbird-playback.md) for measured update costs
and the still-failing strict real-asset CPU/GPU image comparison.

## Workstreams and completion evidence

- [ ] **Reproducible baselines.** Record revision, asset hash, build profile,
  feature set, device, resolution, quality settings, warmup/sample counts and
  concurrent load. Separate CPU scene-update time, total/native frame cadence,
  GPU passes, first-open latency, retained payload and whole-process RSS. Retain
  raw samples, not only averages. Historical Bevy 0.19 measurements are routing
  aids, not current baselines.
- [ ] **Load and scene construction.** Profile source/package IO and composition,
  texture decode/transform/mips, mesh/subset construction, instance projection,
  readiness and editor publication. Optimize the dominant measured work without
  changing fidelity or hiding blocking work outside the reported interval.
- [ ] **Animation and seeks.** Profile skin/morph queries, palettes, topology,
  normals/tangents, materials and Bevy asset/entity updates. Separate invariant
  preparation from time-varying data with tested invalidation. Exercise forward,
  reverse, repeated and unique clocks and multiple independently timed instances.
- [ ] **Rendering.** Measure actual native and GPU cost, including uploads,
  shadows/transparency, deformation and instancing/culling. Preserve visual
  correctness and resolve or explicitly retain failing acceptance evidence;
  a compile-only check is not renderer acceptance.
- [ ] **Editor and reload.** Measure idle, snapshot/inspection publication,
  selection/hierarchy work, representative edits, undo/redo and dependency reload.
  Preserve dirty-layer protection, last-good recovery and unrelated state.
- [ ] **Memory and lifecycle.** Measure representative retained payload and peak
  RSS; stress unique clocks, repeated opens/closes and reloads. Bound caches,
  retain useful sharing, release obsolete assets and preserve mutation tracking.
- [ ] **Build/platform behavior.** Measure release rather than extrapolating from
  debug. Check developer-profile bottlenecks separately. Revalidate wasm and its
  browser rendering path; distinguish browser software rendering from native GPU
  results. Do not claim untested OS/device performance.
- [ ] **Final regression and performance audit.** Run repository correctness,
  native oracle/reload/export, GPU capture, browser and lifecycle gates covering
  changed behavior. Repeat before/after performance runs under comparable
  conditions, account for noise and confirm no important workload regresses.
  Record remaining limitations against this full list, not just completed edits.

Working playback target: 60 Hz for the representative small animated release
scenes on the available machine. This is a target, not a current result or a
guarantee for arbitrary USD scenes. Set measurable workload-specific budgets
from the baseline before each optimization; do not redefine them afterward to
fit the result. Opening large scenes must also retain responsive progress and
correct readiness reporting.

## Representative matrix

- Hummingbird: packaged textures, many skinned meshes, transparency, live playback.
- Cow and robot arm: locate the available complete sources and exercise independent
  skeletal/transform animation rather than tuning only for the bird.
- `flagship_showcase.usda` and deformation fixtures: CPU/native/GPU reference,
  morphs, animated normals/tangents and material values.
- `material_subsets.usda` plus available ANYmal/Spot sources: subset payload and
  source/instance lifecycle; verify source availability and hashes first.
- `PointInstancedMedCity.usdz` and compact-instancing fixtures: scale/culling.
- Available textured machine or kitchen scene: large static loading/texture path.
- Focused editing/reload fixtures: warm idle, edits, undo/redo, failure and repair.

## Current checkpoint

- [Native residency reveal verification](benchmarks/native-residency.md) exposes
  a real shared-device limit failure: the morph/skin pipeline needs nine vertex
  storage buffers but Mara's device requested eight. `00d9bd6` uses the host's
  existing limit hook, bounded by adapter capabilities; native readbacks confirm
  16 requested buffers and no remaining pipeline validation errors. Hardening
  passes 860 / 983 executions. `b487f49` exposes explicit viewer deferral for
  testing, but both library and viewer remain eager by default. Sixteen native
  command-prefix captures preserve selection, clock, entity, history, visibility,
  residency and camera; only 16 of 24 exact image comparisons pass. Five failures
  show a substantial GPU morph/shape discrepancy, and three show 219–220 pixels
  differing by at most two RGB levels. Broader isolated-host diagnostics also
  fail and remain recorded. Do not enable the policy by default or rerun this
  matrix unchanged. Next trace parent/subset deformation state and render-world
  morph descriptor/index publication across reveal and redo; low-limit adapters
  still need capability-specific fallback acceptance. Full performance, earlier
  visual discrepancies, lifecycle and platform gates remain open.

- The broader [viewer deferral policy comparison](benchmarks/viewer-residency.md)
  (`7184abe`) passes its declared CPU target: cow paired update medians fall
  approximately 68%, from roughly 57 to 18 ms, and every bird/arm/subset ordering
  stays within the 5% paired-median regression limit. Eight saved release runs
  validate 8,000 pairs / 16,000 updates with exact hierarchy-visible geometry at
  every measured clock, bounded caches/preparation and zero runtime diagnostics.
  Opening/idle/p95/max observations include regressions and remain documented.
  Hardening passes 855 / 978 executions; `cf382de` repairs a composed-source
  example's single-update readiness assumption with default/zero-budget checks.
  EditorBridge tests cover hidden edits, selection, reveal, history, reload and
  replacement. This is not native rendering/FPS or accepted viewer-default
  behavior: both library and viewer stay eager. Next verify native viewport
  reveal/selection/undo, framing and materials against eager controls. This
  policy comparison does not retroactively pass the failed incremental scheduler
  experiment below, or resolve the older visual/platform/lifecycle/60 Hz gates.

- [Resident-only preparation under opt-in deferral](benchmarks/deferred-preparation.md)
  (`d5ad5ab`) preserves eager library/viewer defaults and the original scheduler.
  Hidden nonresident meshes stay on synchronous visibility-aware routing; already
  resident meshes can prepare in parallel. Hardening passes 854 / 976 executions,
  native deformation/watchers/exports and wasm checks pass. CPU/GPU tests cover
  hidden edits, current-clock reveal, re-hide, independent roots, reload and close.
  Sixteen release processes validate 16,000 updates with unchanged payload/cache/
  asset counts, but six paired and seven whole-run checks fail; the 15% incremental
  bird target is not met. This remains an unaccepted local optimization checkpoint,
  not a completed performance gate. Four native animations (1,200 frames) match
  both eager and deferred pre-change final-pose renders pixel-exactly in all eight
  comparisons. Cow's MDL warning and 60 Hz misses remain. Any broader viewer policy
  change needs live reveal/selection/undo/reload acceptance and a paired comparison
  against current eager defaults; it has not been implemented. Do not rerun the
  same opt-in incremental comparison unchanged or retroactively pass its failures.

- The corrected [paired tangent diagnostic](benchmarks/tangent-paired.md) passes
  800 identical-input comparisons with balanced clone/execution order: cow kernel
  paired medians improve 19–23%, without bird regression. This is a private-pool,
  owned-mesh result, not whole-application performance. The follow-up
  [paired full-editor test](benchmarks/tangent-stepped.md) validates 16,000
  updates but finds only 12–13% cow improvement, below its 15% target; seven
  paired and seven whole-run timing checks fail. Production source and the normal
  benchmark executable are restored; the queue stays withdrawn. Stop repeating
  this scheduler experiment. Investigate reducing unnecessary geometry work,
  including the existing opt-in hidden-mesh path, while retaining eager library
  defaults and requiring reveal/current-clock/edit/reload and visible-fidelity
  proof before changing viewer policy. Current-source hardening passes 853 / 975
  executions and strict Clippy; no new native/platform acceptance is claimed.

- The larger tangent-queue trial (`a4fa55d`) is withdrawn: it halves cow batch
  barriers but fails its primary performance target under severe variable host
  load. Sixteen processes validate 16,000 updates, unchanged payload/cache/asset
  counts and bounded input/RSS, yet nine performance checks fail. Production
  preparation is restored byte-exactly to `181239d`; no queue speedup is claimed.
  See [the retained trial](benchmarks/tangent-queue.md). Further scheduler changes
  require isolated, tightly paired job evidence rather than another unchanged
  full-scene timing matrix. Restored-source hardening passes 852 / 971 executions
  after `f7ec2cc` repairs three test-only projection-readiness assumptions. The
  release viewer and benchmark/capture examples rebuild; no new rendered or
  performance acceptance is claimed for those rebuilt executables.

- Bounded animation primvar ownership reuse (`f3a7501`) now shares ancestor
  resolution within built-in animation passes, with fresh sampled values,
  custom-route fallback and 4,096-entry / 1 MiB key-text caps. Hardening passes
  852 / 971 executions; native deformation/watchers/exports and wasm checks
  pass. Sixteen release processes validate 16,000 distinct-time updates and
  unchanged payload/cache/asset counts. Bird mesh-read time falls 37.0% and
  its aggregate update median 21.5%, but twelve timing checks still fail;
  this is not full performance acceptance. Four candidate native animations
  (1,200 measured frames) match independent baseline final poses pixel-exactly.
  Cow MDL diagnostics remain, and bird/cow still miss 60 Hz. Tangent dispatch
  remains the dominant cow preparation cost. See
  [animation primvars](benchmarks/animation-primvars.md).

- Built-in live animation order (`8d22fec`) now sorts borrowed paths per pass,
  preserving public HashSet membership and custom callback order/snapshots.
  Hardening passes 841 no-default / 958 all-feature executions, and native
  deformation/watchers/exports plus wasm checks pass. A matched 32-process
  CPU matrix repeats candidate tangent-cache footprints for every workload;
  baseline bird/cow quick-seek footprints still vary. Geometry/image payloads,
  capacity and asset counts match. Three timing checks still fail, so neither
  a causal speedup nor 60 Hz is established. Capture harness `72a2d73` raises
  hardening to 845 / 962 executions and wasm still compiles. Twenty actual
  live-editor runs acknowledge forward/reverse/repeated seeks. Seven image pairs
  are pixel-exact; three bird pairs fail by up to two RGB levels on roughly 200
  pixels, visually aligned with the grid. Same-binary controls repeat exactly,
  so the difference is not dismissed as noise. All twenty captures also retain
  headless-environment diagnostic failures. Grid/camera-state diagnosis and the
  full performance workstreams remain open; no acceptance threshold is relaxed.
  The shared phase probe and readback observer now pass hardening at 848 / 967
  executions. Unseeked before/after controls match at clocks zero and five.
  Probed images also match, but the probe/rebuild changes which of two grid
  images is produced; these results do not replace the original failures.
  Actual extracted grid/camera/order records match in the probed case, leaving
  the failing unprobed pass/uniform state to diagnose. Private capture portal
  selection is now isolated from the user's desktop; capture-tool tests pass.
  See [live animation order](benchmarks/live-animation-order.md).

- Mip output reservation (`fdf88b7`): checked complete-chain sizing and one exact reserve
  replace geometric output-buffer growth, without changing encoded payload or
  float reduction. Sixty format/dimension/reservation cases preserve all bytes,
  levels and existing larger reservations. Hardening passes 839 no-default /
  955 all-feature executions; native deformation/watchers/exports and wasm
  compilation pass. The unrelated point/curve fixture now asserts readiness
  with an unlimited test budget (`86315b2`). Four native fixed-pose image pairs
  remain pixel-exact; the cow's missing-MDL warning still fails the combined
  diagnostic. Recorder `626198c` passes updated hardening (839 / 956 executions).
  A matched thirty-two-process CPU matrix confirms 23.8%/28.6% less bird/cow
  image-buffer capacity, with unchanged payload, both idle and after seeks;
  process RSS stays approximately unchanged. Eleven timing comparisons and six
  tangent-cache equality checks still fail, including different cache footprints
  between identical baseline runs. Live animation directly iterates a HashSet;
  the ordered-traversal checkpoint above tests that cause while preserving custom
  callback ordering and the public resource type. No prior failure is waived. See
  [mip capacity](benchmarks/mip-capacity.md).

- Exact sRGB mip encoding (`05b3106`): a bounded 5124-byte quantizer replaces
  per-channel power evaluation without changing float mip averages, source
  resolution or output bytes. Linux release comparison covers 107,011,437 float
  inputs; the encoder-only kernel takes 75.5% less time. Hardening passes 837
  no-default / 953 all-feature tests; native deformation/watchers/exports and
  wasm compilation pass. Twelve native image pairs remain pixel-exact.
  End-to-end acceptance still fails eight checks: bird texture finalization
  improves only 9.6%, both bird/cow request-to-Ready improve less than the 5%
  target, and untouched small controls show large startup regressions. Check
  byte-identical-binary controls before attributing these differences or making
  another loading change. Those controls reproduce 65%/162% pipeline-observation
  differences without any code change; own-process traces find different
  populated/empty NVIDIA cache files for the executable labels. A common-path,
  warmed control removes the consistent label penalty but remains variable.
  A matched common-path run measures 19.3%/29.8% lower bird/cow texture time,
  with sixteen exact image pairs, but still fails four original thresholds:
  bird request-to-Ready and subset Ready/pipeline/readback. An interrupted driver
  was resumed only for the missing subset block; all evidence is retained.
  These diagnostics do not clear earlier failures. See
  [mip encoding](benchmarks/mip-encoding.md).

- Native opening recorder (`9cd2119`): asset-request to all-root Ready,
  post-scene render-pipeline observation and settled image readback now have
  separate wall-clock milestones; device/bootstrap time is separate too.
  Hardening passes 835 no-default / 950 all-feature tests and strict Clippy.
  Five native before/after image pairs are pixel-exact, including independently
  timed roots. Bird request-to-Ready measures 6075 ms while the five recorded
  source-publication phases total only 740 ms, exposing a large previously
  unmeasured loader/scheduling interval. This is instrumentation evidence, not
  a speedup or clearance of opening regressions. The unlimited-projection
  offscreen path and pipeline observation do not establish editor responsiveness
  or first-visible latency. A subsequent matched sixteen-process comparison
  fails six of eight 5% opening milestone gates, including bird/cow
  request-to-Ready regressions, despite twelve pixel-exact image comparisons.
  Follow-up `20c43b2` snapshots existing texture-finalization counters at Ready:
  that synchronous `PostUpdate` work consumes 51% of bird and 72% of cow opening
  in the diagnostic runs, outside the projection budget. Three more image pairs
  are pixel-exact and updated hardening passes 951 all-feature executions. This
  identifies the next loading target; no earlier loading failure is cleared. See
  [opening milestones](benchmarks/opening-milestones.md).

- Rendered playback baseline (`9f4212f`, `1fad2a5`): a bounded recorder now uses
  production asset-root playback, collecting main intervals, separate published
  render CPU/GPU pass timings and actual stage clocks. All six bird/arm/cow
  play/idle cases complete 300 samples; all three settled final poses match
  independent fixed-pose renders exactly. The small-scene playback target is
  still unmet: play median offscreen intervals are 48.4 / 18.1 / 92.1 ms, versus
  approximately 16.7 ms at idle, under heavy variable host load. Most interval
  cost is in the main update. The cow MDL warning keeps the diagnostic driver
  nonzero; default framing and intermediate-frame/native-window validation are
  also incomplete. See [rendered playback](benchmarks/rendered-playback.md).

- Asset-root scan profiling (`f81d2d2`): opt-in timers confirm repeated animation
  classification consumes 96.6% of the arm's enclosing animation-update time,
  26.5% for bird and 24.2% for cow in the instrumented native runs. All three
  settled poses remain pixel-exact against the saved pre-instrumentation binary.
  Hardening passes 820 no-default / 928 all-feature tests and strict Clippy.
  This identifies the classification-reuse optimization below. Invalidation,
  isolation, bounds and performance acceptance are declared in
  [asset-root animation](benchmarks/instance-animation.md).

- Asset-root classification reuse (`967bd89`, `c624928`): each root now retains
  one bounded composition's positive/negative animation decisions. Flushed
  revision/layer/load/mask/mute keys invalidate it; custom routes stay uncached
  to preserve callback mutation order. Hardening passes 829 no-default / 937
  all-feature tests and strict Clippy; refreshed native deformation/watcher/
  export checks and wasm compilation pass. The 40-process native offscreen ABBA run
  reduces arm main-update medians by 88.6%, with classification calls falling
  from 45,540 to 138 and unchanged animated decisions. All 12,000 frame/clock
  rows validate, six exact-clock final poses and twelve idle images are
  pixel-exact at tolerance zero. Full acceptance remains open: cow main cost
  regresses 23.9%, material subsets 18.1%, and partial source phases also show
  regressions under highly variable host load. Two targeted ABBA repeats
  reproduce the cow and subset failures in one block each, with opposite-sign
  results in the other; neither gate is accepted. Additional phase profiling
  is needed, not a noise waiver or a claim that partial source phases cover
  complete opening.
  The bird/cow 60 Hz target, native-window/browser performance and previous
  rendering/loading failures remain open. See
  [asset-root animation](benchmarks/instance-animation.md).

- Mesh-preparation phase profiling (`274562f`): opt-in non-overlapping main
  phases and separate overlapping worker-wall counters preserve the existing
  preparation policy. Hardening passes 831 no-default / 940 all-feature tests,
  strict Clippy, release capture build and wasm compilation. Three native
  playback captures validate 900 raw frames and match independent pre-timer
  final poses exactly. Cow's dominant batch phase is tangent dispatch/join
  (31.0 ms per changed update); bird's is eligibility/mesh/skin sampling
  (11.6 ms), not tangents alone. Subsets submit no tangent jobs. These are
  diagnostics, not another speedup or clearance of the failed regressions.
  Both real assets still miss 60 Hz under the observed lower host load.
  Follow-up must reduce bird eligibility costs and actual cow tangent work
  without changing normal/tangent fidelity. See
  [parallel preparation](benchmarks/parallel-mesh-preparation.md).

- Scoped skeleton relationships (`ffb32c6`): repeated local skeleton/animation
  relationship reads now share a bounded stage-specific read-only-pass map,
  including absent targets. It retains no historical poses, skips custom
  registries and drops with the pass; entry/text caps have fresh fallback.
  Hardening passes 835 no-default / 944 all-feature tests, strict Clippy,
  native deformation/watcher/export gates, wasm compilation and release build.
  A sixteen-process native comparison records 31.5% lower bird and 15.1% lower
  cow mean main medians, but unrelated phase/idle variability prevents assigning
  the full gain to this cache. All 4,800 raw frames validate; six exact-clock
  final poses and three idle comparisons are pixel-exact. Full acceptance is
  still open: bird partial source phases regress 35.1%, prior regression gates
  remain, and playback tails/native-window/browser/lifecycle targets are unmet.
  See [scoped skeleton sampling](benchmarks/skel-relationships.md).

- Point-instancer parent links (`3562782`): isolated ECS scaling confirms
  quadratic repeated replacement of unchanged `ChildOf` links. Stable instance
  links are now retained, source-ID order restored in bulk and removed/cleared
  siblings detached before despawning. Runtime components, unrelated children,
  stable IDs and last-good validation behavior have explicit regression coverage.
  Hardening passes 819 no-default / 919 all-feature tests, strict Clippy, native
  deformation/watcher/export checks and wasm compilation. The 20-process release
  comparison lowers city seek medians from 864–1,370 ms to 51–54 ms (95.3% by the
  average-of-medians rule), preserving 40,008 mesh entities and eight mesh assets.
  City, reversed independent hierarchy roots, affine subsets and deformation
  renders are all pixel-exact before/after at tolerance zero. Full performance
  acceptance remains open: the small affine fixture regresses in two noisy
  default-affinity comparisons; a separate P-core diagnostic puts both affine
  and an unaffected control within 1%, but does not erase those failures.
  Prototype-part parenting and sustained native cadence remain separate work.
  See [instancer parenting](benchmarks/instancer-parenting.md).

- Editor hierarchy reuse (`41cb1ce`, `6d4f8ae`): broad profiling identifies
  repeated complete hierarchy/visibility reads during inspector publication.
  The editor now retains one bounded current-composition hierarchy, caches
  constant visibility separately for default/numeric time, and freshly samples
  animated visibility. Selected properties and complete owned snapshots remain
  unchanged. Eight new regressions cover clocks, clips, variants, direct edits,
  payload rules, masks, native proxies, undo/redo, failed/repaired reload and
  retention bounds. Hardening passes 817 no-default / 916 all-feature tests and
  strict Clippy; native deformation/watcher/export checks and wasm compilation
  also pass. The 28-process saved-release comparison records a 38.0% reduction
  in the average arm update medians and 89.6% in its snapshot medians. Opening
  regressions exceed the 5% gate in three noisy workloads, so full acceptance
  remains open. The completed 36-open follow-up clears the subset regression,
  but kitchen/bird aggregate opening medians still exceed the limit; first/later
  App strata show substantial variance. City seeks
  expose a separate large [point-instancer cost](benchmarks/instancer-parenting.md).
  See
  [editor hierarchy sampling](benchmarks/editor-hierarchy.md).

- Posed per-draw sorting (`fe32713`, `291b4af`, `53a34e6`): corrects the observed
  wing/grid order without changing shared mesh AABBs, grid settings or image
  tolerance. The original bird time-zero CPU/GPU comparison now passes (zero
  pixels above RGB tolerance one, previously 2,053); times five and ten retain
  one failing pixel each. Three independently timed/reversed roots are checked
  for bird, morph and cow; all CPU before/after controls are pixel-exact. The
  bird's one-pixel failure and cow's unavailable MDL warning remain recorded.
  Hardening passes 809 no-default / 908 all-feature tests, strict Clippy and
  formatting; native deformation/watcher/export gates and wasm compilation pass.
  Cache invalidation, source reload, subset/instance isolation and cleanup have
  regression coverage. Sustained native cadence and render-path rebuild cost
  are still unverified. See [posed sorting centers](benchmarks/deformed-sort-centers.md).

- Tightly paired scheduling evidence (`79e80bc`): six CPU/GPU-prepared workloads,
  two independent app pairs each and 1,000 unique clocks per pair complete with
  equal retained payload counts. Cow GPU median update cost falls 21.9–28.6%,
  bird GPU 7.2–8.3%; CPU cow/bird also improve, and small controls stay within the
  5% median-regression limit. Hardening passes 804 no-default / 902 all-feature
  tests and strict Clippy. This supports the parallel path but is not native
  60 Hz acceptance or a replacement for saved-revision comparisons. Reopened
  hash-set prim ordering can change batch partitions. See the paired section in
  [parallel preparation](benchmarks/parallel-mesh-preparation.md) for raw evidence
  and unchanged scope/limitations.

- Local performance repairs and material/image invalidation regressions are
  committed through `d5b87b6`.
- Preceding-work hardening passed: 781 no-default tests, 874 all-feature tests,
  strict Clippy, native deformation and watcher checks, and wasm compilation.
  The additional hummingbird CPU/GPU image comparison remains failing as recorded.
- `c804fe1` adds debug/release profile selection to the example build/run recipes.
  Invalid-profile rejection and both profiles were exercised. The clean release
  build completed in 17m27s rather than reusing an old Bevy artifact.
- `5d5d4b6` adds optional raw seek samples (`USD_BENCH_RAW_SAMPLES=1`) and process
  high-water RSS to `editor_benchmark`. The latter includes prior fresh-App samples
  in the same process; it is not per-document peak allocation or VRAM. Default
  workspace/all-target tests pass: 868 passed, 20 ignored. A debug probe emitted
  eight measured records with clocks, nanoseconds and a separate CSV summary.
  Logs: `/tmp/bevy-optimization-benchmark-tests.log` and
  `/tmp/bevy-optimization-raw-probe.log`.
- Release CPU characterization completed under
  `/tmp/bevy-optimization-baseline.KRJgwm/`, with revision/toolchain/host metadata,
  entry-file hashes, per-workload logs, raw seeks and status records. Eight
  workloads cover bird, cow, arm, showcase, morphs, subsets, kitchen and city.
  Referenced dependency closures are not yet hashed. This run shares host load.
  The initial bird case overlapped a one-off probe and was repeated serially;
  the latter records 56.814–67.931 ms median updates. Cow medians are
  82.124–106.466 ms with an unexplained 2.915-second outlier. Saved baseline
  executables and `run-headless-benchmark` (`1ed2f92`) support alternating
  comparisons without rebuilding. See [the baseline report](benchmarks/whole-pipeline.md).
  GPU, native cadence and complete lifecycle baselines remain outstanding.
- Final debug bird profiling through `d5b87b6` records 52.642 ms median and 61.210
  ms max headless updates. Across eight seeks, skin routes take 278.657 ms, mesh
  routes 78.715 ms and materials 39.203 ms. Mesh reads take 50.650 ms inside route
  time, not in addition to it. There are 944 mesh interning misses for 328 mesh
  projections, inserting 15,448,960 bytes: investigate intermediate/rest geometry
  construction and upload churn before adding more cross-frame caches.
- `d2256aa` reuses immutable rest geometry through the existing bounded assembly
  cache before color/skin/morph mutation. Default tests pass (869 / 20 ignored);
  all 32 alternating release matrix processes succeed with identical retained
  asset counts/payloads. Before/after captures are pixel-exact in five cases;
  cow still warns about its unavailable MDL dependency. Additional cache payload
  is about 1 MiB for bird and 3.3 MiB for cow. See the baseline report for exact
  measurements and limits. Hardening passes (782 no-default / 875 all-feature
  tests), as do strict Clippy, four native deformation checks, wasm compilation
  and pixel-exact synthetic CPU/GPU captures at 0, 5 and 10.
- Next targets: intermediate mesh asset churn and skin/tangent preparation,
  plus the remaining source-verification cost. GPU pass
  samples are now recorded, but native animated frame pacing and full lifecycle
  acceptance remain open. Retain all workstreams above; this is not whole-pipeline
  completion.
- Texture checkpoint: `d709b52` isolates finalization cost. `e573343` streams the
  first mip's decode and reuses the exact sRGB code table, reducing paired release
  bird opening from about 4.9 to 2.9 seconds and cow opening from about 2.5 to
  1.5 seconds, with lower peak RSS and identical retained payload. Default tests
  pass; before/after captures are pixel-exact, and hardening passes (787
  no-default / 880 all-feature tests plus strict Clippy), as do nine native watcher
  tests and wasm compilation. At this checkpoint, the cow's initial texture reload
  remained a separate follow-up, with a roughly two-second worst seek. See
  [texture finalization](benchmarks/texture-finalization.md).
- Initial texture verification: `9ded82f` records the hashes of decoded source
  bytes; `4eac681` preserves package-entry overrides; `1eff934` checks edited paths,
  undo/redo and failed refresh publication. Unchanged external textures no longer
  trigger image/material/mip regeneration after Open. Production watcher settings
  and fidelity are unchanged. Default tests pass (878 / 20 ignored) after a
  test-only projection-budget race repair (`bc95517`). Hardening at `1eff934`
  passes (791 no-default / 884 all-feature tests, strict Clippy), as do nine native
  watcher tests and wasm compilation. See [texture verification](benchmarks/texture-reload.md)
  for measurements, evidence and remaining animation/native-rendering limits.
- Skinning checkpoint: `e66e070` retains warmup timings, `dc89bfb` separates GPU
  preparation phases, and `69708b6` reports exact CPU fallback reasons. `739deac`
  reuses only the current mesh binding inside a read-only projection pass;
  authored influences, deformation and eager residency are unchanged. Default
  tests pass (881 / 20 ignored), hardening passes (793 no-default / 887 all-feature
  tests and strict Clippy), as do four native deformation checks, nine watcher
  checks, 15 native export checks and wasm compilation. All 16 alternating release
  processes pass with matching final geometry/image payload and peak asset counts.
  Bird tangent-deformation time falls, but host load prevents a reliable
  whole-frame speedup claim. Five before/after renders are pixel-identical; the
  cow's unavailable MDL diagnostic remains. See
  [skinning preparation](benchmarks/skinning-preparation.md) for exact evidence,
  memory/measurement limits and unresolved CPU/GPU fidelity checks. Larger tangent
  assembly, CPU fallback, native cadence and the full workstreams remain open.
- Tangent checkpoint: `edb462c` separates assembly/generation timing, `6e71684`
  retains recently used entries within the existing 16 MiB budget, and `bc9963f`
  reuses exact deformed tangent inputs only after recurrence. Default tests pass
  (885 / 20 ignored), hardening passes (797 no-default / 891 all-feature tests,
  strict Clippy), as do native deformation, watcher/export and wasm checks. All
  32 final alternating processes pass with matching final geometry/image payload
  and peak asset counts. Unique-time cow preparation avoids about 5.7 GB of
  cumulative unnecessary tangent-cache copies from the rejected unconditional
  trial; warmup data remains retained and total tangent payload is higher than
  the pre-integration baseline. Eight before/after renders, including explicit
  CPU controls, are pixel-identical. Cow MDL warnings and the strict bird CPU/GPU
  discrepancy remain failing gates. See [tangent reuse](benchmarks/tangent-reuse.md)
  for timings, memory, trial failures and exact evidence. Cow unique-time
  preparation remains roughly 83 ms/update; this is not 60 Hz native playback
  acceptance or completion of the workstreams above.
- CPU-input checkpoint: `c9006b3` reuses resolved influences and mesh-order
  transforms through upstream classic-linear math; `8e9001f` repairs test-only
  lint errors. Default tests pass (886 / 20 ignored), hardening passes (798
  no-default / 892 all-feature tests and strict Clippy), as do native deformation,
  watcher/export and wasm checks. Eight before/after renders remain pixel-exact;
  the same cow diagnostic and strict bird mismatch remain open. The final
  timeline matrix has 28 successful processes and one preserved invalid-control
  attempt. Raw hybrid-PMU probes show small instruction reductions for the
  classic-linear cases and a 0.210% DQ increase; neither these counters nor the
  noisy timing ranges prove native FPS acceptance. See
  [CPU skinning inputs](benchmarks/cpu-skin-inputs.md). The next larger target is
  bounded parallel pure-mesh preparation while preserving stage/thread, route,
  visibility, instance and lifecycle contracts.
- Parallel-preparation implementation: `0035a9e` separates tangent-cache
  lookup/admission from result insertion; `7483e05` batches built-in animation
  across bounded owned-mesh jobs while keeping USD reads and ordered publication
  on the main thread. `d740239` reports the complete batch interval rather than
  hiding moved work in smaller route times. `51da2f4` covers independent root
  clocks, referenced geometry, subsets, reload, close and hidden residency;
  `c2e3142` repairs a timing-dependent initial-projection test, not production
  loading policy. Hardening passes 803 no-default and 897 all-feature tests plus
  strict Clippy. Release elapsed, native render/oracle and wasm acceptance remain
  pending for this change. See [parallel preparation](benchmarks/parallel-mesh-preparation.md)
  for limits, retained failures and the predeclared performance target. No
  speedup or native cadence result is claimed yet.
- The first parallel matrix completes 24 processes but does not meet the cow
  GPU target. Sparse groups contain too few meshes (21,000 batches for 13,986
  tangent jobs), so offloading often adds scheduling without concurrency.
  `2900832` groups by mesh count, caps lightweight contexts separately, runs
  single jobs inline and has the caller work alongside workers on larger groups.
  `8a06876` records this utilization in benchmarks and render captures. The
  corrected code passes hardening (804 no-default / 899 all-feature tests and
  strict Clippy), but its release elapsed and render acceptance are still pending.
  The full workstreams and existing rendering failures remain open.
- Corrected batching validation: native deformation/watchers/export and wasm
  compilation pass. Six CPU/GPU-mode before/after captures of independently
  timed bird, cow and skin/morph roots are pixel-exact after live clock reversal;
  metadata proves parallel work executes in the bird/cow production-plugin path.
  The cow MDL warning still fails the combined capture gate. The original strict
  single-bird CPU/GPU comparison is rerun and still fails at 2,053 pixels, maximum
  RGB error 33. The corrected cow unique-clock trial averages 85.364 ms before
  versus 76.856 ms after across per-process medians (9.97% lower), below the
  predeclared 15% target amid large host-load swings. This is not accepted native
  playback performance. Next: tighter temporal pairing, remaining performance
  controls and observed render-order/bounds diagnosis; preserve all workstreams.
