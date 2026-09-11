use super::*;
use bevy::math::DQuat;

fn cardinality<T>(value: &MeshPrimvar<T>, geometry: &Centerlines, name: &str) -> Result<(), String> {
    let expected = match value.interpolation {
        Interpolation::Constant => 1,
        Interpolation::Uniform => geometry.spans.len(),
        Interpolation::Vertex => geometry.spans.iter().map(|(_, layout)| layout.count).sum(),
        Interpolation::Varying => geometry.spans.iter().map(|(_, layout)| if layout.cubic {
            layout.segments + usize::from(!layout.periodic)
        } else { layout.count }).sum(),
        Interpolation::FaceVarying => return Err(format!("unsupported faceVarying {name}")),
    };
    let actual = if value.indices.is_empty() { value.values.len() } else { value.indices.len() };
    if actual != expected { return Err(format!("{name} has {actual} samples; expected {expected}")); }
    Ok(())
}

fn frames(points: &[DVec3], periodic: bool) -> Result<(Vec<DVec3>, Vec<DVec3>), String> {
    let n = points.len();
    if n < 2 { return Err("curve surface needs at least two distinct samples".into()); }
    let segments = n - usize::from(!periodic);
    let mut directions = Vec::with_capacity(segments);
    let mut distances = vec![0.0; n];
    let mut total = 0.0;
    for i in 0..segments {
        let edge = points[(i+1)%n] - points[i];
        let direction = edge.try_normalize().ok_or("coincident curve samples cannot form a surface")?;
        total += edge.length();
        if i+1 < n { distances[i+1] = total; }
        directions.push(direction);
    }
    let mut tangents = Vec::with_capacity(n);
    for i in 0..n {
        let previous = directions[if i == 0 { if periodic { segments-1 } else { 0 } } else { i-1 }];
        let next = directions[if i == segments { segments-1 } else { i }];
        tangents.push((previous+next).try_normalize().ok_or("curve cusp cannot form a surface")?);
    }
    let axis = if tangents[0].x.abs() <= tangents[0].y.abs() && tangents[0].x.abs() <= tangents[0].z.abs() { DVec3::X }
        else if tangents[0].y.abs() <= tangents[0].z.abs() { DVec3::Y } else { DVec3::Z };
    let mut transverse = vec![tangents[0].cross(axis).normalize()];
    for i in 1..n {
        let rotated = DQuat::from_rotation_arc(tangents[i-1], tangents[i]) * transverse[i-1];
        transverse.push((rotated-tangents[i]*rotated.dot(tangents[i])).try_normalize().ok_or("invalid transported curve frame")?);
    }
    if periodic {
        let closing = DQuat::from_rotation_arc(tangents[n-1], tangents[0]) * transverse[n-1];
        let correction = tangents[0].dot(closing.cross(transverse[0])).atan2(closing.dot(transverse[0]));
        for i in 1..n { transverse[i] = DQuat::from_axis_angle(tangents[i], correction*distances[i]/total) * transverse[i]; }
    }
    Ok((tangents, transverse))
}

fn build(ctx: &RouteCtx, steps: usize, sides: usize) -> Result<Option<Mesh>, String> {
    if !(3..=32).contains(&sides) { return Err("curve surface sides must be 3..=32".into()); }
    let Some(widths) = crate::read::curves::read_widths_at(ctx.stage, ctx.path, ctx.time).map_err(|error| error.to_string())? else { return Ok(None) };
    let normals = crate::read::curves::read_normals_at(ctx.stage, ctx.path, ctx.time).map_err(|error| error.to_string())?;
    let Some(geometry) = centerlines(ctx, steps) else { return Ok(None) };
    cardinality(&widths, &geometry, "widths")?;
    if let Some(normals) = &normals { cardinality(normals, &geometry, "normals")?; }
    if geometry.points.iter().flatten().any(|value| !value.is_finite())
        || geometry.colors.as_ref().is_some_and(|colors| colors.iter().flatten().any(|value| !value.is_finite())) {
        return Err("non-finite tessellated curve data".into());
    }
    let widths = MeshPrimvar { values: widths.values.into_iter().map(f64::from).collect(), interpolation: widths.interpolation, indices: widths.indices };
    let normals = normals.map(|normals| MeshPrimvar { values: normals.values.into_iter().map(|value| DVec3::from_array(value.map(f64::from))).collect(), interpolation: normals.interpolation, indices: normals.indices });
    let ring_size = if normals.is_some() { 2 } else { sides };
    let faces_per_ring = if normals.is_some() { 1 } else { sides };
    let mut budget = (0, 0);
    for (range, layout) in &geometry.spans {
        if range.is_empty() { continue; }
        let rings = range.len() - usize::from(layout.cubic && layout.periodic);
        let segments = rings.saturating_sub(usize::from(!layout.periodic));
        let vertices = rings.checked_mul(ring_size).ok_or("curve surface vertex overflow")?;
        let indices = segments.checked_mul(faces_per_ring).and_then(|count| count.checked_mul(6)).ok_or("curve surface index overflow")?;
        accumulate_curve_output(&mut budget, vertices, indices)?;
    }
    let mut positions = Vec::<[f32;3]>::with_capacity(budget.0);
    let mut shading = Vec::<[f32;3]>::with_capacity(budget.0);
    let mut colors = geometry.colors.as_ref().map(|_| Vec::with_capacity(budget.0));
    let mut indices = Vec::<u32>::with_capacity(budget.1);
    for (range, layout) in &geometry.spans {
        if range.is_empty() { continue; }
        let rings = range.len() - usize::from(layout.cubic && layout.periodic);
        let points = geometry.points[range.start..range.start+rings].iter().map(|point| DVec3::from_array(point.map(f64::from))).collect::<Vec<_>>();
        let (tangents, transverse) = frames(&points, layout.periodic)?;
        let base = positions.len();
        for i in 0..rings {
            let width = layout.sample(&widths, i, f64::NAN);
            if !width.is_finite() || width < 0.0 { return Err("invalid interpolated curve width".into()); }
            let u = if let Some(normals) = &normals {
                let normal = layout.sample(normals, i, DVec3::ZERO).try_normalize().ok_or("zero interpolated ribbon normal")?;
                tangents[i].cross(normal).try_normalize().ok_or("ribbon normal is parallel to its tangent")?
            } else { transverse[i] };
            let v = tangents[i].cross(u);
            for side in 0..ring_size {
                let radial = if normals.is_some() { u * if side == 0 { -1.0 } else { 1.0 } }
                    else { let angle = std::f64::consts::TAU*side as f64/sides as f64; u*angle.cos()+v*angle.sin() };
                let position = (points[i]+radial*(width*0.5)).as_vec3().to_array();
                if position.iter().any(|value| !value.is_finite()) { return Err("curve surface position exceeds finite f32".into()); }
                positions.push(position);
                shading.push(if normals.is_some() { u.cross(tangents[i]).as_vec3().to_array() } else { radial.as_vec3().to_array() });
                if let (Some(output), Some(input)) = (&mut colors, &geometry.colors) { output.push(input[range.start+i]); }
            }
        }
        for i in 0..rings-usize::from(!layout.periodic) {
            let next = (i+1)%rings;
            for side in 0..faces_per_ring {
                let a = (base+i*ring_size+side) as u32;
                let b = (base+i*ring_size+(side+1)%ring_size) as u32;
                let c = (base+next*ring_size+side) as u32;
                let d = (base+next*ring_size+(side+1)%ring_size) as u32;
                indices.extend([a,b,c,b,d,c]);
            }
        }
    }
    let mut sums = vec![DVec3::ZERO; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let p: [DVec3;3] = std::array::from_fn(|i| DVec3::from_array(positions[triangle[i] as usize].map(f64::from)));
        let Some(normal) = (p[1]-p[0]).cross(p[2]-p[0]).try_normalize() else { continue };
        for i in 0..3 {
            let a = p[(i+1)%3]-p[i]; let b = p[(i+2)%3]-p[i];
            sums[triangle[i] as usize] += normal*a.cross(b).length().atan2(a.dot(b));
        }
    }
    for (sum, normal) in sums.into_iter().zip(&mut shading) {
        if let Some(value) = sum.try_normalize() { *normal = value.as_vec3().to_array(); }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, shading);
    if let Some(colors) = colors { mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors); }
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    Ok(Some(mesh))
}

pub(super) fn project(ctx: &RouteCtx, world: &mut World, entity: Entity, steps: usize, sides: usize) -> Result<bool, String> {
    let Some(mesh) = build(ctx, steps, sides)? else { return Ok(false) };
    let translucent = matches!(mesh.attribute(Mesh::ATTRIBUTE_COLOR), Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) if colors.iter().any(|color| color[3] < 1.0));
    let handle = super::super::cache::intern_mesh(world, mesh);
    let mut material = super::super::material::default_material(ctx);
    material.cull_mode = None;
    material.alpha_mode = if translucent { AlphaMode::Blend } else { AlphaMode::Opaque };
    let material = super::super::cache::intern_material(world, material);
    world.entity_mut(entity).remove::<bevy::camera::primitives::Aabb>()
        .insert((Mesh3d(handle), MeshMaterial3d(material), super::super::geom::GeometryOwner::Curves));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn positions(mesh: &Mesh) -> &[[f32;3]] {
        let Some(VertexAttributeValues::Float32x3(values)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("positions") };
        values
    }

    #[test]
    fn surface_budget_counts_triangle_expansion_before_upload() {
        let source = crate::UsdSource::new("widths.usda", include_bytes!("../../../../../assets/curve_widths.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        stage.attribute("/ConstantTube.points").unwrap().set(Value::Vec3fVec(
            (0..12000).map(|i| [i as f32,0.,0.].into()).collect(),
        )).unwrap();
        stage.attribute("/ConstantTube.curveVertexCounts").unwrap().set(Value::IntVec(vec![12000])).unwrap();
        let path = openusd::sdf::path("/ConstantTube").unwrap();
        let ctx = RouteCtx::new(&stage, &path);
        validate_curves(&ctx, 8).unwrap();
        let error = build(&ctx, 8, 32).unwrap_err();
        assert!(error.contains("2303808 indices exceeds limits"), "{error}");
        assert!(build(&ctx, 8, 3).unwrap().is_some());
    }

    #[test]
    fn periodic_indexed_surfaces_close_without_connecting_curves() {
        let source = crate::UsdSource::new("loops.usda", include_bytes!("../../../../../assets/curve_surface_loops.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/Loops").unwrap();
        let ctx = RouteCtx::new(&stage, &path);
        validate_curves(&ctx, 8).unwrap();
        let center = centerlines(&ctx, 8).unwrap();
        let mesh = build(&ctx, 8, 12).unwrap().unwrap();
        let points = positions(&mesh);
        let vertices_per_curve = 32*12;
        assert_eq!(points.len(), vertices_per_curve*2);
        let mut edges = std::collections::HashMap::<(usize,usize), (usize,i32)>::new();
        let indices = mesh.indices().unwrap().iter().collect::<Vec<_>>();
        assert_eq!(indices.len(), 2*32*12*6);
        for triangle in indices.chunks_exact(3) {
            assert!(triangle.iter().all(|index| index/vertices_per_curve == triangle[0]/vertices_per_curve));
            let [a,b,c] = std::array::from_fn::<_,3,_>(|i| DVec3::from_array(points[triangle[i]].map(f64::from)));
            assert!((b-a).cross(c-a).length() > 1e-8);
            for i in 0..3 {
                let (a,b) = (triangle[i], triangle[(i+1)%3]);
                let entry = edges.entry((a.min(b), a.max(b))).or_default();
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
        assert!(edges.values().all(|value| *value == (2,0)), "open or inconsistently wound seam");
        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("colors") };
        for curve in 0..2 {
            let radius = [0.2,0.06][curve];
            for ring in 0..32 {
                let midpoint = Vec3::from_array(center.points[center.spans[curve].0.start+ring]);
                for side in 0..12 {
                    let index = curve*vertices_per_curve+ring*12+side;
                    assert!((Vec3::from_array(points[index]).distance(midpoint)-radius).abs() < 1e-6);
                    assert_eq!(colors[index], [[0.1,0.6,1.,1.],[1.,0.3,0.05,1.]][curve]);
                }
            }
        }
    }

    #[test]
    fn transported_frames_are_orthonormal_and_reject_degeneracy() {
        let points = (0..24).map(|i| {
            let angle = std::f64::consts::TAU*i as f64/24.;
            DVec3::new(angle.cos(), angle.sin(), 0.2*(2.*angle).sin())
        }).collect::<Vec<_>>();
        for periodic in [false, true] {
            let (tangents, transverse) = frames(&points, periodic).unwrap();
            for (t, u) in tangents.iter().zip(&transverse) {
                assert!((t.length()-1.).abs() < 1e-12);
                assert!((u.length()-1.).abs() < 1e-12);
                assert!(t.dot(*u).abs() < 1e-12);
            }
        }
        assert!(frames(&[DVec3::ZERO], false).is_err());
        assert!(frames(&[DVec3::ZERO, DVec3::ZERO], false).is_err());
        assert!(frames(&[DVec3::ZERO, DVec3::X, DVec3::ZERO], false).is_err());
        assert!(frames(&[DVec3::ZERO, DVec3::X], true).is_err());
    }

    #[test]
    fn surfaces_match_authored_diameters_and_orientation() {
        let source = crate::UsdSource::new("widths.usda", include_bytes!("../../../../../assets/curve_widths.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        for (name, radii) in [("ConstantTube", [0.1,0.1]), ("VertexTube", [0.025,0.2]), ("PrimvarWidth", [0.15,0.15])] {
            let path = openusd::sdf::path(format!("/{name}")).unwrap();
            let ctx = RouteCtx::new(&stage, &path);
            validate_curves(&ctx, 8).unwrap();
            let mesh = build(&ctx, 8, 12).unwrap().unwrap();
            let points = positions(&mesh);
            assert_eq!(points.len(), 24);
            for (ring, radius) in points.chunks_exact(12).zip(radii) {
                let center = ring.iter().map(|p| Vec3::from_array(*p)).sum::<Vec3>() / 12.;
                for point in ring { assert!((Vec3::from_array(*point).distance(center)-radius).abs() < 1e-6); }
            }
            let indices = mesh.indices().unwrap().iter().collect::<Vec<_>>();
            for triangle in indices.chunks_exact(3) {
                let [a,b,c] = std::array::from_fn::<_,3,_>(|i| Vec3::from_array(points[triangle[i]]));
                let radial = Vec3::new(0., a.y-points.iter().map(|p| p[1]).sum::<f32>()/24., a.z);
                assert!((b-a).cross(c-a).dot(radial) > 0., "inward tube face");
            }
        }
        let path = openusd::sdf::path("/Ribbon").unwrap();
        let mesh = build(&RouteCtx::new(&stage, &path), 8, 12).unwrap().unwrap();
        assert_eq!(positions(&mesh).len(), 18);
        for pair in positions(&mesh).chunks_exact(2) {
            assert!((Vec3::from_array(pair[0]).distance(Vec3::from_array(pair[1]))-0.2).abs() < 1e-6);
            assert!(pair.iter().all(|p| p[2] == 0.));
        }
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!("normals") };
        assert!(normals.iter().all(|n| n[2] > 0.999));
    }

    #[test]
    fn surface_errors_clear_geometry_and_can_recover() {
        let source = crate::UsdSource::new("widths.usda", include_bytes!("../../../../../assets/curve_widths.usda").as_slice()).unwrap();
        let stage = source.open_stage().unwrap();
        let path = openusd::sdf::path("/VertexTube").unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.insert_resource(UsdCurveSettings::default().with_surface_sides(Some(12)).unwrap());
        let entity = world.spawn_empty().id();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_some());
        stage.attribute("/VertexTube.widths").unwrap().set(Value::FloatVec(vec![0.2])).unwrap();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_none());
        assert!(world.get::<UsdCurveError>(entity).unwrap().0.contains("expected 2"));
        stage.attribute("/VertexTube.widths").unwrap().set(Value::FloatVec(vec![0.2,0.4])).unwrap();
        CurvesRoute.project(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_some());
        assert!(world.get::<UsdCurveError>(entity).is_none());
        CurvesRoute.remove(&RouteCtx::new(&stage, &path), &mut world, entity);
        assert!(world.get::<Mesh3d>(entity).is_none());
    }
}
