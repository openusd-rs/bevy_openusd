# bevy_openusd — Rethink: USD as a live, composed BSN

Research + architecture for rebuilding `bevy_openusd` from scratch on top of
**openusd-rs latest** (`mxpv/openusd` @ `06d619d`) and **Bevy 0.19**, turning a
read-only viewer into a **live USD editor**.

Status: research / design. Nothing here is built yet. API names from Bevy 0.19
that a research pass couldn't independently confirm are tagged
**[verify]** — confirm against docs/source before relying on them.

---

## 0. Thesis — "USD *is* BSN, with the parts BSN is missing"

Bevy 0.19 shipped **BSN** (Bevy Scene Notation): a declarative, composable way
to describe entity hierarchies as **patches over `Default`** that get **spawned**
into entities. It is excellent, but it has three gaps the release is explicit
about:

1. it's **spawn-time only** — there is **no diff-and-reconcile** of a re-evaluated
   patch onto already-live entities;
2. there is **no first-party `.bsn` file loader / serializer** yet (the code
   workflow ships first);
3. it has no notion of **layers / variants / overrides resolved at runtime**.

USD is the same shape — **opinions over fallbacks, composed, then resolved to a
stage** — but it *already* has all three missing pieces, battle-tested:

| BSN concept | USD equivalent |
|---|---|
| patch over `Default` | opinion over schema fallback |
| `bsn!{ … Children[…] }` template | composed prim tree |
| scene function (parameterized) | reference / class / inherits |
| composition of patches | **LIVER**: sublayers, references, variants, payloads |
| spawn → entities | **stage → entity projection** |
| *(missing)* live reconcile | **`UsdNotice` change notification** |
| *(missing)* file format | **`.usd[a/c/z]`** + `Sdf` layers |
| *(missing)* runtime variants/overrides | **variant sets, edit targets, layer muting** |

**So the thesis of the rethink:** treat the **composed USD stage as the
authoritative scene** (BSN's role), **project it into Bevy 0.19 entities** (BSN's
spawn), and use **`UsdNotice` → Bevy observers** to get the **live reconcile BSN
can't do**. USD becomes "BSN with industrial composition, a file format, and a
change stream." Editing is just authoring back into the stage; the change stream
re-projects.

This is not (necessarily) literally compiling USD into `bsn!` tokens — it's USD
*playing BSN's role*. (A literal `usd-prim-tree → bsn_list! → spawn_scene` bridge
is *possible* and discussed in §5, but optional.)

---

## 1. The three pillars

- **openusd-rs (latest)** — the data + edit + change model. Pure Rust, **no Bevy
  dependency**, so it can be bumped independently of the Bevy upgrade.
- **Bevy 0.19** — the runtime, rendering, interaction (transform gizmo), and the
  **observer/event** machinery that drives the live-sync loop.
- **mara** — the panel UI (just rebuilt as declarative Pods). Must move to Bevy
  0.19 / egui 0.35 / bevy_egui 0.40 (the gating dependency — see §8).

---

## 2. openusd-rs is editor-ready (verified API)

Everything we needed has landed in `mxpv/openusd`. Key fact for architecture:
**every authoring + change method takes `&self`** (interior-mutable
`RefCell<LayerGraph>` / `RefCell<IndexCache>`), so a single shared `Stage` can be
read and authored from anywhere on its owning thread without `&mut`.

### 2.1 Authoring (`usd::Stage`, `src/usd/stage.rs`)
```rust
stage.define_prim(path) -> Result<Prim>            // specifier = Def
stage.override_prim(path) -> Result<Prim>          // specifier = Over
stage.create_attribute(path, type_name) -> Result<Attribute>
stage.create_relationship(path) -> Result<Relationship>
stage.remove_prim(path) -> Result<bool>
stage.remove_property(path) -> Result<bool>
stage.set_default_prim / set_start_time_code / set_time_codes_per_second / …
stage.batch_edit(&[layer_ids], |edits: &mut [LayerEdit]| { … }) -> Result<bool>  // one transaction
```

### 2.2 Edit targets — "which layer do edits land in"
```rust
EditTarget::for_layer(id)                          // identity → root stack
EditTarget::for_local_direct_variant(id, var_sel)  // author inside a variant
stage.edit_target() / set_edit_target(t)
stage.edit_context(t) -> EditContext   // RAII, restores previous target on drop
stage.edit_target_for_node(&node)      // build a target for an arc/variant node
```
The editor's "current layer / variant" is just the active `EditTarget`. Session-
layer overrides, variant editing, and "edit this reference" all fall out of this.

### 2.3 Namespace editing — `usd::NamespaceEditor` (`src/usd/editor.rs`)
Batched, validated, atomic:
```rust
let mut ed = NamespaceEditor::new(&stage);
ed.move_prim(old, new); ed.delete_prim(p);
ed.rename_prim(&prim, "NewName")?; ed.reparent_prim(&prim, &new_parent)?;
ed.can_apply()?;   // dry-run validate (rolls back) — expensive, full composition query
ed.apply()?;       // atomic commit; fires ONE merged CommittedChange across all touched layers
ed.layers_to_edit()?;  // which layers apply() would touch
```
`NamespaceEditError` enumerates every failure (SourceNotFound, DestinationExists,
RequiresRelocate, KindMismatch, …). Cross-arc moves synthesize `layerRelocates`
automatically; relationship/connection targets are fixed up in place.

### 2.4 Change notification — `StageSink` / `CommittedChange` (THE key feature)
```rust
let id = stage.add_sink(move |stage: &Stage, change: &CommittedChange| {
    // change.resynced:           &[Path]  — composition restructured (full reproject of subtree)
    // change.changed_info_only:  &[Path]  — field/value/target changed, namespace intact
    // change.layer_identifier:   &str
    // change.change_list:        &sdf::ChangeList
    // change.provenance:         LocalStack | EditTarget(MapFunction) | DirectLayerEdit
    // change.changed_fields(path) -> &BTreeSet<Token>
});
stage.remove_sink(id);
```
Plus `StageSink::edit_target_changed(&stage)` and
`layer_muting_changed(&stage, layer, muted)`. Closures implement `StageSink`. The
callback is **borrowed-only** (valid during the callback) → copy what you need out.

**This is the reconcile BSN lacks.** Per-frame pattern: push
`(resynced, changed_info_only)` into a queue from the sink, drain it in a Bevy
system, and patch exactly the affected entities. `resynced` ⇒ rebuild that
subtree's entities; `changed_info_only` ⇒ refresh that property's component.

### 2.5 Diff — `usd::Diff` / `apply_diff` (`src/usd/diff.rs`) → undo/redo + mirroring
```rust
let diff: Diff = stage.extract_diff(&committed_change)?;   // capture an edit as data
mirror.apply_diff(&diff, ApplyMode::CurrentEditTarget)?;   // replay (retargeted) or ExactLayer
```
`Diff { edits: Vec<Edit>, mapping }` where `Edit ∈ {Create, SetField, EraseField,
RemoveSpec}`. **Undo/redo is application-level** (openusd has no built-in undo):
capture `extract_diff` on each commit, keep an undo stack; redo replays, undo
replays an inverse / a captured pre-state. CoW layer transactions make each apply
atomic.

### 2.6 Layers, variants, time
```rust
stage.mute_layer(id) / unmute_layer(id) / is_layer_muted(id) / muted_layers()  // incremental recompose
// variants: select via VariantSets / authoring a variant selection (no full reload)
usd::TimeCode::new(t)                       // value resolution
let q = AttributeQuery::new(&attr);         // cached source; reuse across a scrub
q.get_at(time)?; attr.time_sample_times()?; attr.bracketing_time_samples(t)?;
```
Layer muting + variant selection replace the entire `incremental.rs` /
manual-variant-switch machinery (269 lines + the viewer plumbing): mute/select →
sink fires `resynced` → reproject the affected subtree.

### 2.7 Copy-on-write transactions (`src/sdf/layer.rs`)
`edit_layers()` stages edits in per-layer CoW overlays, runs a veto/`before_commit`
phase, commits all atomically, then fires `after_commit` sinks; `GroupEditGuard`
rolls back every overlay on error/panic. `dry_run_layers()` validates with no
side effects (this is what `can_apply()` uses). Atomic, all-or-nothing — exactly
what an editor wants for a single user action / undo checkpoint.

---

## 3. Bevy 0.19 is editor-ready

(From the 0.19 release + migration guide; **[verify]** = single-source, confirm
the exact name before use.)

### 3.1 The live-sync engine: Observers + Events (best fit for `UsdNotice`)
- Observers register via `app.add_observer(sys)` or `entity.observe(sys)`, fire on
  **triggers** (lifecycle add/insert/replace/remove **and** user events), and now
  support **run conditions** (`.run_if(…)`, chainable).
- Manual dispatch: `commands.trigger(Event)` (global) /
  `commands.entity(e).trigger(Event)` (targeted, runs synchronously on flush).
- **Architecture:** model each USD change as a Bevy `Event`. A drain system reads
  the sink queue, maps `SdfPath → Entity`, and `commands.entity(e).trigger(UsdPrimChanged{…})`;
  observers (optionally `run_if`-gated, e.g. "not mid-undo") apply the precise
  component patch. Targeted, immediate, per-entity — no broad per-frame diffing.

### 3.2 Transform gizmo (viewer → editor in one step) **[verify field names]**
```rust
app.add_plugins(TransformGizmoPlugin);
camera: (Camera3d, TransformGizmoCamera)
target: (Mesh3d(..), TransformGizmoFocus)
// modes via TransformGizmoMode resource; tuning via TransformGizmoSettings
// NOT wired to input — you own selection / mode-switch
```
Read-back is the killer detail: the gizmo mutates the entity's `Transform`, so a
`Query<(Entity, &Transform), (With<TransformGizmoFocus>, Changed<Transform>)>`
system writes the delta back to the USD layer (author `xformOp:transform` /
decomposed T/R/S under the current `EditTarget`).

### 3.3 Drop bevy_glacial — these are native now
- **Infinite Grid** (`InfiniteGridPlugin` + `InfiniteGrid` + `InfiniteGridSettings`) **[verify]**
- **Text Gizmos** — `gizmos.text(iso, "label", size, …)` world-space labels **[verify args]**
- **Diagnostics Overlay** — FPS / mesh+material stats / custom **[verify presets]**
- Plus the **transform gizmo** above.
This removes `bevy_glacial` entirely (grid + axis/joint gizmos + labels) and its
rev-pinning headache.

### 3.4 Resources-as-Components, Relationships, hooks
- `#[derive(Resource)]` now also implements `Component` (stored on a singleton
  entity). You can observe / relate / hook resources. Marker `IsResource`;
  exclude with `Without<IsResource>` in broad queries.
- **Relationships** (`#[relationship]` / `#[relationship_target]`, new
  `allow_self_referential`) — `ChildOf`/`Children` is the canonical pair (and BSN
  `[ … ]` lists drive relationships at spawn). USD relationships (material
  binding, skel binding, collection membership) can be modeled as Bevy
  relationships.
- **Required components + hooks** let every USD-prim entity auto-acquire
  `Transform` + `Visibility` and run validation on insert.

### 3.5 BSN (for the *initial* projection — with caveats)
- `bsn!{ Comp { field: val } Children[ … ] }` — patches over `Default`; scene
  **functions** are plain `fn() -> impl Scene`, so we can **generate BSN
  programmatically** from the USD prim tree (`Children[…]` recursively). Spawn via
  `commands.spawn_scene(…)` / `queue_spawn_scene(…)` (waits on asset deps).
- **Hard constraints:** BSN is **spawn-time only** (no reconcile — live edits go
  through observers/Commands, §3.1), and there is **no `.bsn` file loader yet** →
  not a persistence path. Sigils (`@SceneComponent`, `#Name`, `:"file.bsn"`) and
  the `Template`/`FromTemplate` construction trait are **[verify]**.
- Realistic use: BSN for the one-shot stage→entity materialization (nice,
  declarative, asset-dependency-aware), then maintain the entity tree by hand via
  observers. Or skip BSN initially and direct-spawn with `Commands` (lower risk);
  adopt BSN once the projection is stable.

### 3.6 Asset saving / persistence
`save_using_saver()` is runtime-usable; handles serialize via path round-trip
(`HandleSerializeProcessor`). But for *USD* persistence the real path is **writing
the stage back to `.usd[a]`** via openusd's layer export — not Bevy's scene
serializer. Bevy asset-saving is only for non-USD side artifacts.

---

## 4. The round-trip architecture

```
                    ┌─────────────────────── openusd Stage (NonSend) ───────────────────────┐
                    │  composed prim tree · layers · variants · edit target · change sinks    │
                    └────────────────────────────────────────────────────────────────────────┘
   (A) PROJECT  ▲ initial + on `resynced`            │ (C) AUTHOR  edits land via EditTarget
               │  traverse → spawn entities          ▼
   ┌───────────┴───────────┐                ┌────────┴───────────────────────────────────────┐
   │  Bevy 0.19 entities    │                │  edit sources:                                  │
   │  UsdPrimRef(path)      │  (B) SYNC      │   • transform gizmo  → xformOp authoring        │
   │  Transform/Mesh3d/…    │ ◀── sink ───   │   • mara panels      → create_attribute/set     │
   │  Usd* schema markers   │  CommittedChange│   • tree ops         → NamespaceEditor          │
   │  SdfPath↔Entity bimap  │  →Event→observer│   • variant/layer    → mute/select             │
   └────────────────────────┘                └─────────────────────────────────────────────────┘
```

- **(A) Project** — stage traversal → one entity per prim, carrying `UsdPrimRef`
  + the typed `Usd*` components + Bevy `Transform`/`Mesh3d`/`MeshMaterial3d`/…
  Maintain a `SdfPath ↔ Entity` bimap resource.
- **(B) Sync** — `add_sink` pushes `CommittedChange` paths into a channel; a drain
  system triggers `UsdPrimChanged`/`UsdSubtreeResynced` events; observers patch or
  re-project exactly those entities. **Replaces `incremental.rs`.**
- **(C) Author** — every edit writes into the stage under the active `EditTarget`;
  that commit fires the sink → (B) re-projects. Single source of truth = the stage;
  entities are a derived view.

### Why this is clean
The editor never mutates entities *and* the stage independently. **All edits flow
into the stage; entities only ever change as a consequence of a stage change.**
That one rule kills the entire class of "panel and viewport disagree" bugs the old
viewer had (variant switching, visibility, material overrides).

---

## 5. "USD as BSN", concretely — three options

1. **Conceptual (recommended baseline).** USD plays BSN's role; projection is
   direct `Commands` spawning from a traversal. Lowest risk, full control, works
   today. The Usd* components already exist.
2. **BSN-generated projection.** Walk the prim tree, build `bsn_list!` /
   `Children[…]` programmatically, `spawn_scene` once. Gains asset-dependency
   resolution + a declarative spawn; still hand-maintained after spawn. Adopt once
   stable.
3. **USD-schema-as-SceneComponent (the deep version) [verify feasibility].** Each
   USD schema becomes a Bevy `SceneComponent`/`Template` whose constructor reads
   the prim's attributes and builds the Bevy components (e.g. a `UsdMeshScene`
   whose template emits `Mesh3d`+`MeshMaterial3d`). This makes the USD↔entity
   mapping declarative on *both* sides — the most "BSN-like" — but depends on BSN's
   `Template` API which is new and unconfirmed. Prototype-only until verified.

Recommendation: ship on (1), evaluate (2) for the initial load, treat (3) as
research.

---

## 6. Component / schema mirror

Keep the existing component vocabulary (`crates/usd_bevy/src/prim_ref.rs`,
`markers.rs`): `UsdPrimRef(path)`, `UsdKind`, `UsdDisplayName`, `UsdLocalExtent`,
the skel set (`UsdSkelRoot`, `UsdSkelAnimDriver`, `UsdBlendShapeBinding`), and the
physics markers (`UsdRigidBody`, `UsdCollider`, `UsdPhysicsJoint`, …). Add:

- `UsdPrimRef` stays the stable key; the **bimap** (`SdfPath↔Entity`) is the
  reconcile index.
- Use Bevy 0.19 **required components** so every projected prim entity gets
  `Transform` + `Visibility` automatically.
- Model USD **relationships** (material binding, skel binding, collections) as
  Bevy **relationships** where it buys traversal/maintenance.
- Per-schema **observers** apply `changed_info_only` field updates
  (e.g. `UsdGeomMesh.points` changed → rebuild that `Mesh`; `xformOp:*` changed →
  set `Transform`).

---

## 7. What to keep / rebuild / delete

| Current | Verdict | Why |
|---|---|---|
| `usd_bevy/src/read/*` (schema decoders) | **KEEP**, reconcile to upstream API | the reader layer is the value; will need signature fixups for openusd `06d619d` |
| `Usd*` components (`prim_ref.rs`, `markers.rs`) | **KEEP** | the schema mirror / ECS vocabulary |
| `usd_rapier` (physics decode) | **KEEP** | pure, reusable |
| mesh/material/texture/curve builders | **KEEP**, refactor for re-projection | reuse in the project step |
| `incremental.rs` (269 lines) | **DELETE** | replaced by `UsdNotice` sync |
| manual variant switching (build.rs + main.rs plumbing) | **DELETE** | replaced by edit-target/variant selection + sink reproject |
| `bevy_glacial` dep | **DELETE** | native grid + gizmos + labels in 0.19 |
| `build.rs` (3486 lines) — one-shot bake | **REBUILD** | restructure into project + incremental-reproject around the bimap |
| `main.rs` (1918 lines) | **REBUILD** | new plugin layout, editor systems |
| egui/mara UI | **KEEP** (just rebuilt), port to 0.19/egui 0.35 | |
| openusd pin `43abaf1` | **BUMP** to upstream `06d619d` | the whole edit/notice/diff model lives there |

---

## 8. Dependency upgrade plan (the gating order)

openusd is bevy-agnostic → bump it **independently and first**; the Bevy bump is
the gated chain.

1. **openusd `43abaf1` → upstream `06d619d`.** Big API churn — reconcile
   `usd_bevy/read/*` + `usd_rapier` against the new typed views. (No Bevy
   interaction.)
2. **mara → Bevy 0.19** (egui 0.35 / bevy_egui 0.40). **Gate**: the viewer cannot
   be on Bevy 0.19 while mara is on 0.18. This is your repo (mara 0.4?).
3. **bevy_glacial → deleted** (native replacements).
4. **viewer crates** (`usdview`, `usd_bevy`, `usd_rapier`) → Bevy 0.19 + egui 0.35
   + wgpu 29.
5. **rapier3d-f64** — confirm a compatible release (not bevy-coupled → likely fine).

### Bevy 0.18→0.19 breakage to budget for (migration guide)
- **`bevy_material` crate extraction** — `StandardMaterial` & friends move out of
  `bevy_pbr`; fix imports. glTF now yields `GltfMaterial` (use `/std` for
  `StandardMaterial`).
- **Custom `AssetLoader`/`Reader`** (`asset.rs` spills bytes to a tempfile and
  reads them) — `Reader` must now implement `Reader::seekable`; `AsyncSeekForward`
  removed; advanced loads use a builder/`NestedLoader`; `AssetPath::resolve` and
  `get_full_extension` signatures changed.
- **Feature flags no longer implied** by `3d` — must enable `ui`, `audio`,
  `bevy_window`, etc. explicitly.
- **Legacy scene rename** — `bevy_scene` → `bevy_world_serialization`,
  `Scene`→`WorldAsset`, `SceneRoot`→`WorldAssetRoot`. The viewer publishes a
  labeled `Scene` sub-asset today → this is a real rename to absorb (or move off
  the legacy scene path entirely in favor of direct spawning).
- Text stack Cosmic→**Parley** (`TextFont.font` is `FontSource`,
  `font_size`=`FontSize::Px`), `Ref: Copy`, `Assets::get_mut` returns `AssetMut`
  guard (avoid spurious `Modified`).

---

## 9. Hard constraints / open questions

1. **The live `Stage` is `!Send`/`!Sync`.** openusd uses `Rc<RefCell<…>>` interior
   mutability → the editable stage must live as a **`NonSend` resource** (main
   thread). Sinks fire on the editing thread (main) → main-thread channel drain is
   fine. *But*: no parallel system access to the stage; projection/sync systems
   must be `NonSend`. Design around this (decode work can still be parallel off
   owned snapshots).
2. **openusd `43abaf1 → 06d619d` is a large jump.** The typed-view reader layer
   (`read/*`) will need real reconciliation. Budget this as its own phase before
   any Bevy work.
3. **Reproject granularity.** Mapping `resynced` / `changed_info_only` paths to
   *minimal* entity edits is the core engineering. `resynced` = rebuild subtree;
   `changed_info_only` + `changed_fields(path)` = targeted component patch. Get the
   field→component routing table right.
4. **Undo/redo** is app-level (diff capture). Decide scope early (per-action diff
   stack vs. layer snapshots).
5. **mara on Bevy 0.19** is the schedule gate.
6. **BSN unknowns** — `Template`/`SceneComponent` API, no file loader. Keep BSN
   optional (§5 option 1) so the rethink doesn't block on unconfirmed APIs.
7. **Persistence = USD export**, not Bevy scene serialization. Confirm openusd's
   layer-save / export API covers the round-trip.

---

## 10. Phased roadmap

- **P0 — openusd bump.** Move pins to `06d619d`; reconcile `read/*` + `usd_rapier`;
  green headless reader tests. (No Bevy.)
- **P1 — Bevy 0.19 + deps.** mara→0.19, drop glacial, bump viewer crates; absorb
  the `bevy_material` / loader / scene-rename breakage. App runs (viewer parity).
- **P2 — live `Stage` + projection + bimap.** Stage as `NonSend`; project →
  entities; `SdfPath↔Entity` resource. Replace one-shot bake with project step.
- **P3 — `UsdNotice` sync.** `add_sink` → channel → events → observers; delete
  `incremental.rs`; variant/layer switching via select+reproject.
- **P4 — editing: transform gizmo.** Selection model; `TransformGizmoFocus`;
  `Changed<Transform>` → author xformOp under `EditTarget`. First real edit.
- **P5 — authoring panels + namespace ops.** mara panels write
  `create_attribute`/`set`; tree ops → `NamespaceEditor` (rename/reparent/delete);
  edit-target / variant / layer-mute UI.
- **P6 — undo/redo + persistence.** Diff stack; USD layer export ("Save").

---

### One-line summary
Rebuild `bevy_openusd` as a **live USD editor**: the composed USD stage is the
authoritative scene (BSN's role), projected into Bevy 0.19 entities, edited via
the native transform gizmo + mara panels, and kept in sync by `UsdNotice` →
observers — the reconcile loop BSN doesn't have. Keep the reader/component layer,
delete `incremental.rs` + `bevy_glacial`, bump openusd to upstream and Bevy to
0.19.

---

## 11. Verified Bevy 0.19 API (supersedes the **[verify]** flags above)

Checked against docs.rs/0.19.0 + the `v0.19.0` source/examples. **Bevy 0.19
requires Rust 1.95.**

- **Transform gizmo — first-party ✅** `bevy::gizmos::transform_gizmo`:
  `TransformGizmoPlugin`, `TransformGizmoCamera` (component on the camera),
  `TransformGizmoFocus` (component on the editable entity — *only one at a time*),
  `TransformGizmoMode` (enum Translate/Rotate/Scale), `TransformGizmoSpace`,
  `TransformGizmoSettings` (impls **both `Resource` and `Component`**; fields:
  `mode, space, axis_length, rotate_ring_radius, axis_hit_distance,
  snap_translate/rotate/scale: Option<f32>, confine_cursor, screen_scale_factor`).
  Needs picking (`MeshPickingPlugin`); **not wired to input** (you own selection).
  Read-back = the focused entity's own `Transform` is mutated →
  `Query<&Transform, (With<TransformGizmoFocus>, Changed<Transform>)>`.
  (Distinct from third-party `transform-gizmo-bevy`/`GizmoTarget`.)
- **Infinite grid — first-party ✅** `bevy::dev_tools::infinite_grid`:
  `InfiniteGrid`, `InfiniteGridPlugin`, `InfiniteGridSettings`.
- **Text gizmos ✅** `gizmos.text(iso: impl Into<Isometry3d>, text: &str,
  font_size: f32, anchor: Vec2, color: impl Into<Color>)` (+ `text_2d`,
  `text_sections`). Pose is bundled into the `Isometry`, no separate position arg.
- **FPS / diagnostics overlay — CORRECTED ❌→✅** It is **`bevy::dev_tools::fps_overlay`**:
  `FpsOverlayPlugin`, `FpsOverlayConfig`, `FrameTimeGraphConfig`. The earlier
  `DiagnosticsOverlay::fps()` / `DiagnosticsOverlayPlugin` names **do not exist**.
- **Observers ✅** `Observer::run_if(impl SystemCondition)` exists (AND, chainable).
  The event-wrapper param in 0.19 is **`On<E>`**, *not* `Trigger<E>`
  (`on(|ev: On<Pointer<Press>>| …)`). `App::add_observer`, `Commands::trigger` /
  `trigger_with`, `EntityCommands::observe`.
- **BSN ✅** crate `bevy::scene`: `bsn!` / `bsn_list!` macros, traits `Scene` /
  `SceneList`, `#[derive(SceneComponent)]`, traits `Template` / `FromTemplate`
  (**no `Construct`**). Spawn: `scene.spawn()` returns a *system*
  (`add_systems(Startup, level.spawn())`), or `CommandsSceneExt::spawn_scene` /
  `queue_spawn_scene`. Sigils confirmed: `#Name` (→ `Name`, and entity refs),
  `@SceneComp` + `@prop: val`, `:scene()` cached include. **No `.bsn` file loader
  in 0.19** (`:'file.bsn'` documented "not yet implemented"). → BSN is
  **initial-projection-only**; persistence is **USD layer export**, never BSN files.
- **Resources-as-Components ✅** `pub trait Resource: Component`; marker
  `IsResource` (+ `ResourceEntities`, `IS_RESOURCE`). `Res`/`ResMut` still the
  daily access; exclude resource entities from broad queries with
  `Without<IsResource>`.

---

## 12. Reproject routing — turning a `CommittedChange` into minimal entity edits

The sink hands us two path sets per commit. The routing:

### `resynced` paths → **rebuild that subtree**
Composition of the prim changed structurally (define / remove / reparent / rename /
reference / payload / variant / inherit / activation / instanceable; layer
mute/unmute reports as **pseudo-root** resync). Action: for each path, look up the
entity via the bimap, **despawn its entity subtree** (`Children`) and **re-project
that prim subtree** from the stage. Pseudo-root ⇒ reproject whole stage (or scope
to the muted layer's affected prims if we want to be surgical later).

### `changed_info_only` paths → **targeted component patch**
A field/value/target changed but namespace is intact. Route by the property name
(`change.changed_fields(path)` gives the touched fields); each maps to a re-read +
component patch (reusing today's `read/*` decoders):

| USD property (on the prim) | re-read | patch |
|---|---|---|
| `xformOp:*`, `xformOpOrder` | `xform::read_transform` | `Transform` |
| `visibility` | `geom::read_visibility` | `Visibility` |
| `purpose` | `geom::read_purpose` | `Visibility` + `UsdPurpose` (proxy/guide → Hidden) |
| `points` `faceVertexCounts` `faceVertexIndices` `normals` `primvars:st` `subsets` `orientation` `doubleSided` `subdivisionScheme` | `geom::read_mesh` | rebuild `Mesh3d` (+ `UsdLocalExtent`) |
| `extent` | (from mesh/extent) | `UsdLocalExtent` |
| Cube/Sphere/Cyl/Capsule `size`/`radius`/`height` | `geom::read_*` | rebuild `Mesh3d` |
| `material:binding` (rel) | `shade::read_material_binding` | swap `MeshMaterial3d` handle |
| UsdPreviewSurface `inputs:*` / connected `UsdUVTexture.inputs:file` | `shade::read_preview_material` | rebuild the bound `StandardMaterial` asset → propagates to *all* meshes binding it |
| light `inputs:intensity/color/radius/coneAngle/…` | `lux::read_light` | patch `DirectionalLight`/`PointLight`/`SpotLight` |
| camera `focalLength/horizontalAperture/clippingRange/projection` | `camera::read_camera` | patch projection (if mounted) |
| `kind` | `read_kind` | `UsdKind` |
| `ui:displayName` | `ui::read_display_name` | `UsdDisplayName` |
| audio `filePath/…`, procedural | `media`/`proc` | `UsdSpatialAudio`/`UsdProcedural` |
| skel `skel:skeleton`, `primvars:skel:joint*`, `restTransforms`, `bindTransforms` | `skel::*` | rebuild `SkinnedMesh` / skel cache — heavier; treat as a **scoped resync** of the `SkelRoot` subtree |
| `xformOp:*.timeSamples`, clip metadata | — | **not** a reproject — these feed the playback clock (`AttributeQuery`/`time_sample_times`); value-varying ≠ structural |

Implementation: a `field_prefix → Reproject` routing table + a `dirty: HashSet<Entity>`
coalesced per frame (multiple field changes on one prim → one patch pass). Material
edits dirty the **material asset**, not each mesh — Bevy's asset change-detection
fans out. This whole module is what replaces `incremental.rs`.

---

## 13. Undo / redo — typed actions with inverse capture (diff as fallback)

openusd has **no built-in undo** (CoW overlays are ephemeral), so it's
application-level. Two layers:

### A. Typed `EditorAction` (the primary path)
Every editor mutation goes through one funnel that captures its inverse *before*
authoring, then performs an atomic openusd edit:

```rust
enum EditorAction {
    SetAttr { path, field, new: Value },          // inverse: SetAttr(old) or EraseAttr if no prior opinion
    DefinePrim { path, type_name },               // inverse: RemovePrim
    RemovePrim { path },                          // inverse: re-author captured subtree Diff
    Rename { prim, new_name }, Reparent { prim, new_parent },  // inverse: reverse NamespaceEditor move
    SelectVariant { prim, set, was, now },        // inverse: SelectVariant(was)
    MuteLayer { id, was },                        // inverse: (un)mute
}

fn apply(stage, action) -> Result<()> {
    let inverse = capture_inverse(stage, &action)?;  // reads current authored state at touched paths
    let _ctx = stage.edit_context(current_edit_target)?;  // RAII target
    perform(stage, &action)?;                        // define/create/set/NamespaceEditor.apply/mute/...
    undo_stack.push((action, inverse)); redo_stack.clear();
    Ok(())  // sink fires → §12 reproject
}
```
- `capture_inverse` reads the *current* value at the edit target (`SetAttr` old
  value; `RemovePrim` → `extract` the authored subtree as a `Diff` to replay on
  undo). For namespace ops the inverse is the reverse `move_prim` (NamespaceEditor
  is symmetric). Variant/mute inverses are trivial (prior selection).
- Each `apply`/`undo` is one **CoW transaction** (atomic), and produces exactly one
  `CommittedChange` → one reproject pass.
- **Coalescing:** a gizmo drag emits a `SetAttr(xformOp:transform)` per frame —
  bracket it (drag-begin captures the inverse once, drag-end commits one undo
  entry), so a drag is a single undoable action.

### B. Diff-based fallback (for edits that bypass the funnel)
For arbitrary/scripted edits, the sink already gives us the forward `Diff`
(`stage.extract_diff(change)`); pair it with a *pre-state* diff (read old values
for `changed_fields(path)`) to synthesize an inverse `Diff`, and push that.
`apply_diff(inverse, ExactLayer)` on undo. Slower + less precise than the typed
path but catches everything.

Recommendation: build (A) first (covers all editor UI actions cleanly); keep (B)
as a safety net once edits can come from outside the UI.

---

### One-line summary (final)
Rebuild `bevy_openusd` as a **live USD editor**: the composed USD stage is the
authoritative scene (BSN's role), projected into Bevy 0.19 entities, edited via the
**first-party transform gizmo** + mara panels, kept in sync by **`StageSink`
(`UsdNotice`) → events → observers** (§12 routing), with **typed-action undo/redo**
(§13). Keep the `read/*` + `Usd*` layer, delete `incremental.rs` + `bevy_glacial`,
bump openusd → upstream and Bevy → 0.19.
