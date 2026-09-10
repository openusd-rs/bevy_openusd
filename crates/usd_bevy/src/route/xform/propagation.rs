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
    let axes = [matrix.x_axis.truncate(), matrix.y_axis.truncate(), matrix.z_axis.truncate()];
    let normalized = axes.map(Vec3::try_normalize);
    let orthogonal = match normalized {
        [Some(x), Some(y), Some(z)] => x.dot(y).abs() < 1e-6 && x.dot(z).abs() < 1e-6 && y.dot(z).abs() < 1e-6,
        _ => false,
    };
    let determinant = matrix.determinant();
    let transform = if orthogonal && determinant.is_finite() && determinant != 0.0 {
        Transform::from_matrix(matrix)
    } else {
        Transform::from_translation(matrix.w_axis.truncate())
    };
    let rebuilt = if transform.rotation.is_finite() && transform.rotation.is_normalized() && transform.scale.is_finite() { transform.to_matrix() } else { Mat4::ZERO };
    let close = [matrix.x_axis, matrix.y_axis, matrix.z_axis].into_iter()
        .zip([rebuilt.x_axis, rebuilt.y_axis, rebuilt.z_axis]).all(|(a,b)|
            a.abs_diff_eq(b, a.abs().max_element().max(f32::MIN_POSITIVE) * 1e-5));
    if rebuilt.is_finite() && rebuilt.w_axis.w == 1.0 && close { return Some((transform, Mat4::IDENTITY)); }
    let mut residual = matrix;
    residual.w_axis = Vec4::W;
    Some((Transform::from_translation(matrix.w_axis.truncate()), residual))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shear() -> Mat4 {
        Mat4::from_cols(Vec4::new(1.0, 0.0, 0.5, 0.0), Vec4::new(0.75, 1.0, 0.0, 0.0), Vec4::Z, Vec4::W)
    }

    #[test]
    fn independent_clocks_transition_between_affine_and_trs() {
        use crate::instance::{UsdInstances, UsdInstanceTime};
        let source = crate::UsdSource::new("xform-animation.usda", include_bytes!("../../../../../assets/xform_animation.usda").as_slice()).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin, bevy::asset::AssetPlugin::default(), crate::UsdPlugin, crate::UsdAssetPlugin));
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>();
        let handle = app.world_mut().resource_mut::<Assets<crate::UsdScene>>()
            .add(crate::UsdScene { source, textures: default() });
        let roots = [0, 1].map(|i| app.world_mut().spawn((crate::UsdSceneRoot(handle.clone()), UsdInstanceTime { current: 0.0 },
            Transform::from_xyz(i as f32 * 20.0, 2.0, 0.0))).id());
        app.update();
        let lookup = |world: &World, root, path| world.get_non_send::<UsdInstances>().unwrap().entity(root, path).unwrap();
        let entities = roots.map(|root| ["/Affine", "/Affine/Following", "/Affine/Reset"].map(|path| lookup(app.world(), root, path)));
        let runtime_local = Transform::from_xyz(1.0, 2.0, 3.0);
        let children = entities.map(|nodes| app.world_mut().spawn((Name::new("runtime child"), runtime_local, ChildOf(nodes[1]))).id());
        for times in [[0.0, 5.0], [2.5, 7.5], [5.0, 10.0], [10.0, 0.0], [5.0, 5.0], [0.0, 10.0]] {
            for (root, current) in roots.into_iter().zip(times) {
                app.world_mut().get_mut::<UsdInstanceTime>(root).unwrap().current = current;
            }
            app.update();
            for (index, (root, time)) in roots.into_iter().zip(times).enumerate() {
                let world = app.world();
                let basis = world.get::<GlobalTransform>(lookup(world, root, "/")).unwrap().to_matrix();
                let (start, end, weight) = if time <= 5.0 {
                    (Mat4::IDENTITY, shear(), time as f32 / 5.0)
                } else {
                    (shear(), Mat4::from_translation(Vec3::X * 2.0), (time as f32 - 5.0) / 5.0)
                };
                let a = start.to_cols_array();
                let b = end.to_cols_array();
                let local = Mat4::from_cols_array(&std::array::from_fn(|i| a[i] + (b[i] - a[i]) * weight));
                let following = basis * local * Mat4::from_translation(Vec3::Z);
                let reset = basis * Mat4::from_translation(Vec3::new(3.0 + time as f32 / 5.0, 0.0, 1.0));
                for (kind, path) in ["/Affine", "/Affine/Following", "/Affine/Reset"].into_iter().enumerate() {
                    assert_eq!(lookup(world, root, path), entities[index][kind]);
                }
                assert_eq!(world.get::<UsdTransformOverride>(entities[index][0]).is_some(), time > 0.0 && time < 10.0);
                assert!(world.get::<GlobalTransform>(entities[index][1]).unwrap().to_matrix().abs_diff_eq(following, 1e-5));
                assert!(world.get::<GlobalTransform>(entities[index][2]).unwrap().to_matrix().abs_diff_eq(reset, 1e-5));
                assert!(world.get::<GlobalTransform>(children[index]).unwrap().to_matrix().abs_diff_eq(following * runtime_local.to_matrix(), 1e-5));
                assert_eq!(world.get::<Name>(children[index]).unwrap().as_str(), "runtime child");
                assert_eq!(world.get::<ChildOf>(children[index]).unwrap().parent(), entities[index][1]);
            }
        }
    }

    #[test]
    fn live_malformed_matrix_edits_keep_last_pose_and_recover() {
        use crate::live::{LiveStage, PrimEntities, project_stage, apply_changes};
        use openusd::{usd::Stage, sdf::Value, gf::Matrix4d};
        let stage = Stage::builder().schema_registry(openusd_schemas::schema_registry()).in_memory("affine-recovery.usda").unwrap();
        stage.define_prim("/Foo").unwrap().set_type_name("Xform").unwrap();
        let matrix = stage.create_attribute("/Foo.xformOp:transform", "matrix4d").unwrap();
        matrix.set(Value::Matrix4d(Matrix4d(shear().to_cols_array().map(f64::from)))).unwrap();
        stage.create_attribute("/Foo.xformOpOrder", "token[]").unwrap()
            .set(Value::TokenVec(vec!["xformOp:transform".into()])).unwrap();
        let live = LiveStage::new(stage);
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin);
        configure(&mut app);
        let mut map = PrimEntities::default();
        project_stage(app.world_mut(), &live, &mut map);
        let entity = map.entity("/Foo").unwrap();
        app.world_mut().entity_mut(entity).insert(Name::new("runtime"));
        app.update();
        let original = *app.world().get::<GlobalTransform>(entity).unwrap();
        for bad in [Mat4::perspective_rh(1.0, 1.0, 0.1, 10.0), Mat4::from_scale(Vec3::splat(f32::NAN))] {
            live.stage.attribute("/Foo.xformOp:transform").unwrap()
                .set(Value::Matrix4d(Matrix4d(bad.to_cols_array().map(f64::from)))).unwrap();
            apply_changes(app.world_mut(), &live, &mut map);
            app.update();
            assert!(app.world().get::<UsdTransformError>(entity).is_some());
            assert_eq!(*app.world().get::<GlobalTransform>(entity).unwrap(), original);
        }
        let recovered = Mat4::from_translation(Vec3::new(3.0, 4.0, 5.0));
        live.stage.attribute("/Foo.xformOp:transform").unwrap()
            .set(Value::Matrix4d(Matrix4d(recovered.to_cols_array().map(f64::from)))).unwrap();
        apply_changes(app.world_mut(), &live, &mut map);
        app.update();
        assert!(app.world().get::<UsdTransformError>(entity).is_none());
        assert!(app.world().get::<UsdTransformOverride>(entity).is_none());
        assert_eq!(app.world().get::<GlobalTransform>(entity).unwrap().to_matrix(), recovered);
        assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime");
        assert_eq!(map.entity("/Foo"), Some(entity));
    }

    #[test]
    fn editor_history_reprojects_affine_reset_without_replacing_entities() {
        use crate::editor::{EditorEdit, EditorSession};
        use crate::live::{LiveStage, PrimEntities, project_stage, apply_changes};
        use openusd::sdf::Value;
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .in_memory("history-projection.usda").unwrap();
        stage.define_prim("/Parent").unwrap().set_type_name("Xform").unwrap();
        stage.define_prim("/Parent/Child").unwrap().set_type_name("Xform").unwrap();
        crate::live::author_transform(&stage, "/Parent", &Transform::from_xyz(10.0, 0.0, 0.0)).unwrap();
        stage.create_attribute("/Parent/Child.xformOp:transform", "matrix4d").unwrap()
            .set(Value::Matrix4d(openusd::gf::Matrix4d(shear().to_cols_array().map(f64::from)))).unwrap();
        stage.create_attribute("/Parent/Child.xformOpOrder", "token[]").unwrap()
            .set(Value::TokenVec(vec!["!resetXformStack!".into(), "xformOp:transform".into()])).unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let mut editor = EditorSession::new(stage.clone());
        let live = LiveStage::new(stage);
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin);
        configure(&mut app);
        let mut map = PrimEntities::default();
        project_stage(app.world_mut(), &live, &mut map);
        let entity = map.entity("/Parent/Child").unwrap();
        app.world_mut().entity_mut(entity).insert(Name::new("runtime"));
        app.update();
        assert!(app.world().get::<GlobalTransform>(entity).unwrap().to_matrix().abs_diff_eq(shear(), 1e-6));
        editor.edit(EditorEdit::TransformMatrix { prim: "/Parent/Child".into(),
            matrix: Mat4::from_translation(Vec3::X * 3.0).to_cols_array().map(f64::from), reset: false }).unwrap();
        let after = live.stage.root_layer().export_to_string().unwrap();
        for edited in [true, false, true, false] {
            if !edited { assert!(editor.undo().unwrap()); }
            else if !editor.snapshot().unwrap().can_undo { assert!(editor.redo().unwrap()); }
            apply_changes(app.world_mut(), &live, &mut map);
            app.update();
            let expected = if edited { Mat4::from_translation(Vec3::X * 13.0) } else { shear() };
            assert!(app.world().get::<GlobalTransform>(entity).unwrap().to_matrix().abs_diff_eq(expected, 1e-6));
            assert_eq!(app.world().get::<UsdTransformOverride>(entity).is_some(), !edited);
            assert!(app.world().get::<UsdTransformError>(entity).is_none());
            assert_eq!(map.entity("/Parent/Child"), Some(entity));
            assert_eq!(app.world().get::<Name>(entity).unwrap().as_str(), "runtime");
            assert_eq!(live.stage.root_layer().export_to_string().unwrap(), if edited { &after } else { &before }.as_str());
        }
    }

    #[test]
    fn authored_reset_discards_prefix_and_reprojection_clears_override() {
        let stage = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/xform_reset.usda")).unwrap();
        let reference = openusd::usd::Stage::builder().schema_registry(openusd_schemas::schema_registry())
            .open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/xform_reset_reference.usda")).unwrap();
        let path = openusd::sdf::path("/Parent/Reset").unwrap();
        let reference_path = openusd::sdf::path("/Reset").unwrap();
        let before = stage.root_layer().export_to_string().unwrap();
        let (matrix, reset) = crate::read::xform::read_transform_stack_at(&stage, &path, None).unwrap().unwrap();
        assert!(reset);
        assert_eq!(Mat4::from_cols_array(&matrix), Mat4::from_translation(Vec3::Z));
        let mut world = World::new();
        let entity = world.spawn(Name::new("runtime name")).id();
        project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<UsdTransformOverride>(entity).unwrap().reset);
        project(&RouteCtx::new(&reference, &reference_path), &mut world, entity);
        assert!(world.get::<UsdTransformOverride>(entity).is_none());
        assert_eq!(world.get::<Transform>(entity).unwrap().translation, Vec3::Z);
        assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "runtime name");
        assert_eq!(stage.root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    fn representation_preserves_affine_and_singular_matrices() {
        for matrix in [shear(), Mat4::from_scale(Vec3::ZERO), Mat4::from_scale(Vec3::splat(1e-20)),
            Mat4::from_scale_rotation_translation(Vec3::new(-2.0, 3.0, 4.0), Quat::from_rotation_y(0.7), Vec3::ONE)] {
            let (transform, residual) = representation(matrix).unwrap();
            assert!((transform.to_matrix() * residual).abs_diff_eq(matrix, 1e-6));
        }
        assert!(representation(Mat4::from_cols(Vec4::splat(f32::NAN), Vec4::Y, Vec4::Z, Vec4::W)).is_none());
        assert!(representation(Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0)).is_none());
    }

    #[test]
    fn affine_globals_follow_runtime_edits_and_reset_to_scene_basis() {
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin);
        configure(&mut app);
        let world = app.world_mut();
        let mount = world.spawn(Transform::from_xyz(10.0, 0.0, 0.0)).id();
        let basis = Transform::from_rotation(Quat::from_rotation_x(0.5));
        let scene = world.spawn((basis, UsdPrimRef::new("/"), ChildOf(mount))).id();
        let local = Transform::from_xyz(0.0, 3.0, 0.0);
        let affine = world.spawn((local, UsdTransformOverride { residual: shear(), reset: false }, ChildOf(scene))).id();
        let child_local = Transform::from_xyz(2.0, 0.0, 0.0);
        let child = world.spawn((child_local, ChildOf(affine))).id();
        let reset = world.spawn((child_local, UsdTransformOverride { residual: Mat4::IDENTITY, reset: true }, ChildOf(child))).id();
        for offset in [3.0, 7.0] {
            app.world_mut().get_mut::<Transform>(affine).unwrap().translation.y = offset;
            app.update();
            let world = app.world();
            let scene_global = world.get::<GlobalTransform>(scene).unwrap().to_matrix();
            let expected = scene_global * world.get::<Transform>(affine).unwrap().to_matrix() * shear() * child_local.to_matrix();
            assert!(world.get::<GlobalTransform>(child).unwrap().to_matrix().abs_diff_eq(expected, 1e-5));
            assert!(world.get::<GlobalTransform>(reset).unwrap().to_matrix().abs_diff_eq(scene_global * child_local.to_matrix(), 1e-5));
        }
    }
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
