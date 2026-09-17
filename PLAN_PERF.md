# USD loading performance plan

Planned against `1dd38c8` on 2026-09-16; reconciled against `2d015e4` and the
working tree on 2026-09-17. Status: **P0–P8 in progress; P4 opt-in only**.
Profiling increments are committed as `a054c56`, `9303920` and `fd79645`.
The matched first-complete-frame benchmark and ≤5× target remain unverified.

## Immediate handoff

Keep implementation incremental. The detailed P0–P8 sections below retain the
research, measurements, rejected experiments and acceptance requirements.

### Current checkpoint and next bounded increment

- **Implemented, not full acceptance:** parsed-root reuse, retained deferred
  queues, traversal-local parent status, reduced repeated ancestry queries, and
  batched property classification and scoped decoded-material reuse. The latest
  material-cache gate records 766 passing tests, 19 ignored and unchanged Oxbo
  and Caldera RGBA captures. Logs are under `target/perf/p1-material-reads/`.
- **Still unresolved:** matched first-complete-frame measurement and the ≤5×
  ratio. Moana's 300-second attempt is killed at the 24 GiB memory cap after
  approximately 191 seconds; its last sampled checkpoint is 333,824 prims.
  This is a confirmed memory-limit failure, not a completed load or timeout.
  Keep hidden-mesh deferral opt-in.
- **Latest bounded Moana outcome:** after `5b9e18b`, all 452,662 fibers instances
  complete expansion. The 300-second-capped attempt still suffers a confirmed
  24 GiB cgroup OOM after 190.188 seconds, now during
  `/island/isBeach/geometry/xgShells/instancer` (70,972 points). Its last coarse
  checkpoint is 334,848 prims; 7,106,420 live entities exist before shells expand.
  Follow the bounded investigation below; do not raise the cap or omit content.
- **Acceptance blocker:** finish P0's revision-specific submitted-frame gate and
  matched native runs for Oxbo/Caldera. Headless CPU improvements do not establish
  the ≤5× rendered-frame target.
- **Property-source investigation:** profile redundant existence,
  sample-source and spec-stack walks in
  `vendor/openusd/crates/openusd/src/pcp/index_cache.rs`. Count-only sample queries
  already exist; do not propose them as a new optimization. Do not repeat the
  rejected per-node property-path cache without new attribution.
- **Exit gate for either investigation:** one isolated patch, differential edit and
  composition checks, Make-based validation, unchanged rendered output, and at
  least five alternating baseline/candidate runs with matching profiler settings.
  Retain raw timings and RSS under the 24 GiB/no-swap cap; reject unconvincing
  changes rather than accumulating speculative caches.

P0 remains the acceptance blocker regardless of CPU progress. The historical
implementation checkpoints below describe the wider sequence, not an instruction
to repeat already completed increments.

| Priority | Work | Completion evidence |
| --- | --- | --- |
| 1 | Extend retained promotion-queue validation to matched rendered workloads. | CPU regression gate and diagnostic timing are recorded under P4; still measure reveal-frame latency, rendered parity and queue allocation under scene churn. |
| 2 | Finish generation-specific first-complete-frame measurement (P0). | Required assets from the current scene revision are ready and included in submitted rendering; matched native settings and at least five alternating runs. |
| 3 | Profile Oxbo independently. | Ranked phase timings, allocation/peak-memory evidence and one measured bottleneck selected before implementation. |
| 4 | Complete deferred-residency correctness before default enablement (P4). | Variant/dependency coverage, failed-save recovery, visibility-demand semantics, reveal latency and unchanged rendered output. |
| 5 | Reduce remaining long updates and reload verification costs (P5/P8). | Bounded work and stale-result rejection without weakening transactional asset-local reload or missing external edits. |
| 6 | Revisit Moana under explicit time and memory limits. | Completion or an attributed timeout, retained logs and no unsupported large-scene performance claim. |

Recent committed work includes skipping unchanged joint-palette updates,
avoiding inspection after no-change reloads, indexing texture-bearing prims,
and rotating deferred preparation across asset roots. These are incremental
improvements, not completion of their architectural phases. The retained queue
has passed the CPU regression gate; rendered-frame acceptance remains separate.

Report CPU open, first complete frame, reveal latency, reload latency and peak
memory separately. Neither historical native timings nor headless CPU gains
establish the ≤5× rendered-frame target.

## Next implementation checkpoints

This is the performance implementation plan, not a claim that its phases are
finished. The immediate handoff and the memory investigation below take precedence
over this earlier checkpoint sequence. Preserve the detailed evidence; remaining
P4 validation is required before default enablement, not a prerequisite for P0.

1. **Finish validating the opt-in P4 prototype.** The initial implementation in
   `crates/usd_bevy/src/route/residency.rs`, `route/mod.rs`, `live.rs`, `asset.rs`
   and `instance.rs` defer initially hierarchy-hidden built-in Mesh preparation.
   The benchmark/capture examples expose `USD_DEFER_HIDDEN_MESHES=1`; eager mode
   remains the control. The test ownership error (`Attribute::set` consumes its
   handle) is corrected; `target/perf/p4-defer/tests.log` records 743 passing
   workspace/all-target tests and 19 ignored. Initial Caldera timing and image
   evidence is recorded under P4 below. Do not enable this feature by default yet.
2. **Prove P4 correctness and benefit independently.** Cover ancestor visibility,
   purpose, per-root clocks, hidden edits/reloads, variant retention, demanded
   prototypes and custom-route fallback. Compare eager/deferred Caldera RGBA
   under identical settings and retain at least five alternating fresh-process
   runs per mode (Caldera CPU results below). Record route times, total CPU
   loading, peak RSS and reveal latency.
   Existing attribution identifies roughly 15 seconds of hidden Caldera work,
   not a guaranteed saving. Oxbo has almost no such work: use it as a regression
   control, not evidence of this strategy's benefit.
3. **Close P0 before claiming the 5× target.** Implement generation-specific
   required-asset readiness through submitted rendering, and match camera,
   purposes, time, textures, renderer settings and cache conditions with the
   native comparator. CPU Ready, global pipeline readiness and screenshot
   process exit are not interchangeable with a first complete frame.
4. **Reprofile Oxbo separately.** Rank remaining stage open, validation,
   geometry, deformation, texture and GPU preparation costs. Select the largest
   measured avoidable component from P1–P3/P6/P7; do not assume Caldera's hidden
   geometry strategy transfers. Keep each change independently measurable.
5. **Only then expand scheduling and residency.** P5 requires bounded owned
   inputs, revision-checked publication and cancellation, not shared mutable
   Stage access. The rejected subset-worker trial below is not evidence that
   all parallel preparation is futile, but repeating it without a new profile
   is not a priority. P8 must preserve asset-local transactional reload.

Acceptance for every increment: a passing Make-based workspace gate, targeted
behavior checks, retained raw before/after measurements and visual evidence
where render output can change. Commit only validated changes; leave incomplete
phases marked incomplete. Validation of an increment does not complete its phase.

### Bounded point-instancer memory investigation (P6)

**Current decisive attribution after `57173b1`:**
`target/perf/p3-shared-sdf-paths/moana-region.log` shows that shells, seaweed,
pebbles, small shells and palm debris complete before the fatal expansion at
`/island/isBeach/geometry/xgGroundCover/instancer`. Ground cover contains
**21,357,212 points and 24 prototypes**. Its decoded position/orientation/scale/
index arrays occupy 256,286,544 / 341,715,392 / 256,286,544 / 85,428,848 bytes.
Between 400,000 and 500,000 spawned points, live entity count rises from
13,882,235 to 14,282,235: four entities per point in this region. The remaining
20.86 million points cannot fit the existing representation under 24 GiB.
`moana-region-scope.log` confirms OOM at 196.349 seconds wall time and the same
25,769,803,776-byte cap. This is later content than the pre-sharing failure, not
evidence that sharing failed to save memory. Initial projection RSS in this run
is 14,636,523,520 bytes, versus approximately 19.07 GB in the earlier diagnostic.

**Next architectural increment:** compact point-instancer rendering, not another
per-instance scratch tweak. Preserve the existing expanded-entity route as the
compatibility path while building an explicit opt-in compact path. Do not enable
the compact route until it actually renders its supported prototypes; a data-only
component that silently omits geometry is not an implementation of this feature.

1. Define shared prototype layout plus per-instancer sampled arrays, stable point
   IDs and masks, independent root/time ownership, and a mapping for picking and
   authoring. Private render storage must not replace the composed USD stage.
2. Render supported static mesh/hierarchy prototypes through Bevy PBR with compact
   transform/ID buffers. Preserve prototype local transforms, material subsets,
   normals/tangent handedness and shadows; retain the expanded path for unsupported
   deformation or custom routes. Avoid creating a full ECS subtree per point.
3. Bound CPU/GPU upload batches by bytes and support cancellation/revision-checked
   publication. Camera culling may avoid offscreen draws, but must not discard
   logical points or change authored masks. Do not claim a complete frame from a
   partially uploaded required instance set.
4. Provide an explicit expanded-entity compatibility/inspection path; preserve
   runtime components and IDs for existing expanded instances across edits. Do
   not silently change the public `UsdInstance`/`UsdPrototypePart` query contract.
5. Gate the route on matched expanded/compact captures for simple, nested,
   multi-material, masked and animated-array cases, plus independent roots,
   picking, failed reload and transactional replacement. Only then opt the viewer
   into it and rerun the real ground-cover asset with the same memory cap.

**Separate source-side opportunity:** `SourceResolver::open_asset` retains
`Arc<[u8]>` file snapshots for editor reload safety, while the default file-format
read and ambiguous `.usd` dispatch call `read_all` into another owned buffer.
A shared immutable-byte decode seam could avoid duplication for USDC without
reading mutable disk contents lazily. Scope such a change to explicit immutable
snapshots with a copy fallback for existing/custom resolvers, and verify edits,
exports and reload isolation. This can improve source residency but cannot solve
the 21-million-point entity expansion on its own.

**Full-stage retest after shared keys:** the existing fibers failure is passed,
but the following shells instancer exhausts the same 24 GiB/no-swap budget.
`target/perf/p6-shared-paths/moana.log` records the fibers PointInstancerRoute
completion, then successful decoding/ID validation for 70,972 shells and one
spawn-progress checkpoint. On entry to shells expansion, RSS is
25,725,472,768 bytes, live entity count is 7,106,420 and inline table component
capacity is 1,617,895,892 bytes. `moana-scope.log` confirms `Result=oom-kill`,
25,769,803,776-byte peak and 190.188 seconds wall time (188.915 seconds CPU).
This is not a timeout or complete load. Retain the successful isolated memory
result, but do not extrapolate it to full-scene acceptance. Broader source/ECS
retained-memory attribution and a compact-instancing design are still required;
another small scratch reduction is not an adequate Moana strategy.

**ECS attribution:** opt-in table diagnostics in `route/profiling.rs` now sum
live table rows independently of allocated entity-index capacity, and rank
tables by component layout size times capacity. The isolated fibers workload
starts expansion at 73 live entities and finishes traversal at **2,263,400**,
with 4,194,304 allocated indices. Its dominant tables contain 1,357,986 hierarchy
nodes, 452,662 mesh nodes and 452,662 instance roots. Native `usdcat` inspection
of the needle prototype confirms three nested Xforms above its Mesh: this is
five ECS entities per point, not merely one mesh entity per point.
Table component capacity accounts for **558,902,016 bytes**; process RSS grows
from approximately 92 MB before expansion to 1.397 GB afterwards. The estimate
excludes component-owned heaps, change ticks, entity metadata, archetypes, sparse
sets and allocator overhead; do not treat it as total ECS allocation or infer
that every remaining byte belongs to hierarchy strings. Evidence:
`target/perf/p6-ecs-layout/fibers.log`. Release builds lack general debug type
names, so diagnostics explicitly identify common scene components by TypeId and
retain component IDs for unknown types without enabling Bevy debug features.
`fibers-labeled.log` confirms the three dominant table layouts with release-safe
component labels. The Make release workspace/all-target gate passed 768 tests,
19 ignored; the benchmark build passed. Profiling remains opt-in and the extra
table scans are diagnostic overhead, not a production loading optimization.

**Shared path-key candidate:** prototype descriptors and private per-instance
reconciliation maps now use `Arc<str>` keys. Public `UsdPrototypePart(String)`
values and all hierarchy entities remain unchanged; path-content matching still
preserves entities across new descriptor allocations on re-projection. Existing
independent-animation coverage additionally checks shared key allocation both
initially and after clock changes. No global path interner or cross-stage state
was introduced.

Five alternating fresh-process isolated fibers runs, profiling disabled, record
baseline RSS-after-idle bytes
`1396862976,1397968896,1398149120,1398226944,1398407168` versus candidate
`1206505472,1206226944,1206403072,1206472704,1206349824`. Median retained RSS falls
**1,398,149,120 → 1,206,403,072 bytes**, saving **191,746,048 bytes (13.7%)**.
Median process peak RSS falls **1,364,044 → 1,176,904 KiB**. Baseline CPU-open
milliseconds `1145.170,1256.137,1149.910,1147.531,1147.403` versus candidate
`1157.356,1115.445,1160.340,1130.362,1123.147` overlap; do not claim a reliable
CPU speedup from this trial. All ten geometry/asset/payload rows match. Logs and
saved binaries: `target/perf/p6-shared-paths/`. This is an isolated memory result,
not evidence that full Moana now fits or that the submitted-frame target is met.
The Make release workspace/all-target gate passed 768 tests, 19 ignored; benchmark
and capture builds passed. Matched baseline/candidate captures of
`assets/point_hierarchy.usda` at time zero are byte-identical in RGBA. This checks
instancer-specific output, while the clock/reconciliation tests cover updates.

**Prototype borrowing trial:** `route/instancer.rs` borrows the per-projection
prototype descriptor and prepared subsets instead of cloning them for each point.
Only handles inserted into ECS components are cloned; entity layout and authored
instance identity are unchanged. The isolated real source
`target/large-scenes/moana/island/usd/elements/isBeach/xgenInstances/xgFibers.usd`
completes under the 24 GiB/no-swap cap. Five alternating fresh-process trials
with profiling disabled gave CPU-open baseline milliseconds
`1266.649,1276.789,1363.687,1287.003,1606.864` versus candidate
`1175.044,1160.917,1163.485,1164.562,1232.143`: medians
**1287.003 → 1164.562 ms (9.5% lower)**. Retain the slow baseline sample;
do not present the best pair as the result. All ten rows retain 452,666 mesh
entities, four meshes, 240 vertices, 11,520 vertex bytes and 5,568 index bytes.
RSS after idle remains approximately 1.398 GB: this does not solve full-island
memory exhaustion. Raw rows and peak RSS are in
`target/perf/p6-prototype-borrow/{baseline,candidate}-pair-{1..5}.log`.
This is an isolated CPU workload, not a full Moana or rendered-frame result.
The release workspace/all-target Make gate passed 768 tests with 19 ignored,
including all 18 instancer tests covering hierarchy, subsets, deformation,
independent clocks, masks and stable IDs. The fresh Oxbo capture is byte-identical
to the retained RGBA control (an unrelated-scene regression check, not an
instancer-specific visual comparison). Validation artifacts are in the same
directory. No idle-machine guarantee was established; repeat timings on a
controlled host before using this result as release acceptance.

**Evidence checked on 2026-09-17:**
`target/perf/p6-instancer-ids/moana.log` records successful array decoding and ID
validation, followed by instance-spawn progress at 1, 100,000, 200,000 and 300,000
of 452,662 instances. RSS grows from 24,803,393,536 bytes before expansion to
25,667,592,192 bytes at the last checkpoint. The associated
`usd-perf-instancer-ids.scope` reports `Result=oom-kill` and
`MemoryPeak=25769803776`. This localizes the failure to expansion, but does not
identify the exact allocation that triggered the kill or explain earlier residency.
The diagnostic and ID-allocation increment removes implicit-ID vector/hash scratch,
skips uniqueness hashing for strictly increasing authored IDs, and releases other
uniqueness scratch before expansion. The Make release workspace/all-target gate
records 768 passing tests and 19 ignored in
`target/perf/p6-instancer-ids/tests.log`. The benchmark/capture build passed; a
fresh Oxbo capture was inspected and its RGBA is byte-identical to
`target/perf/oxbo-current/original.rgba`. This is regression evidence, not Blender
parity or a demonstrated full-load speedup. The OOM remains unresolved.

The earlier `target/perf/p0-memory-region/moana.log` reports approximately
19.07 GB RSS before projection, including 2.71 GB of image payload. Near the
failure, mesh payload is approximately 2.70 GB. These payload figures omit
capacity, allocator, source, ECS and GPU overhead; cache figures can overlap.
Do not add overlapping counters or call the residual exclusively ECS memory.
Bevy 0.19.1 `Entities::len()` counts allocated entity indices, not live entities;
the earlier diagnostic label `entities` must not be interpreted as a live count.

**Execution order and deliverables:**

1. **Establish a reproducible expansion profile.** Scope source inspection to
   `crates/usd_bevy/src/route/{instancer,profiling,cache}.rs`, `read/geom.rs`,
   `live.rs` and the benchmark example. Retain the full-stage bounded control;
   additionally isolate the same instancer with its required prototype/material
   dependencies for attribution. Label isolated results separately, not as Moana
   load times. Sample live entity counts outside the per-instance hot loop and
   distinguish root instances from prototype descendants. Measure bytes and time
   before arrays, prototype baking, instance construction and GPU preparation.
2. **Test smaller representation-preserving reductions first.** Profile repeated
   `Prototype::Hierarchy` cloning, path/map allocations and per-instance child
   bookkeeping. Share immutable prototype descriptors where safe, without sharing
   mutable root state. Retain ID ordering, invisible IDs, hierarchy transforms,
   picking, updates and custom-route behavior. Reject changes without repeatable
   time or peak-memory benefit. The implicit-ID scratch removal is not a
   demonstrated full-scene solution.
3. **Design compact instancing only if expansion remains dominant.** Specify a
   logical point-ID-to-render-instance mapping and shared prototype draw data
   instead of assuming every render instance needs a complete ECS subtree.
   Before implementation, define compatibility for public entity access, picking,
   material subsets, affine transforms, shadows, visibility, animated arrays,
   independent clocks and transactional reload. Keep an explicit fallback for
   unsupported prototypes and custom routes. Never silently drop points or
   substitute flattened, non-editable geometry to meet the benchmark.
4. **Validate in layers.** First cover implicit/explicit IDs, duplicate rejection,
   reordered IDs, invisible points, multiple prototypes, nested hierarchies and
   changed array lengths in existing instancer tests. Then compare eager and
   candidate captures at matched time/camera, exercise two independently clocked
   roots, and edit/reload one root while preserving the other's state. Use the
   Make workspace gate listed below; do not substitute a test count for these
   behavioral and visual acceptance requirements.
5. **Accept only measured progress.** Run at least five alternating fresh-process
   baseline/candidate trials on a completing instancer workload, with identical
   profiler settings and retained raw times/RSS. Run full Moana separately under
   the same 24 GiB/no-swap and explicit time limit. Record OOM, timeout and complete
   frame as different outcomes. A surviving CPU load still requires P0's submitted
   frame gate before it counts as rendered acceptance.

**Stop conditions:** an API/semantic incompatibility requires a design decision,
not an implicit behavior change. No memory-limit increase, unsafe `Stage`
threading, lower visual settings, instance-count cap or unrelated environment
changes. Keep this increment separate from the P0 readiness work and commit only
after its own verification. The current tests/build logs are historical evidence;
this plan update does not represent a new build or benchmark run.

## Objective

Bring the Bevy viewer's time to its first complete scene frame within **5× a
matched native OpenUSD/Storm reference** on the same machine and scene revision.
Keep Bevy as the renderer. Blender/EEVEE remains the visual-fidelity reference;
Storm is a performance comparator, not a replacement renderer.

Improve three separately reported workloads:

1. Initial opening to first complete frame.
2. Subsequent visibility, purpose, variant and animation changes.
3. Blender-save-to-Bevy asset-local hot reload.

Do not claim success from stage opening alone, a partial frame, a CPU `Ready`
state, fewer rendered objects, or a faster screenshot harness.

## Constraints

- The composed live USD stage remains the source of truth.
- Preserve independent roots, clocks, overrides, edits and material bindings.
- Preserve asset-local reload, last-good state, dirty-layer protection,
  transactional publication, stable matching entities and runtime components.
- Keep previously loaded switched-off variant assets. Deferring initially
  unprepared content is permitted; evicting retained variant assets is not.
- Preserve geometry, winding, holes, primvars, material subsets, skin/morph
  deformation, normals, tangents and CPU fallback behavior.
- Preserve custom route ordering/contracts; provide a synchronous fallback
  where the built-in preparation pipeline cannot safely handle a custom route.
- Never make `Stage` Send/Sync through unsafe declarations. Its current
  `Rc`/`RefCell` internals are not a worker-thread API.
- Bound CPU preparation queues by bytes as well as job count. Track CPU and
  GPU memory separately; a faster run that exhausts memory is not acceptable.
- Use Make for builds, checks, tests and benchmark targets. Use patch/edit tools,
  not Python, for file changes. Do not change environment files or Makefile
  conventions without separate approval. Never run `make clean`: large datasets
  reside under `target/`.
- Make incremental unsigned, title-only Conventional Commits, with a message
  no longer than 50 characters. Do not push unless explicitly requested.

## Evidence and baseline limitations

Existing local measurements, not guarantees or fresh acceptance results:

| Caldera workload | Observed cost |
| --- | --- |
| Latest CPU/editor open | 44.591 / 45.790 s |
| MeshRoute application | 11.575 / 11.712 s |
| SkinRoute application | 3.991 / 4.136 s |
| SubsetRoute application | 3.390 / 3.452 s |
| Validation phase, excluding preceding open | 9.097 / 8.055 s |
| Latest capture process to exit | 84.74 s |
| Earlier native usdrecord process to exit | 3.20 / 2.91 / 2.87 s |

The route timings are not a full additive decomposition. Animation discovery,
composition and other work occur outside the timed route applications. Other
project builds overlapped the latest CPU measurements; rebaseline on an idle
machine before accepting a performance claim.

The latest Caldera CPU run retained 50,125 mesh entities, 23,291 subset entities
and 3,312 mesh assets. The capture reported 4,621 hierarchy-visible mesh entities.
These entity counts do not establish visible geometry bytes or proportional
potential savings. The last geometry change reduced assets from 3,586 to 3,312
and produced a pixel-identical Caldera capture, but did not establish a meaningful
loading speedup.

The earlier stage-only run took 36 ms to open and 4.928 s to traverse, without
instance proxies. The editor traverses proxies too. Composition is lazy:
validation includes necessary composition, so its entire duration cannot be
counted as removable overhead.

The native median implies a provisional 14.55 s budget, **not an accepted target
measurement**: native output was 1440×1050, capture was 1280×720, lighting differed,
and capture waited for 60 ready frames plus optional GPU timing samples.

Existing evidence is in `/tmp/caldera-geometry-reuse.log`,
`/tmp/caldera-geometry-reuse-capture.log`, `/tmp/caldera-stage.log`,
`/tmp/caldera-native-profile-{1,2,3}.log`, and `/tmp/oxbo-geometry-reuse.log`.
These files are ephemeral and may be absent on another machine. Preserve new
raw measurements under ignored `target/perf/` with commit and run metadata.

## Current architecture and confirmed costs

Paths below are relative to the repository root; line numbers refer to the
planned commit and must be rechecked after changes.

- `src/main.rs:584–591`: actual viewer uses `LiveStagePlugin + EditorPlugin`.
- `examples/editor_benchmark.rs:67–84`: CPU-only editor open; no render pipeline.
- `examples/viewer_capture.rs:283–290,489–495`: asset-instance pipeline, unlimited
  projection budget, pipeline gate, 60 ready frames and optional GPU samples.
- `crates/usd_bevy/src/live.rs:475–500,574–588`: serial prim projection.
- `crates/usd_bevy/src/route/mod.rs:284–288`: routes run through exclusive World.
- `route/geom.rs:93–117` beneath that crate: geometry is read before sharing is
  checked, regardless of effective visibility/purpose.
- `route/subset.rs:144–150`: clone full parent mesh before preparing subsets.
  `mesh/compact.rs:11–30`: parent-sized scratch scans and attribute copies precede
  output interning.
- `read/skel.rs:21–32,530–588`: classification reads weights/sample times; GPU
  preparation rebuilds binding/influence data and pose-dependent corrections.
- `route/gpu_skin.rs:36–42`: every frame computes and writes all joint globals.
- `route/cache.rs:255–259`: exceeding the projection cache budget clears all
  entries, while entities can continue retaining the corresponding assets.
- `source.rs:352–385`: validation reads all authored default attribute values.
- `asset.rs:109–147,294–300`: loader validation precedes another stage open and
  validation per root. The editor's initial open validates once, not twice.
- `source.rs:106–159` and `asset.rs:486–494`: texture discovery traverses the stage
  and material samples; per-instance discovery repeats even with loaded images.
- `reload.rs:21–57`: reload preparation serializes layers, may rebuild a USDZ,
  creates a candidate stage and validates it before and after patching.
- `vendor/openusd/crates/openusd/src/usdc/mod.rs:233–256`: ordinary crate fields
  decode into owned values. Composition-index caching is not decoded-array reuse.
- `vendor/openusd/crates/openusd/src/usd/attribute.rs:1000–1118`: revision-aware
  `AttributeQuery` already exists, but ordinary geometry reads do not retain it.
- `vendor/openusd/crates/openusd/src/sdf/file_format.rs:172–174`: resolver-backed
  parsing calls `read_all`, copying buffers despite shared snapshot ownership.

Current benchmark artifacts already enable Bevy `multi_threaded`. The bottleneck
is not a missing feature flag. Bevy 0.19.1's render-asset extraction prepares
changed `RENDER_WORLD` assets without checking entity visibility, so hidden
geometry also incurs GPU work.

## Execution order

M means approximately day-scale; L means multi-day work including verification.
Update status only with linked evidence and a commit.

| ID | Work | Depends on | Effort / risk | Status |
| --- | --- | --- | --- | --- |
| P0 | Matched first-frame benchmark and attribution | None | M–L / medium | IN PROGRESS |
| P1 | Cache subset products before construction | P0 attribution | M–L / medium | IN PROGRESS |
| P2 | Cache deformation discovery and preparation | P0 attribution | L / high | IN PROGRESS |
| P3 | Separate validation from discarded geometry decoding | P0 attribution | M–L / high | IN PROGRESS |
| P4 | Demand-driven initial geometry residency | P0, P1, P2 | L / high | OPT-IN; INITIAL GATES PASS |
| P5 | Bounded owned-data worker pipeline | P0, P1, P2 | L / high | SCHEDULING IN PROGRESS |
| P6 | Pre-decode prototype reuse and cache indexing | P0, P1; coordinate P4/P5 | L / high | LOOKUP IN PROGRESS |
| P7 | Shared input buffers and texture manifests | P0, P3 | M–L / medium-high | MANIFEST IN PROGRESS |
| P8 | Revision-based transactional hot reload | P0, P3; reuse P7 | L / high | NO-OP INSPECTION IN PROGRESS |

Implement P1–P3 as independently measured changes. Reprofile before P4–P8 and
adjust their order using measured bytes, duplicate work and serial CPU fractions.
Do not add projected speedups together: several phases remove the same work.

## P0 — Establish trustworthy measurements

Live projection attribution: `USD_PROFILE_LOADING=1` now reports projection
start, the first eight prims, every 1,024th prim, traversal return and completion
of materialization. Reports split animation discovery from registry projection.
With `ProjectionTimings` installed (`USD_PROFILE_ROUTES=1` in the benchmark), each
checkpoint also reports the five largest cumulative route costs. Animation
reports preserve the existing short-circuit order and split instancer scans,
authored sample checks, subset checks, deformation, inherited primvars and
material queries. Reset counts distinguish recomputation from normal discovery.
The normal path performs no clock reads or progress output from this probe.

The 180-second Moana diagnostic still times out (exit 124), but now identifies
the work in flight rather than leaving an empty end-of-update report. Its last
completed sample is 229,376 prims, 92.670 s into live projection:

| Work | Elapsed in sampled prefix |
| --- | --- |
| Animation discovery, total | 33.024 s |
| ↳ Authored attribute sample checks | 13.431 s |
| ↳ Material animation queries | 12.961 s |
| ↳ Inherited primvars | 2.883 s |
| ↳ Deformation checks | 2.218 s |
| Registry projection, total | 56.725 s |
| ↳ MaterialRoute application | 27.964 s |
| ↳ MeshRoute application | 17.124 s |
| ↳ VisibilityRoute application | 3.334 s |

Indented rows are included in their parent totals, not additional costs. These
are incomplete-prefix diagnostic costs, with timer/log overhead; do not
extrapolate them into a full-load or first-frame result. Discovery resets are
zero. Peak RSS is 21,320,384 KiB under the 24 GiB/no-swap cap. The earlier broad
probe also showed continued progress rather than a single hung call; its
235,520-prim prefix is a different sample, not a before/after performance claim.
Artifacts: `target/perf/p0-projection-progress/{moana,moana-initial}.log`.

763 Make release workspace/all-target tests pass (19 ignored). The existing
purpose/time-change projection regression also passes with profiling enabled.
Oxbo completes its diagnostic CPU open in 2.208 s and reports projection through
materialization for all 3,743 prims. This is not GPU/frame timing or a claimed
speedup. Logs and builds: `target/perf/p0-projection-progress/`.
The profiling-disabled Oxbo run emits none of the new progress records and has
identical geometry, asset-count and cached-payload CSV fields to the enabled run;
elapsed times and process RSS are excluded from that equality check.

Moana validation attribution: `USD_PROFILE_LOADING` now logs validation traversal
start/completion, attribute progress every 4,096 prims, attribute completion and
successful validation completion. Counters describe visited prims and inspected
attributes, not geometry bytes or draw calls. Validation semantics are unchanged;
the normal path emits no new logs. The full Make release workspace/all-target
gate passes 759 tests (19 ignored), and the benchmark builds.

The current original Moana root still fails the bounded 180-second CPU-open run:
exit 124, peak RSS 19,986,164 KiB, no swap, under the 24 GiB cap. Stage open
reported 13 ms, but composition is lazy: validation completed at 71.390 s,
texture preparation at 97.947 s and save baselines at 102.675 s. A later editor
snapshot cost 7.875 s. No initial-open benchmark row or rendered frame completed.
This is not an OOM result and not evidence that loading only costs 13 ms.

A second, deliberately 110-second diagnostic with validation attribution found
416,549 traversed prims and 6,400,861 inspected attributes. Traversal alone took
40.534 s; the attribute loop took another 36.884 s; remaining validation was
about 5 ms. Do not interpret the difference from the earlier 71-second result
as a regression or compare these diagnostic samples as a controlled speedup.
Artifacts are `target/perf/moana-current/{baseline,validation,tests,build}.log`.
A debugger attach was rejected by the host; no ptrace/security settings changed.

Next Moana optimization should target traversal and validation reads, not more
small mesh-route changes. The upstream traversal recomputes inherited status
along ancestor chains for each prim; inspect a traversal-local inherited-state
implementation with parity for inactive/undefined/abstract branches, load rules,
instance proxies and edits made by traversal callbacks. Attribute validation
also needs shared query/value-source work across its get/type/sample checks.
Do not skip proxy subtrees or validation errors to make the benchmark complete.
Moana render acceptance remains unproven.

Material-image dependency increment: the opt-in viewer probe now visits image
dependencies of retained/referenced StandardMaterial assets and the base of
FlatMaterial assets. Unloaded texture handles remain in the required image set;
multiple texture slots referencing one image are deduplicated. Material changes
rebuild these requirements through the existing generation triggers. Visiting
ExtendedMaterial directly did not expose its base textures in this checkout,
so the flat-material path explicitly visits its StandardMaterial base.

Coverage checks missing textures, stable idle generations, texture replacement
without an image event, duplicate texture slots and flat-material removal.
This is dependency-presence tracking, not a GPU revision or frame-submission
acknowledgment. Unloaded material assets are already pending themselves; their
image dependencies become known when the material arrives. Environment bindings,
mesh morph/skin bindings and view-specific readiness remain separate work.
Validation artifacts are under `target/perf/p0-texture-dependencies/`.
The Make release workspace/all-target gate passes 759 tests (19 ignored).
The release viewer builds, and a time-limited isolated-settings run of
`assets/clearcoat_texture.usda` observes 14 images, four meshes and three
materials with no pending uploads/pipelines at 617 ms from probe setup.
It exits at the deliberate 12-second timeout. This runtime smoke check does
not establish a first complete frame or a performance improvement.

Entity-reference probe increment: the actual-viewer manifest now includes mesh,
StandardMaterial and FlatMaterial handles referenced by entities, even if their
assets have not loaded into CPU storage. Handle additions/replacements/removals
advance its generation without requiring an asset event. Duplicate references
are deduplicated. Known CPU-only meshes are excluded using their asset usage;
unresolved mesh references remain required. Missing mesh IDs are sampled in the
render log (up to eight). The probe stays opt-in and `complete_frame: false`.

The regression reserves unloaded handles, verifies they remain pending, changes
a mesh handle without an asset event, despawns its entity and verifies removal.
It also checks stable idle generations with an absent optional FlatMaterial
asset resource, and exclusion of known CPU-only entity meshes. Broader image
dependency closure, revision-specific GPU contents, view eligibility and
submitted-frame acknowledgment remain outstanding.

An isolated-settings, time-limited Oxbo viewer run with the probe enabled
reported 1,598 meshes, 31 standard materials and 10 images, with all observed
uploads and global pipelines ready at 3.254 s from probe configuration. This is
not a first-complete-frame result. An initial diagnostic overcounted two
CPU-only entity meshes; preserving the existing RENDER_WORLD eligibility rule
resolved that without excluding unresolved handles. The owned viewer exited at
its intentional 15-second timeout. Logs/build/test evidence are under
`target/perf/p0-references/`.

Actual-viewer render probe: launch with `USD_PROFILE_RENDER=1` to emit JSON
`render_asset_profile` records from `src/perf_render.rs`. Main-world manifests
carry document identity and an asset-event generation into the render world.
The post-render probe counts missing `RenderMesh`/`GpuImage` entries, prepared
StandardMaterial/FlatMaterial entries and global pending/error pipelines. It
covers all currently retained RENDER_WORLD meshes/images and supported retained
materials, including environment assets, not just visible scene dependencies.
Manifests rebuild on document/asset events rather than scanning geometry every
frame. Time starts at probe configuration, not process creation or open-command
submission. Every record explicitly says `complete_frame: false`: material
binding revisions, deformation resource bindings, relevant view queues and
submitted-generation acknowledgment remain outstanding. Prepared entry presence
alone does not prove an in-place asset update reached the GPU.

Material-probe increment: material-only changes rebuild the extracted manifest,
including optional flat-material resource addition/removal. Pending material
entries now prevent `observed_uploads_ready`. 102 release viewer tests pass;
the release binary builds with the same three existing camera-plan warnings.
An isolated-settings Oxbo viewer run observed 31 prepared standard materials,
1,598 meshes and 10 images; all observed queues became ready at 4.809 s from
probe setup, not process start or a proven first complete frame. Its viewport
screenshot was inspected. A GPU skin fixture separately observed one prepared
flat material. Logs/screenshot/build evidence are in `target/perf/p0-material/`.
Both owned viewer processes ended at their deliberate timeouts; no persistent
viewer was left running. This probe remains opt-in via `USD_PROFILE_RENDER`.

Probe verification: 100 release viewer tests pass and the release viewer builds.
An isolated-settings X11 Oxbo run produced a mapped 1440×920 window and a verified
viewport screenshot at `target/perf/p0-render/oxbo.png`. Its log observed 1,598
retained mesh assets and 10 images after cleanup, and pending pipelines resolving
to zero at 3.903 s from probe setup. This is a single diagnostic observation,
not a matched timing or complete-frame claim. Logs and build/test evidence are
in `target/perf/p0-render/`. The owned viewer process uses a 35-second timeout;
that expected timeout is not a loader failure. X11 launching required unsetting
`WAYLAND_DISPLAY` and `WAYLAND_SOCKET`; no user environment files were changed.

First increment: `route::cache::MeshCacheMetrics` is an opt-in resource enabled
by `USD_PROFILE_ROUTES` in `editor_benchmark`. It reports cumulative key/value
records separately for initial open and measured seeks (reset after seek warmup).
`assembly_handle_hits` avoids owned mesh cloning; `assembly_clone_hits` still
clones an assembled mesh; `assembly_builds` counts actual assembly executions.
`assembly_build_ns` includes tangent generation but excludes input USD decoding,
cache lookup and snapshot insertion. Build-input bytes count array payload passed
to assembly, not total bytes decoded from USD. Intern counters distinguish hits,
misses, missing-cache and budget bypasses, and whole-cache flushes. Evicted bytes
mean removed cache entries, not memory freed: entities may still own assets.
Absent counter keys mean zero. These counters do not constitute GPU readiness
or a first-complete-frame benchmark; the remainder of P0 is still outstanding.

Second increment: opt-in `route::MeshReadTiming` measures geometry reads through
registry-owned `RouteCtx` objects. It distinguishes requests from actual reads
shared by the context's `OnceCell`, including missing and failed reads. Returned
array bytes exclude material subsets and allocator overhead. Elapsed time includes
geometry/primvar/subset resolution and overlaps route matching/application;
never add it to route time. Direct reads outside registry contexts, validation,
animation discovery and standalone nested contexts are not included. Benchmark
reporting and seek-warmup reset use the same `USD_PROFILE_ROUTES` switch.

Second-increment evidence: `target/perf/p0-reads/` contains the patch, base commit,
release test/build logs and two runs per scene. The focused suite passes 574
tests, with 19 ignored. Caldera recorded 55,642 requests but only 19,156 actual
context reads, returning 1,807,177,852 geometry-array bytes per open; read time
was 5.832/5.839 s and assembly/tangent-build time 4.453/4.042 s. Oxbo recorded
3,524 requests, 1,762 reads and 63,628,972 returned bytes; reads took 87/84 ms
and assembly 305/298 ms. Both had zero missing/error context reads. Other
project builds and a running Gearbox process overlapped these runs, so use the
timings for attribution rather than speedup acceptance. Mesh/entity/vertex
counts remained unchanged. Remaining P0 work includes validation/animation and
GPU attribution plus the actual-viewer complete-frame manifest and benchmark.

Initial instrumentation verification (2026-09-16): focused release suite passes
573 tests, with 19 ignored; release editor benchmark builds. Evidence resides in
`target/perf/p0-cache/`, including the base commit and instrumentation patch.
Two sequential Caldera opens took 47.195/42.396 s and produced identical event
counts: 3,282 assembly builds, 16,640 clone hits, 15,874 direct handle hits,
65,719 intern hits, and 16 whole-cache flushes. Actual assembly/tangent building
took 4.477/3.885 s; this excludes USD decoding and lookup/copy costs. Oxbo took
3.706/3.265 s, with 1,586 assembly builds, 176 direct handle hits, 17 intern hits
and no whole-cache flushes. These are diagnostic iteration runs, not matched
first-frame acceptance or a demonstrated speedup. The counts support prioritizing
earlier reuse for Caldera; they do not identify every intern hit as a subset.

**Scope:** actual viewer/editor open instrumentation in `src/main.rs`,
`crates/usd_bevy/src/editor.rs`, `live.rs`, `route/mod.rs`; benchmark/capture
examples and focused helpers under `scripts/` if needed.

1. Add explicit spans/counters for composition traversal, validation metadata,
   array decode bytes, animation discovery, mesh preparation, subset construction,
   cache hits/misses/evictions, ECS publication and GPU preparation.
2. Exercise the actual editor open path with the same renderer configuration as
   the viewer. Keep the asset-instance benchmark as a separately named workload.
3. Build a per-scene-generation required-resource manifest covering meshes,
   images, materials, skin/morph resources and pipeline specializations.
4. Acknowledge readiness from the render world only when that generation's
   required assets are prepared and its required renderables participate in the
   relevant view queues. Exclude legitimately culled objects using explicit
   eligibility rules, not a requirement that every entity draw a pixel.
5. Associate acknowledgment with a submitted frame. Report readback and PNG
   encoding separately. Fixed warmup frames belong only to steady-state timing.
6. Record asset revision, camera matrix/projection, time, variants, load rules,
   purposes, refinement, resolution, shadows, MSAA and color configuration.
   Record renderer differences rather than hiding them.
7. Run native and Bevy sequentially on the same machine. Use at least five
   measured runs after a warmup; retain every sample and report median and range.
   Label cold-process/warm-filesystem separately from cold-storage experiments.
   Do not drop system caches or terminate other projects to manufacture results.

**Gate:** deliberately delayed mesh/image preparation must delay complete-frame
acknowledgment; missing/error assets must not produce false success. CPU-ready,
first submitted complete frame and PNG completion must be separate fields.
Run the core and capture test commands below, then both matched render runs.
Store a machine-readable report with the configuration and asset manifest.

## P1 — Reuse prepared material subsets

Material-resolution attribution: `MaterialResolveTimings`, enabled by the
benchmark's `USD_PROFILE_ROUTES`, separates binding lookup, shader reads,
preparation and interning. Progress reports include bounded distinct
binding/time/sidedness keys (65,536 maximum; saturation is explicit), successful
existing memo hits and binding/shader query errors. Key identity is stage-agnostic
and only measures reuse within this fresh single-stage benchmark.

Moana's 180-second diagnostic reaches a 216,064-prim checkpoint with 145,466
bound material requests, 9,234 distinct keys, no memo hits and no query errors.
Binding lookup takes 6.191 s; shader reads 20.769 s; preparation 0.294 s;
interning 0.759 s. These overlap MaterialRoute application and describe an
unfinished prefix, not an end-to-end result. 763 Make release tests pass,
19 ignored. Artifacts: `target/perf/p1-material-attribution/`.

Decoded-read reuse is now implemented for initial live projection. A stage-pinned
FIFO retains at most 4,096 successful `ReadPreviewMaterial` values, keyed by
binding path and time-code bits. This is an entry-count bound, not a byte budget.
A stage change sink marks the cache dirty; the next lookup discards prior reads.
Reads interrupted by a stage change are not inserted. Errors are not cached.
Binding resolution, sidedness, texture preparation, warnings derived from Bevy
assets and material interning still run live. The scope restores any surrounding
read cache, drops its own values and unregisters its sink after projection.

Five alternating fresh-process Oxbo pairs, profiling disabled:

| Mode | CPU-open samples (seconds) | Median |
| --- | --- | --- |
| Baseline | 2.210 / 2.189 / 2.184 / 2.171 / 2.171 | 2.184 s |
| Decoded-read reuse | 2.266 / 2.150 / 2.083 / 2.066 / 2.070 | 2.083 s |

The median is 4.6% lower; retain the slower first candidate rather than selecting
only favorable runs. Geometry, asset-count and cached-payload CSV fields match
across all ten runs. These are headless CPU opens, not first-complete-frame times.

At the same 216,064-prim Moana checkpoint, the sequential diagnostic records
145,466 bound requests and 9,234 distinct keys in both builds. Shader-read time
falls from 20.769 s to 1.310 s, with 136,232 decoded-read hits. Projection elapsed
is 68.712 s versus 91.178 s. This is matched-prefix attribution, not a repeated
end-to-end Moana benchmark. The candidate reaches 303,104 sampled prims before
the 180-second deadline (exit 124), with peak RSS 22,578,252 KiB and no swap.

766 Make release workspace/all-target tests pass (19 ignored). New regressions
cover shader edits during projection, time codes, independent stages, sidedness,
replacement texture handles, failed-read recovery, scope restoration, capacity
eviction and sink removal. Oxbo and Caldera captures are RGBA byte-identical to
their existing controls; Caldera's pre-existing color/noise artifacts remain,
so this is regression evidence rather than Blender/EEVEE parity. Evidence:
`target/perf/p1-material-reads/` and baseline executable/logs in
`target/perf/p1-material-attribution/`.

The longer 300-second Moana attempt does **not** time out: systemd scope
`run-p2020278-i39748898.scope` reports `Result=oom-kill`, a 24 GiB peak and
190.879 s wall time. The kernel identifies the benchmark PID and
`CONSTRAINT_MEMCG`; no higher memory budget was used. The last completed sampled
prim is 333,824, 102.145 s into projection, with 173,044 bound requests,
17,569 distinct keys and 155,475 decoded-read hits. There is no CPU-open CSV row.
Logs: `moana-300s.log` and `moana-300s-scope.log` in the same artifact directory.
The read cache did not resolve full-scene memory residency; do not describe this
as successful Moana loading or attribute the failing allocation to a specific
route without finer evidence.

**Scope:** `route/subset.rs`, `mesh/compact.rs`, `route/cache.rs`, related tests.

First increment removes the unconditional full-parent mesh clone from
`SubsetRoute::project`. Preparation holds a strong source handle and borrows the
current asset only while compacting each partition. Point-instancer prototypes
use the same path after interning their source mesh. Materials remain resolved
per context; missing source assets abort preparation without publishing partial
subsets. Compaction's existing malformed-layout fallback may still clone.
This is not the subset-product cache: repeated compaction remains outstanding.
P0 route attribution supports this change; P0's matched first-frame acceptance
is still open and remains mandatory for overall performance acceptance.

Verification: 575 focused release tests pass (19 ignored), including a new
source-mutation/removal regression and existing subset animation, skin/morph,
prototype and live-edit coverage. Evidence is in `target/perf/p1-borrow/`.
Caldera CPU opens measured 44.213 / 42.874 s; SubsetRoute application measured
3.035 / 2.999 s versus P0 read-profile observations of 3.428 / 3.397 s.
Geometry counts remain 50,125 mesh entities, 23,291 subset entities, 3,312 mesh
assets and 66,456,232 vertices. Oxbo opens measured 3.712 / 3.395 s, retaining
1,786 mesh entities, 14 subset entities and 1,597 mesh assets. These are two-run
diagnostics with an external Gearbox process active, not a controlled speedup
or first-frame acceptance. A fresh visual comparison remains outstanding.

Rejected experiment: a 64 MiB source/product snapshot cache, limited to 64
sources and 64 products per source, checked complete source and output equality
before returning weakly referenced assets. It passed 576 focused tests but
Caldera's SubsetRoute took 4.056 / 3.859 s enabled versus 3.556 / 3.089 s with
the same binary's cache disabled. Each enabled run avoided 6,890 of 42,402
compactions but evicted 16,103 source entries. Geometry counts stayed unchanged.
Whole-open times (46.107 / 44.242 s enabled, 47.450 / 44.264 s disabled) do not
prove a speedup; the experiment was sequential and externally contended.
The implementation was removed, not shipped. Logs and the rejected patch are
retained in `target/perf/p1-cache/`; its `USD_SUBSET_CACHE_BYTES` switch belongs
only to that experimental patch, not the current benchmark. Do not repeat this
snapshot-per-source design or increase budgets without new evidence. Next,
evaluate batched/sparse partition remapping and a reliable synchronous source
mutation token before another persistent product-cache attempt. Asset events
polled only between frames cannot invalidate mutations during one projection.

Sparse-remapping increment: `mesh/compact.rs` sorts/deduplicates selected vertex
indices and binary-searches their remapping when index count is at most one
eighth of the source vertex count. Scratch storage then scales with selected
indices instead of allocating/scanning parent-sized marking and remapping
arrays. Dense subsets retain the original path and ascending source-vertex
ordering. Both index widths, repeated/out-of-order indices, joint attributes
and target-major morph data are covered by the new regression test.

Verification: 576 focused release tests pass, 19 ignored. Caldera CPU opens
measured 49.294 / 42.498 s; SubsetRoute measured 3.118 / 3.013 s with unchanged
entity, mesh and vertex counts. Oxbo opens measured 5.297 / 4.471 s with
unchanged geometry counts. External Gearbox activity continued: these samples
do not establish a loading speedup. The memory benefit is the removal of
parent-sized scratch arrays for sparse selections, not reduced final residency.
The inspected Caldera capture completed in 65.14 s (not first-frame timing),
peak RSS 16,596,520 KiB, and its raw RGBA is byte-identical to
`/tmp/caldera-geometry-reuse.rgba`. That proves this change preserved the
baseline image, not full renderer fidelity. Evidence, logs, PNG and matching
SHA-256 hashes are under `target/perf/p1-sparse/`.

1. Resolve per-instance materials separately from immutable subset geometry.
2. Check a subset-product cache before cloning the parent or compacting it.
   Key by source asset identity plus a reliable mutation generation, face
   partition and layout. Asset ID alone is insufficient.
3. Split read-only geometry preparation from World-mutating asset installation,
   avoiding the full-parent clone where borrowing/ownership permits.
4. Batch partition construction or use sparse remapping when beneficial;
   preserve the established vertex ordering, skin attributes and morph layout.
5. Keep the existing invalid/overlapping-subset fallback and empty remainder
   behavior. Do not share contextual materials under a geometry-only key.

**Gate:** core tests pass; repeated identical subsets show cache hits with zero
new compaction calls after the first preparation. Mutations/removal, sampled
partitions, holes, both orientations and CPU/GPU deformation invalidate or fall
back correctly. Compare Caldera/Oxbo captures and SubsetRoute CPU time against P0.

## P2 — Separate deformation structure from changing poses

GPU influence increment: adjacent points with identical influence indices and
bit-identical weights reuse their normal-correction matrix within one sampled
pose. The one-entry memo borrows input slices and does not retain stage data,
poses or mesh assets. Flat-normal meshes skip the comparison. Weight validation
still runs for every point; the first occurrence still performs singular-matrix
and finite-result checks. This does not implement cross-frame structural reuse.

Verification: 584 focused release tests pass, 19 ignored. The new test checks
repeated, changing and alternating influences at five times against uncached
matrix calculations; existing CPU/GPU normal, tangent and fallback tests pass.
Caldera CPU opens measured 53.463 / 48.431 s, with SkinRoute 4.864 / 4.438 s and
unchanged entity/asset/payload counts versus the FIFO baseline. These diagnostic
runs do not establish a loading speedup. Evidence is in
`target/perf/p2-influences/`. The Caldera capture is byte-identical to the texture-
manifest baseline (66.41 s process duration, 15,287,396 KiB peak RSS). The animated
skin/morph fixture also completed two runs of 1,000 distinct timeline seeks with
unchanged asset counts; seek medians were 714 / 683 microseconds. Neither capture
duration nor this small-fixture timing proves the large-scene first-frame target.

First increment reuses SkinRoute's local skin/morph classification in CPU
fallback preparation and GPU attachment. CPU normal preparation no longer
decodes default joint weights a second time just to repeat `is_skinned`.
Standalone prototype deformation also evaluates classification once per call.
Influence discovery uses `num_time_samples()` rather than materializing sample
timestamps; the vendored implementation requests the composed sample count,
including clips, and retains the existing nonzero-count semantics.
No answer survives a projection call: live edits, independent roots/clocks,
binding resolution and pose-dependent normal/tangent corrections remain intact.
Persistent structural-query reuse and palette-only updates remain TODO.

Verification: 576 focused release tests pass (19 ignored), covering sampled
influences without defaults, CPU/GPU deformation and bounds. Two CPU and two
GPU-prepared `assets/skel_influences.usda` editor runs each completed 1,000
timeline seeks. Raw timings, asset counts, test/build logs and patch evidence
are in `target/perf/p2-classify/`. These small-fixture runs exercise the paths;
they do not establish a large-scene speedup or GPU first-frame acceptance.

Geometry-reuse increment: CPU skin/blend preparation and CPU/GPU morph sampling
now accept the route's already decoded `ReadMesh`. CPU normal-target sampling
uses the same original sample, not a second decode or the deformed point array.
Standalone public readers retain their existing stage-reading behavior;
standalone skinning still checks bindings/influences before decoding geometry.
No persistent cache or cross-frame ownership was added. The skinning path also
borrows unmorphed base points rather than allocating another fallback copy.

Verification: 577 focused release tests pass (19 ignored). A new five-pose
comparison checks borrowed versus standalone skin results and morph position,
normal-target and weight arrays for skin-only and combined animated fixtures.
Two CPU and two GPU-prepared runs of `skel_morph_blended_animated.usda` each
completed 1,000 timeline seeks with stable peak asset counts. Logs, patch and
test/build evidence are in `target/perf/p2-mesh/`. Registry mesh-read counters
still exclude independent standalone reads, so those counters alone are not
proof of the removed duplicate reads. No large-scene speedup is asserted.

CPU palette-sharing increment: a sample now owns the remapped joint palette,
skinning resolver and sampled influence arrays alongside its deformed points.
CPU normal preparation consumes that same sample instead of rediscovering the
binding, rebuilding skeleton topology/inverse binds, resampling animation and
rereading influences. Rigid normal transforms are evaluated lazily once per
sample, while vertex-weighted normals retain their per-joint lazy palette.
The duplicate joint-palette remap used only to obtain its length is removed.
All reuse is sample-local; no pose, resolver or influence data survives a route
call. Persistent structural reuse and GPU palette-only updates remain open.

Verification: 577 focused release tests pass (19 ignored). Borrowed/standalone
normal parity now covers five poses each for vertex influences, constant
influences and combined animated skin/morph geometry. All four ignored
`native_baked_` position/normal tests pass against the existing native OpenUSD
tool. Two CPU fixture runs each completed 1,000 timeline seeks, with medians
631 / 602 microseconds and stable peak asset counts; this is not a controlled
speedup comparison. Evidence is under `target/perf/p2-palette/`. Native gate:
`USD_NATIVE_DEFORMATION_TOOL="$PWD/target/sample-native-deformation" make test
CARGO='cargo --offline' APP_TARGET='--release -p usd_bevy --lib native_baked_ --
--ignored --nocapture'`. This focused deformation gate is not `make test-native`
(which currently covers persistence exports) or full-workspace acceptance.

**Scope:** `read/skel.rs`, `route/skel.rs`, `route/gpu_skin.rs`,
`route/gpu_morph.rs`, `live.rs`, focused existing deformation fixtures/tests.

1. Count negative classification separately from actual skin/morph preparation.
   Cache enclosing SkelRoot, binding and no-binding results within a valid stage
   revision. Avoid reading full weight arrays just to repeatedly classify meshes.
2. Use sample-count queries where only sample existence is required. Preserve
   sampled-only influences, authored blocks and inherited binding semantics.
3. Retain immutable skeleton topology, joint mappings, influence packing and
   vertex mappings separately from per-instance pose-dependent state.
4. Add dependency-specific invalidation: topology/influences, pose, morph
   structure, morph weights, normal/tangent corrections and material changes.
5. Use palette-only updates only for cases proven compatible with current
   normal/tangent semantics. Keep singular-transform checks and CPU fallback.
6. Avoid equal-value joint-global writes and skip unaffected skins when neither
   their effective global transform nor skin state changed.
7. Investigate retained upstream `AttributeQuery` objects for repeated timed
   samples. Retain queries rather than cloning them each frame: their clone
   starts with an empty memo. Value clips retain their full-resolution fallback.

**Gate:** core tests and existing native deformation tests pass. Fixed-time idle
updates produce no unnecessary joint writes. Eligible pose-only seeks do not
rebuild mesh buffers. Sampled influences, nonuniform scale, morph combinations,
normal corrections and independent root clocks preserve existing output.

## P3 — Remove redundant value materialization safely

### Shared immutable SdfPath text trial

The first trial changed private `Path` storage from `String` to `Arc<str>`,
sharing cloned text without a global interner. Derivations
still create independent paths; validation, hashing, ordering, display and serde
remain content-based. This differs from the rejected per-node property-path cache:
it changes ownership of immutable text, not query caching or invalidation.
The trade-off is an atomic reference count and a copy when converting an owned
String into shared storage.

Five alternating fresh-process Moana `stage_benchmark ... proxies` runs under
24 GiB/no-swap, with no own concurrent builds, preserve **416,549 prims and 459
layers** in every row. Baseline traversal milliseconds:
`19214.989,19242.988,19190.095,19025.213,18868.766`; candidate:
`16975.994,16857.449,16996.987,16856.918,16810.543`. Median traversal falls
**19,190.095 → 16,857.449 ms (12.2%)**. Median post-traversal RSS falls
**12,303,516 → 8,343,096 KiB**, saving **3,960,420 KiB (3.78 GiB, 32.2%)**.
Raw rows, process peak RSS and saved binaries are under
`target/perf/p3-shared-sdf-paths/`. This excludes validation, Bevy projection,
textures, GPU preparation and UI; it is not a completed Moana viewer load or
the ≤5× first-frame acceptance. Full regression/native export and rendered
controls must pass before accepting this vendor increment.

**Rejected representation, not discarded ownership strategy:** the `Arc<str>`
Oxbo control regressed consistently: median CPU open 2,061.196 → 2,117.033 ms
(2.7% slower), despite unchanged geometry counts. The follow-up representation
in `patches/openusd-shared-path-text.patch` uses **`Arc<String>`**, retaining
owned input buffers rather than copying them into a new `Arc<str>` allocation.
The public path API and content-based semantics remain unchanged; the private
representation is smaller, but unique paths retain a separate String allocation.
The path regression test checks input-buffer reuse as well as clone sharing.

Five fresh alternating Oxbo controls for this representation record baseline
milliseconds `2040.775,2051.421,2049.000,2061.587,2043.151` versus candidate
`2019.656,2010.308,2027.707,2011.150,2022.259`: medians
**2,049.000 → 2,019.656 ms**. This removes the observed regression; the small
1.4% reduction is not the primary acceptance argument. All geometry, asset and
payload counts match. A preliminary Moana traversal uses 8,072,812 KiB RSS;
the repeated Moana trial and final validation below must supersede this one run.
Artifacts are `owned-oxbo-*.log`, `arcstring-stage-1.log` in the same directory.

The final five alternating Moana pairs for `Arc<String>` preserve 416,549 prims
and 459 layers. Baseline traversal milliseconds
`19015.276,18943.741,18804.090,18828.552,18861.525`; candidate
`17262.340,17217.293,17228.653,17367.598,17260.008`. Median traversal falls
**18,861.525 → 17,260.008 ms (8.5%)**; median post-traversal RSS falls
**12,303,432 → 8,071,992 KiB**, saving **4,231,440 KiB (4.04 GiB, 34.4%)**.
Use these accepted-representation measurements rather than the faster but
Oxbo-regressing `Arc<str>` numbers. Raw results: `owned-moana-*.log`.

Final `Arc<String>` validation: 768 workspace/all-target release tests pass
(19 ignored), 14 native export tests pass, and 40 upstream path tests pass with
serde enabled, including clone sharing, owned-buffer preservation, derivations,
content hashing and string/empty-path serialization. Run the standalone path
gate through Make with
`CARGO_WORKSPACE_DIR="$PWD/vendor/openusd/" CARGO_BUILD_JOBS=4 make test CARGO='cargo --offline' APP_TARGET='--manifest-path vendor/openusd/Cargo.toml --release -p openusd --lib sdf::path::tests --features serde'`.
The command needs that workspace environment variable even with a path-only
filter because the complete upstream test module compiles; external fixture
tests are not part of this 40-test gate. Its generated standalone Cargo.lock is
not retained. Final logs use the `owned-` prefix. Fresh Oxbo and Caldera captures
match their retained RGBA controls byte-for-byte; Caldera's known visual defects
remain unchanged. No Blender parity or complete-frame claim follows from these
regression checks. The full Moana editor workload must be rerun separately.

The first full-editor retest after `57173b1` still hits the 24 GiB/no-swap limit:
`moana-editor-scope.log` records `Result=oom-kill`, 191.326 seconds wall time and
25,769,803,776-byte peak. `moana-editor.log` reaches the coarse 334,848-prim
checkpoint at `/island/isBeach/geometry/xgShells`, but has no per-route trace in
this attempt; it does not establish that shells itself is the fatal allocation.
The stage-only saving remains valid and must not be described as a completed
Moana load. A follow-up with `USD_PROFILE_MEMORY=1` and the bounded
`USD_PROFILE_MEMORY_RANGE=334848:338943` attributes the subsequent region rather
than guessing from the last coarse checkpoint.

Traversal-local parent status: `DEFAULT_PROXIES` now carries a population-epoch
witness from a matching parent to queued children. With unchanged population
and no authored load rules, each child resolves its own active/specifier opinions
instead of walking proven ancestry again. Other predicates and load rules retain
the full queries. Visitor edits, pending changes and lazy-load retries must pass
the epoch check before reuse. No witness survives the traversal call.
Patch: `patches/openusd-traversal-parent-status.patch`.

Five alternating baseline/candidate traversal-only Moana pairs, same asset and
`USD_PROFILE_LOADING=1`, all visited 416,549 prims:

| Mode | Traversal samples (seconds) | Median | Peak RSS range (KiB) |
| --- | --- | --- | --- |
| Baseline | 29.621 / 24.646 / 24.535 / 24.417 / 24.493 | 24.535 s | 15,462,476–15,481,368 |
| Parent status | 22.638 / 22.260 / 22.203 / 22.155 / 22.286 | 22.260 s | 15,478,488–15,480,112 |

This is a **9.3% lower median for the traversal phase**, not an initial-load or
first-frame result. Runs intentionally terminate after the traversal marker;
their RSS is not full-load peak memory. Each used a fresh process under a
24 GiB/no-swap scope; filesystem caches were not flushed. Other host workloads
were not disabled, and the slower first baseline is retained in the table.
Raw logs, retained executables and the stop script are under
`target/perf/p3-inherited-status/`. Earlier wrapper-shell measurements under
`invalid-wrapper/` are excluded: stopping the wrapper left benchmark children
running. Those owned processes were terminated; the replacement uses `/bin/bash`
and verifies no benchmark remains after each sample.

763 Make release workspace/all-target tests pass (19 ignored), with 14 native
export checks run separately. The new differential test compares an uncached
walk with the optimized traversal across 36 combinations of masks, initial load
rules and visitor edits, including ancestor def/class/over changes, deactivation,
queued siblings, referenced instances, lazy payloads and newly authored children.
Oxbo capture succeeds and is RGBA byte-identical to
`target/perf/oxbo-current/original.rgba`; this is regression evidence, not full
Blender/EEVEE parity.

The separate 180-second full-open attempt still exits 124 without an initial-open
CSV row. Validation completes at 55.985 s, texture decoding at cumulative
77.550 s, texture installation at 82.039 s, and the inspector reports 5.426 s.
Peak RSS reaches 21,344,448 KiB with no swap. This is not OOM or successful Moana
acceptance; the remaining update is not attributed by the current end-of-update
route report. Full-load log: `target/perf/p3-inherited-status/moana-full.log`.

Property-classification batching: each prim now classifies its property names
inside one mask-gated cache query, instead of entering the stage query layer for
every property. Authored spec types are still read live; composed-only schema
properties still use their declaration. There is no cross-call classification
cache. Patch: `patches/openusd-property-classification.patch`.

762 Make release workspace/all-target tests pass (19 ignored), plus 14 explicit
native export checks. The new regression covers schema-only attributes,
authored relationships, missing/masked prims, added properties, and relationship-
to-attribute edits propagated to a referencing instance. The Oxbo capture is
RGBA byte-identical to `target/perf/oxbo-current/original.rgba`.

Moana's diagnostic still visits 416,549 prims and inspects 6,400,861 attributes.
Its attribute phase was 35.293 s versus 36.438 s in the previous unmodified
getter-profile run; total validation was 61.585 s versus 63.036 s. Getter totals
also changed slightly, so this is a small diagnostic observation, not an
established end-to-end speedup. The code removes per-property stage-query entry
without introducing a result cache; the remaining per-property source walk is
unchanged. The 110-second run still times out, with peak RSS 19,990,740 KiB under
the 24 GiB/no-swap cap. Evidence: `target/perf/p3-property-types/`.

Rejected per-node property-path trial: the value-resolution walk temporarily
retained one node/path pair and lent that property path to successive layer
sites of the same node. It kept no cross-query cache and passed all 761
workspace/all-target tests (19 ignored), but did not isolate a useful timing
improvement on Moana. Sample checks measured 11.529 s versus 11.781 s; unrelated
default/type reads also improved (7.703/4.674 s versus 7.908/4.886 s), while
traversal and total validation worsened (29.340/65.097 s versus 26.593/63.036 s).
The diagnostic still timed out at 110 seconds with unchanged 416,549 prims and
6,400,861 attributes. This is insufficient evidence for the added borrowed-site
representation, not proof that property-path allocation is free.

The trial is removed from source; its patch and logs are retained under
`target/perf/p3-site-path/`, including `rejected.patch`. The normal benchmark
is rebuilt after restoration. No dependency refresh, content pruning or
validation shortcut was retained. Next work should measure/optimize the broader
property-source walk rather than reintroducing this local cache on assumption.

Validation getter attribution: `USD_PROFILE_VALIDATION_VALUES=1` adds separate
timers for default-value reads, declared-type lookup, and sample checks. For
asset-valued attributes, the sample group includes reading each authored sample;
other attributes use the existing count query. This extra instrumentation is
off by default and adds per-attribute timer overhead when enabled. Traversal,
attribute enumeration and prim metadata are outside these three groups.

Moana's 6,400,861 attributes measured 7.908 s default reads, 4.886 s type lookup,
and 11.781 s sample checks. The full attribute phase was 36.438 s, leaving about
11.864 s of enumeration, per-prim metadata, loop and instrumentation work outside
the measured getter groups. The existing dependency already implements
`num_time_samples` via `Want::Count`; ordinary sample maps do not need a times
vector for that query. Do not implement a duplicate count-only API. Investigate
the source-resolution walk and its default/sample reads; clip-count queries still
have a separate schedule-materialization path. Preserve source-strength, blocks,
clip discovery and decode-error semantics instead of blindly skipping reads.

The Make release workspace/all-target gate passes 761 tests (19 ignored); the
benchmark builds. The bounded 110-second Moana diagnostic still times out,
without a completed CPU-open row. Raw timing/build/test logs are retained under
`target/perf/p3-validation-values/`. This is attribution with extra timing
overhead, not a loading-speed improvement or a first-frame result.

Combined specifier-status increment: status queries requesting both DEFINED and
ABSTRACT now resolve ancestor specifiers once for both answers. Individual-bit
queries keep their existing implementations. The combined walk only stops when
both answers are settled; nothing is retained across edits or traversal callbacks.
Review patch: `patches/openusd-specifier-status.patch`.

761 Make release workspace/all-target tests pass (19 ignored). The existing
ancestry differential regression now checks combined status against individual
defined/abstract queries, including masks, instance proxies and class-to-def
edits. Moana's bounded diagnostic retains 416,549 prims and 6,400,861 attributes.
Traversal measured 24.468 s versus 31.625 s after the abstract-walk change;
total validation measured 58.890 s versus 66.833 s. These are sequential
diagnostics, not controlled paired or first-frame acceptance. The 110-second
run still exits 124 without completed CPU open, at 19,990,932 KiB peak RSS
within the 24 GiB/no-swap cap. Evidence: `target/perf/p3-specifier-walk/`.
All 14 explicit native export checks pass, and the new Oxbo capture is RGBA
byte-identical to `target/perf/oxbo-current/original.rgba`.

Abstract-ancestry increment: `Prim::is_abstract` now delegates its ancestor
specifier walk to one mask-gated composition-cache query, rather than repeating
stage query/settlement for each ancestor. It retains the same composed field
resolution and no result survives the query. Review patch:
`patches/openusd-abstract-ancestry.patch`.

761 Make release workspace/all-target tests pass (19 ignored). Differential
checks compare with individual ancestor field reads for classes, undefined and
missing prims, instance proxies, population masks and live class-to-def edits.
The 110-second Moana diagnostic retains 416,549 prims and 6,400,861 attributes;
traversal measured 31.625 s versus 33.908 s after active-state reuse. Attribute
checks stayed effectively unchanged at 35.204 s versus 35.186 s. Total validation
was 66.833 s versus 69.099 s. These are diagnostic, sequential comparisons,
not matched first-frame acceptance or a demonstrated general loading ratio.
Moana still exits 124 without finishing CPU open. Artifacts:
`target/perf/p3-abstract-walk/`. Larger reductions still require eliminating
remaining repeated ancestry and value-source queries.
All 14 explicit native export checks pass; the new Oxbo capture completes and
is RGBA byte-identical to `target/perf/oxbo-current/original.rgba`.

Traversal active-state increment: a prim-status query requesting both ACTIVE
and LOADED now reuses its computed active result instead of walking active
ancestry again inside `is_loaded`. Payload/load-rule checks remain unchanged;
loaded-only predicates retain their own active query. Reuse is limited to one
status evaluation, with no cache across prims, visitors or edits. Review patch:
`patches/openusd-traversal-active.patch`.

The full Make release workspace/all-target gate passes 760 tests (19 ignored).
The added regression compares loaded-only and ACTIVE+LOADED traversal against
direct loaded queries through repeated payload load/unload and active edits.
A 110-second bounded Moana diagnostic visits the same 416,549 prims and checks
the same 6,400,861 attributes. Traversal measured 33.908 s versus the preceding
40.534 s; total validation measured 69.099 s versus 77.423 s. These are sequential
diagnostics, not controlled paired performance acceptance. The run still exits
124 without a complete CPU-open row; peak RSS is 19,988,312 KiB, under the same
24 GiB/no-swap cap. Artifacts: `target/perf/p3-active-reuse/`.
All 14 explicit native export checks pass. The current Oxbo capture completes
and is RGBA byte-identical to `target/perf/oxbo-current/original.rgba`.
This removes one redundant ancestry walk, not the remaining inherited-status
walks or per-attribute validation work; Moana completion remains outstanding.

Mesh-read increment: the five inherited primvar owner searches now walk each
ancestor together, obtaining one prim handle per level instead of one per
attribute. Owner resolution preserves local/constant inheritance, blocking and
fallback behavior; value reads still evaluate both UV candidates and propagate
errors. The batching is local to one mesh read, with no cross-stage or cross-edit
memo. The single-owner reader remains the independent test reference and serves
other callers.

Verification: the release workspace/all-targets gate passes 739 tests in 34
suites, 19 ignored. A nested hierarchy test compares batched and separate walks
for missing, blocked, local and nonconstant inherited values before/after a
metadata edit; existing inherited geometry tests pass. Caldera CPU opens measured
41.378 / 40.689 s, with mesh reads 5.644 / 5.682 s and unchanged read counts,
decoded bytes, entities, assets and retained payloads. These are diagnostic runs,
not a controlled speedup or first-frame result. Evidence is in
`target/perf/read-owners/`. The capture is pixel-identical to the GPU-influence
baseline, at 59.60 s process duration and 15,277,036 KiB peak RSS.

Paired A/B follow-up: separately built `b20f599` and `cb39477` benchmark binaries
were run in alternating before/after order, five separate processes per variant
and asset, with identical profiling and 24 GiB/no-swap limits. The current source
and normal benchmark binary were restored before measurement. Binary hashes,
raw logs, per-run load/background-build snapshots and calculated results are in
`target/perf/read-owners-ab/`.

| CPU workload | Before median (range), s | After median (range), s |
| --- | --- | --- |
| Oxbo open | 3.135 (3.125–3.425) | 3.115 (3.105–3.122) |
| Caldera open | 41.807 (41.338–46.756) | 41.670 (41.318–42.700) |
| Caldera mesh reads | 5.814 (5.787–6.388) | 5.697 (5.651–5.722) |

All entity, mesh, vertex and payload counts match across variants. The observed
median open reductions are only 0.62% on Oxbo and 0.33% on Caldera. Other project
builds occurred during some samples; this is not an isolated-machine experiment
or statistical evidence of an end-to-end speedup. Retain the local reduction in
owner-walk work, but do not use the earlier 53-to-41-second diagnostic difference
as its benefit. These results make further owner-walk micro-optimization a low
priority: focus subsequent work on duplicate preparation before decoding,
bounded parallel preparation and the still-missing complete-frame gate.

Snapshot-proof increment (2026-09-17): `UsdSource` retains a bounded shared
default-composition validation stamp keyed by its exact source revision.
Successful dependency probes record it only when no dependencies are missing;
fresh default instances can also record it after strict validation. Every
source-byte/dependency mutation changes the revision, so the stamp cannot
authorize a different snapshot. Filesystem-backed sources never qualify.
Instances with any variant or attribute override still validate exhaustively.
Each instance still opens its own mutable stage, applies overrides and resolves
textures. This removes repeated validation, not initial strict validation or
the lazy composition work still needed by each independent stage.

`UsdSceneTimings::validation_reuses` counts reuse. `USD_PROFILE_SOURCES=1`
reports it in `source_benchmark` and in the capture's `capture_source_profile`.
Tests cover revision/dependency replacement, failed/missing sources, filesystem
exclusion and overridden instances. 579 focused tests pass (19 ignored), plus
the source benchmark's integration test. Three benchmark samples with 128
prototypes per root report three validation reuses for four-root loads and
source replacements, preserving entity/runtime tags and asset sharing.

The inspected Caldera capture reports one loader-proof reuse, zero repeated
instance-validation time and 30.630 s projection time. It is pixel-identical
to the P1 sparse-remapping baseline. Process-to-exit was 73.71 s versus that
earlier run's 65.14 s; peak RSS was 16,801,432 KiB. This is not a speedup or
matched first-frame claim. Evidence, source benchmark, tests/build logs and
RGBA hashes are in `target/perf/p3-proof/`. Initial validation value decoding,
composition/dependency attribution and the wider integration gates remain open.

**Scope:** `source.rs`, `asset.rs`, `editor.rs`, dependency/validation tests;
upstream parser changes require the separate P7 boundary.

1. Split measured composition traversal, dependency discovery, metadata checks
   and exhaustive value decoding. Do not attribute all validation time to waste.
2. Keep arc/offset validation, missing-dependency discovery and value-clip and
   asset-valued sample handling. Validate consumed geometry before publication.
3. Preserve strict exhaustive validation as an explicit mode if normal opening
   stops decoding unused values. Document any changed error-reporting timing;
   do not silently weaken the existing strict contract.
4. Produce an internal validation/dependency manifest bound to immutable source
   revision and composition settings. Reuse only when those inputs match.
5. Keep independent mutable stages. Revalidate affected composition for overrides,
   variants, load/mask/mute changes and manually constructed unvalidated assets.

**Gate:** corrupt/missing assets and invalid offsets still fail transactionally;
last-good scenes remain intact. Dependency repair and nested-package reload tests
pass. Counters show removed duplicate array decoding and manifest reuse without
skipping necessary composition. Benchmark editor and asset-instance paths separately.

## P4 — Separate logical prims from initial geometry residency

Initial opt-in increment, 2026-09-17: `route::residency::DeferHiddenMeshes`
defers Mesh/Material/Skin/Subdivision/Subset routes for initially hidden Mesh
prims in the built-in registry. Logical hierarchy and non-render metadata remain
projected. Promotion reads the current stage, clock and settings. Resident meshes
are not evicted. Registering custom routes disables deferral; removing the
resource materializes pending geometry. Both LiveStage and asset-instance paths
have promotion hooks; instance roots now also respond to DisplayPurposes changes.

Validation: 743 workspace/all-target release tests passed, 19 ignored. New cases
cover independent roots, hidden geometry edits, clock changes before reveal,
on/off/on handle retention, disabling the feature, purpose changes and initial
custom-route fallback. Make-built benchmark and capture examples expose the
opt-in environment flag; no default behavior is changed.

Same-binary diagnostic CPU comparison (two opens per mode, deferred then eager):

| Caldera | Eager | Deferred |
| --- | --- | --- |
| CPU open samples | 42.297 / 40.652 s | 26.089 / 24.272 s |
| Retained mesh entities | 50,125 | 8,111 |
| Retained subset entities | 23,291 | 209 |
| Unique mesh assets | 2,509 | 188 |

These samples show approximately 16 seconds less initial CPU work, not a
five-process alternating acceptance benchmark or first-complete-frame result.
The deferred offscreen capture completed in 41.18 s with peak RSS 15,258,772 KiB;
its RGBA is byte-identical to `target/perf/p4-cpu/caldera.rgba` (SHA-256
`02377dd93ade5d9955a330b1e49d80822671c089d256df4fe4cb77712f56117f`).
The inspected image retains the existing baseline's color/noise artifacts;
identity proves non-regression for this view, not Blender fidelity. Raw logs and
images are in `target/perf/p4-defer/`.
Oxbo deferred CPU opens were 3.558 / 3.106 s; this is a smoke measurement,
not an established improvement or a matched regression bound.

Follow-up regression gate: 746 workspace/all-target release tests pass, with
19 ignored (`target/perf/p4-defer/reload-tests.log`). A source asset replacement
while hidden preserves the matching entity and its runtime component, performs
no mesh preparation, then reveals the replacement geometry. Runtime registration
of a custom route materializes existing deferred meshes before that route runs.
Animated ancestor visibility promotes at the current instance clock and retains
residency when the clock returns to the hidden sample.

Alternating fresh-process Caldera CPU comparison: five processes per mode,
one open each, same binary, round order E/D, D/E, E/D, D/E, E/D. All ten runs
completed under the 24 GiB/no-swap cap; raw logs and binary/environment identity
are retained in `target/perf/p4-defer/paired/`.

| Mode | Median CPU open | Range | Peak RSS range, KiB |
| --- | --- | --- | --- |
| Eager | 41.880 s | 41.542–42.221 s | 6,093,836–6,169,256 |
| Deferred | 25.316 s | 25.079–25.762 s | 5,293,364–5,346,416 |

Median CPU loading decreased 39.6% (1.65× faster) in this comparison. OS page
caches were not flushed; desktop/background processes remained active, but no
concurrent Cargo/rustc build was observed at the recorded checkpoint. This is
a repeated warm-filesystem CPU result, not cold-disk, GPU first-frame, native
reference or reveal-latency acceptance. It does not establish the ≤5× target.

Native filesystem follow-up: an actual write to a referenced sublayer now has
an explicit deferred-mesh regression test. Two independently clocked roots retain
their matching entities/runtime components and remain unprepared through reload;
revealing one root produces the new geometry while the other stays deferred.
The test passed with `--features file_watcher`; a filtered invocation without
that feature runs zero tests and is not evidence. All four native watcher tests
passed together, including existing missing-root recovery, removed-dependency
invalidation and layer/texture recovery. The feature-enabled library suite
passed 575 tests with 28 ignored; the ordinary workspace/all-target suite passed
746 with 19 ignored. Logs: `target/perf/p4-defer/native-deferred-watcher.log`,
`native-all-watchers.log`, `watcher-feature-tests.log` and `watcher-build-tests.log`.

```sh
CARGO_BUILD_JOBS=4 make test CARGO='cargo --offline' \
  APP_TARGET='--release --workspace --lib --features file_watcher native_file_watcher_ -- --ignored --nocapture'
```

Deferred preparation latency increment: `editor_benchmark` accepts
`USD_BENCH_PREWARM_HIDDEN=1` together with `USD_DEFER_HIDDEN_MESHES=1`. After
measuring initial open and idle, it removes the deferral resource and times one
update preparing all remaining deferred meshes. It reports before/after pending
counts, mesh-asset counts, RSS and separate `deferred-prewarm` route counters,
and errors if any markers remain. CSV asset payloads then describe the post-
prewarm state; initial open/idle timing remains separate. This measures the
all-deferred CPU workload, not a particular visibility edit or GPU reveal frame.

The first Caldera runs exposed randomized HashMap promotion order: preparation
took 28.278 / 28.356 s and ended with 3,315 / 3,390 mesh assets. Sorting promotion
paths in namespace order reduced this to 17.023 / 16.841 s, with 2,854 mesh assets
in both runs. MeshRoute application dropped from about 18.6 s to 7.9 s in the
first respective samples. All runs moved 18,932 deferred markers to zero and
ended with 50,125 mesh entities / 23,291 subset entities. The locality improvement
does not eliminate retained copies or establish image equivalence for every
hidden asset. These are two-run diagnostics, not the paired startup benchmark.

746 workspace/all-target tests pass (19 ignored). Logs are retained under
`target/perf/p4-prewarm/`: `caldera.log` before ordering, `caldera-ordered.log`
after ordering, and `ordered-tests.log`. Preparing every hidden mesh still
blocks for about 17 seconds: bounded preparation/publication remains necessary;
do not characterize deferred startup as eliminating the work.

Remaining before default enablement: variant/dependency closure cases,
deferred failed-save recovery and visibility-specific reveal latency. Direct application edits
to Bevy Visibility outside USD are not currently a promotion trigger. Non-Mesh
geometry remains eager. Asynchronous jobs and hard per-frame bounds remain
unimplemented; the optional cooperative scheduler below is not a substitute.

Cooperative promotion increment: `UsdResidencyBudget(Duration)` opts into a
shared soft route-preparation budget, reset in the app's First schedule. With
no resource, promotion stays synchronous. `UsdMeshPreparationQueued` identifies
requested but not yet prepared entities; `UsdDeferredMesh` remains until their
geometry routes run. LiveStage and asset-instance updates resume queued work
under the appropriate stage/time/textures. Queue entries are ECS markers, not
decoded geometry or mutable Stage objects sent to threads. Each attempt reads
the current stage and clock; re-hidden entries are removed from the requested
queue without losing their deferred marker. Custom registries retain eager
execution. Zero budget advances one mesh per update globally across roots.

The benchmark accepts `USD_RESIDENCY_BUDGET_MS=10` alongside the deferral and
prewarm flags. Prewarm now drains over updates with a 120-second bound, reporting
update count, p95 and maximum duration separately from total preparation time.
748 workspace/all-target tests pass, 19 ignored, including shared budget across
three roots, current-clock preparation, re-hidden cancellation and budget
removal. Evidence: `target/perf/p4-budget/tests.log` and `caldera-10ms-final.log`.

**Experimental, not default-ready:** the final Caldera diagnostic drained all
18,932 meshes in 1,552 updates / 49.980 s, with update p95 45.645 ms and maximum
2,232.132 ms despite a requested 10 ms route budget. Final mesh/subset entity
counts remain 50,125 / 23,291; peak RSS was 5,709,748 KiB. An earlier version
measured 47.904 s, so removing repeated queue-marker writes and exhausted-pass
sorting did not establish a throughput improvement. Both are substantially
slower in total than the ~17-second synchronous prewarm. One prim remains
indivisible; sorting/scanning and ordinary update work sit outside the route
budget. Investigate these costs and split heavy preparation before making any
hard latency claim. Fair scheduling among roots and viewer loading/readiness
presentation remain follow-up work; logical Ready is not a rendered-frame gate.

Asset-root fairness increment: instance ticking orders root entities and starts
after the last root that actually consumed promotion work. Exhausted or idle
roots do not advance that cursor. The shared preparation budget therefore rotates
among queued asset roots instead of repeatedly favoring the same HashMap entry.
Stage clocks, textures and existing per-root synchronization remain installed
only for the root being processed. Initial projection's separate scheduler is
unchanged. Fair arbitration between a standalone LiveStage and asset roots in
the same app remains outside this increment.

755 workspace/all-target tests pass (19 ignored). A zero-budget regression with
three roots and three hidden meshes per root verifies that every three updates
prepare exactly one additional mesh in each root, rather than draining one root
first. Existing shared-budget, latest-clock and cancellation checks also pass.
Evidence: `target/perf/p4-fairness/tests.log`. This is a scheduling correctness
change, not a new single-scene loading-speed claim.

Retained promotion queue increment: each stage root now owns an ordered deque
of entity IDs instead of rescanning queued entities, cloning their paths and
sorting them on every update. Demand changes rebuild ordering; execution reads
the entity's current path from its owning map and projects the current stage
and clock. Despawned, unmapped, cancelled and rehidden entries are discarded.
Root despawn drops the queue; rootless maps retain the reconstruction fallback.
No decoded geometry, source snapshot or asset handles are retained by this queue.
Storage scales with pending entity IDs, not path lengths or mesh payloads;
allocator/capacity overhead has not been measured separately.

The current-tree Make release workspace/all-target gate passes 756 tests with
19 ignored across 34 suites. The added despawn regression ran as a Rust test,
not fixture text, and verifies all surviving roots finish and queues disappear.
Existing latest-clock, cancellation and root-fairness checks pass as well.
Evidence: `target/perf/p4-retained-queue/tests-final.log`.

Caldera's first diagnostic run with the same 10 ms soft preparation budget
drained 18,932 meshes in 18.076 s (1,541 updates), versus the preceding texture
index run's 21.514 s (1,525 updates). Update p95 was 15.346 ms versus 16.862 ms;
maximum was 102.854 ms versus 111.069 ms. Final mesh assets remained 2,369.
Peak process RSS was 5,671,228 KiB under the 24 GiB/no-swap cap. Logs are
`target/perf/p4-retained-queue/caldera.log` and
`target/perf/p8-texture-index/caldera.log`. This is an approximately 16% CPU
promotion reduction in a historical single-run comparison, not a controlled
paired speedup, memory reduction, reveal-frame result or the ≤5× acceptance.
A second fresh-process run (`caldera-repeat.log` in the retained-queue directory)
drained all 18,932 entries in 17.840 s across 1,525 updates, with 15.006 ms p95
and 119.368 ms maximum. Final mesh assets again numbered 2,369. This supports
the CPU trend but does not establish improved worst-case latency: the largest
update included 107.279 ms of reload processing outside the preparation budget.
Discovery, queue rebuilding and other update systems remain outside the soft
route timer; individual mesh preparation remains indivisible.

Palette-update follow-up: opt-in `GpuSkinUpdateTiming` exposed 929,252 generated
joint transforms rebuilt on every eager Caldera update. Over 100 idle updates,
the old pass processed another 92,925,200 joints and spent about 3.26 s inside
the pass. `gpu_skin::update_joint_globals` now uses Bevy change tracking on mesh
GlobalTransform and UsdGpuSkin, and writes a joint GlobalTransform only when its
value differs. Initial palettes and changed placements/poses still update; idle
meshes perform no palette multiplication or joint writes. The benchmark reports
cumulative palette counts/times at initial-open, after-idle and prewarm checkpoints.

Diagnostic before/after results (single processes, not controlled acceptance):

| Caldera CPU workload | Before | After |
| --- | --- | --- |
| Mean eager idle update, 100 updates | 55.693 ms | 0.095 ms |
| 10 ms-budget all-deferred preparation | 49.980 s | 23.276 s |
| Preparation update p95 | 45.645 ms | 16.805 ms |
| Preparation maximum update | 2,232.132 ms | 2,250.883 ms |

The large maximum did not identify its cause; follow-up attribution below
locates most of that update outside individual prim preparation. The final queue drained 18,932 meshes in 1,507 updates,
preserving the previous budgeted mesh/subset/entity payload counts. Eager CPU
open did not improve in these samples (41.006 vs 42.014 s); the win is redundant
per-update work, not faster initial projection or a GPU frame-rate measurement.

749 workspace/all-target tests pass (19 ignored), including explicit idle,
placement and pose change checks. Four native baked deformation comparisons pass.
The eager Caldera capture is byte-identical to `target/perf/p4-cpu/caldera.rgba`;
its 58.21-second process duration is not first-frame latency. Logs, images and
test evidence are in `target/perf/p4-skin-updates/`. Engine-generated joints are
derived from their owning mesh placement and UsdGpuSkin pose; independently
authoring their GlobalTransform is not a supported pose-edit path.

Slow-update attribution corrects the earlier heavy-prim hypothesis. Optional
`ResidencyPreparationTiming` retains attempt count, total route-preparation time
and only the slowest prim (bounded profiling storage). With `USD_PROFILE_ROUTES`,
the benchmark reports promotion/palette contributions for updates over 100 ms.
`USD_PROFILE_LOADING` additionally times editor ReloadSources commands.

Caldera's traced update 17 took 2,259.507 ms, but only 10.027 ms was promotion
and 0.001 ms was palette updating. A 265-file editor reload command in that
update took 1,199.800 ms. Across the entire queue, the slowest individual prim
was 43.932 ms at
`/world/mp_wz_island/mp_wz_island_paths/mp_wz_island_geo/map_vehicle_spawns/script_struct_mp_openjeep_113/veh_s4_mil_lnd_m151/geo/prs_bodyShape`.
Thus the ~2.25-second maximum is not a single huge mesh. Individual work still
exceeds the 10 ms soft budget, but splitting it alone will not fix this stall.

Source inspection: `editor/reload.rs::watch` marks every observed file pending
when it first sees a document. The next stable poll enqueues ReloadSources even
when nothing changed on disk. `reload_paths` traverses texture requests and
reads/hashes selected paths; afterward `process_commands` unconditionally builds
a new editor snapshot. Snapshot time was not separately measured in this trace,
so do not assign the entire remaining second to it without further evidence.
Next priority: preserve the load-to-watch race checks while avoiding redundant
initial source verification and no-op inspector reconstruction. Do not disable
watching or blindly seed current metadata: supplied root bytes can already be
stale, and changes during opening must still reload safely.

749 workspace/all-target tests pass (19 ignored). Logs and profiler builds are
in `target/perf/p4-heavy-prim/`; `reload-trace.log` contains correlated command,
update and slowest-prim timings. This increment adds attribution, not a fix for
the initial watcher stall.

Rejected follow-up: a temporary per-promotion `ProjectionMaterials` memo passed
747 experimental workspace/all-target tests (19 ignored), including shared
material handles and restoration of a surrounding stage's memo. Caldera's
MaterialRoute cost remained 1.225 / 1.239 s, versus roughly 1.235 s in the first
ordered baseline sample. Full prewarm was 16.692 / 16.788 s versus 17.023 /
16.841 s; these two-run differences do not establish a useful gain. The memo
starts after binding resolution and cannot skip that prerequisite. The trial
was removed rather than adding unproven caching complexity. The rejected patch,
raw measurements, experimental tests and restored benchmark build log are in
`target/perf/p4-promotion-materials/`. Existing projection-job material caches
remain unchanged; this result does not reject their use or prove a memo could
never help scenes with different material workloads.

CPU attribution: `ProjectionVisibilityTimings` splits matching-route application
time by hierarchy visibility at route entry. It walks `Visibility`/`ChildOf`
without waiting for transform/visibility propagation, honors explicit Visible
overrides and missing visibility boundaries, and reports unknown for missing
entities or chains exceeding 128 levels. Classification time is outside route
application timing but still contributes to wall time. Early metadata routes
may run before their own VisibilityRoute; use the mesh/skin/subset rows below
for geometry demand, not sums of every visible/hidden row. Frustum/shadow demand
and final submitted-frame visibility are not measured by this probe.

Enable with `USD_PROFILE_VISIBILITY=1` alongside `USD_PROFILE_ROUTES=1` in the
editor benchmark, or on its own in `viewer_capture`. Two diagnostic Caldera
opens found 18,932 hierarchy-hidden mesh prims versus 224 visible mesh prims:

| Caldera route | Hidden application samples, s | Visible application samples, s |
| --- | --- | --- |
| MeshRoute | 8.856 / 8.698 | 2.353 / 2.068 |
| SkinRoute | 3.468 / 3.334 | 0.007 / 0.007 |
| SubsetRoute | 2.956 / 2.883 | 0.017 / 0.017 |

The hidden contribution across these three sequential routes is about 15.28 /
14.91 s, not a promised saving: some deferred data may still be dependencies of
demanded content, and reveal/reload work remains mandatory. Visible shape-route
prims add 4,187 entities, so the 224 mesh-prim count is not the capture's total
visible-mesh-entity count. Oxbo has only three hidden mesh prims and about 0.15 ms
of hidden work in these routes; this strategy will not materially improve Oxbo.

The offscreen Caldera asset-instance path independently reports 14.737 s hidden
across these routes and is pixel-identical to the owner-walk baseline. Its capture
process duration is 58.64 s, not first-frame time. Full workspace release tests
pass (740 tests, 19 ignored); inheritance/override/depth and timing-bucket tests
cover the profiler. Evidence is in `target/perf/p4-cpu/`. This promotes authored-
visibility/purpose deferral ahead of another narrow worker experiment for Caldera,
while preserving the dependency, reveal, retained-variant and custom-route gates
below. No geometry deferral is enabled by this profiling increment.

Attribution increment: embedded-viewer and offscreen capture metadata now group
unique retained mesh assets into visible-only, hidden-only, shared and
unreferenced buckets using propagated `InheritedVisibility`. Per-bucket vertex,
index and inline morph bytes are reported separately, with render-world usage
and unavailable CPU data counts. This is not frustum visibility, actual GPU
allocation, texture residency or a preparation-time profile. The collector runs
at screenshot request, not every update; it does not alter loading policy.

Caldera findings: 168 visible-only assets hold 3,171,023,264 render-world CPU
payload bytes; 3,142 hidden-only assets hold 387,153,840 bytes. The two shared
assets contain no geometry payload. Thus hidden-only geometry represents about
10.9% of this payload, despite dominating the asset/entity counts. Oxbo has
76,604,544 visible-only bytes and only 37,776 hidden-only bytes (nine assets).
Deferring hidden assets alone is therefore not a credible explanation for a
large memory/transfer improvement on these configurations. It may still avoid
substantial per-prim CPU work; that needs separate attribution.

Verification: 101 release viewer tests and 22 capture tests pass. The new test
covers duplicate/shared references, hidden and unreferenced assets, missing
assets and already-extracted CPU data. Both captures completed; Caldera's RGBA
is byte-identical to the P3 baseline. Evidence and reports are under
`target/perf/p4-residency/`. Caldera completed in 67.77 s, Oxbo in 12.06 s;
these include warmup/readback and are not first-frame timings or speedup claims.
Caldera's report predates a final defensive check for extracted morph data;
all its CPU data was available, and the final build/tests include that check.
Deferred-state implementation remains TODO. Prioritize profiling visible-mesh
decoding, assembly and transfer costs alongside (not after) hidden-prim work.

Visible-assembly follow-up: static expanded/flat meshes now triangulate from
their already assembled corner positions instead of allocating another expanded
position buffer and corner-point mapping. Valid deformation reference positions
still use the separate original-position path, preserving reference diagonals;
invalid-length references retain the existing current-position fallback.
Expanded construction no longer reserves normal/color buffers when those
attributes will not be emitted. No vertex layout or final mesh payload changed.

Verification: 581 focused release tests pass (19 ignored), including new parity
checks for concave faces, holes, malformed indices, both orientations, subsets
and invalid-length reference arrays, plus the existing deformed-diagonal test.
Caldera retains the same 50,125 mesh entities, 23,291 subsets, 3,312 mesh assets
and 66,456,232 vertices. CPU opens were 45.143 / 49.174 s, assembly counters
4.790 / 4.312 s: no established timing improvement. The capture completed in
66.06 s with peak RSS 16,518,040 KiB and byte-identical RGBA to the preceding
capture. Logs and hashes are in `target/perf/assembly-borrow/`. This removes
specific transient allocations; it does not reduce the 3.17 GB visible payload
or implement parallel preparation/deferred residency.

**Scope:** `live.rs`, `asset.rs`, `instance.rs`, built-in geometry/material/
deformation routes and their tests. Preserve arbitrary custom routes.

1. Attribute geometry decode/preparation/upload bytes to effective visibility
   and purpose before predicting savings.
2. Introduce explicit states such as `Unprepared`, `Queued`, `Resident` and
   `DirtyDeferred`, with source revision, instance time and render settings.
3. Project the logical hierarchy, transforms and required metadata first.
   Defer expensive built-in render preparation for initially excluded geometry.
4. Compute inherited visibility/purpose correctly and include dependency closure:
   a hidden prototype may still be required by a visible instancer.
5. Materialize newly required geometry after visibility, purpose, variant,
   ancestor or clock changes. Keep already-resident switched-off assets.
6. Update dirty deferred revisions on reload; reject stale jobs. Keep last-good
   rendering or explicit loading state until replacement data is ready.
7. Start with authored visibility/purpose only. Defer frustum streaming, which
   additionally requires shadow, bounds, framing and camera-motion policies.

**Gate:** initially excluded geometry produces no mesh preparation/upload until
requested; all demanded dependencies do. Toggling on/off/on retains previously
loaded assets. Reload while hidden and reveal at a different clock shows the
latest data. Compare complete-frame images and required-asset manifests, not
old eager mesh counts. Keep an eager mode as a correctness/performance control.

## P5 — Add bounded pure preparation jobs

Rejected scoped-subset worker trial: an opt-in dedicated pool split large
parents' subsets in ordered batches, with 1/2/4/8-thread configurations and a
64 MiB estimated scratch/output limit. Workers borrowed only immutable mesh and
face-index data; USD/material reads and asset insertion stayed serial, and every
scope joined before publication. Small/oversized batches fell back to serial.
740 experimental workspace tests passed, including serial/worker attribute,
index and morph parity and budget fallback. Caldera counts stayed unchanged.

| Threads | CPU open samples, s | SubsetRoute samples, s | Worker jobs/run | Peak estimated batch bytes |
| --- | --- | --- | --- | --- |
| 1 (serial) | 42.092 / 40.904 | 2.934 / 2.871 | 0 | 0 |
| 2 | 41.684 / 40.801 | 2.943 / 2.915 | 2,686 | 17,390,922 |
| 4 | 43.070 / 40.949 | 3.132 / 2.917 | 3,120 | 34,781,844 |
| 8 | 42.853 / 41.542 | 3.153 / 3.003 | 3,144 | 34,781,844 |

There is no useful scaling signal: more workers do not consistently reduce
subset time or total CPU opening. These two-sample diagnostic runs are not
acceptance statistics. The production changes and experimental environment
switch were removed; the normal benchmark/capture binaries were rebuilt.
The patch, build/test logs and raw measurements are archived under
`target/perf/p5-subset-workers/`. No scoped subset pool is shipped. This trial
does not validate an asynchronous pipeline: further worker work should overlap
owned preparation across prims, rather than joining a tiny batch per mesh, and
must still meet revision, cancellation, memory and custom-route gates below.

Scheduling increment: finite initial-projection budgets are now shared across
loading roots. Newly opened roots enqueue their projection instead of taking a
full slice immediately and then another in the continuation pass. The pass
rotates its first root after the last serviced entity, spends only the remaining
budget on subsequent roots, and guarantees one prim of progress even at zero
budget. Despawning a root does not stall rotation. `Duration::MAX` retains the
synchronous initial-projection path. Continuation work now contributes to
`UsdSceneTimings::projection`.

This is a soft budget for initial prim routing, not a hard frame-time bound:
individual routes, stage opening/traversal, validation, texture decoding and
existing reload reconciliation remain synchronous and can exceed it. There are
no CPU worker queues yet. The source benchmark now explicitly updates until all
roots are Ready rather than assuming one update completes every root.

Focused verification: 581 release tests pass, 19 ignored (558 library, one source
benchmark, 22 capture). A zero-budget three-root test verifies exactly one prim
per update across all roots, first-turn fairness, despawn handling and later
unlimited publication. Three 128-prototype source-benchmark samples pass with
one/four independent roots, reload entity retention and geometry sharing.
Evidence is in `target/perf/p5-budget/`. Worker scaling and first-frame acceptance
remain open; this change bounds multiplied projection slices, not total load time.
The broader release workspace/all-targets gate also passes: 735 tests in 34
suites, 19 ignored. Oxbo's capture is pixel-identical to the package-directory
baseline. The native export tests were not rerun for this scheduling-only change.

Per-job material increment: `ProjectionJob` now owns its material memo between
slices. Each `step` temporarily installs that job's memo, captures it afterward
if work remains, and restores any surrounding World memo. `begin` no longer
overwrites another job's memo. Completion/cancellation drops the job-owned memo.
This preserves material reuse as the scheduler rotates among independent stages;
the old World-global slot only retained whichever stage began last.

Verification: the release workspace/all-targets gate passes 736 tests in 34
suites, 19 ignored. An interleaved two-stage, zero-budget test runs without the
global material interner: matching bindings share handles within each job but
not across stages, and a surrounding memo survives every slice. The one/four-root
source benchmark completes load/reload checks. Evidence is in
`target/perf/p5-materials/`. No large-scene speedup or worker-pipeline completion
is claimed by this cache-lifetime change. Oxbo's capture is pixel-identical to
the shared-budget baseline.

Clock-coherence correction: continuation slices now use the runtime's initial
sample time, rather than reading a possibly different root clock for each slice.
On completion, the existing instance tick applies the latest requested clock.
Previously a 0 → 10 → 0 scrub during loading could leave a middle prim at time
10 because the final clock equaled the runtime's initial sample, suppressing
the final animation update. The regression test uses three independently sliced
animated transforms and verifies their final values and subsequent clock updates.
The release workspace/all-targets gate passes 737 tests in 34 suites, 19 ignored;
evidence is in `target/perf/p5-clock/`. This checks clock coherence, not asynchronous
worker cancellation or a loading-speed improvement.

Pending-edit correction: completing initial projection no longer drains and
discards its `LiveStage` change queue. The ordinary instance synchronization pass
consumes changes made while loading and updates already-projected entities. The
regression test edits a projected transform while another prim remains queued,
then verifies the Ready scene contains the edit on the same entity and the change
queue has been consumed. The release workspace/all-targets gate passes 738 tests
in 34 suites, 19 ignored; evidence is in `target/perf/p5-edits/`. This preserves
live-stage edits across projection slices; it is not the planned asynchronous
revision/cancellation protocol or a new performance result.

**Scope:** `live.rs`, `route/mod.rs`, geometry preparation modules and scheduler
helpers within `crates/usd_bevy/src/`; do not rewrite upstream Stage threading.

1. Extract owned immutable inputs on the Stage-owning thread.
2. Run pure triangulation, normals/tangents, subset remapping and eligible skin
   math in CPU workers. Never send Stage handles or exclusive World access.
3. Publish on the World-owning thread only when source generation, root identity,
   time and settings still match. Discard obsolete results without leaking assets.
4. Limit in-flight bytes and apply backpressure. Avoid full-scene duplicate
   snapshots. Preserve custom-route barriers and a serial reference path.
5. Give interactive projection a global fair frame budget rather than the full
   budget per root. Separate CPU production and GPU-upload budgets.
6. Report worker utilization, serial fraction, queue bytes and peak memory.
   Upload throttling is a latency/memory tradeoff, not an automatic speedup.

**Gate:** serial/worker outputs match; reload, cancellation, despawn and clock
changes reject obsolete jobs. Many roots do not multiply the nominal frame
budget without accounting. Measure 1/2/4/8-worker scaling and memory sequentially;
retain only configurations with an evidenced end-to-end benefit.

## P6 — Move sharing ahead of decoding and improve cache indexing

Lookup increment: the existing assembly cache now hashes and searches geometry
once per preparation request. The interned-handle path passes that lookup to
assembly rather than repeating it, then attaches the result to the known MRU
entry instead of searching/equality-checking the input again. Full input
equality, live output equality, mutation/removal checks and existing byte
budgets remain enforced. Uncached oversized builds cannot overwrite a previous
entry's asset ID. This is independent of the still-open subset-product cache
and does not yet share prototype data before decoding.

Verification: 581 focused release tests pass, 19 ignored. Extended cache tests
check one lookup per call and oversized-build isolation alongside mutation,
removal and disabled-budget behavior. Both Caldera runs report 35,796 lookups,
equal to 3,282 builds + 16,640 owned-clone hits + 15,874 handle hits; mesh/entity
and vertex counts are unchanged. CPU opens measured 46.237 / 44.392 s on
Caldera and 3.880 / 3.696 s on Oxbo. These are diagnostic iterations, not a
controlled speedup claim or first-frame result. Evidence is in
`target/perf/p6-lookup/`. Hash indexing and pre-decode prototype sharing remain
TODO.

Selective-eviction increment: `ProjectionCache` now evicts oldest insertions
only until the entry and payload limits admit the new mesh, instead of clearing
the entire cache. Hits do not promote entries. The FIFO stores weak asset IDs;
normal cache pruning removes its stale records, and eviction does not delete
assets held by entities. Existing full-output equality and byte budgets remain.
The metric `intern_eviction_batches` replaces `intern_flushes` for this policy.

Verification: 582 focused release tests pass, 19 ignored, including partial
eviction, hit ordering, external-handle survival and FIFO pruning. Both Caldera
runs reduce intern misses from 4,628 to 3,152 and evicted entries from 4,619 to
3,143. Retained mesh assets fall from 3,312 to 2,509 through sharing; mesh entities
remain 50,125 and subset entities 23,291. Unique retained vertex/index payload
falls from 3,558,177,104 to 3,349,077,812 bytes. The overview capture is byte-for-
byte identical to the assembly-borrow baseline. CPU opens measured 47.007 /
53.611 s, versus the preceding 46.237 / 44.392 s: no loading speedup is established.
Oxbo measured 3.860 / 3.367 s with unchanged counts. The first Caldera iteration
overlapped the Oxbo diagnostic; these are not controlled timing comparisons.
Capture process duration was 61.43 s with 16,670,800 KiB peak RSS, not a first-frame
measurement. Evidence is in `target/perf/p6-fifo/`. This increment improves cache
reuse at the existing budget but does not complete P6 or the performance target.

**Scope:** `route/native.rs`, `route/cache.rs`, `route/mod.rs`, `read/geom.rs`,
instance/reload invalidation and focused tests.

1. Count repeated prototype reads, decoded bytes and cache eviction/rebuilds.
2. Key prototype-local immutable geometry by stage generation, prototype identity
   and relevant sample/dependency revisions before reading large arrays.
3. Keep inherited primvars, collection material bindings, visibility and transforms
   contextual. Never assume all inputs are prototype-global or share root clocks.
4. Replace whole-cache clearing with selective bounded eviction or a weak index
   of externally retained assets. Distinguish index metadata from owned payload.
5. Preserve collision protection and external mutation/removal detection. Remove
   full-byte checks only if reliable revision tracking covers every mutation path.

**Gate:** repeated compatible instances avoid redundant decoding; incompatible
inherited inputs and clocks do not alias. External edits/removals and source
reload invalidate correctly. Cache churn counters improve without increasing
retained payload budgets. Native prototype identities are never persisted as
stable cross-run names. PointInstancer work requires its own measured workload.

## P7 — Reduce source copies and repeated texture discovery

Oxbo source-format investigation at `f0a60e7`: the original USDZ contains a single
165,860,567-byte USDA entry. A native `usdcat` conversion into an ignored USDC
under `target/perf/oxbo-current/` leaves the original untouched. This is a source-
format experiment, not an implemented cache or a matched native-render benchmark.

Original CPU open measured 3.461 / 3.104 / 3.083 s; USDC measured
1.502 / 1.509 / 1.491 s, each in a fresh process using the same release binary.
Order was original/binary, binary/original, original/binary. Filesystem caches
were not cleared. The first original sample spent 468 ms reading/constructing
the source and another 1,771 ms opening its stage; MeshRoute was 420 ms,
MaterialRoute 193 ms and SkinRoute only 9 ms. Thus further skin-check optimization
is not the priority for this Oxbo workload. USDC's first sample spent 27 ms on
source construction and 40 ms on stage open. Its total CPU open still includes
validation, projection and editor work.

All six samples retain 1,786 mesh entities, 14 subsets, 1,597 mesh assets and
1,739,712 vertices, with identical reported vertex/index payload sizes. Original
and binary captures both completed and their RGBA files are byte-identical:
`cb192f7a8647daacaaed2ac55ecbaaab4b7428bbe41288c716ed30142fed0b0c`.
The binary image was visually inspected. This verifies the captured static view,
not animation, editing or a general guarantee for relocated asset dependencies.
First-run process peak RSS was 731,608 KiB for original versus 251,476 KiB for
binary. Raw CPU logs, conversion output and both captures are retained under
`target/perf/oxbo-current/`; all runs used the 24 GiB/no-swap cap.

**Identified source-loader duplication:** `StageBuilder::open` calls
`root_stack_expression_variables`, whose `LayerRegistry::own_expression_variables`
reads/parses the root. `collect_layers` then calls `open_stack`, which reads/parses
it again. The registry explicitly has no read cache. Pass already-parsed root
and session-root data through this single open operation, preserving canonical
identifiers, resolved paths, expression-variable precedence, muted-session
behavior and error reporting. Do not add a long-lived pathname-only cache.
Prove one parse per ordinary root, expression-valued sublayer behavior, fresh
contents after a subsequent open, and independent mutable stages; run the full
workspace gate and repeat original-format Oxbo timing and capture. Keep binary
conversion optional evidence, not a prerequisite for faster original USD loading.

Prepared-root implementation: the vendored registry now returns owned parsed
root data with its canonical identifier and resolved path. StageBuilder reads
root/session expression variables from that data, then consumes the same data
for stack collection. No cross-open cache or shared mutable stage is added.
The review patch is `patches/openusd-prepared-roots.patch`; dependency revision
and original assets are unchanged.

757 Make release workspace/all-target tests pass (19 ignored), plus 14 native
export checks run explicitly. A counted-resolver test verifies one read of each
root/session layer, session-over-root variable precedence, root variables in
session sublayers, muted session behavior, fresh dependency data on reopen and
independent stage edits. Evidence: `target/perf/p7-root-parse/tests-final.log`
and `native-exports.log`.

Three original-format Oxbo CPU opens measured 2.265 / 2.260 / 2.265 s, versus
the prior warm original runs' 3.104 / 3.083 s (about 27% lower). Source construction
took 99–110 ms; stage opening after it took 923–941 ms. These are sequential
before/after diagnostics, not randomized paired acceptance. Entity/mesh/payload
counts match the previous baseline. The new original-asset capture completed
and its RGBA is byte-identical to `target/perf/oxbo-current/original.rgba`.
Logs, capture and build output are under `target/perf/p7-root-parse/`.
This improves original USDA loading without requiring binary conversion; the
matched first-complete-frame target and broader Moana acceptance remain open.

Package-directory increment: the vendored USDZ format now opens its default-layer
directory through the resolver's seekable asset, instead of `read_all()` copying
the entire package first. Public `Archive::from_asset` remains unchanged; corrupt
or rootless archives still retain their package path for format-error reporting.
The patch is recorded in `patches/openusd-package-directory.patch`, without a
dependency refresh. This removes a root-lookup copy, not entry extraction or
parser-owned buffer copies.

Verification: 585 focused release tests pass, 19 ignored, plus 14 native export
tests run explicitly. A counted in-memory resolver verifies default-layer lookup
reads less than one quarter of a 4 MiB stored package and preserves corrupt-package
fallback. Existing nested package and dependency reload tests pass. Oxbo CPU opens
measured 3.499 / 3.102 / 3.123 s with unchanged mesh/entity/payload counts; capture
was 10.23 s with 1,290,148 KiB peak RSS and pixel-identical to the manifest baseline.
These are diagnostic timings, not a controlled or matched first-frame speedup.
Evidence is in `target/perf/p7-directory/`.

Manifest increment: immutable `UsdSource` snapshots retain one shared texture-
request manifest tagged with the exact source revision. The asset loader records
it after dependency discovery; freshly opened instances without overrides reuse
it. Manually constructed snapshots populate it on their first default instance.
Source/dependency replacement changes the revision. Filesystem-backed sources,
attribute/variant overrides and in-place variant switches still scan the stage.
Edited editor stages retain their uncached discovery path. Failed scans are not
cached. The manifest retains the original complete sample and raw/sRGB keys;
image decoding and missing-texture checks are unchanged.

Verification: 578 focused release tests pass, 19 ignored (555 library, one source
benchmark, 22 capture). New coverage checks shared allocation reuse across fresh
stages, bypass after edits, dependency/root revision changes, and filesystem
exclusion. Existing texture color-space/dependency-reload and variant tests pass.
Three source-benchmark samples cover independent one/four-root loads and reloads.
Caldera's overview capture is pixel-identical to the FIFO baseline; process time
was 80.70 s with 15,317,308 KiB peak RSS. Oxbo's capture is pixel-identical to the
P4 residency baseline, at 10.82 s and 1,276,340 KiB peak RSS. This is not evidence of a loading speedup
or a matched first-frame result. Logs and captures are in
`target/perf/p7-manifest/`. Shared parser buffers, package-range reuse and bounded
image jobs remain unimplemented.

**Scope:** `source.rs`, `asset.rs`, texture preparation; explicitly reviewed
`vendor/openusd` asset/file-format changes if required. No dependency refresh.

1. Reuse revision-bound texture-request manifests for matching composition.
2. Decode independent images in bounded jobs from owned/shared bytes, preserving
   raw/sRGB distinctions and dependency tracking.
3. If separating future/unused textures from first-frame requirements, define
   later-load failure and completion behavior before enabling it by default.
4. Extend the asset/file-format boundary to accept shared immutable buffers or
   bounded ranges. Cache package directories by source revision.
5. Use shared ranges for eligible stored entries and bounded decompression for
   compressed entries. Preserve nested-package handling and extraction limits.

**Gate:** package corruption, bounds, nested entries, dependency repair, animated
texture paths, color spaces and independent roots remain covered. Measure bytes
copied/decompressed, peak memory and Oxbo loading. No pathname-only cache may
serve old contents after Blender saves a new file.

## P8 — Make reload preparation proportional to changed layers

No-op inspector increment: `editor::process_commands` retains the existing
inspector snapshot when a command batch contains only successful ReloadSources
checks with no publication, revision change or detected external edits. Actual
reload publication, errors, other commands and external/texture changes still
refresh it. Source reading/hashing and watcher behavior are unchanged; this is
not a metadata shortcut or removal of the initial load-to-watch verification.
Optional `EditorSnapshotTiming` counts command-driven inspection work, and
`USD_PROFILE_LOADING` enables it with per-snapshot timing output.

750 workspace/all-target tests pass (19 ignored). The regression checks prove
that an unchanged reload avoids a new snapshot, while changed disk content,
mixed Select/reload batches, malformed-file failures and an external stage edit
all refresh inspection and preserve document/last-good state as appropriate.

Caldera diagnostic: the initial 265-file reload still costs 1,191.722 ms, but
the slow update falls from 2,259.507 ms to 1,210.093 ms. Initial-open inspection
separately costs 987.856 ms; no second snapshot is emitted for the no-op reload.
Budgeted prewarm completes all 18,932 meshes in 22.583 s / 1,520 updates, with
16.846 ms p95. This single comparison isolates avoided inspection work; it does
not establish a general load-speed ratio or remove the remaining verification
stall. Evidence: `target/perf/p8-noop-inspection/` (`caldera.log`, `tests.log`).

Reload subphase attribution: `USD_PROFILE_LOADING` now separates current-stage
texture discovery from selected-file verification. Caldera's unchanged 265-file
check spent 1,100.573 ms in discovery and 91.891 ms in reading/hashing sources.
The next target is therefore repeated stage discovery, not a weaker file check.
`UsdSource::stage_texture_requests` now reuses each prim's first resolved type
instead of querying it again for non-dome prims. This is a local read reduction,
not a persistent cache or change to texture request/error semantics.

The after sample measured 1,072.160 ms discovery / 94.955 ms verification and
a 1,185.147 ms maximum update versus 1,210.895 ms before. These single samples
do not establish a meaningful end-to-end speedup; most discovery cost remains.
750 workspace/all-target tests pass (19 ignored). Evidence is in
`target/perf/p8-texture-scan/` (`before.log`, `after.log`, `tests.log`). A future
request-manifest cache must be bound to stage/layer and composition revisions,
including variants, payload/load rules, population masks, muted layers, authored
material edits and external changes. Do not reuse by pathname or editor clock
alone, or suppress existing discovery errors for modified stages.

Revision-bound candidate index increment: `editor/texture_index.rs` retains
only paths of Material/DomeLight/DomeLight_1 prims for one EditorSession. It does
not cache resolved texture requests, asset values or errors: each request scan
still reads current attributes/time samples and resolves current asset paths.
This preserves filesystem-dependent resolution, including a missing texture
appearing without any stage edit. The key contains tracked per-layer revisions,
structural revision, participating layers, load rules, population mask and muted
layers. Saturated counters and changes observed during candidate discovery
prevent reuse. Initial editor texture preparation seeds the same index used by
reload checks; standalone source request scans remain uncached.

754 workspace/all-target tests pass (19 ignored). New differential checks compare
indexed requests with full discovery after prim type/attribute authoring, load
rule changes, layer mute/unmute, variant switches and direct Sdf layer edits.
Missing-file recovery verifies resolution changes while the candidate scan count
stays unchanged. Existing reload, malformed-file, last-good and editor tests pass.

Caldera's diagnostic unchanged-stage texture discovery is now 0.039 ms versus
1,072.160 ms in the preceding run. File verification still reads/hashes 265 paths
in 92.793 ms; total reload check is 92.942 ms. Budgeted preparation drains all
18,932 meshes in 21.514 s / 1,525 updates, with 16.862 ms p95 and 111.069 ms max,
versus the previous 1,185.147 ms max. These are single-run diagnostic comparisons,
not matched first-frame acceptance. Source verification can still exceed a frame
budget; watching and its initial race check have not been disabled or weakened.
Logs and build/test evidence: `target/perf/p8-texture-index/`.

**Scope:** `reload.rs`, `editor/reload.rs`, `source.rs`, persistence/conflict tests;
review upstream immutable-layer APIs separately if required.

1. Measure source-change detection through first updated complete frame. Separate
   byte reading, serialization, composition, validation and asset publication.
2. Replace whole-document text serialization with immutable layer revisions and
   isolated copy-on-write candidate layers where APIs permit.
3. Reuse unchanged immutable data, never mutable stage opinions across roots.
4. Use revision-bound conflict checks only where they detect all mutation paths;
   retain content-based fallback otherwise.
5. Validate affected composition and commit atomically. Retain exhaustive fallback
   for broad arc/variant/load changes and preserve undo and dirty-layer protection.

**Gate:** a single-layer Blender save preserves unrelated entity/component and
asset identities; unchanged geometry does not reprepare. Failed/corrupt saves,
concurrent edits and rapid repeated saves preserve last-good state and ordering.
Record reload latency and changed/unchanged bytes for single and multiple roots.

## Verification commands

Workspace checkpoint, 2026-09-17, implementation `cc09616`:

- `CARGO_BUILD_JOBS=4 make check-all CARGO='cargo --offline'`: passed. Three
  existing camera-plan dead-code warnings remain; no new compiler errors.
- `CARGO_BUILD_JOBS=4 make test-all CARGO='cargo --offline'`: 726 tests passed
  across 34 suites, zero failures, 19 ignored. This includes viewer, examples,
  integration targets and the core library, not just the focused release lane.
- `USD_CAT="$(command -v usdcat)" CARGO_BUILD_JOBS=4 make test-native
  CARGO='cargo --offline'`: 14 native persistence-export tests passed against
  OpenUSD 25.05.01. Four native deformation comparisons were separately verified
  for the P2 palette increment; the remaining ignored large-scene prerequisite
  is not treated as passed by either gate.

Full command logs, validated commit and native tool path are retained in
`target/perf/integration/`. These gates validate the implemented increments;
they do not complete P0–P3 or prove the ≤5× first-complete-frame requirement.
Before implementing demand-driven residency, measure unique asset bytes that
are used by visible entities versus exclusively hidden entities: Caldera's
4,621 hierarchy-visible mesh entities alone do not quantify avoidable work
because visible and hidden entities can share the same mesh assets.

Run from repository root. These existing commands do not imply unimplemented
benchmark options already exist. Add and document P0's new invocation when it
lands; do not substitute the current capture endpoint for that acceptance gate.

```sh
git status --short
git diff --stat 1dd38c8..HEAD -- crates/usd_bevy/src src examples vendor/openusd

# Focused release suite used for the current baseline.
CARGO_BUILD_JOBS=4 make test CARGO='cargo --offline' \
  APP_TARGET='--release --workspace --lib --example editor_benchmark --example viewer_capture'

CARGO_BUILD_JOBS=4 make build CARGO='cargo --offline' \
  APP_TARGET='--release --example editor_benchmark --example viewer_capture --bin usdview'

# Wider integration gates before accepting architectural phases.
make check-all
make test-all
make test-native
git diff --check
```

Expected: all commands exit zero; ignored/native prerequisites must be reported,
not silently counted as passed. The focused baseline passed 572 tests with 19
ignored; do not treat that historical count as the current full-workspace count.
Use existing tests in `route/cache.rs`, `route/subset.rs`, `mesh.rs`,
`mesh/compact.rs`, `read/skel.rs`, `asset.rs` and `editor/reload.rs` as patterns.

Existing CPU baseline invocation, through Make and a 24 GiB process-group cap:

```sh
USD_PROFILE_LOADING=1 USD_PROFILE_ROUTES=1 \
make --eval='perf-caldera:; @systemd-run --user --scope --quiet -p MemoryMax=24G -p MemorySwapMax=0 /usr/bin/time -v timeout --kill-after=10s 150s target/release/examples/editor_benchmark target/large-scenes/caldera/caldera.usda 2 gpu-prepared' perf-caldera
```

For Oxbo, use `../machines/usd/oxbo_harvester.usdz`. The two-run invocation is
an iteration check, not the five-run acceptance comparison.

Existing visual regression capture, not first-frame timing:

```sh
USD_CAPTURE_CAMERA=/cameras/map_airfield_overview \
USD_CAPTURE_TIMEOUT_SECS=150 USD_CAPTURE_GPU_SAMPLES=20 RUST_LOG=warn \
make --eval='capture-caldera:; @systemd-run --user --scope --quiet -p MemoryMax=24G -p MemorySwapMax=0 /usr/bin/time -v timeout --kill-after=10s 170s nixVulkan target/release/examples/viewer_capture target/large-scenes/caldera/caldera.usda /tmp/caldera-perf.png 0' capture-caldera
```

Expected: `CAPTURE_OK`, PNG, raw RGBA and capture metadata. Inspect the image;
compare raw RGBA against a same-settings baseline for geometry-only changes.
Nondeterministic rendering needs a documented comparison tolerance, not a silent
relaxation. Native reference uses `make capture-reference USD_RECORD=... ARGS=...`;
resolve the actual native binary, not a potentially stale `usdview` wrapper.

## Final acceptance

- [ ] P0 matched benchmark runs through the actual viewer/editor path.
- [ ] Caldera and Oxbo each meet median Bevy/native first-complete-frame ratio
  ≤5 under recorded matching settings; every raw sample and range is retained.
- [ ] Moana has a separately bounded matched run. If it cannot complete, report
  that explicitly; no general large-scene acceptance claim is permitted.
- [ ] No missing geometry/material/deformation is hidden by the completion gate.
- [ ] Hot reload, independent clocks, variant retention and custom routes pass.
- [ ] Peak memory and queued bytes are recorded; local large-scene tests remain
  within the 24 GiB/no-swap cap unless a changed budget is explicitly approved.
- [ ] Architectural phases pass the wider integration gates, not only unit tests.
- [ ] Each accepted change has a focused commit, before/after evidence and updated
  status here. Regressions are investigated or reverted, not described as wins.

## Stop conditions and deferred ideas

Stop and report if source drift invalidates a phase's assumptions, a proposed
shortcut changes visible output or error/publication semantics without an agreed
design, correctness gates fail repeatedly, or an upstream API change is required
outside the reviewed scope. Do not bypass failures by skipping assets or tests.

Do not prioritize these without new evidence:

- A missing multithreading flag: current benchmark builds already enable it.
- PointInstancer redesign for Caldera: current route profile has zero matches.
- Blindly increasing cache budgets: entity residency is already much larger.
- Merely reducing screenshot warmup: it does not fix CPU projection.
- Assuming N cores give N-fold speedup: stage reads/commits remain serial.
- Replacing the renderer, flattening away editable USD composition, or silently
  lowering geometry quality to meet a timing number.
- Allocator/compiler-flag experiments before dominant structural work is removed.

## Research references

- [OpenUSD performance methodology](https://openusd.org/release/ref_performance_metrics.html)
  — distinguishes stage opening and first-image rendering; documents warm-cache runs.
- [OpenUSD performance guidance](https://openusd.org/release/maxperf.html)
  — data structure, payloads, binary layers and allocator considerations.
- [Native Hydra synchronization](https://github.com/PixarAnimationStudios/OpenUSD/blob/v25.05.01/pxr/imaging/hd/renderIndex.cpp#L1556-L1629)
  — parallel synchronization stages, not a promise of equivalent Rust threading.
- [OpenUSD skeletal cache](https://openusd.org/release/api/class_usd_skel_cache.html)
  — persistent structural query reuse.
- [OpenUSD attribute queries](https://openusd.org/release/api/class_usd_attribute_query.html)
  — repeated value-source queries and invalidation responsibilities; inspect the
  vendored Rust implementation for its own revision-aware semantics.
- [OpenUSD scenegraph instancing](https://openusd.org/dev/api/_usd__page__scenegraph_instancing.html)
  — prototype reuse and instance-context constraints.

## Key Learnings

1. Reuse must move before decoding and subset construction, not only before asset insertion.
2. Visibility and geometry residency are currently separate; hiding an entity does not avoid uploads.
3. Safe parallelism requires owned preparation inputs and revision-checked publication.
