//! `usd_bevy` — OpenUSD → Bevy as a **live editor**.
//!
//! The composed USD stage is the source of truth: [`live`] projects it into
//! Bevy entities and keeps them in sync off openusd's `StageSink`, [`authoring`]
//! applies edits + persistence, and [`read`] decodes the stage. Entities carry
//! [`UsdPrimRef`] linking them back to their prim path.

// Let the `usd!` macro's generated `::usd_bevy::…` paths resolve inside this
// crate's own tests/examples too.
extern crate self as usd_bevy;

pub mod asset;
pub mod authoring;
pub mod editor;
pub mod instance;
pub mod live;
pub mod mesh;
mod persistence;
pub mod subdivision;
pub mod prim_ref;
pub mod read;
pub mod route;
pub mod snippet;
pub mod source;
pub mod sync;

pub use asset::{UsdAssetLoader, UsdAssetPlugin, UsdScene, UsdSceneRoot, UsdSceneState};
pub use source::UsdSource;
pub use prim_ref::UsdPrimRef;
pub use route::{DisplayPurposes, PrimRoute, RouteCtx, SchemaRegistry};
/// The inline-USD macro (see [`snippet::UsdSnippet`]).
pub use usd_macro::usd;

use bevy::app::{App, Plugin};

/// Registers the [`UsdPrimRef`] reflect type and installs the built-in
/// [`SchemaRegistry`] (transform / visibility / mesh / reflect routes). Pair
/// with [`live::LiveStagePlugin`] (which runs the project + reproject loop).
///
/// Apps that want their own components authorable from USD add routes after
/// this plugin: `app.world_mut().resource_mut::<SchemaRegistry>().register(..)`
/// — or, for the reflect route, just `app.register_type::<MyComponent>()`.
#[derive(Default)]
pub struct UsdPlugin;

impl Plugin for UsdPlugin {
    fn build(&self, app: &mut App) {
        route::xform::configure(app);
        app.register_type::<UsdPrimRef>();
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
        // Intern projected meshes so identical prims share one GPU asset (6d).
        app.init_resource::<route::cache::ProjectionCache>();
        app.init_resource::<route::cache::MaterialCache>();
        // Which USD `purpose` classes are displayed (Phase A). Default: show
        // proxy, hide render + guide.
        app.init_resource::<DisplayPurposes>();
    }
}
