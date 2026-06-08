# PLAN_04_UI — UsdUI for `mxpv/openusd`

Design doc for contributing **UsdUI** (read + author). ROADMAP lists it as
`:construction:`. UsdUI is cosmetic metadata authoring tools use to label
outliners and lay out shading-network editors — relevant for the GUI editor.

## 0. Status

Not upstream. Downstream `usd_schema/src/ui.rs` reads `displayName` /
`displayGroup` against an older rev — port up + author + the merged
conventions.

## 1. The schema surface (`pxr/usd/usdUI`)

All **applied API** schemas (single-apply) plus some token types:

- **`UsdUITokens`** — token values for the schemas below.
- **`SceneGraphPrimAPI`** (applied) — `ui:displayGroup` (token),
  `ui:displayName` (token): outliner label + grouping for a prim.
- **`NodeGraphNodeAPI`** (applied) — node-editor layout for a shading node:
  `ui:nodegraph:node:pos` (`float2`), `ui:nodegraph:node:size` (`float2`),
  `ui:nodegraph:node:displayColor` (`color3f`),
  `ui:nodegraph:node:icon` (`asset`),
  `ui:nodegraph:node:expansionState` (token: `open`/`closed`/`minimized`),
  `ui:nodegraph:node:stackingOrder` (`int`),
  `ui:nodegraph:node:docURI` (`asset`).
- **`Backdrop`** (concrete typed prim) — a node-editor backdrop box;
  `ui:description` (token).

Note these are namespaced under `ui:` and are **applied APIs** (except
`Backdrop`), so authoring adds the API to `apiSchemas` and writes the `ui:*`
attributes — mirrors the lux applied-API pattern.

## 2. Scope — read + author

- **Read:** `read_scene_graph_prim` (displayName/displayGroup),
  `read_nodegraph_node` (the layout fields), `read_backdrop`. Type/applied
  gated.
- **Author:** `apply_scene_graph_prim` / `apply_nodegraph_node` (add the API +
  set `ui:*`), `define_backdrop`. The downstream only reads displayName/Group;
  the node-editor layout fields are the higher-value add for an editor.

## 3. Conformance

Hand-authored `fixtures/usdUI_scene.usda` + in-memory roundtrips. Cover
displayName/Group, a couple of nodegraph layout fields, and a Backdrop.

## 4. Commit plan

1. `feat(ui): UsdUI tokens + types`
2. `feat(ui): SceneGraphPrimAPI + Backdrop read/author`
3. `feat(ui): NodeGraphNodeAPI read/author`
4. `docs: mark UsdUI supported in ROADMAP`

Builds + tests `--all-features`; fmt/clippy/doc clean.

## 5. Decision / dependency note

Mostly applied-API + namespaced attrs, so it slots into the existing
applied-schema authoring pattern cleanly. If **#93** lands first, the applied
APIs become mix-in traits over `Prim` (`SceneGraphPrim`, `NodeGraphNode`);
otherwise current style + migrate. This is the family the GUI editor will lean
on most (node layout), so worth doing well.
