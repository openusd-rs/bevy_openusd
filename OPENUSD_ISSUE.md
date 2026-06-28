# openusd-rs — `.usd` layer extension rejected inside USDZ

## Symptom

`Stage::open("assets/external/Kitchen_set.usdz")` fails:

```
failed to read first layer from USDZ archive:
Unsupported file format for 'Kitchen_set.usd'. Expected .usda or .usdc extension
```

So the stage never loads and the viewer shows an empty scene.

## What works vs what doesn't (measured headlessly)

| file | result |
|---|---|
| `two_xforms.usda` (`.usda`) | ✅ opens (3 Xforms, 0 meshes — transform-only) |
| `agilex/scout_v2/scout_v2_base.usd` (plain `.usd`) | ✅ opens — 53 prims, **5 meshes with points** |
| `Kitchen_set.usdz` (`.usdz` with a `.usd` first layer) | ❌ rejected |

So **plain `.usd` and `.usda`/`.usdc` are fine** — the failure is specifically a
USDZ whose first/inner layer has the **`.usd`** extension.

## Cause

openusd-rs maps a layer's file format by **extension**, and the USDZ reader only
accepts `.usda` (text) or `.usdc` (crate binary). `.usd` is the
"format-agnostic" extension in USD — it should be resolved by **sniffing the
content** (a `.usdc` file starts with the magic bytes `PXR-USDC`; otherwise it is
`.usda` text). USD's own `Kitchen_set.usdz` ships its root layer as
`Kitchen_set.usd`, so this hits the most common reference asset.

## Suggested fix in openusd-rs

When opening a layer whose extension is `.usd` (or unknown) inside a USDZ archive
*and* for plain files, **detect the format from the first bytes** instead of
rejecting it:

```rust
fn layer_format(name: &str, head: &[u8]) -> Format {
    match ext(name) {
        "usda" => Format::Usda,
        "usdc" => Format::Usdc,
        // `.usd` (and anything else) → sniff content
        _ if head.starts_with(b"PXR-USDC") => Format::Usdc,
        _ => Format::Usda,
    }
}
```

This fixes Kitchen_set.usdz and any USDZ/`.usd` that uses the agnostic extension,
with no change to `.usda`/`.usdc` handling.

## Status

Fixed on branch `fix/usdz-usd-content-sniff` (off `06d619d`). Kitchen_set.usdz
now **opens** (452 prims compose).

---

# Issue 2 — references inside a USDZ are not resolved

After issue 1, `Kitchen_set.usdz` opens but renders **nothing**: 425 of its 452
prims have **no type** (`<none>`) and there are **0 meshes**. Kitchen_set is
built almost entirely from references to sub-assets *packaged inside the usdz*
(`@./assets/Cup/Cup.usd@`, …), and those references never resolve.

## Cause

`UsdzFileFormat::read` (`src/usdz/mod.rs`) reads only the archive's **first
layer** and never exposes the archive's other entries to the asset resolver:

```rust
fn read(&self, resolver, resolved) -> Result<LayerData> {
    let bytes = resolver.open_asset(resolved)?.read_all()?;
    let mut archive = Archive::from_reader(Cursor::new(bytes))?;
    archive.read_first_layer()          // ← other entries are dropped
}
```

So when the root layer references `./assets/Cup/Cup.usd`, the resolver looks for
that path on the **real filesystem** (next to the `.usdz`), doesn't find it, and
the reference arc fails → the prim stays `<none>` with no geometry.

## Suggested fix in openusd-rs

Make opening a `.usdz` install a **package-aware `ar::Resolver`** (or resolver
context) so that asset paths referenced from within the package are served from
the archive's entries (by name) instead of the host filesystem. Mirrors USD's
`Usd_UsdzResolver` / package-relative path handling. This is bigger than issue 1
(touches the resolver seam), but it's what real packaged assets like Kitchen_set
need to actually render.

## Workaround in the meantime

Use **self-contained** assets (geometry inline, no external references): the
robots under `assets/external/{agilex,clearpath,…}` — e.g.
`agilex/scout_v2/scout_v2_base.usd` (53 prims, **5 meshes**) — load and render.
