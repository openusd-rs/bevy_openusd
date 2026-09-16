# USD loading performance plan

Planned against `1dd38c8` on 2026-09-16. Status: **P0/P1 in progress; P2–P8 planned**.
Profiling increments are committed as `a054c56`, `9303920` and `fd79645`.
The matched first-complete-frame benchmark and ≤5× target remain unverified.

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
| P2 | Cache deformation discovery and preparation | P0 | L / high | TODO |
| P3 | Separate validation from discarded geometry decoding | P0 | M–L / high | TODO |
| P4 | Demand-driven initial geometry residency | P0, P1, P2 | L / high | TODO |
| P5 | Bounded owned-data worker pipeline | P0, P1, P2 | L / high | TODO |
| P6 | Pre-decode prototype reuse and cache indexing | P0, P1; coordinate P4/P5 | L / high | TODO |
| P7 | Shared input buffers and texture manifests | P0, P3 | M–L / medium-high | TODO |
| P8 | Revision-based transactional hot reload | P0, P3; reuse P7 | L / high | TODO |

Implement P1–P3 as independently measured changes. Reprofile before P4–P8 and
adjust their order using measured bytes, duplicate work and serial CPU fractions.
Do not add projected speedups together: several phases remove the same work.

## P0 — Establish trustworthy measurements

Actual-viewer render probe: launch with `USD_PROFILE_RENDER=1` to emit JSON
`render_asset_profile` records from `src/perf_render.rs`. Main-world manifests
carry document identity and an asset-event generation into the render world.
The post-render probe counts missing `RenderMesh`/`GpuImage` entries and global
pending/error pipelines. It covers all currently retained RENDER_WORLD meshes
and images, including environment assets, not just visible scene dependencies.
Manifests rebuild on document/asset events rather than scanning geometry every
frame. Time starts at probe configuration, not process creation or open-command
submission. Every record explicitly says `complete_frame: false`: materials,
deformation resource bindings, relevant view queues and submitted-generation
acknowledgment remain outstanding.

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
