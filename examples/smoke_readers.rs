//! Headless smoke-test of the migrated openusd readers (no GPU/bevy render):
//! open a stage, walk it, run the readers, print what they decode.
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::read::{camera, geom, lux, skel, xform};

fn main() {
    let path = std::env::args().nth(1).expect("usage: smoke_readers <usd>");
    let stage = Stage::open(&path).expect("open stage");
    let (mut meshes, mut verts, mut skels, mut joints, mut lights, mut cams, mut xforms) = (0, 0usize, 0, 0, 0, 0, 0);
    stage
        .traverse(PrimPredicate::default(), |p| {
            if let Ok(Some(m)) = geom::read_mesh(&stage, p) {
                meshes += 1;
                verts += m.points.len();
                if meshes <= 5 {
                    println!("  mesh {}  pts={} faces={} normals={} uvs={}", p.as_str(), m.points.len(), m.face_vertex_counts.len(), m.normals.is_some(), m.uvs.is_some());
                }
            }
            if let Ok(Some(s)) = skel::read_skeleton(&stage, p) {
                skels += 1;
                joints += s.joints.len();
                println!("  skeleton {}  joints={} bind={} rest={}", p.as_str(), s.joints.len(), s.bind_transforms.len(), s.rest_transforms.len());
            }
            if let Ok(Some(_)) = lux::read_light(&stage, p) { lights += 1; }
            if let Ok(Some(_)) = camera::read_camera(&stage, p) { cams += 1; }
            if let Ok(Some(_)) = xform::read_transform(&stage, p) { xforms += 1; }
        })
        .unwrap();
    println!("TOTAL meshes={meshes} verts={verts} skeletons={skels} joints={joints} lights={lights} cameras={cams} xformed_prims={xforms}");
}
