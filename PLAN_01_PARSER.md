# PLAN_01_PARSER — tolerate arbitrary prim metadata in the USDA parser

Design doc for making `mxpv/openusd`'s USDA text parser accept unknown
prim-metadata fields instead of erroring, so real-world (Omniverse, DCC)
`.usda` loads without preprocessing.

## 0. The problem

Pixar's Sdf grammar accepts arbitrary identifier-keyed fields in a prim's
metadata block — the `( … )` after the prim header — and stores the
unrecognized ones rather than rejecting them. `mxpv/openusd`'s parser
currently errors on anything it doesn't know, e.g. Omniverse authors:

```usda
def Xform "Root" (
    hide_in_stage_window = false
    no_delete = true
)
{ }
```

and the parser bails (`Unsupported … metadata …`). Downstream
(`usd_schema/src/3rd_party/strip_metadata.rs` + `resolver.rs`) works around
this by stripping those bytes from every layer before parsing — a hack that
exists only because the parser is stricter than the spec.

**Goal:** the parser tolerates unknown prim-metadata fields, deleting the
need for the strip workaround entirely.

## 1. Scope

- **In:** prim-metadata block (`( … )` after `def/over/class "Name"`). Accept
  an unknown `identifier = <value>` (and known value shapes — string, token,
  bool, number, dict, list) without failing the parse.
- Confirm the same tolerance for **layer metadata** and **attribute/property
  metadata** blocks, or scope to prim-only if those already differ. (The
  earlier clip work hit `Unsupported property metadata value token` — check
  whether property metadata needs the same treatment; if so, fold it in.)
- **Out:** inventing typed fields for these — they are unknown by definition.

## 2. The decision — ignore vs. stash

Two faithful options; pick per how the repo models unknown fields:

1. **Tolerate-and-ignore** — parse past the `key = value` and drop it. Simplest;
   loses the data on round-trip. Acceptable if the data is cosmetic
   (Omniverse hints) and round-trip fidelity for unknown fields isn't a goal.
2. **Tolerate-and-stash** — keep the field on the prim spec (the C++ behavior:
   unknown metadata is preserved). Round-trips. Needs a place to put it — a
   generic `(field → Value)` on the spec, which the layer model may already
   support since `sdf::Data` is a raw `(path, field) → Value` store.

Lean **tolerate-and-stash** if the data model makes it cheap (it likely does,
given `Data` is already untyped key/value); otherwise tolerate-and-ignore is
a fine first step. Confirm against the parser + `sdf::Data` shape.

## 3. Conformance approach

- Find the exact rejection site in `src/usda/parser.rs` (the prim-metadata
  loop) and the error it raises.
- Unit tests: a prim with `hide_in_stage_window = false` / `no_delete = true`
  parses; a prim with an unknown dict-valued field parses; (if stashed)
  the field reads back via the layer's field accessor and round-trips through
  the USDA writer.
- Regression: the existing known prim-metadata (specifier, references,
  variantSets, `clips`, etc.) still parse unchanged.

## 4. Commit plan

1. `fix(usda): tolerate unknown prim metadata fields` — parser change +
   unit tests for the Omniverse fields.
2. (if needed) `fix(usda): tolerate unknown property metadata` — same for the
   property-metadata block, if it rejects too.

Each builds + tests under `--all-features`; fmt + clippy + doc clean.

## 5. Downstream payoff

Once landed, delete `usd_schema/src/3rd_party/strip_metadata.rs` and
`resolver.rs` and load Omniverse `.usda` directly through `Stage::open`.

## 6. Decisions baked in

- Match Pixar: stricter-than-spec parsing is the bug; the fix is tolerance,
  not a longer allow-list.
- Prefer preserving unknown fields (stash) if the model allows; never silently
  change known-field behavior.
