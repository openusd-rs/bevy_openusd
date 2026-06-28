# bevy_openusd

> This project was supported by **[Wageningen University and Research (WUR)](https://www.wur.nl/)**.
> A lot of the code was carved out of an internal repo to be open-sourced. Special thanks
> to the team for letting it ship.

A live [OpenUSD](https://openusd.org) editor on [Bevy](https://bevy.org) 0.19.

The composed USD stage is the source of truth: it's held live (not baked), projected
into Bevy entities, and kept in sync off openusd's change notifications. Edits flow
both ways — author back to the stage, undo/redo, and save.

## Crates

- **`usd_bevy`** — the editor library:
  - `live` — `LiveStage` (stage + change sink), the `SdfPath ↔ Entity` bimap, the
    project/reproject loop, and `LiveStagePlugin`.
  - `authoring` — namespace ops (define/remove/rename/reparent/move), attribute
    authoring, undo/redo, and persistence (export/save).
  - `read` — decode geometry/transforms/visibility off the composed stage via
    [`mxpv/openusd`](https://github.com/mxpv/openusd).
  - `mesh` — `UsdGeom.Mesh` → `bevy::mesh::Mesh`.
- **`usdview`** (`src/main.rs`) — a minimal viewport host: opens a USD file, projects
  it with `LiveStagePlugin`, renders with a camera + light + grid.

## Usage

```sh
cargo run -- path/to/stage.usda
```

Requires Rust 1.95+ (Bevy 0.19). See `RETHINK.md` for the architecture.
