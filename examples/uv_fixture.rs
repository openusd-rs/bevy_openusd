//! Writes a deterministic textured UV regression scene into a new directory.

use std::path::Path;
use bevy::{prelude::Image, render::render_resource::TextureFormat};

fn write_fixture(directory: &Path) -> Result<(), String> {
    std::fs::create_dir(directory).map_err(|error| error.to_string())?;
    let mut image = Image::new_target_texture(2, 2, TextureFormat::Rgba8UnormSrgb, None);
    image.data = Some(vec![255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,0,255]);
    image.try_into_dynamic().map_err(|error| error.to_string())?.save(directory.join("quadrants.png"))
        .map_err(|error| error.to_string())?;
    std::fs::write(directory.join("scene.usda"), include_str!("../assets/inherited_uv.usda"))
        .map_err(|error| error.to_string())
}

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 { eprintln!("usage: uv_fixture NEW_DIRECTORY"); return std::process::ExitCode::from(2); }
    match write_fixture(Path::new(&args[0])) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => { eprintln!("fixture: {error}"); std::process::ExitCode::from(1) }
    }
}

#[test]
fn fixture_is_source_preserving_and_refuses_overwrite() {
    let directory = std::env::temp_dir().join(format!("usd-uv-fixture-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    write_fixture(&directory).unwrap();
    let path = directory.join("scene.usda");
    let source = usd_bevy::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap();
    let stage = source.open_stage().unwrap();
    let mesh = openusd::sdf::path("/Root/Inherited").unwrap();
    let uvs = usd_bevy::read::geom::read_mesh_at(&stage, &mesh, Some(10.0)).unwrap().unwrap().uvs.unwrap();
    assert_eq!(uvs.indices, [1]);
    assert_eq!(uvs.values, [[0.25,0.25], [0.75,0.75]]);
    assert!(directory.join("quadrants.png").is_file());
    assert!(write_fixture(&directory).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
