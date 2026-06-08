# PLAN_02_MEDIA — UsdMedia for `mxpv/openusd`

Design doc for contributing **UsdMedia** (read + author) to the upstream
crate. ROADMAP lists UsdMedia as `:construction:`. The only concrete schema
in the family is `SpatialAudio`.

## 0. Status

Not upstream. The downstream `usd_schema/src/media.rs` has a reader
(`read_spatial_audio`) against an older openusd rev — port it up, add
authoring, and follow the merged schema conventions (feature gate, typed
view / handle, `sdf::FieldKey` constants, manual spec-correct `Default`s).

## 1. The schema surface (`pxr/usd/usdMedia`)

**`SpatialAudio`** — a concrete typed prim, IS-A `UsdGeomXformable`
(Imageable + Xformable: it has a transform, visibility, purpose). Attributes:

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `filePath` | `uniform asset` | `@@` | the audio file |
| `auralMode` | `uniform token` | `spatial` | `spatial` / `nonSpatial` |
| `playbackMode` | `uniform token` | `onceImageVisible` | `onceImageVisible` / `onceImageInactive` / `onStart` / `none` |
| `startTime` | `uniform timecode` | `0` | media start, in stage time |
| `endTime` | `uniform timecode` | `0` | media end, in stage time |
| `mediaOffset` | `uniform double` | `0.0` | offset into the media file |
| `gain` | `double` | `1.0` | volume multiplier |

(`timecode` values track the stage's `timeCodesPerSecond`; read as the raw
double the parser surfaces.)

## 2. Scope — read + author

- **Read:** `read_spatial_audio(stage, prim) -> Option<ReadSpatialAudio>`,
  type-gated on `SpatialAudio`, spec-default fallbacks; decoded enums for
  `auralMode` / `playbackMode`.
- **Author:** `define_spatial_audio(...)` returning a chainable handle with
  the shared Xformable/Imageable setters (reuse the geom transform stack) +
  the media-specific setters. Mirrors the lux `define_*` pattern.
- It IS-A Xformable, so it should compose with the geom transform readers/
  setters rather than re-implement them.

## 3. Conformance

No vendored asset — hand-authored `fixtures/usdMedia_scene.usda` + in-memory
author→read roundtrips. Cover both `auralMode`/`playbackMode` token values
and the timecode/gain defaults.

## 4. Commit plan

1. `feat(media): SpatialAudio tokens + types`
2. `feat(media): SpatialAudio reader`
3. `feat(media): SpatialAudio authoring + roundtrip tests`
4. `docs: mark UsdMedia supported in ROADMAP`

(Or fold 1–3 into one PR with per-concern commits — it's a single small
schema.) Each commit builds + tests `--all-features`; fmt/clippy/doc clean.

## 5. Decision / dependency note

If the **#93 typed-view reshape** lands first, write this directly in the new
`Prim`-view style (`SpatialAudio(Prim)` with `Xformable`/`Imageable`
supertraits). If not, write it in the current `read_*`/`define_*` style and
migrate with the rest. Either way the attribute set above is unchanged.
