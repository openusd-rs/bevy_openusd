# PLAN_RENDER — UsdRender for `mxpv/openusd`

Design doc for contributing **UsdRender** (render settings, products, vars,
passes, and the computed render spec) to the upstream `mxpv/openusd` crate.
Greenfield — nothing render-related exists today. Captures the schema surface,
the one hard algorithm (the computed render spec), the conformance approach, and
a detailed multi-commit plan.

---

## 0. Status — greenfield

`rg -i render src/` finds only incidental substrings; there is no
`src/schemas/render/`, no `render` feature, no `RenderSettings`/`Product`/`Var`/
`Pass` type anywhere. This is a clean new schema domain, built the same way as
the merged usdGeom / usdLux / usdSkel: a feature-gated `schemas/render/` module
(`tokens.rs`, `types.rs`, `read.rs`, `author/`), hand-authored fixtures, and an
in-memory roundtrip + reader integration test (`tests/render_reader.rs`).

**Dependency note:** UsdRender leans on two adjacent subsystems. `camera` (a
relationship to a `UsdGeomCamera`) and the aperture pull need **UsdGeom camera**
— already merged. `namespacedSettings` node-graph-driven values and `RenderPass`
collections need **UsdShade outputs** and **UsdCollectionAPI** respectively —
both gated below.

---

## 1. The schema surface

Six typed schemas; two are abstract/base.

| Schema | Family | Inherits | Role |
|---|---|---|---|
| `RenderSettingsBase` | abstract typed | `Typed` | shared camera + framing attrs |
| `RenderSettings` | concrete | `RenderSettingsBase` | top-level config; enumerates products |
| `RenderProduct` | concrete | `RenderSettingsBase` | one output artifact; overrides base attrs |
| `RenderVar` | concrete | `Typed` | one output channel (AOV) |
| `RenderPass` | concrete | `Typed` | a node in a multi-pass graph (+ 4 collections) |
| `RenderDenoisePass` | concrete (dev) | `Typed` | a denoise pass (thin, dev-era) |

**`RenderSettingsBase`** (inherited by Settings *and* Product — this is what makes
"product overrides settings" work): `camera` (rel), `resolution`
(`uniform int2`, `(2048,1080)`), `pixelAspectRatio` (`uniform float`, `1.0`),
`aspectRatioConformPolicy` (`uniform token`, `expandAperture`), `dataWindowNDC`
(`uniform float4`, `(0,0,1,1)`), `instantaneousShutter` (`uniform bool`, `false`,
**deprecated**), `disableMotionBlur` (`uniform bool`, `false`),
`disableDepthOfField` (`uniform bool`, `false`).

**`RenderSettings`** adds: `products` (rel), `includedPurposes`
(`uniform token[]`, `["default","render"]`), `materialBindingPurposes`
(`uniform token[]`, `["full",""]`), `renderingColorSpace` (`uniform token`, **no
fallback**). Plus stage metadata `renderSettingsPrimPath` (root/session layer
only) + `get_stage_render_settings`.

**`RenderProduct`** adds: `productType` (`uniform token`, `raster`;
`raster`/`deepRaster`), `productName` (`token`, **not uniform**, `""`),
`orderedVars` (rel).

**`RenderVar`**: `dataType` (`uniform token`, `color3f`), `sourceName`
(`uniform string`, `""`), `sourceType` (`uniform token`, `raw`;
`raw`/`primvar`/`lpe`/`intrinsic`).

**`RenderPass`**: `passType` (`uniform token`), `command` (`uniform string[]`,
`{var}` brace substitution), `fileName` (`uniform asset`), `renderSource` (rel),
`inputPasses` (rel, 1:1 by frame number), and four multiple-apply
`CollectionAPI` instances — `renderVisibility`, `cameraVisibility`, `prune`,
`matte` (`prune`/`matte` are dev-era) — each with a `collection:<n>:includeRoot`
(`uniform bool`, `1`).

**`RenderDenoisePass`** (dev): thin — `denoiseEnable` (bool), `denoisePass` (rel),
input wiring via `inputPasses`. Pin to a specific dev revision; verify its attr
set against that revision.

---

## 2. The crux — `UsdRenderComputeSpec`

Reading the schemas is mechanical. The one hard, normative piece is flattening a
`RenderSettings` prim (+ products + vars + camera) into a self-contained,
fallback-resolved `RenderSpec` value. Everything interesting is here.

### 2a. Product-overrides-settings inheritance

Because `RenderProduct` **is-a** `RenderSettingsBase`, the base attrs are read
twice:
1. From the Settings prim **with fallbacks** (`get_default = true`) → resolved base.
2. Per product: copy the resolved base, then re-read base attrs from the product
   **authored-only** (`get_default = false`). A product value overrides **only if
   explicitly authored** (`HasAuthoredValue`); product fallbacks do *not* override.

The precise gate (from C++ `_Get`): `get_default || attr.has_authored_value()`.
Replicate exactly — this is the single most common place to get inheritance wrong.

### 2b. Aspect-ratio conform math

Pull aperture from the camera; reconcile camera aperture aspect vs image aspect
per `aspectRatioConformPolicy`. From C++ `_ApplyAspectRatioPolicy`:

```
res_aspect      = res.x / res.y
image_aspect    = pixelAspectRatio * res_aspect
aperture_aspect = size.x / size.y          // size = camera aperture
```

- `adjustPixelAspectRatio` — keep aperture; set `pixelAspectRatio = aperture_aspect / res_aspect`.
- `adjustApertureWidth`  — `size.x = size.y * image_aspect`.
- `adjustApertureHeight` — `size.y = size.x / image_aspect`.
- `expandAperture` (default) — grow: adjust **Height** if `aperture_aspect > image_aspect` else **Width**.
- `cropAperture` — shrink: adjust **Width** if `aperture_aspect > image_aspect` else **Height** (mirror of expand).

Then the chosen dimension is set so `size.x / size.y == image_aspect`. **Expand
and crop differ only in which branch they pick** under the same test — invert it
and you crop where you should expand. The result mutates the computed spec's
`apertureSize` / `pixelAspectRatio`; it **never** writes back to the camera.

### 2c. RenderVar de-duplication

Walk `products` (`GetForwardedTargets`), and per product walk `orderedVars`.
Maintain a global `render_vars` list; each product holds `render_var_indices`
into it. For each var path, reuse an existing index or parse+append a new
`RenderVar`. Output is a flat global var list + per-product index lists.

### 2d. namespacedSettings gathering

How render-delegate settings (`ri:`, `karma:`, `arnold:` …) ride along. Per prim
(settings, each product, each var — **per level, not merged**):
- For each authored attr (and rel), derive a basename; if it is a
  `UsdShadeOutput` (`outputs:`-prefixed connectable), strip via the shade base
  name, else use the raw name.
- Compute its namespace; **skip if empty** (unnamespaced attrs never enter).
- If a requested-namespace set is given, skip attrs outside it (empty request =
  gather all).
- For connectable outputs, resolve value producers (UsdShade
  `get_value_producing_attributes`) and store the connected **paths**, not a
  literal value — lets settings be node-graph-driven.

**Gated on UsdShade** (the connectable-output path). Without shade, gather only
plain namespaced attrs and leave a `// TODO(shade)` seam for the output case.

The computed `RenderSpec` (value types): top-level `products`, `render_vars`,
`included_purposes`, `material_binding_purposes`, `namespaced_settings`. Per
`Product`: path, type, name, camera path, `disable_motion_blur`,
`disable_depth_of_field`, `resolution`, `pixel_aspect_ratio`,
`aspect_ratio_conform_policy`, `aperture_size` (computed), `data_window_ndc`
(as a range), `render_var_indices`, `namespaced_settings`. Per `RenderVar`:
path, `data_type`, `source_name`, `source_type`, `namespaced_settings`.

---

## 3. Conformance approach

No vendored UsdRender assets (`def RenderSettings` appears only incidentally in
two composition test files). So, as with every merged schema:

- hand-authored `fixtures/usdRender_scene.usda` (a `</Render>` scope: one
  `RenderSettings` → two `RenderProduct`s → shared + per-product `RenderVar`s, a
  camera, and a couple of `ri:` namespaced settings);
- in-memory author → read-back roundtrips;
- **focused unit-test matrices** for the crux (§2): the five conform policies
  against known aperture/resolution inputs; product-authored-override vs
  inherit; var de-dup across two products sharing a var; namespacedSettings
  empty-namespace skip + requested-namespace filter.

---

## 4. Commit plan

Each commit builds + tests under `--all-features`, fmt + clippy (`-D warnings`,
1.89) clean, `cargo doc --no-deps` clean. Module under `schemas/render/` with
authoring in `author/` per the lux convention; feature `render`; integration
test `tests/render_reader.rs` (`required-features=["render"]`).

### Commit 1 — `feat(render): tokens + module scaffold`
- `UsdRenderTokens` (attr names, allowedTokens, schema-type tokens,
  `renderSettingsPrimPath` metadata token); `types.rs` enums
  (`AspectRatioConformPolicy`, `ProductType`, `SourceType`) with
  `as_token`/`from_token`; feature wiring in `Cargo.toml` + `schemas/mod.rs`.
- Tests: token round-trip, allowedTokens coverage.

### Commit 2 — `feat(render): RenderSettingsBase attrs (read + author)`
- The 8 base attrs with exact fallbacks + `camera` rel; get/create accessors;
  a shared setter trait (lux `*ApiSetters` idiom) reused by Settings + Product.
- Tests: author→read each attr, fallback when unauthored.

### Commit 3 — `feat(render): RenderSettings schema`
- `products` rel, `includedPurposes`, `materialBindingPurposes`,
  `renderingColorSpace` (no fallback); inherit base. `renderSettingsPrimPath`
  stage metadata get/set + `get_stage_render_settings`.
- Tests: stage-metadata lookup, list-attr defaults.

### Commit 4 — `feat(render): RenderProduct + RenderVar schemas`
- Product: `productType`, `productName` (non-uniform), `orderedVars`; inherit
  base. Var: `dataType`, `sourceName`, `sourceType`.
- Tests: author→read; `productName` non-uniform preserved.

### Commit 5 — `feat(render): RenderSpec value types`
- The `RenderSpec` / `Product` / `RenderVar` plain data structs (§2 fields).
- No compute yet; pure types so later commits are reviewable in isolation.

### Commit 6 — `feat(render): settings-base read + product override`  ⚠ subtle
- `_Get` authored-only gate + `read_settings_base`; the copy-resolved-base →
  re-read-product-authored flow (§2a).
- Tests: product overrides only authored attrs; product fallback does **not**
  override; full inherit when product authors nothing.

### Commit 7 — `feat(render): aspect-ratio conform math`
- `apply_aspect_ratio_policy` — all five branches + camera-aperture pull (§2b).
- Tests: each policy against known `(resolution, pixelAspectRatio, aperture)`
  inputs; expand vs crop branch selection.

### Commit 8 — `feat(render): render-var dedup + product assembly`
- `orderedVars`/`products` forwarded-target walk; global var list +
  `render_var_indices` de-dup (§2c).
- Tests: two products sharing a var → one global entry, two index lists.

### Commit 9 — `feat(render): namespacedSettings gathering`
- Per-level namespaced-attr gather: empty-namespace skip, requested-namespace
  filter (§2d). **UsdShade output path gated** — plain namespaced attrs now;
  `// TODO(shade)` seam for connectable-output-driven settings.
- Tests: `ri:` attrs gathered, unnamespaced skipped, requested-namespace filter.

### Commit 10 — `feat(render): compute_render_spec entry point`
- Wire 6–9 into `compute_render_spec(settings, namespaces) -> RenderSpec`.
- Tests: end-to-end fixture → spec (two products, shared var, conform applied,
  `ri:` settings present).

### Commit 11 — `feat(render): RenderPass schema`
- `passType`/`command`/`fileName`/`renderSource`/`inputPasses` + the four
  multiple-apply collections with `includeRoot`. **Gated on multiple-apply
  CollectionAPI infra** — if absent, land pass attrs + rels now and defer the
  collection instances with a documented seam.
- Tests: author→read attrs/rels; collection apply (if infra present).

### Commit 12 — `feat(render): RenderDenoisePass (dev schema)`
- `denoiseEnable`/`denoisePass`/input wiring, pinned to a named dev revision.
- Tests: author→read; note the dev-pin in the doc comment.

### Commit 13 — `docs(render): ROADMAP + render-spec notes`
- Flip the ROADMAP UsdRender row to supported; document the computed-spec model
  and the gated edges (CollectionAPI, shade-driven namespacedSettings).

---

## 5. Decisions baked in

- **Schemas first, compute second**: mechanical readers/authoring (commits 1–5)
  before the one hard algorithm (the computed spec, commits 6–10).
- **Computed spec is the centerpiece** — product-override (§2a), conform math
  (§2b), var dedup (§2c), namespacedSettings (§2d) each get an isolated commit
  with a focused matrix; replicate the C++ `_Get` / `_ApplyAspectRatioPolicy`
  logic exactly.
- **Gate, don't fake**: `RenderPass` collections gate on multiple-apply
  CollectionAPI; shade-driven `namespacedSettings` gate on UsdShade. Each gap is
  a documented seam, never a silently-wrong result.
- **`disableMotionBlur` is canonical**; `instantaneousShutter` stays readable for
  back-compat. `renderingColorSpace` has no fallback — don't invent one.
  `productName` stays non-uniform.
- **RenderDenoisePass pinned to a dev revision** (not in core `release`).
- **Conformance**: hand-authored fixtures + in-memory roundtrips + crux unit
  matrices (no vendored oracle), matching every merged schema PR.
