# Mara Bevy host patch

Source: `hosts/bevy` from [Mara PR #1](https://github.com/bresilla/mara/pull/1),
revision `0ba23f4de3971a965d54fbe8cb01deb7b9c76741`, MIT licensed.
`Cargo.toml` expands the source workspace dependencies at that same revision.

`src/egui_view.rs` retains the render-start frame pacing fix from Mara commit
`7ddc9af30b626f245234db5282f1e67fdd70e51f`, which the existing Bevy 0.19 lockfile
already used but the 0.20 draft does not include. Frame rendering time counts
against the interval instead of being added to the next repaint delay.
The fix's regression test uses egui 0.37's repaint-delay API.

Remove this override when an upstream revision contains both the Bevy 0.20 port
and the pacing fix, after the embedded viewport and host-capture tests pass.
