# mara — missing defaults (notes from building a consumer)

Context: I built a small app (`usdview`) as a `mara::window::WindowApp` that embeds
a `MaraBevyViewport` and wants a left ribbon + a couple of panes. To get anything
that looks/works like mara, I had to copy ~150 lines of boilerplate out of
`example/src/app.rs`. The fact that the **demo itself hand-rolls all of this**
is the tell: it should live in the library, or be a default. Below is everything
a bare consumer has to do manually that arguably mara should provide.

## 0. A bare `WindowApp` is NOT themed — biggest surprise

`mara::window::run::<App>()` + `WindowApp::update(host)` gives you a window, but
**no mara theme is applied**. Unless you call, every frame:

```rust
mara_core::style::set_theme(mara_core::style::theme_pro(Mode::Dark));
host.apply_theme(accent, glass);
```

…your panes/ribbons render with raw-egui defaults (i.e. it looks like *not mara*).
I skipped this and the result looked like dogshit — which is on me, but the point
is: **the runner should apply a sensible default theme** (pro/dark + a default
accent) so a bare app looks like mara out of the box. Make `set_theme`/
`apply_theme` an opt-*out*, not an opt-in.

## 1. There are no default ribbons / chrome

I expected a fresh `WindowApp` to come with *some* default ribbon scaffold (an app
bar, an empty left rail you can drop buttons into). It comes with nothing. Every
consumer rebuilds the entire ribbon system. Suggested: a default `ShellBar` + at
least one empty, ready-to-populate rail, toggleable off.

## 2. No high-level "declare ribbon + buttons" API — you build `ResolvedSlotRibbon` by hand

The only public entry is `host.draw_slot_ribbons_featureful(accent, &[ResolvedSlotRibbon], open, placement, drag)`,
which forces the consumer to construct `ResolvedSlotRibbon` manually:

```rust
ResolvedSlotRibbon {
    id: MaraId::new((ribbon.id, cluster)),
    chrome_id: Some(ribbon.id),
    scope: ribbon_scope(ribbon.id),   // <-- see §3
    edge, role, mode, cluster,
    accepts: ribbon.accepts,
    items,                            // Vec<RibbonSlotItem::featureful(...)>
}
```

…inside a manual `for ribbon in RIBBONS { for cluster in [Start, Middle, End] { … } }`
loop. The demo wraps this in its own `RibbonSpec` / `RibbonButtonSpec` structs +
`draw_unified_ribbons` helper — **that helper should be mara's, not the demo's.**

Suggested API (something like):
```rust
let mut bar = Ribbon::left();                 // edge + sane role/mode defaults
bar.pane_button("outliner", "list", "Outliner");   // id==pane id, auto-published
bar.pane_button("properties", "options", "Properties");
bar.action_button("save", "save", "Save stage");
let clicks = host.ribbons([top_bar, bar], open, placement, drag);
```

## 3. `RibbonScope` is consumer boilerplate

Had to write:
```rust
fn ribbon_scope(id) -> RibbonScope {
    if id == TOP { RibbonScope::Permanent } else { RibbonScope::View(ViewId::new("…")) }
}
```
copied from the demo's `demo_ribbon_scope`. The "permanent top bar vs per-view
rail" distinction should be implied by which builder you use (`Ribbon::permanent()`
vs `Ribbon::view()`), not computed by hand per id.

## 4. Pane ↔ ribbon wiring is an implicit convention with a silent-failure footgun

To make a ribbon button toggle a pane, three undocumented things must line up:
1. the ribbon button **id must equal the pane id**,
2. the button **role must be `None`** (an `Icon` role makes it an action, not a pane),
3. you must call **`host.publish_ribbon_pane_ids([...])` every frame** or the pane
   is silently rejected (no error, just nothing renders).

Then you hand-roll the open/dispatch loop:
```rust
for (ribbon, pane, anchor, title) in PANES {
    if open.is_open(ribbon, pane) {
        host.show_pane(Pane::new(pane, title, anchor, accent).resize(SPAN), |body| …);
    }
}
```
All of this should collapse into one registration: a pane button *is* the pane, the
id is shared automatically, publishing is automatic, and `open` state +
`show_pane` are driven by the framework. Forgetting `publish_ribbon_pane_ids` with
no diagnostic is the worst part — at minimum that should `warn!`, ideally not exist.

## 5. `ui_system` plumbing every app must repeat

Beyond theme (§0), every frame you must: `host.publish_full_shelf_layout()`,
build a `view_ctx`, and **paint ribbons AFTER panes** (a load-bearing ordering
requirement called out in a demo comment — get it wrong and ribbons render under
the panes). This ordering should be the framework's job, not a comment in an
example.

## 6. `MaraBevyViewport` auto-adds plugins, but silently — duplicate = panic

The viewport's embedded app already adds `GroundGridPlugin` (and the core/render
plugins). Adding `GroundGridPlugin` yourself — which is natural, since you *do*
insert the `GroundGrid` resource — panics at startup:

```
Error adding plugin GroundGridPlugin: plugin was already added in application
```

The set of plugins the viewport pre-installs isn't documented, so a consumer
either duplicates (panic) or doesn't know what's available. Suggested: document
the pre-installed plugin set, and/or make `configure_app` run *after* them so
re-adding is a no-op, and/or expose the grid purely as the resource (no consumer-
facing plugin).

---

## What already works well (keep it)

- `MaraBevyViewport::with_render_state_and_content(fn(&mut App))` — clean embed.
- `ChaseCamera` + `apply_rig` + `apply_viewport_camera_input_system` + `GroundGrid`
  — good viewport defaults once you know they exist.
- `PaneBody::add_normal(id, title, icon, Vec<Pod>)` + `Pod::with_readout/with_button/
  with_tree` — the content surface is genuinely nice and high-level.
- `host.show_pane` / `view_ctx` / the sealed `MaraUi` surface — good.

## TL;DR

The content API (pods/panes/trees) is high-level and good. The **app-shell layer**
(theme, ribbons, pane wiring, ordering) has no defaults — every consumer copies the
demo's ~150 lines. A `WindowApp` should boot themed, with a default shell bar +
empty rails, a `Ribbon` builder, pane buttons that auto-wire+auto-publish, and
framework-owned ribbon/pane ordering.
