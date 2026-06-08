# PLAN_COLLECTION — UsdCollectionAPI for `mxpv/openusd`

Design doc for contributing **UsdCollectionAPI** (spec §15) — the
working-set / membership system — to the upstream `mxpv/openusd` crate.
Captures the schema surface, the membership-resolution crux, the
relationship-vs-expression scope split, the conformance approach, and a
detailed multi-commit plan.

---

## 0. Status — unstarted, explicitly on the roadmap

ROADMAP has a dedicated **Collections (Spec 15)** section, both rows
`:construction:`:

- `CollectionAPI` (§15.1)
- Authoring and evaluating collections (§15.2)

No collection code exists today. The relationship targets are *read raw*
in a couple of places without any membership evaluation:
`src/schemas/physics/read.rs` reads `collection:colliders:includes`
directly, and `src/schemas/shade/binding.rs` reads/authors the
collection-binding relationship — neither evaluates membership.

**Home:** UsdCollectionAPI is **core USD** (it lives in `pxr/usd/usd`, not
a schema family), and it is consumed across features. So it belongs in
`src/usd/collection.rs` (alongside `connections.rs`), **always compiled,
not feature-gated** — that way `shade` / `render` / `physics` / `lux` can
all use it without a feature-dependency tangle.

## 1. Why now — the seams it closes

Collections are the missing shared primitive under four consumers:

| Consumer | What it unblocks |
|---|---|
| **UsdShade** `MaterialBindingAPI` | collection-based binding → the collection-aware `ComputeBoundMaterial` (PLAN_SHADE commit 7) |
| **UsdRender** `RenderPass` | `renderVisibility` / `cameraVisibility` / `prune` / `matte` memberships (the gated seam in the merged render PR) |
| **UsdPhysics** `CollisionGroup` | `colliders` collection evaluation (the `// tracked under Spec 15` note) |
| **UsdLux** light-linking | `light:filters` / linkage collections (future) |

The merged render code already documents the gap: `render/types.rs` +
`render/author/pass.rs` say the collection memberships are "deferred until
collection-membership evaluation lands." This plan is that landing.

## 2. The schema surface (`UsdCollectionAPI`, multi-apply)

A multiple-apply API: applied with an **instance name**, all properties
under `collection:<name>:`. A prim can carry several collections under
different instance names; they surface in `apiSchemas` as
`CollectionAPI:<name>` (the same way physics decodes `PhysicsLimitAPI:<dof>`).

| Property | Type | Default | Role |
|---|---|---|---|
| `collection:<name>:expansionRule` | `uniform token` | `expandPrims` | how includes/excludes expand |
| `collection:<name>:includeRoot` | `uniform bool` | `false` | treat `</>` as an implicit include |
| `collection:<name>:includes` | `rel` | — | included paths (prims / properties / other collections) |
| `collection:<name>:excludes` | `rel` | — | excluded paths (should be descendants of an include) |
| `collection:<name>:membershipExpression` | `uniform pathExpression` | — | expression-mode predicate — **deferred** |

`expansionRule` tokens: `explicitOnly` (exact paths only), `expandPrims`
(prim descendants of includes), `expandPrimsAndProperties` (also each
included prim's properties).

Newer Pixar revisions add `collection:<name>:mode`
(`automatic`/`relationship`/`expression`) and an opaque `collection:<name>`
chaining handle. Whether the pinned revision has these needs a check; the
plan targets the **classic relationship model** and infers the mode the
old way (relationship if `includes`/`excludes`/`includeRoot` authored).

**Note on the value type:** `Value::PathExpression` already exists in the
crate (round-trips as a string), so `membershipExpression` can be
read/authored as a raw string today — only its *evaluation* is deferred
(§4).

## 3. The crux — membership resolution

This is the entire value of the feature and the only subtle part. Two
layers, both pure (stage-free) over a resolved rule map.

### 3a. `CollectionMembershipQuery` + `PathExpansionRuleMap`

`PathExpansionRuleMap = HashMap<Path, ExpansionRule>` mapping each
authored/derived path to its rule — plus an `Exclude` sentinel for
excluded paths. Built once per collection, then cheap to query and clone
(so consumers can cache `HashMap<Path, MembershipQuery>`).

### 3b. `is_path_included` — the resolution rule

Two overloads, mirroring C++:

1. **`is_path_included(path)`** — walk ancestors from `path` to root; at the
   **closest ancestor with an opinion** in the map:
   - `Exclude` → not included;
   - prim path → included if the rule isn't `explicitOnly`, *or* the
     opinion is on the path itself;
   - property path → included only if the matched rule is
     `expandPrimsAndProperties`, or the property itself is an explicit
     target. "Closest-ancestor-opinion-wins" is the whole game.
2. **`is_path_included_with_parent(path, parent_rule)`** — the O(1)
   downward-traversal fast path: given the parent's resolved rule, decide
   the child without re-walking. `Exclude`/`explicitOnly` parent → child
   not auto-included; otherwise inherit the parent's rule. This is what the
   stage traversal (§3d) uses to propagate rules top-down.

### 3c. `compute_membership_query` + nested collections

Fold authored opinions into the map: each `includes` target gets the
collection's `expansionRule`; each `excludes` target gets `Exclude`;
`includeRoot && rule != explicitOnly` injects `</>`. When an `includes`
target **is itself another collection's path**, recurse and **merge
(overwrite)** its map in — guarded by a `chained_collection_paths` set
seeded with this collection's own path, warning + skipping on a cycle.

### 3d. `compute_included_paths` / `compute_included_objects`

Enumerate members against a live stage. Per map entry: skip `Exclude`;
`explicitOnly` emits the exact path; `expandPrims` /
`expandPrimsAndProperties` descend the subtree, emitting prims (and, for
the latter, properties), **pruning excluded subtrees**.

> Implementation note: the crate's `Stage::traverse(predicate, visitor)`
> visits a whole subtree and **can't prune**, so the excludes-pruning walk
> uses manual recursion over `Stage::prim_children` (checking
> `is_path_included` / the parent-rule fast path as it descends). Takes a
> prim-flags predicate like the rest of the schema readers.

## 4. Scope split — relationship mode now, expression mode deferred

**In scope (foundational, this plan):** the relationship-linking mode —
`includes` / `excludes` / `expansionRule` / `includeRoot`, the membership
query, nested collections, computed includes, and authoring. This already
satisfies material binding, render-pass visibility, collision groups, and
light-linking for the common case.

**Deferred (large, separate subproject):** the `membershipExpression` /
`SdfPathExpression` engine — the glob lexer (`*`, `?`, `//`), set algebra
(`+`/`&`/`-`/`~`), the predicate library (`isa`, `hasAPI`, `kind`,
`model`, `variant`, …), and expression references with cycle handling. The
value type exists; the engine is its own multi-commit effort. Until it
lands, expression-mode collections are reported (`has_expression()`) but
evaluated as empty — Pixar's own graceful-degradation behavior.

## 5. Conformance approach

`vendor/usd-wg-assets` may carry a collection asset or two; worth a scan,
but the plan assumes **no reliable vendored oracle** and validates like the
schema PRs:

- hand-authored `fixtures/usdCollection_*.usda` exercising each shape;
- in-memory author → query roundtrips;
- a focused **unit matrix** for §3 (the crux): `explicitOnly` vs
  `expandPrims` vs `expandPrimsAndProperties`; `includeRoot` + excludes
  ("everything but X"); nested-collection merge; cycle guard;
  closest-ancestor precedence; property-vs-prim membership.

## 6. Commit plan

Lands in `src/usd/collection.rs` (+ a `membership` submodule if it grows),
always compiled. Each commit builds + tests under `--all-features`, fmt +
clippy (`-D warnings`, 1.89) + `cargo doc` clean, and uses `sdf::FieldKey`
constants (not string literals — the recurring review flag).

### Commit 1 — `feat(usd): UsdCollectionAPI schema + tokens`
- `UsdTokens`-style consts (`collection:`, `expansionRule`, `explicitOnly`,
  `expandPrims`, `expandPrimsAndProperties`, `includeRoot`, `includes`,
  `excludes`, `membershipExpression`); `ExpansionRule` enum
  (`as_token`/`from_token`, default `ExpandPrims`).
- Instance-name model: `is_collection_api_path`, `named_collection_path`,
  `collections_on(prim)` (scan `apiSchemas` for `CollectionAPI:<name>`),
  property-name templating, `CanContainPropertyName`-equivalent.
- Read accessors: `expansion_rule`, `include_root`, `includes`, `excludes`,
  raw `membership_expression`.
- Tests: instance decode, property naming, accessor reads.

### Commit 2 — `feat(usd): collection membership query`  ⚠ crux
- `CollectionMembershipQuery` + `PathExpansionRuleMap`, both
  `is_path_included` overloads, `has_excludes`, `uses_path_expansion_rule_map`.
- Pure / stage-free. The §3 unit matrix lives here.

### Commit 3 — `feat(usd): compute membership query + nested collections`
- `compute_membership_query`: fold includes/excludes/includeRoot/rule into
  the map; recurse into chained collections with the cycle guard +
  merge-overwrite.
- Tests: nested-collection merge, cycle skip, includeRoot injection.

### Commit 4 — `feat(usd): compute included paths + objects`
- `compute_included_paths(query, stage, predicate)` /
  `compute_included_objects` via manual prunable recursion over
  `prim_children`; `explicitOnly` / `expandPrims` /
  `expandPrimsAndProperties` + excludes pruning + property emission.
- Tests: each expansion rule, excludes-pruning, "everything but X".

### Commit 5 — `feat(usd): collection authoring`
- `apply_collection(prim, name)`, `IncludePath` / `ExcludePath` with the
  edit-minimization semantics (drop a stale exclude rather than add a
  redundant include; special-case `</>` → `includeRoot`), `set_expansion_rule`,
  `set_include_root`, `reset_collection`, `block_collection`,
  `has_no_included_paths`.
- Tests: include/exclude roundtrip, edit minimization, reset vs block.

### Commit 6 — `docs(usd): ROADMAP — flip Collections (Spec 15)`
- Mark §15.1/§15.2 supported; note expression mode deferred.

### Follow-ups (separate PRs — the payoff this unblocks)
- `feat(shade): collection-aware ComputeBoundMaterial` — PLAN_SHADE commit 7,
  with the `CollectionQueryCache` shape.
- `feat(render): RenderPass collection memberships` — fills the merged
  render PR's gated seam.
- `feat(physics): CollisionGroup collider evaluation`.

## 7. Decisions baked in

- **Core, not feature-gated** — `src/usd/collection.rs`, like `connections.rs`;
  reusable by every schema feature.
- **Query is a cheap, clone-friendly value**; the heavy `compute_included_*`
  traversal is free functions taking a stage — mirrors Pixar's split and
  makes consumer-side caching trivial.
- **Relationship mode fully; expression mode deferred** (value type exists,
  engine doesn't) — never silently wrong: expression-only collections
  report `has_expression()` and evaluate empty until the engine lands.
- **Manual prunable recursion** for `compute_included_*` since
  `Stage::traverse` can't prune excluded subtrees.
- **Closest-ancestor-opinion-wins** with the parent-rule fast path for
  top-down traversal — the exact C++ resolution.
- **Conformance** via hand-authored fixtures + in-memory roundtrips + the
  §3 unit matrix (no assumed vendored oracle).
