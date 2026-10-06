//! Ptex diffuse color baked into meshes.
//!
//! Each face's average from its Ptex file is spread onto the face's points,
//! so a mesh shows its texture's colors as vertex colors. Only a file's
//! header and per-face block are read, never its texels.

use std::sync::Arc;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::read::geom::ReadMesh;
use crate::read::ptex::PtexFaces;

/// Face colors of every Ptex file read so far, by resolved path.
#[derive(Resource, Default)]
pub(crate) struct PtexCache(HashMap<String, Option<Arc<PtexFaces>>>);

/// The face colors of the Ptex file at `path`, read once per world.
pub(crate) fn faces(world: &mut World, path: &str) -> Option<Arc<PtexFaces>> {
    world.init_resource::<PtexCache>();
    if let Some(cached) = world.resource::<PtexCache>().0.get(path) {
        return cached.clone();
    }
    let faces = read(world, path)
        .inspect_err(|error| warn!("Ptex {path}: {error}"))
        .ok()
        .map(Arc::new);
    world
        .resource_mut::<PtexCache>()
        .0
        .insert(path.to_string(), faces.clone());
    faces
}

/// Reads from disk, or through the default asset source for paths the asset
/// loader resolved under its virtual root.
fn read(world: &World, path: &str) -> anyhow::Result<PtexFaces> {
    let file = std::path::Path::new(path);
    if file.is_file() {
        return crate::read::ptex::read_face_colors(std::io::BufReader::new(std::fs::File::open(
            file,
        )?));
    }
    let root = crate::source::absolute(std::path::Path::new(crate::asset::ASSET_ROOT))?;
    let relative = file
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("not a file or asset path"))?
        .to_path_buf();
    anyhow::ensure!(
        !cfg!(target_arch = "wasm32"),
        "Ptex is read synchronously and is unavailable on the web"
    );
    let server = world
        .get_resource::<AssetServer>()
        .ok_or_else(|| anyhow::anyhow!("no asset server"))?;
    let source = server.get_source(bevy::asset::io::AssetSourceId::Default)?;
    bevy::tasks::block_on(async {
        use bevy::tasks::futures_lite::AsyncReadExt;
        let mut reader = source.reader().read(&relative).await?;
        let mut head = vec![0u8; 64];
        reader.read_exact(&mut head).await?;
        let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap()) as usize;
        let mut rest = vec![0u8; word(28) + word(32) + word(36)];
        reader.read_exact(&mut rest).await?;
        head.extend(rest);
        crate::read::ptex::read_face_colors(head.as_slice())
    })
}

/// `mesh` colored by `faces`, its texels decoded from sRGB when `srgb`;
/// `None` when the file does not match the mesh's faces.
pub(crate) fn color_mesh(
    mesh: &Mesh,
    read: &ReadMesh,
    faces: &PtexFaces,
    srgb: bool,
) -> Option<Mesh> {
    let face_colors = faces.mesh_face_colors(&read.face_vertex_counts)?;
    let mut sums = vec![[0.0f32; 4]; read.points.len()];
    let mut corner = 0;
    for (face, &count) in read.face_vertex_counts.iter().enumerate() {
        let [r, g, b] = face_colors[face];
        let color = if srgb {
            LinearRgba::from(Srgba::new(r, g, b, 1.0)).to_f32_array_no_alpha()
        } else {
            [r, g, b]
        };
        let count = count.max(0) as usize;
        for &point in read.face_vertex_indices.get(corner..corner + count)? {
            let sum = sums.get_mut(point as usize)?;
            for channel in 0..3 {
                sum[channel] += color[channel];
            }
            sum[3] += 1.0;
        }
        corner += count;
    }
    let points = crate::mesh::vertex_point_indices(read);
    if points.len() != mesh.count_vertices() {
        return None;
    }
    let colors: Vec<[f32; 4]> = points
        .into_iter()
        .map(|point| {
            let [r, g, b, count] = sums[point];
            if count > 0.0 {
                [r / count, g / count, b / count, 1.0]
            } else {
                [1.0; 4]
            }
        })
        .collect();
    let mut colored = mesh.clone();
    colored.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    Some(colored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptex_face_colors_become_linear_vertex_colors() {
        let directory = tempfile::tempdir().unwrap();
        let texture = directory.path().join("faces.ptx");
        std::fs::write(
            &texture,
            crate::read::ptex::tests::ptex(0, 3, &[255, 0, 0, 0, 0, 255], false),
        )
        .unwrap();
        let root = directory.path().join("scene.usda");
        std::fs::write(
            &root,
            format!(
                r#"#usda 1.0
def Mesh "Ground" (prepend apiSchemas = ["MaterialBindingAPI"]) {{
    point3f[] points = [(0,0,0),(1,0,0),(2,0,0),(0,0,1),(1,0,1),(2,0,1)]
    int[] faceVertexCounts = [4, 4]
    int[] faceVertexIndices = [0, 1, 4, 3, 1, 2, 5, 4]
    rel material:binding = </Mat>
}}
def Material "Mat" {{
    token outputs:ri:surface.connect = </Mat/Disney.outputs:bxdf_out>
    def Shader "Disney" {{
        uniform token info:id = "PxrDisneyBsdf"
        color3f inputs:baseColor.connect = </Mat/Texture.outputs:resultRGB>
        token outputs:bxdf_out
    }}
    def Shader "Texture" {{
        uniform token info:id = "PxrPtexture"
        asset inputs:filename = @{}@
        int inputs:linearize = 1
        color3f outputs:resultRGB
    }}
}}
"#,
                texture.display()
            ),
        )
        .unwrap();
        let source = crate::UsdSource::from_file(&root).unwrap();
        let live = crate::live::LiveStage::new(source.open_stage().unwrap());
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Ground").unwrap();
        let mesh = world
            .resource::<Assets<Mesh>>()
            .get(&world.get::<Mesh3d>(entity).unwrap().0)
            .unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("no Ptex vertex colors")
        };
        assert_eq!(colors[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(colors[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(colors[1], [0.5, 0.0, 0.5, 1.0]);
        let material = world
            .resource::<Assets<StandardMaterial>>()
            .get(
                &world
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .unwrap()
                    .0,
            )
            .unwrap();
        assert_eq!(material.base_color.to_linear(), LinearRgba::WHITE);
    }
}
