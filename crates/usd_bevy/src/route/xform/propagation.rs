use bevy::prelude::*;
use crate::{UsdPrimRef, route::RouteCtx};
use super::{UsdTransformOverride, UsdTransformSystems, UsdTransformError};

pub(super) fn configure(app: &mut App) {
    app.add_systems(PostUpdate, propagate.in_set(UsdTransformSystems::Propagate)
        .after(bevy::transform::TransformSystems::Propagate)
        .before(bevy::camera::visibility::VisibilitySystems::UpdateFrusta));
}

pub(super) fn project(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let sample = match crate::read::xform::read_transform_stack_at(ctx.stage, ctx.path, ctx.time) {
        Ok(sample) => sample,
        Err(error) => { world.entity_mut(entity).insert(UsdTransformError(error.to_string())); return; }
    };
    let (matrix, reset) = sample.map(|(matrix, reset)| (Mat4::from_cols_array(&matrix), reset)).unwrap_or((Mat4::IDENTITY, false));
    let Some((transform, residual)) = representation(matrix) else {
        world.entity_mut(entity).insert(UsdTransformError("non-finite or projective USD transform".into()));
        return;
    };
    if let Ok(mut entity) = world.get_entity_mut(entity) {
        entity.insert(transform);
        entity.remove::<UsdTransformError>();
        if reset || residual != Mat4::IDENTITY {
            entity.insert(UsdTransformOverride { residual, reset });
        } else {
            entity.remove::<UsdTransformOverride>();
        }
    }
}

fn representation(matrix: Mat4) -> Option<(Transform, Mat4)> {
    if !matrix.is_finite() || matrix.row(3) != Vec4::W { return None; }
    let transform = Transform::from_matrix(matrix);
    let rebuilt = if transform.rotation.is_finite() && transform.scale.is_finite() { transform.to_matrix() } else { Mat4::ZERO };
    let close = [matrix.x_axis, matrix.y_axis, matrix.z_axis].into_iter()
        .zip([rebuilt.x_axis, rebuilt.y_axis, rebuilt.z_axis]).all(|(a,b)|
            a.abs_diff_eq(b, a.abs().max_element().max(f32::MIN_POSITIVE) * 1e-5));
    if rebuilt.is_finite() && rebuilt.w_axis.w == 1.0 && close { return Some((transform, Mat4::IDENTITY)); }
    let mut residual = matrix;
    residual.w_axis = Vec4::W;
    Some((Transform::from_translation(matrix.w_axis.truncate()), residual))
}

fn parent(world: &World, entity: Entity) -> Option<Entity> { world.get::<ChildOf>(entity).map(ChildOf::parent) }

fn scene_basis(world: &World, mut entity: Entity) -> GlobalTransform {
    while let Some(ancestor) = parent(world, entity) {
        if world.get::<UsdPrimRef>(ancestor).is_some_and(|prim| prim.path == "/") {
            return world.get::<GlobalTransform>(ancestor).copied().unwrap_or_default();
        }
        entity = ancestor;
    }
    GlobalTransform::IDENTITY
}

fn propagate(world: &mut World) {
    let roots: Vec<_> = world.query_filtered::<Entity, With<UsdTransformOverride>>().iter(world).filter(|&entity| {
        let mut ancestor = parent(world, entity);
        while let Some(entity) = ancestor {
            if world.get::<UsdTransformOverride>(entity).is_some() { return false; }
            ancestor = parent(world, entity);
        }
        true
    }).collect();
    for root in roots {
        let basis = parent(world, root).and_then(|parent| world.get::<GlobalTransform>(parent)).copied().unwrap_or_default();
        let mut pending = vec![(root, basis)];
        while let Some((entity, mut basis)) = pending.pop() {
            let Some(transform) = world.get::<Transform>(entity).copied() else { continue };
            let residual = world.get::<UsdTransformOverride>(entity).map(|state| {
                if state.reset { basis = scene_basis(world, entity); }
                state.residual
            }).unwrap_or(Mat4::IDENTITY);
            let global = GlobalTransform::from(basis.to_matrix() * transform.to_matrix() * residual);
            if let Some(mut current) = world.get_mut::<GlobalTransform>(entity) {
                if *current != global { *current = global; }
            }
            if let Some(children) = world.get::<Children>(entity) {
                pending.extend(children.iter().map(|child| (child, global)));
            }
        }
    }
}
