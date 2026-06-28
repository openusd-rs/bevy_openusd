//! USD readers — decode composed attribute/relationship values straight off
//! openusd's `Prim`/`Attribute`/`Relationship` handles. The live projection
//! (`crate::live`) reads geometry/transforms/visibility through these.

pub mod geom;
pub mod shade;
pub mod util;
pub mod xform;
