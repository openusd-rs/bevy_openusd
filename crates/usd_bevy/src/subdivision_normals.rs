use std::collections::{BTreeMap, BTreeSet};
use glam::DVec3;
use crate::read::geom::{Interpolation, MeshPrimvar, Orientation, ReadMesh};

pub(crate) fn limit_normals(mesh: &ReadMesh) -> MeshPrimvar<[f32; 3]> {
    let mut rings = vec![BTreeMap::new(); mesh.points.len()];
    let mut invalid = BTreeSet::new();
    for face in mesh.face_vertex_indices.chunks_exact(4) {
        for i in 0..4 {
            let vertex = face[i] as usize;
            let sector = (face[(i+2)%4] as usize, face[(i+3)%4] as usize);
            if rings[vertex].insert(face[(i+1)%4] as usize, sector).is_some() { invalid.insert(vertex); }
        }
    }
    let mut values = crate::mesh::compute_point_smooth_normals(mesh);
    for (vertex, ring) in rings.iter().enumerate() {
        if ring.is_empty() || invalid.contains(&vertex) { continue; }
        let incoming = ring.values().map(|&(_, edge)| edge).collect::<BTreeSet<_>>();
        let starts = ring.keys().filter(|edge| !incoming.contains(edge)).copied().collect::<Vec<_>>();
        if starts.len()>1 { continue; }
        let boundary = starts.len()==1;
        let start = starts.first().copied().unwrap_or(*ring.keys().next().unwrap());
        let mut edges = Vec::new(); let mut diagonals = Vec::new(); let mut seen = BTreeSet::new();
        let mut edge = start;
        while seen.insert(edge) {
            edges.push(edge);
            let Some(&(diagonal, next)) = ring.get(&edge) else { break; };
            diagonals.push(diagonal); edge = next;
        }
        if diagonals.len()!=ring.len() || (!boundary && edge!=start) || edges.len()!=ring.len()+usize::from(boundary) { continue; }
        let center = DVec3::from_array(mesh.points[vertex].map(f64::from));
        let relative = |i: usize| DVec3::from_array(mesh.points[i].map(f64::from))-center;
        let e = edges.into_iter().map(relative).collect::<Vec<_>>();
        let f = diagonals.into_iter().map(relative).collect::<Vec<_>>();
        let (u,v) = tangents(&e,&f,boundary);
        let sign = if mesh.orientation == Orientation::LeftHanded { -1.0 } else { 1.0 };
        values[vertex] = (u.cross(v).normalize_or_zero()*sign).as_vec3().to_array();
    }
    MeshPrimvar { values, interpolation: Interpolation::Vertex, indices: Vec::new() }
}

fn tangents(e: &[DVec3], f: &[DVec3], boundary: bool) -> (DVec3,DVec3) {
    let n = f.len();
    if boundary {
        let u = (e[0]-e[n])*0.5;
        let v = match n {
            1 => (e[0]+e[1])*3.0,
            2 => (-e[0]+e[1]*4.0-e[2]+f[0]+f[1])/6.0,
            _ => {
                let theta = std::f64::consts::PI/n as f64;
                let c = theta.cos(); let s = theta.sin(); let r = (c+1.0)/s;
                let mut value = (e[0]+e[n])*(-r*(1.0+2.0*c)) + f[0]*s;
                for i in 1..n {
                    let a = (i as f64*theta).sin(); let b = ((i+1) as f64*theta).sin();
                    value += e[i]*(4.0*a)+f[i]*(a+b);
                }
                value/(n as f64*(3.0+c))
            }
        };
        return (u,v);
    }
    if n==2 { return (e[0],e[1]); }
    let theta = std::f64::consts::TAU/n as f64;
    let c = theta.cos();
    let lambda = (5.0+c+(theta*0.5).cos()*(2.0*(9.0+c)).sqrt())/16.0;
    let scale = 1.0/(4.0*lambda-1.0);
    let mut u = DVec3::ZERO; let mut v = DVec3::ZERO;
    for i in 0..n {
        let a = (i as f64*theta).cos(); let b = ((i+1) as f64*theta).cos();
        let previous = ((i as f64-1.0)*theta).cos();
        u += e[i]*(4.0*a)+f[i]*(scale*(a+b));
        v += e[i]*(4.0*previous)+f[i]*(scale*(previous+a));
    }
    (u,v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonplanar_limit_normal_is_independent_of_ring_start() {
        for n in [3,4,5,8] {
            let mut e = (0..n).map(|i| {
                let a=i as f64*std::f64::consts::TAU/n as f64;
                DVec3::new(a.cos(),a.sin(),0.2*(a*2.0).sin()+0.3*a.cos())
            }).collect::<Vec<_>>();
            let mut f = (0..n).map(|i| e[i]+e[(i+1)%n]+DVec3::Z*(i as f64*0.07)).collect::<Vec<_>>();
            let (u,v)=tangents(&e,&f,false);
            let expected=u.cross(v).normalize();
            for _ in 0..n {
                e.rotate_left(1); f.rotate_left(1);
                let (u,v)=tangents(&e,&f,false);
                assert!(u.cross(v).normalize().abs_diff_eq(expected,1e-12));
            }
        }
    }
    #[test]
    fn planar_limit_tangents_preserve_orientation_and_scale() {
        for n in [3,4,5,8] {
            for scale in [1e-12,1.0,1e12] {
                let points = |angle: f64| DVec3::new(angle.cos(),angle.sin(),0.0)*scale;
                let e = (0..n).map(|i| points(i as f64*std::f64::consts::TAU/n as f64)).collect::<Vec<_>>();
                let f = (0..n).map(|i| e[i]+e[(i+1)%n]).collect::<Vec<_>>();
                let (u,v) = tangents(&e,&f,false);
                assert!(u.cross(v).normalize().abs_diff_eq(DVec3::Z,1e-12));
            }
        }
        for n in 1..6 {
            let e = (0..=n).map(|i| { let a=i as f64*std::f64::consts::PI/n as f64; DVec3::new(a.cos(),a.sin(),0.0) }).collect::<Vec<_>>();
            let f = (0..n).map(|i| e[i]+e[i+1]+DVec3::Y).collect::<Vec<_>>();
            if n==1 { continue; }
            let (u,v) = tangents(&e,&f,true);
            assert!(u.cross(v).normalize().abs_diff_eq(DVec3::Z,1e-12));
        }
        let (u,v) = tangents(&[DVec3::X,DVec3::Y], &[DVec3::X+DVec3::Y],true);
        assert!(u.cross(v).normalize().abs_diff_eq(DVec3::Z,1e-12));
    }
}
