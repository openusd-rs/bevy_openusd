# PLAN_SHADE — deepening UsdShade for `mxpv/openusd`

Design doc for taking **UsdShade** from "reader + basic authoring" (PR #88) to a
spec-faithful network/binding implementation in the upstream `mxpv/openusd`
crate. Captures the baseline, the parity gap, the two hard resolution
algorithms (connection value-production, bound-material resolution), the
conformance approach, and a detailed multi-commit plan.

---

## 0. Baseline — what PR #88 already lands

UsdShade currently lives **only in open PR #88** (`feat(shade): UsdShade reader
+ authoring behind shade feature`), built on the merged connection substrate
(`src/usd/connections.rs` `ConnectionGraph`; `Attribute::{set,add,remove,clear}_connections`
in `src/usd/prim.rs`). #88 ships, behind the `shade` feature:

- `schemas/shade/{tokens,types,connectable,shader,material,binding,read,preview}.rs`
- **Author**: `define_material` / `define_node_graph` / `define_shader`; connectable
  `inputs:` / `outputs:` creation; surface/displacement/volume terminals incl.
  render-context-namespaced (`outputs:ri:surface`); `NodeDefAPI` (`info:id`,
  implementationSource, sourceAsset/+subId, sourceCode); `MaterialBindingAPI`
  direct + collection + purpose-namespaced + `bindMaterialAs` strength.
- **Read**: input values, input/output connections, shader id, `resolve_surface_shader`
  (single-hop terminal follow with universal fallback), `find_shade_prims`,
  `read_preview_surface` (UsdPreviewSurface + UsdUVTexture channel harvest).

This plan **assumes #88 is merged** and extends it. If #88 is still in review,
commit 1 below rebases onto it.

## 1. The parity gap (what #88 does *not* do)

| Capability | #88 | This plan |
|---|---|---|
| Connection **value production** (`GetValueProducingAttributes`): follow a chain to the attr that actually supplies the value | single-hop `.connect` follow only | full DFS resolver (commit 3) |
| Typed **read** wrappers (`Material`/`Shader`/`NodeGraph`/`Input`/`Output` over a `Prim`) | free functions only | yes (commit 2) |
| **Connectability** (`full` / `interfaceOnly`) author + read + enforce | const declared, never wired | yes (commit 2) |
| **NodeGraph** interface inputs, output→source compute, interface-consumer map | author handle only | yes (commit 4) |
| **Material** `ComputeSurfaceSource` w/ ordered render-context vector | single-hop, single context | yes (commit 5) |
| **ComputeBoundMaterial** (namespace inheritance + strength + purpose + collection precedence) | per-rel read only | direct (commit 6), collection-aware (commit 7) |
| **CoordSysAPI** (multi-apply `coordSys:<n>:binding` + inheritance) | absent | yes (commit 8) |
| **UsdPreviewSurface** as built-in node defs (typed defaults, not just a reader) | reader only | yes (commit 9) |

**Out of scope** (documented, not attempted): the live Sdr/Ndr shader registry,
`UsdShadeShaderDefUtils` / parser plugins, `ConnectableAPIBehavior` plugin
registration, legacy `look:binding` / non-applied `coordSys` compatibility,
renderer dialects (MDL, MaterialX `standard_surface`) — left to consumers via
`read_shader_id`.

---

## 2. The two crux algorithms

Everything novel in this plan is one of two resolutions. Both are pure read-side
graph walks over the composed stage; both are easy to get subtly wrong.

### 2a. Connection value production (`GetValueProducingAttributes`)

Given a shading `Input`/`Output`, return the attribute(s) that actually supply
the value (or shader output(s) to wire), following connections transitively.
The spec rules (from the "UsdShade Connections" doc, normative):

1. **Connection beats authored value.** If an input has both an authored value
   and a connection, the connection alone is transmitted; the value is ignored.
2. **Missing-output exception (the one case the value wins).** A connection that
   targets an output that does not exist in the containing Material is ignored;
   *then, and only then*, an authored value on that input is used.
3. **Outermost default wins.** In an interface chain (shader input → NodeGraph
   input → outer NodeGraph/Material input), the **outermost authored default**
   in the chain provides the value.
4. **No fallback.** If nothing in the chain authors a value, emit nothing.

Implementation: depth-first trace, "flatten" logical connections, return a
`Vec` (multi-connection) that may be empty. A `shader_outputs_only` flag reports
only non-container (shader) outputs. **This is the riskiest commit** — it gets a
dedicated network-shape test matrix.

### 2b. Bound-material resolution (`ComputeBoundMaterial`)

Given a prim and a requested purpose, resolve the winning bound material. The
six rules (normative):

1. **Namespace inheritance** — bindings inherit down; closer-to-leaf is stronger
   **unless** an ancestor binding is `strongerThanDescendants`.
2. **Collection scope** — a collection binding applies only to members at/below
   the prim that owns the binding rel.
3. **Purpose match/fallback** — resolved purpose must equal the requested
   restricted purpose **or** be all-purpose; a restricted match is preferred.
4. **Collection beats direct** at a single prim.
5. **Native property order** among collection bindings — earlier = stronger.
6. **Name specificity in a collection is irrelevant** to strength.

The standard bug is rule 1 vs 3–4: walk leaf→root, evaluate
collection-then-direct with purpose-matching at each level, and apply
`strongerThanDescendants` as a **separate override pass** (it flips the normal
"closer wins"). Rules 2/4/5 need `UsdCollectionAPI` membership — hence the
direct/collection split (commits 6 / 7).

---

## 3. Conformance approach

There are **no vendored UsdShade conformance assets** in `vendor/` (the AOUSD
reference skips them). So, exactly like the merged schema PRs, validation is:

- hand-authored `fixtures/usdShade_*.usda` exercising each network/binding shape;
- in-memory **author → read-back roundtrips** (`Stage::builder().in_memory(..)`);
- a dedicated unit-test matrix for the two crux algorithms (§2), enumerating
  network shapes (straight chain, interface chain, missing output, multi-connect,
  cycle) and binding shapes (nested direct, ancestor-stronger, purpose fallback,
  collection-vs-direct).

#88's `fixtures/usdShade_scene.usda` + `tests/shade_reader.rs` are the seed;
each commit extends them.

---

## 4. Commit plan

Each commit builds + tests under `--all-features`, fmt + clippy (`-D warnings`,
1.89) clean, `cargo doc --no-deps` clean. Authoring lands under `schemas/shade/`
following the lux `author/` convention where it grows past one file.

### Commit 1 — `feat(shade): typed Input/Output + connectability`
- `UsdShadeInput`/`UsdShadeOutput` wrappers over a single `Attribute`: full/base
  name, `is_input`/`is_output` predicates, typed value get/set, `renderType`.
- Connectability metadata: `set_connectability`/`get_connectability`/`has`/`clear`
  (`full` ↔ `connectable=1`, `interfaceOnly` ↔ `0`); wire the dormant
  `Connectability` type + `META_CONNECTABILITY` const.
- `sdrMetadata` get/set/has/clear (+ by-key) as a composed dict.
- Tests: prefix round-trip, empty-token aliasing (universal ctx == allPurpose ==
  universalSourceType all `""`), connectability author/read.

### Commit 2 — `feat(shade): typed read wrappers over prims`
- Read-side `Material`/`Shader`/`NodeGraph` structs wrapping a `Prim` (mirror the
  lux author-handle idiom but read-only), type-gated constructors returning
  `Option`, `ConnectableAPI`-style `get_input(s)`/`get_output(s)`, `is_container`.
- Migrate the #88 free-function readers to delegate to these (no behavior change).
- Tests: type-gating (a Mesh is not a Material), input/output enumeration.

### Commit 3 — `feat(shade): connection value-production resolver`  ⚠ riskiest
- `get_value_producing_attributes(input_or_output, shader_outputs_only) -> Vec<Path>`
  implementing §2a: DFS chain follow, connection-beats-value, missing-output
  exception, outermost-default, multi-connection, cycle-safe.
- Instance front-ends on `Input`/`Output`.
- Tests: the §2a network-shape matrix; reuse `ConnectionGraph::resolve_chain`
  where it already does cycle-safe DFS.

### Commit 4 — `feat(shade): NodeGraph interface + output source`
- `get_interface_inputs`, `compute_output_source(output) -> Option<(Path, name, type)>`,
  `compute_interface_input_consumers_map(transitive)`.
- Depends on commit 3.
- Tests: interface-input → consumer mapping; output resolves to a shader output.

### Commit 5 — `feat(shade): Material terminal source compute`
- `compute_surface_source(contexts: &[&str])` / displacement / volume: try each
  context in order, fall back to universal (`""`); follow the terminal via
  commit 3 to the producing shader. Replace #88's single-hop `resolve_surface_shader`.
- Tests: ordered context vector (`["ri",""]`), universal fallback, no-terminal.

### Commit 6 — `feat(shade): direct bound-material resolution`
- `compute_bound_material(prim, purpose) -> Option<(material_path, binding_rel)>`
  for **direct** bindings only: leaf→root namespace walk, purpose match/fallback,
  `strongerThanDescendants` override pass (§2b rules 1, 3).
- `BindingsCache` analogue (optional ancestor-binding memo).
- Tests: nested binding (closer wins), ancestor `strongerThanDescendants` flips,
  restricted-purpose preferred over all-purpose, purpose fallback to all-purpose.

### Commit 7 — `feat(shade): collection-aware bound-material resolution`
- Extend commit 6 with collection bindings (§2b rules 2, 4, 5): collection-beats-
  direct, native property order, collection scope. **Gated on `UsdCollectionAPI`
  membership** — if absent in core, this commit also lands a minimal
  membership-query shim or is deferred with a `// TODO(collection)` seam and the
  resolver documents the limitation.
- Tests: collection vs direct at one prim, two collections in property order.

### Commit 8 — `feat(shade): CoordSysAPI bindings`
- Multi-apply `coordSys:<name>:binding` author/read; `find_binding_with_inheritance`
  (ancestor walk); `local_binding`. Adopt the multi-apply form only (skip legacy).
- Tests: local binding, inherited binding from ancestor.

### Commit 9 — `feat(shade): UsdPreviewSurface built-in node defs`
- Hardcoded definitions (input names, types, spec defaults) for `UsdPreviewSurface`,
  `UsdUVTexture`, `UsdPrimvarReader_*` — a stand-in for the absent Sdr registry,
  so callers get correct fallbacks without the plugin system.
- Fold the #88 `preview.rs` reader onto these defs (single source of defaults);
  honor `wrapS/wrapT = useMetadata` and `sourceColorSpace = auto` (don't force).
- Tests: default harvest, texture vs scalar channel, primvar reader result type.

### Commit 10 — `docs(shade): ROADMAP + network/binding notes`
- Flip the ROADMAP UsdShade row's "Remaining" to reflect networks + binding
  resolution + coordSys; document the out-of-scope set (§1).

---

## 5. Decisions baked in

- **Read wrappers, not free functions** for the deepened surface; migrate #88's
  readers onto them without behavior change.
- **Two crux algorithms isolated** in their own commits with dedicated matrices
  (connection production §2a → commit 3; bound material §2b → commits 6/7).
- **Direct binding fully; collection binding gated** on `UsdCollectionAPI`
  membership — never silently return a wrong material when membership is unknown.
- **Base material via specializes** reuses the existing PrimIndex specializes
  arc; if a gap surfaces it is documented, not worked around.
- **No Sdr registry**: `UsdPreviewSurface` et al. ship as built-in defs; renderer
  dialects stay with consumers.
- **Conformance**: hand-authored fixtures + in-memory roundtrips (no vendored
  oracle), matching every merged schema PR.
