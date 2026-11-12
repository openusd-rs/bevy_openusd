use std::fmt::Debug;

mod tangent_allocations;

#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TangentBits {
    values: [u32; 12],
    orientation: bool,
}

#[derive(Clone)]
struct Geometry {
    vertices: Vec<Vertex>,
    faces: Vec<Vec<usize>>,
    written: Vec<Vec<Option<Option<TangentBits>>>>,
}

impl Geometry {
    fn new(vertices: Vec<Vertex>, faces: Vec<Vec<usize>>) -> Self {
        assert!(faces.iter().flatten().all(|&i| i < vertices.len()));
        let written = faces.iter().map(|face| vec![None; face.len()]).collect();
        Self {
            vertices,
            faces,
            written,
        }
    }
    fn vertex(&self, face: usize, corner: usize) -> Vertex {
        self.vertices[self.faces[face][corner]]
    }
}

macro_rules! adapter {
    ($name:ident, $backend:ident) => {
        struct $name(Geometry);
        impl $backend::Geometry for $name {
            fn num_faces(&self) -> usize {
                self.0.faces.len()
            }
            fn num_vertices_of_face(&self, face: usize) -> usize {
                self.0.faces[face].len()
            }
            fn position(&self, face: usize, corner: usize) -> [f32; 3] {
                self.0.vertex(face, corner).position
            }
            fn normal(&self, face: usize, corner: usize) -> [f32; 3] {
                self.0.vertex(face, corner).normal
            }
            fn tex_coord(&self, face: usize, corner: usize) -> [f32; 2] {
                self.0.vertex(face, corner).uv
            }
            fn set_tangent(
                &mut self,
                value: Option<$backend::TangentSpace>,
                face: usize,
                corner: usize,
            ) {
                let bits = value.map(|value| {
                    let t = value.tangent();
                    let b = value.bi_tangent();
                    let e = value.tangent_encoded();
                    TangentBits {
                        values: [
                            t[0],
                            t[1],
                            t[2],
                            b[0],
                            b[1],
                            b[2],
                            value.tangent_magnitude(),
                            value.bi_tangent_magnitude(),
                            e[0],
                            e[1],
                            e[2],
                            e[3],
                        ]
                        .map(f32::to_bits),
                        orientation: value.is_orientation_preserving(),
                    }
                });
                assert!(self.0.written[face][corner].replace(bits).is_none());
            }
        }
    };
}

adapter!(Original, tangent_original);
adapter!(Candidate, tangent_candidate);

#[derive(Default)]
struct Counts {
    negative_control: bool,
    cases: usize,
    corners: usize,
    missing: usize,
    skipped: usize,
}

fn compare(name: &str, geometry: Geometry, counts: &mut Counts) {
    for threshold in [-1.0, -0.999, -0.0, 0.5, 0.999, 1.0] {
        let mut a = Original(geometry.clone());
        let mut b = Candidate(geometry.clone());
        tangent_original::generate_tangents_with_threshold(&mut a, threshold).unwrap();
        tangent_candidate::generate_tangents_with_threshold(&mut b, threshold).unwrap();
        if counts.negative_control {
            if let Some(Some(Some(bits))) =
                b.0.written
                    .iter_mut()
                    .flatten()
                    .find(|value| matches!(value, Some(Some(_))))
            {
                bits.values[0] ^= 1;
            }
        }
        assert_eq!(
            a.0.written, b.0.written,
            "case={name} threshold={threshold}"
        );
        for (face, output) in geometry.faces.iter().zip(&a.0.written) {
            for value in output {
                if matches!(face.len(), 3 | 4) {
                    assert!(value.is_some(), "unwritten supported corner: {name}");
                    counts.corners += 1;
                    counts.missing += usize::from(value == &Some(None));
                } else {
                    assert!(value.is_none(), "written unsupported corner: {name}");
                    counts.skipped += 1;
                }
            }
        }
        counts.cases += 1;
    }
}

fn fan(count: usize, mirror: bool, warp: f32) -> Geometry {
    let mut vertices = vec![Vertex {
        position: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
        uv: [0.5; 2],
    }];
    for i in 0..count {
        let angle = std::f32::consts::TAU * i as f32 / count as f32;
        let (s, c) = angle.sin_cos();
        vertices.push(Vertex {
            position: [c, s, warp * (3.0 * angle).sin()],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5 + if mirror { -c } else { c } * 0.4, 0.5 + s * 0.4],
        });
    }
    let faces = (0..count)
        .map(|i| vec![0, i + 1, (i + 1) % count + 1])
        .collect();
    Geometry::new(vertices, faces)
}

fn expanded(mut mesh: Geometry, seam: bool) -> Geometry {
    let mut vertices = Vec::new();
    for (index, face) in mesh.faces.iter_mut().enumerate() {
        for corner in face {
            let mut vertex = mesh.vertices[*corner];
            if seam && index % 2 == 1 {
                vertex.uv[0] = 1.0 - vertex.uv[0];
            }
            *corner = vertices.len();
            vertices.push(vertex);
        }
    }
    Geometry::new(vertices, mesh.faces)
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn grid(size: usize, disconnected: bool, degenerates: bool) -> Geometry {
    let mut vertices = Vec::new();
    for y in 0..=size {
        for x in 0..=size {
            vertices.push(Vertex {
                position: [x as f32, y as f32, ((x * y) % 7) as f32 / 32.0],
                normal: [0.0, 0.0, 1.0],
                uv: [x as f32 / size as f32, y as f32 / size as f32],
            });
        }
    }
    let mut faces = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let a = y * (size + 1) + x;
            let b = a + 1;
            let c = a + size + 1;
            let d = c + 1;
            faces.extend([vec![a, b, d], vec![a, d, c]]);
            if degenerates {
                faces.extend([vec![a, a, d], vec![a, a, a], vec![b, b, d, c]]);
            }
        }
    }
    let mut mesh = Geometry::new(vertices, faces);
    if disconnected {
        mesh = expanded(mesh, false);
        for (index, face) in mesh.faces.iter().enumerate() {
            for &corner in face {
                mesh.vertices[corner].uv[0] += 2.0 * index as f32;
            }
        }
    }
    mesh
}

fn allocations(negative: bool) {
    tangent_allocations::self_test();
    for size in [8, 32, 64] {
        for disconnected in [false, true] {
            let mesh = grid(size, disconnected, false);
            let mut a = Original(mesh.clone());
            let mut b = Candidate(mesh);
            let original = tangent_allocations::measure(|| {
                tangent_original::generate_tangents(&mut a).unwrap();
            });
            let candidate = tangent_allocations::measure(|| {
                tangent_candidate::generate_tangents(&mut b).unwrap();
            });
            assert_eq!(a.0.written, b.0.written);
            println!(
                "tangent_allocations size={size} disconnected={disconnected} original={original:?} candidate={candidate:?} scope=requested-call-local-storage-not-RSS-or-FPS"
            );
            let calls = if negative {
                original.calls
            } else {
                candidate.calls
            };
            assert!(calls < original.calls, "allocation calls did not decrease");
            assert!(candidate.peak <= original.peak, "peak live bytes increased");
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--sparse-allocations"] {
        tangent_allocations::self_test();
        for active in [0, 1, 8, 2048] {
            let mut mesh = expanded(grid(32, false, false), false);
            for face in mesh.faces.iter().skip(active) {
                for &index in face {
                    mesh.vertices[index].uv = [0.0; 2];
                }
            }
            let mut a = Original(mesh.clone());
            let mut b = Candidate(mesh);
            let original = tangent_allocations::measure(|| {
                tangent_original::generate_tangents(&mut a).unwrap();
            });
            let candidate = tangent_allocations::measure(|| {
                tangent_candidate::generate_tangents(&mut b).unwrap();
            });
            assert_eq!(a.0.written, b.0.written);
            println!(
                "tangent_sparse_allocations active_faces={active} total_faces=2048 original={original:?} candidate={candidate:?} scope=diagnostic-not-performance-acceptance"
            );
        }
        return;
    }
    if args == ["--retention-negative-control"] {
        let mut retained = None;
        tangent_allocations::measure(|| {
            retained = Some(std::hint::black_box(Box::new([1u8; 128])));
        });
        std::hint::black_box(retained);
        panic!("retained allocation was not detected");
    }
    if args == ["--allocations"] || args == ["--allocation-negative-control"] {
        allocations(args == ["--allocation-negative-control"]);
        return;
    }
    assert!(args.is_empty() || args == ["--negative-control"]);
    let mut counts = Counts {
        negative_control: !args.is_empty(),
        ..Counts::default()
    };
    compare("empty", Geometry::new(Vec::new(), Vec::new()), &mut counts);
    for n in [3, 4, 5, 8, 17, 64, 257, 1024] {
        for mirror in [false, true] {
            let mesh = fan(n, mirror, 0.2);
            compare(&format!("fan-{n}-{mirror}"), mesh.clone(), &mut counts);
            compare(
                &format!("expanded-{n}-{mirror}"),
                expanded(mesh.clone(), false),
                &mut counts,
            );
            compare(
                &format!("seam-{n}-{mirror}"),
                expanded(mesh, true),
                &mut counts,
            );
        }
    }
    let square = fan(4, false, 0.0);
    for faces in [
        vec![vec![1, 2, 3, 4]],
        vec![vec![1, 2, 3], vec![1, 3, 4]],
        vec![vec![1, 2, 4], vec![2, 3, 4]],
        vec![vec![0, 0, 1]],
        vec![vec![1, 1, 1]],
        vec![vec![], vec![0], vec![0, 1], vec![0, 1, 2, 3, 4]],
    ] {
        compare(
            "quad-degenerate-unsupported",
            Geometry::new(square.vertices.clone(), faces),
            &mut counts,
        );
    }
    for size in [2, 8, 32] {
        for disconnected in [false, true] {
            for degenerates in [false, true] {
                compare(
                    &format!("grid-{size}-{disconnected}-{degenerates}"),
                    grid(size, disconnected, degenerates),
                    &mut counts,
                );
            }
        }
    }
    let mut state = 0x92c45ab39f01d783;
    for case in 0..512 {
        let n = 3 + (random(&mut state) % 48) as usize;
        let mut mesh = fan(n, case % 2 == 0, (case % 7) as f32 / 10.0);
        for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
            match case % 8 {
                0 => vertex.uv = [0.0; 2],
                1 => vertex.normal = [0.0; 3],
                2 => vertex.normal = [-0.0, 0.0, 1.0],
                3 => vertex.position = vertex.position.map(|x| x * 1048576.0),
                4 => vertex.position = vertex.position.map(|x| x / 1048576.0),
                5 if index % 3 == 0 => vertex.position = [0.0; 3],
                6 => vertex.uv = [f32::from_bits(1), -0.0],
                _ => {
                    vertex.uv[0] += (random(&mut state) % 17) as f32 / 128.0;
                    vertex.normal = if index % 2 == 0 {
                        [0.0, 0.0, 1.0]
                    } else {
                        [0.0, 1.0, 0.0]
                    };
                }
            }
        }
        mesh.faces.extend([
            vec![0, 0, 1],
            vec![1, 1, 1],
            vec![0, 1],
            vec![0, 1, 2, 3, 0],
        ]);
        if case % 3 == 0 {
            mesh.faces.reverse();
        }
        if case % 5 == 0 {
            for face in &mut mesh.faces {
                face.reverse();
            }
        }
        mesh = Geometry::new(mesh.vertices, mesh.faces);
        if case % 4 == 0 {
            mesh = expanded(mesh, case % 8 == 0);
        }
        compare(&format!("seeded-{case}"), mesh, &mut counts);
    }
    assert!(counts.missing > 0 && counts.skipped > 0 && counts.corners > 100_000);
    println!(
        "tangent_corpus cases={} supported_corners={} missing_tangents={} skipped_corners={} failures=0 thresholds=6 seed=92c45ab39f01d783 scope=finite-synthetic-geometry-not-rendering-or-FPS",
        counts.cases, counts.corners, counts.missing, counts.skipped
    );
}
