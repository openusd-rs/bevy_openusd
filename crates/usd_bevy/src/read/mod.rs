//! USD readers — decode composed attribute/relationship values straight off
//! openusd's `Prim`/`Attribute`/`Relationship` handles. The live projection
//! (`crate::live`) reads geometry/transforms/visibility through these.

pub mod curves;
pub mod displacement;
pub mod geom;
pub mod ptex;
pub mod shade;
pub mod skel;
pub mod subdivision;
pub mod util;
pub mod variants;
pub mod xform;
