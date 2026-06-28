//! `usd_bevy` — OpenUSD → Bevy as a **live editor**.
//!
//! The composed USD stage is the source of truth: [`live`] projects it into
//! Bevy entities and keeps them in sync off openusd's `StageSink`, [`authoring`]
//! applies edits + persistence, and [`read`] decodes the stage. Entities carry
//! [`UsdPrimRef`] linking them back to their prim path.

pub mod authoring;
pub mod live;
pub mod mesh;
pub mod prim_ref;
pub mod read;

pub use prim_ref::UsdPrimRef;

use bevy::app::{App, Plugin};

/// Registers the [`UsdPrimRef`] reflect type. Pair with
/// [`live::LiveStagePlugin`] (which runs the project + reproject loop).
#[derive(Default)]
pub struct UsdPlugin;

impl Plugin for UsdPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<UsdPrimRef>();
    }
}
