# mara — issues from building a consumer

> The previous batch (no default theme, no ribbon builder, manual
> `publish_ribbon_pane_ids`, etc.) was **resolved in mara 0.3.2** via
> `RibbonRail` + `host.show_ribbon_rail`. Thanks. One new footgun below.

## `RibbonRail::pane()` decouples the button's cluster from the pane's anchor

`RibbonRail::pane(id, icon, title, anchor, body)` is a shorthand that hardcodes
the **button's ribbon cluster to `Start`** regardless of the `anchor` (RailZone)
you pass for the pane (`mara/src/host.rs`):

```rust
pub fn pane(self, id, icon, title, anchor, body) -> Self {
    self.pane_in(RibbonCluster::Start, id, icon, title, anchor, body)
    //           ^^^^^^^^^^^^^^^^^^^^^ button always lands in Start
}
```

So two panes declared with `.pane(...)`:

```rust
RibbonRail::view_left(RIBBON_LEFT, "view")
    .pane(OUTLINER,   "list",    "Outliner",   PaneAnchor::LeftRail(RailZone::Start))   // button Start, pane Start  ✅
    .pane(PROPERTIES, "options", "Properties", PaneAnchor::LeftRail(RailZone::Middle))  // button Start, pane Middle ✗
```

…put **both buttons in the Start cluster (stacked at the top of the rail)**, but
the Properties **pane opens in the Middle**. Symptom: the Properties button sits
directly under the Outliner button at the top, yet clicking it opens its pane in
the middle of the sidebar — button and pane are visually disconnected.

### Cause

The button's ribbon **cluster** (where the icon renders: Start/Middle/End) and the
pane's **anchor** (where the panel opens: a `RailZone`) are two independent fields
on `RibbonPane`, and the `.pane()` shorthand silently fixes the cluster to `Start`.
Any anchor other than `LeftRail(Start)` via `.pane()` produces a mismatch.

### Workaround (what we did)

Use the lower-level `pane_in(cluster, …)` and pass a cluster that matches the
anchor's zone:

```rust
.pane_in(RibbonCluster::Middle, PROPERTIES, "options", "Properties",
         PaneAnchor::LeftRail(RailZone::Middle), body)   // button Middle, pane Middle ✅
```

### Suggested fix in mara

Make `.pane()` **derive the button cluster from the anchor's `RailZone`** instead
of hardcoding `Start`:

```rust
pub fn pane(self, id, icon, title, anchor, body) -> Self {
    let cluster = match anchor.rail_zone() {       // Start→Start, Middle→Middle, End→End
        RailZone::Start  => RibbonCluster::Start,
        RailZone::Middle => RibbonCluster::Middle,
        RailZone::End    => RibbonCluster::End,
    };
    self.pane_in(cluster, id, icon, title, anchor, body)
}
```

Then button and pane can never disagree, `pane_in` stays available for the rare
case where you deliberately want them apart, and the common path is correct by
construction. (Optionally, in `show_ribbon_rail`, `debug_assert!` that a pane's
cluster matches its anchor zone unless built via `pane_in`, to catch the rest.)
