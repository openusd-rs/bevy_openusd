//! Shared ownership of GPU deformation culling overrides.

use bevy::{camera::visibility::NoFrustumCulling, prelude::*};

pub(crate) const SKIN: u8 = 1;
pub(crate) const MORPH: u8 = 2;

#[derive(Component)]
struct CullingOverride { claims: u8, previous: bool }

pub(crate) fn acquire(world: &mut World, entity: Entity, claim: u8) {
    if let Some(mut state) = world.get_mut::<CullingOverride>(entity) {
        state.claims |= claim;
    } else {
        let previous = world.get::<NoFrustumCulling>(entity).is_some();
        world.entity_mut(entity).insert(CullingOverride { claims: claim, previous });
    }
    world.entity_mut(entity).insert(NoFrustumCulling);
}

pub(crate) fn release(world: &mut World, entity: Entity, claim: u8) {
    let Some(mut state) = world.get_mut::<CullingOverride>(entity) else { return };
    state.claims &= !claim;
    if state.claims != 0 { return; }
    let previous = state.previous;
    world.entity_mut(entity).remove::<CullingOverride>();
    if !previous { world.entity_mut(entity).remove::<NoFrustumCulling>(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_claims_restore_only_after_last_release() {
        for previous in [false, true] {
            for order in [[SKIN, MORPH], [MORPH, SKIN]] {
                let mut world = World::new();
                let entity = world.spawn_empty().id();
                if previous { world.entity_mut(entity).insert(NoFrustumCulling); }
                acquire(&mut world, entity, order[0]);
                acquire(&mut world, entity, order[1]);
                acquire(&mut world, entity, order[0]);
                release(&mut world, entity, order[0]);
                assert!(world.get::<NoFrustumCulling>(entity).is_some());
                release(&mut world, entity, order[1]);
                assert_eq!(world.get::<NoFrustumCulling>(entity).is_some(), previous);
                assert!(world.get::<CullingOverride>(entity).is_none());
            }
        }
    }
}
