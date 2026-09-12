use openusd::usd::TimeCode;
use usd_bevy::UsdSource;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    verify()?;
    println!("Verified showcase composition, retimed samples and remapped bindings.");
    Ok(())
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/flagship_showcase.usda");
    let source = UsdSource::new(&path, std::fs::read(&path)?)?;
    let stage = source.open_stage()?;
    for (time, size, weight) in [(0.0, 1.0, 0.0), (30.0, 1.5, 0.5), (60.0, 2.0, 1.0)] {
        let time = Some(TimeCode::new(time));
        assert_eq!(stage.prim("/Showcase/ChangingCube")?.attribute("size").get_at::<f64>(time)?, Some(size));
        assert_eq!(stage.prim("/Showcase/Morph/Skel/Anim")?.attribute("blendShapeWeights")
            .get_at::<Vec<f32>>(time)?, Some(vec![weight]));
    }
    for (prim, relationship, expected) in [
        ("/Showcase/Skeleton/Bar", "skel:skeleton", "/Showcase/Skeleton/Skel"),
        ("/Showcase/Morph/Face", "skel:blendShapeTargets", "/Showcase/Morph/Face/smile"),
        ("/Showcase/GrowingPrototypes", "prototypes", "/Showcase/Prototypes/Tetrahedron"),
        ("/Showcase/Skeleton/Bar", "material:binding", "/Showcase/Materials/Animated"),
    ] {
        let targets = stage.prim(prim)?.relationship(relationship).targets()?;
        assert_eq!(targets.len(), 1, "{prim}: {relationship}");
        assert_eq!(targets[0].to_string(), expected);
    }
    Ok(())
}

#[test]
fn bundled_showcase_preserves_composition_and_retiming() { verify().unwrap(); }
