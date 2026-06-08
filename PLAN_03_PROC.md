# PLAN_03_PROC — UsdProc for `mxpv/openusd`

Design doc for contributing **UsdProc** (read + author). ROADMAP lists it as
`:construction:`. Small family centered on `GenerativeProcedural`.

## 0. Status

Not upstream. Downstream `usd_schema/src/proc.rs` has `read_procedural`
against an older rev — port up, add authoring, follow merged conventions.

## 1. The schema surface (`pxr/usd/usdProc`)

**`GenerativeProcedural`** — a concrete typed prim, IS-A `UsdGeomBoundable`
(Boundable + Imageable + Xformable: transform, visibility, purpose, extent).
It marks a prim whose children are generated at runtime by a named
procedural system (Houdini Engine, RenderMan procedurals, …).

| Attribute | Type | Meaning |
|---|---|---|
| `proceduralSystem` | `uniform token` | names the runtime that evaluates the procedural |

Plus the inherited Boundable `extent` (`float3[]`) and the Xformable/Imageable
stack. Subclasses (e.g. a renderer's own procedural type) are out of scope —
the base `GenerativeProcedural` + `proceduralSystem` is the contribution.

## 2. Scope — read + author

- **Read:** `read_generative_procedural(stage, prim) -> Option<…>`, type-gated,
  surfacing `proceduralSystem` + inherited `extent` and Xformable data
  (reuse the geom Boundable/Xformable readers).
- **Author:** `define_generative_procedural(...)` with the shared
  Boundable/Imageable/Xformable setters + `set_procedural_system`.

## 3. Conformance

Hand-authored `fixtures/usdProc_scene.usda` + in-memory roundtrips. Cover
`proceduralSystem` and an authored `extent`.

## 4. Commit plan

1. `feat(proc): GenerativeProcedural tokens + types`
2. `feat(proc): GenerativeProcedural reader`
3. `feat(proc): GenerativeProcedural authoring + roundtrip tests`
4. `docs: mark UsdProc supported in ROADMAP`

(Foldable into one PR with per-concern commits.) Builds + tests
`--all-features`; fmt/clippy/doc clean.

## 5. Decision / dependency note

It IS-A Boundable, so it composes with the geom transform/extent layer rather
than duplicating it. If **#93** lands first, write as `GenerativeProcedural(Prim)`
with `Boundable`/`Xformable` supertraits; otherwise current style + migrate.
