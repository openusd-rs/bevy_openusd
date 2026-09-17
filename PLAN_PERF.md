# USD loading performance plan

Planned against `1dd38c8` on 2026-09-16; reconciled against `9ea5ced` and the
working tree on 2026-09-17. Status: **P0–P7 in progress; P4 opt-in only; P8 planned**.
Profiling increments are committed as `a054c56`, `9303920` and `fd79645`.
The matched first-complete-frame benchmark and ≤5× target remain unverified.

## Next implementation checkpoints

This is the performance implementation plan, not a claim that its phases are
finished. Preserve the detailed evidence below and execute in this order:

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
   under identical settings, then alternate at least five fresh process runs
   per mode. Record route times, total CPU loading, peak RSS and reveal latency.
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
| P8 | Revision-based transactional hot reload | P0, P3; reuse P7 | L / high | TODO |

Implement P1–P3 as independently measured changes. Reprofile before P4–P8 and
adjust their order using measured bytes, duplicate work and serial CPU fractions.
Do not add projected speedups together: several phases remove the same work.

## P0 — Establish trustworthy measurements

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

Remaining before default enablement: broader source-reload/variant/dependency
closure cases, live custom-registry switching, animated ancestor visibility,
reveal latency and alternating process measurements. Direct application edits
to Bevy Visibility outside USD are not currently a promotion trigger. Non-Mesh
geometry remains eager, and promotion is synchronous; bounded asynchronous
states/cancellation are not implemented by this increment.

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
