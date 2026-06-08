# PLAN_05_VOL — UsdVol for `mxpv/openusd`

Design doc for contributing **UsdVol** (read + author). ROADMAP lists it as
`:construction:`. This is the one remaining concrete-typed schema family with
no downstream reader either — a clean greenfield, included for full schema
coverage.

## 0. Status

Not upstream, and not in the downstream `usd_schema` crate (never needed it).
Greenfield — build it the way the merged schema families are built.

## 1. The schema surface (`pxr/usd/usdVol`)

- **`Volume`** — concrete typed prim, IS-A `UsdGeomGprim` (Boundable +
  Imageable + Xformable). A renderable volume that aggregates fields. It owns
  **relationships** named `field:<name>` (one per field), each targeting a
  `FieldAsset`-derived prim. Reading a Volume = enumerating its `field:*`
  relationships → `(field name, target path)`.
- **`FieldBase`** — abstract base for field prims (IS-A Xformable).
- **`FieldAsset`** — abstract, IS-A `FieldBase`. Common attributes:
  `filePath` (`asset`), `fieldName` (`token`), `fieldIndex` (`int`),
  `fieldDataType` (`token`), `vectorDataRoleHint` (`token`).
- **`OpenVDBAsset`** — concrete, IS-A `FieldAsset`. Adds `fieldClass` (token)
  and uses `grid`-style fields. The common DCC case.
- **`Field3DAsset`** — concrete, IS-A `FieldAsset`. Adds `fieldPurpose` (token)
  (Field3D files; less common).

## 2. Scope — read + author

- **Read:** `read_volume` (enumerate `field:<name>` rels → name+target),
  `read_openvdb_asset` / `read_field3d_asset` (the FieldAsset attrs), type-gated.
- **Author:** `define_volume` (+ `add_field(name, target)` authoring a
  `field:<name>` rel), `define_openvdb_asset` / `define_field3d_asset` with the
  FieldAsset setters. Reuse the geom Gprim/Xformable stack for Volume and the
  Xformable stack for the field prims.
- The `field:<name>` relationship namespace is the one non-obvious bit — model
  it like the coordSys / collection multi-name relationship patterns.

## 3. Conformance

Hand-authored `fixtures/usdVol_scene.usda`: a `Volume` with two `field:density`
/ `field:temperature` rels pointing at `OpenVDBAsset` prims. In-memory
author→read roundtrips covering the field-rel enumeration and the OpenVDB attrs.

## 4. Commit plan

1. `feat(vol): UsdVol tokens + types`
2. `feat(vol): FieldAsset / OpenVDBAsset / Field3DAsset read + author`
3. `feat(vol): Volume + field relationships read + author`
4. `docs: mark UsdVol supported in ROADMAP`

Builds + tests `--all-features`; fmt/clippy/doc clean.

## 5. Decision / dependency note

Greenfield, so it can land in whichever style is current. If **#93** has
settled, build directly as typed `Prim` views (`Volume`, `OpenVDBAsset` with
`Gprim`/`Xformable` supertraits) — UsdVol is a good early test of the field-
relationship pattern under the new model. Lowest-priority of the five (least
used downstream); fine to do last or skip if time-constrained.
