//! `UsdPrimRef` — the bridge between a Bevy entity and the USD prim path it
//! was projected from. The live editor's `SdfPath ↔ Entity` bimap keys off
//! it, and ECS-side code uses it to ask "what prim is this?".

use bevy::ecs::component::Component;
use bevy::ecs::reflect::ReflectComponent;
use bevy::reflect::{Reflect, std_traits::ReflectDefault};

/// The composed absolute prim path an entity was projected from
/// (e.g. `"/World/ChildA"`).
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq, Hash)]
#[reflect(Component, Default)]
pub struct UsdPrimRef {
    pub path: String,
}

impl UsdPrimRef {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}
