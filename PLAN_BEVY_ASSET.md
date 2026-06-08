# PLAN — Bevy asset integration

Make `usd_bevy` integrate with Bevy's asset system the way `bevy_gltf` does:
a root `Usd` asset holding handle maps to labeled sub-assets, textures loaded
as dependencies through the `AssetServer`, and — eventually — openusd reading
every layer through Bevy's `AssetReader` instead of a tempfile.

Grounded in two verified research passes (Bevy 0.18.1 source + openusd's actual
I/O surface). Key facts that shape everything:

- **openusd buffers-then-parses:** `open_layer` does `open_asset()` →
  `read_all()` into `Vec<u8>` → parse from a `Cursor`. So `.usdc`'s random-access
  `Seek` is satisfied **in memory after fetch** — the async story only needs to
  cover "produce the bytes," not streaming-seek.
- **~85-90% of I/O is eager at `Stage::open`** (the recursive `collect`); pcp
  composition then runs on in-memory layers. Async only has to wrap the
  **collect byte-fetch boundary**, not thread through pcp.
- **Bevy 0.18:** `AssetLoader` now needs `#[derive(TypePath)]`; multi-asset
  graphs use `labeled_asset_scope`/`begin_labeled_asset` (NOT `add_labeled_asset`
  - that one doesn't track deps); deps via `load_context.load`; sibling paths via
  `AssetPath::resolve_embed` (never `Path::join`); every handle field needs
  `#[dependency]`. External-file hot-reload is broken upstream (#18267) - we
  inherit it, glTF has it too.

## Design decisions (settled)

1. Mirror `bevy_gltf`: a `Usd` root asset with `Vec<Handle<_>>` + `named_*`
   maps; meshes/materials/scenes/images/animations as labeled sub-assets
   (`scene.usd#Mesh0`, `#Material0`, ...).
2. Textures load through `AssetServer` (`load_context.load`), not hand-decoded.
3. The openusd async path is an **executor-agnostic driver** (no forced runtime,
   no `block_on` in the hot path): a sans-IO-style `collect` whose I/O is
   supplied by the host. This matches mxpv's existing task-queue indexer
   direction and stays runtime-neutral (Bevy `IoTaskPool` or a CLI both drive it).
4. **Out of scope here** (noted, not built): cross-prim parallel compose via a
   `ComputeTaskPool`/rayon hook (the "parallelism" convergence) - same
   executor-agnostic principle applied to the compute phase, but a separate
   upstream effort. See Phase 4.

---

## Phase 1 — Idiomatic asset graph (OUR side, ships now)

No openusd change. Root layer still tempfile-spilled (Phase 3 removes that).
This is the big correctness/UX win and is independent of both #103 and the
deferred `usd_schema` migration - it changes how the loader *publishes* assets,
not which readers it calls. Touches `crates/usd_bevy/src/{asset,build,material,
texture,mesh,light}.rs` and the viewer (`src/main.rs`, `src/ui.rs`).

**C1 — `Usd` root asset + `TypePath`.** Define
```rust
#[derive(Asset, TypePath, Clone)]
struct Usd {
    #[dependency] scenes:    Vec<Handle<Scene>>,
    #[dependency] meshes:    Vec<Handle<Mesh>>,
    #[dependency] materials: Vec<Handle<StandardMaterial>>,
    #[dependency] images:    Vec<Handle<Image>>,
    #[dependency] animations:Vec<Handle<AnimationClip>>,
    named_scenes: HashMap<String, Handle<Scene>>, named_meshes: ...,
    // existing side-data kept: cameras, light_tally, variants, instance counts,
    // curves/points source-of-truth, anim tracks
}
```
Add `#[derive(TypePath)]` to `UsdLoader`. Keep `UsdAsset` as a deprecated alias
for one release so the viewer compiles, or migrate call sites in the same PR.

**C2 — meshes as labeled sub-assets.** In `build`, emit each mesh via
`load_context.labeled_asset_scope("Mesh{i}", |lc| ...)` → `Handle<Mesh>`; the
scene references the handle instead of an inlined mesh. Populate
`meshes` + `named_meshes` (keyed by prim path). GeomSubsets become
`Mesh{i}/Primitive{j}` labels, matching glTF.

**C3 — materials as `Handle<StandardMaterial>` sub-assets.** Promote
`material.rs` output to labeled `#Material{i}` sub-assets, deduped (one handle
per distinct UsdShade material, shared across meshes). Root holds `materials` +
`named_materials`; a `DefaultMaterial` label for unbound prims (glTF parity).

**C4 — textures as dependencies.** Replace in-loader image decoding with
`load_context.load(resolve_embed(base, tex_path))` → `Handle<Image>` for
**file-backed** textures (sibling/relative). usdz-embedded textures stay on the
current zip-crack path for now (Phase 3 moves them into the resolver). Materials
reference the image handles (with `#[dependency]`).

**C5 — scenes + the rest.** Emit the projected `Scene`(s) as `#Scene{i}` labeled
sub-assets referencing C2-C4 handles; cameras/lights stay as decoded side-data
(or `#Camera{i}` labels if useful to the viewer); animation clips as
`#Animation{i}` `Handle<AnimationClip>` where we already decode tracks.

**C6 — viewer migration.** Update `src/main.rs` + `src/ui.rs` to consume `Usd`
(spawn `SceneRoot(usd.scenes[0].clone())`, read handle maps for the dropdowns
that today read side-data). Confirm variant-switch / camera-mount / curve-tuning
paths still work against the new shape.

**C7 — fixtures + tests.** Loader test asserting the labeled sub-assets exist
(`#Mesh0`, `#Material0`, `#Scene0`), textures register as deps, and a multi-mesh
shared-material file dedups to one material handle.

---

## Phase 2 — openusd async/sans-IO `collect` (UPSTREAM; coordinate w/ mxpv)

The piece that lets the Bevy loader drive layer I/O without a tempfile or
`block_on`. Lands in `openusd`, so it needs mxpv's nod and should be sequenced
against #103 (which reshapes `Stage` into a ref-counted handle).

**C1 — sans-IO collect core (no behavior change).** Refactor
`layer::collect_recursive` so the recursion decision (which dependency to fetch
next, cycle/visited bookkeeping) is separated from the actual `open_asset` call.
The existing sync `collect` becomes a thin driver over this core. Pure
refactor; no new dependency; all current tests green. This is the same
"pure work-units + external driver" shape as the task-queue indexer.

**C2 — async driver entry.** Add a runtime-agnostic async path
(`AsyncResolver` via boxed-future / `futures-io`, or a host-driven
"give me bytes for X" loop) and `StageBuilder::open_async`. No `tokio`; the
host supplies the executor. Buffer each layer into `Cursor<Vec<u8>>` and feed
the unchanged sync parser. Tests with an in-memory async resolver.

**C3 — docs + ROADMAP.** Note the async-host capability; flag clips/deferred
payloads as still-sync (Phase 4).

Coordinate on Discord: frame it as the executor-agnostic-driver principle mxpv
already adopted for the indexer; agree shape before coding.

---

## Phase 3 — Bevy-backed resolver, kill the tempfile (OUR side; needs Phase 2)

**C1 — `BevyResolver`/async driver.** Reads layers via the `AssetSource`'s
`AssetReader` into `Cursor<Vec<u8>>`; `create_identifier` implements
`AssetPath::resolve_embed` semantics so USD relative refs resolve as virtual
asset paths (works from any asset source, not just filesystem).

**C2 — route the loader through `open_async`.** Remove the tempfile spill.

**C3 — dependency edges for reload.** Collect the set of layer `AssetPath`s the
resolver touched; register each (tiny `RawUsdLayer(Arc<[u8]>)` asset +
`#[dependency]` field) so sublayer edits trigger reload + dedup (modulo #18267).

**C4 — usdz via the resolver.** Package-relative reads (`scene.usdz[tex/x.png]`)
handled in the resolver using openusd's `*_package_relative_path` helpers;
retire the loader's hand zip-cracking (C4 of Phase 1).

---

## Phase 4 — Future (noted, NOT built here)

- **Clips / deferred payloads async** (the ~10-15% lazy I/O via `cache.rs:163`);
  `block_on` is an acceptable interim since clips are a cold, cached path.
- **usdz cross-file layer references** - openusd gap at `layer.rs:222`
  (`"cross-file references within USDZ archives are not yet supported"`); an
  upstream contribution.
- **Parallel compose hook** - expose cross-prim composition as an
  executor-agnostic driver so Bevy supplies `ComputeTaskPool` (not a hardcoded
  rayon dep). Same principle as Phase 2, applied to the CPU phase; rides on
  mxpv's task-queue indexer + the `TODO(rayon)` seams (`cache.rs:1426`,
  `mod.rs:260`). The two-pool end state: `IoTaskPool` fetch → `ComputeTaskPool`
  compose.

---

## Sequencing & risks

- **Phase 1 is independent** - of #103, of the `usd_schema`→openusd migration,
  and of Phase 2. Start here; it ships value immediately.
- **Phase 2 waits on mxpv + #103.** Until it lands, Phase 1's tempfile path is
  the fallback; a sync `BevyResolver` + `block_on` is the bridge if we want to
  kill the tempfile before the async path exists.
- **Hot-reload of external files is limited upstream (#18267)** - document; not
  a regression (glTF is the same).
- **Per-source seekability isn't guaranteed** - always buffer-whole, never rely
  on Bevy `AsyncSeek`.
- The deferred `usd_schema` migration (post-#103) and Phase 1 both touch
  `build.rs`; do Phase 1 first, then the migration rebases onto the new shape.
