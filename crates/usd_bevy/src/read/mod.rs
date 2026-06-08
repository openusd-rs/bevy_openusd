//! openusd-backed readers: typed decode of the composed stage into the
//! structs the Bevy projection consumes. Replaces the former `usd_schema`
//! reader crate; everything here reads through openusd only.

pub mod anim;
pub mod camera;
pub mod clips;
pub mod geom;
pub mod lux;
pub mod media;
pub mod proc;
pub mod render;
pub mod shade;
pub mod skel;
pub mod skel_anim_text;
pub mod ui;
pub mod util;
pub mod xform;
