//! Ptex texels packed into one texture for meshes whose faces are large.
//!
//! Vertex colors carry one Ptex average per face, which blurs terrain whose
//! faces span many pixels. With [`UsdPtexAtlas`] such a Catmull-Clark mesh
//! is refined, every control face's texels fill one tile of a texture, and
//! each refined corner addresses its face's tile by UV.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::RouteCtx;
use crate::read::geom::{Interpolation, MeshPrimvar, ReadMesh, SubdivScheme};
use crate::read::ptex::{PtexLayout, PtexLevel};

/// Draws Ptex-colored Catmull-Clark meshes with large faces through a
/// texture of their texels, rather than one color per face.
#[derive(Resource, Clone, Copy, Debug)]
pub struct UsdPtexAtlas {
    /// The most texels along a face's side.
    pub texels: usize,
    /// The most texels in one mesh's texture.
    pub max_texels: usize,
    /// Meshes whose faces average less than this across, in meters, keep
    /// their vertex colors.
    pub min_face_meters: f32,
    /// The most faces one mesh may refine into.
    pub max_faces: usize,
}

impl Default for UsdPtexAtlas {
    fn default() -> Self {
        Self {
            texels: 64,
            max_texels: 24 << 20,
            min_face_meters: 0.2,
            max_faces: 1 << 20,
        }
    }
}

/// The texture a mesh's Ptex material samples instead of vertex colors.
#[derive(Component, Clone, Debug)]
pub(crate) struct PtexAtlasImage(pub Handle<Image>);

const LARGEST_SIDE: usize = 8192;

/// `read` refined and addressing a texture of the Ptex file at `path`, when
/// [`UsdPtexAtlas`] asks for it and the mesh qualifies.
pub(crate) fn atlas_mesh(
    ctx: &RouteCtx,
    world: &mut World,
    read: &ReadMesh,
    path: &str,
    srgb: bool,
) -> Option<(Mesh, Handle<Image>)> {
    let settings = *world.get_resource::<UsdPtexAtlas>()?;
    if read.subdivision_scheme != SubdivScheme::CatmullClark
        || !world.contains_resource::<Assets<Image>>()
    {
        return None;
    }
    let faces = read.face_vertex_counts.len();
    let across = (face_area(read) / faces.max(1) as f32).sqrt();
    if across * crate::live::stage_meters_per_unit(ctx.stage) < settings.min_face_meters {
        return None;
    }
    let grid = Grid::new(faces, &settings)?;
    let rules =
        crate::read::subdivision::read_subdivision_at(ctx.stage, ctx.path, ctx.time).ok()?;
    let (levels, (mut refined, coordinates)) = (1..=3u32)
        .rev()
        .filter(|&levels| faces << (2 * levels) <= settings.max_faces)
        .find_map(|levels| {
            crate::subdivision::refine_mesh_with_coordinates(read, &rules, levels)
                .inspect_err(|error| {
                    debug!("{}: Ptex atlas at {levels} levels: {error}", ctx.prim_str())
                })
                .ok()
                .map(|refined| (levels, refined))
        })?;
    let (layout, level) = super::ptex::read_level(world, path, |layout| {
        let mut picks: Vec<usize> = (0..layout.faces.len())
            .map(|face| layout.level_for(face, grid.side))
            .collect();
        picks.sort_unstable();
        picks.get(picks.len() / 2).copied().unwrap_or(0)
    })
    .inspect_err(|error| warn!("Ptex {path}: {error}"))
    .ok()?;
    let tiles = ptex_faces(&layout, &read.face_vertex_counts)?;
    let image = grid.image(&tiles, &layout, &level, srgb);
    if rules.crease_indices.is_empty() && rules.corner_indices.is_empty() && rules.holes.is_empty()
    {
        refined.points = crate::subdivision_limit::limit_points(&refined, &rules);
    }
    if let Some(displacement) = crate::read::shade::read_material_binding(ctx.stage, ctx.path)
        .ok()
        .flatten()
        .and_then(|material| {
            crate::read::displacement::read_displacement(ctx.stage, &material)
                .ok()
                .flatten()
        })
    {
        displace(
            world,
            &mut refined,
            &coordinates,
            &tiles,
            &displacement,
            1 << levels,
        );
    }
    refined.uvs = Some(MeshPrimvar {
        values: coordinates
            .corners
            .iter()
            .enumerate()
            .map(|(corner, &uv)| grid.uv(coordinates.faces[corner / 4], uv))
            .collect(),
        interpolation: Interpolation::FaceVarying,
        indices: Vec::new(),
    });
    refined.display_color = None;
    debug!(
        "{}: Ptex atlas {}x{} at {} texels a face, {} faces",
        ctx.prim_str(),
        grid.width,
        grid.height,
        grid.side,
        refined.face_vertex_counts.len()
    );
    let mesh = crate::mesh::assemble_mesh(&refined, None, false);
    let image = world.resource_mut::<Assets<Image>>().add(image);
    Some((mesh, image))
}

/// Moves `refined`'s points along their normals by `displacement`, its
/// textures sampled `side` texels across each control face, and renews the
/// normals; leaves the mesh as it is when a texture cannot be read.
fn displace(
    world: &World,
    refined: &mut ReadMesh,
    coordinates: &crate::subdivision::FaceCoordinates,
    tiles: &[std::ops::Range<usize>],
    displacement: &crate::read::displacement::ReadDisplacement,
    side: usize,
) {
    let mut product = vec![1.0f32; coordinates.corners.len()];
    for path in &displacement.textures {
        let Ok((layout, level)) = super::ptex::read_level(world, path, |layout| {
            let mut picks: Vec<usize> = (0..layout.faces.len())
                .map(|face| layout.level_for(face, side))
                .collect();
            picks.sort_unstable();
            picks.get(picks.len() / 2).copied().unwrap_or(0)
        })
        .inspect_err(|error| warn!("Ptex {path}: {error}")) else {
            return;
        };
        if layout.faces.len() != tiles.last().map_or(0, |tile| tile.end) {
            return;
        }
        for (corner, value) in product.iter_mut().enumerate() {
            let subfaces = &tiles[coordinates.faces[corner / 4]];
            let [u, v] = coordinates.corners[corner];
            *value *= match level.faces[subfaces.start].as_ref() {
                Some(texels) if subfaces.len() == 1 => texels.sample(u, v)[0],
                _ => {
                    subfaces
                        .clone()
                        .map(|face| layout.average(face)[0])
                        .sum::<f32>()
                        / subfaces.len() as f32
                }
            };
        }
    }
    let Some(normals) = refined
        .normals
        .take_if(|normals| normals.interpolation == Interpolation::Vertex)
        .map(|normals| normals.values)
        .or_else(|| Some(crate::subdivision_normals::limit_normals(refined).values))
        .filter(|normals| normals.len() == refined.points.len())
    else {
        return;
    };
    let mut sums = vec![(0.0f32, 0u32); refined.points.len()];
    for (corner, &point) in refined.face_vertex_indices.iter().enumerate() {
        let sum = &mut sums[point as usize];
        sum.0 += product[corner];
        sum.1 += 1;
    }
    for ((point, normal), (sum, count)) in refined.points.iter_mut().zip(&normals).zip(sums) {
        if count > 0 {
            let offset = displacement.amount * displacement.remap.apply(sum / count as f32);
            *point = (Vec3::from_array(*point) + Vec3::from_array(*normal) * offset).to_array();
        }
    }
    refined.normals = Some(crate::subdivision_normals::limit_normals(refined));
}

/// The area of `read`'s faces, fanned from their first corners.
fn face_area(read: &ReadMesh) -> f32 {
    let point = |index: i32| Vec3::from_array(read.points[index as usize]);
    let mut area = 0.0;
    let mut corner = 0;
    for &count in &read.face_vertex_counts {
        let count = count.max(0) as usize;
        let Some(face) = read.face_vertex_indices.get(corner..corner + count) else {
            break;
        };
        corner += count;
        if face
            .iter()
            .any(|&index| index as usize >= read.points.len())
        {
            continue;
        }
        for pair in face[1..].windows(2) {
            area += 0.5
                * (point(pair[0]) - point(face[0]))
                    .cross(point(pair[1]) - point(face[0]))
                    .length();
        }
    }
    area
}

/// The Ptex faces of each mesh face: one for a quad, one per corner for
/// any other face; `None` when the file was written for other topology.
fn ptex_faces(layout: &PtexLayout, counts: &[i32]) -> Option<Vec<std::ops::Range<usize>>> {
    if layout.triangles {
        return None;
    }
    let mut next = 0;
    let ranges: Vec<_> = counts
        .iter()
        .map(|&count| {
            let subfaces = if count == 4 { 1 } else { count.max(0) as usize };
            next += subfaces;
            next - subfaces..next
        })
        .collect();
    (next == layout.faces.len()).then_some(ranges)
}

/// Square tiles of `side` texels and a one-texel border, in rows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    side: usize,
    columns: usize,
    width: usize,
    height: usize,
}

impl Grid {
    /// The largest tiles up to the settings' side that fit the settings'
    /// budget and the largest texture side.
    fn new(faces: usize, settings: &UsdPtexAtlas) -> Option<Self> {
        let columns = (faces as f64).sqrt().ceil() as usize;
        let rows = faces.div_ceil(columns.max(1));
        let mut side = settings.texels.next_power_of_two();
        while side >= 4 {
            let tile = side + 2;
            if faces * tile * tile <= settings.max_texels
                && columns.max(rows) * tile <= LARGEST_SIDE
            {
                return Some(Self {
                    side,
                    columns,
                    width: columns * tile,
                    height: rows * tile,
                });
            }
            side /= 2;
        }
        None
    }

    fn origin(&self, face: usize) -> (usize, usize) {
        let tile = self.side + 2;
        ((face % self.columns) * tile, (face / self.columns) * tile)
    }

    /// Where face coordinates `uv` of mesh face `face` land in the texture,
    /// as a USD texture coordinate.
    fn uv(&self, face: usize, [u, v]: [f32; 2]) -> [f32; 2] {
        let (left, top) = self.origin(face);
        let x = (left as f32 + 1.0 + u * self.side as f32) / self.width as f32;
        let y = (top as f32 + 1.0 + v * self.side as f32) / self.height as f32;
        [x, 1.0 - y]
    }

    /// Fills each mesh face's tile from its Ptex texels, or with its
    /// average where the level holds none.
    fn image(
        &self,
        tiles: &[std::ops::Range<usize>],
        layout: &PtexLayout,
        level: &PtexLevel,
        srgb: bool,
    ) -> Image {
        let tile = self.side + 2;
        let mut data = vec![255u8; self.width * self.height * 4];
        for (face, subfaces) in tiles.iter().enumerate() {
            let texels = (subfaces.len() == 1)
                .then(|| level.faces[subfaces.start].as_ref())
                .flatten();
            let average = subfaces.clone().fold([0.0f32; 3], |sum, subface| {
                let color = layout.average(subface);
                std::array::from_fn(|channel| sum[channel] + color[channel] / subfaces.len() as f32)
            });
            let (left, top) = self.origin(face);
            for y in 0..tile {
                for x in 0..tile {
                    let color = texels.map_or(average, |texels| {
                        texels.sample(
                            (x as f32 - 0.5) / self.side as f32,
                            (y as f32 - 0.5) / self.side as f32,
                        )
                    });
                    let at = ((top + y) * self.width + left + x) * 4;
                    for channel in 0..3 {
                        data[at + channel] = (color[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
        let mut image = Image::new(
            Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            if srgb {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            },
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::linear();
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_ptex_faces_draw_their_texels_through_an_atlas() {
        let directory = tempfile::tempdir().unwrap();
        let texture = directory.path().join("ground.ptx");
        let (red, green, blue, white) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255; 3]);
        std::fs::write(
            &texture,
            crate::read::ptex::tests::ptex_texels(&[[red, green, blue, white], [white; 4]]),
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
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdPtexAtlas {
            min_face_meters: 0.0,
            ..default()
        });
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Ground").unwrap();
        let atlas = world.get::<PtexAtlasImage>(entity).unwrap().0.clone();
        let material = world
            .resource::<Assets<StandardMaterial>>()
            .get(
                &world
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .unwrap()
                    .0,
            )
            .unwrap();
        assert_eq!(material.base_color_texture.as_ref(), Some(&atlas));
        let mesh = world
            .resource::<Assets<Mesh>>()
            .get(&world.get::<Mesh3d>(entity).unwrap().0)
            .unwrap();
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
        // Two faces refined three times, each corner its own vertex.
        assert_eq!(mesh.count_vertices(), 2 * 64 * 4);
        let image = world.resource::<Assets<Image>>().get(&atlas).unwrap();
        assert!(image.texture_descriptor.format.is_srgb());
        let pixel = |x: u32, y: u32| image.get_color_at(x, y).unwrap().to_srgba();
        let tile = 64 + 2;
        assert_eq!(pixel(1, 1), Srgba::RED);
        assert_eq!(pixel(tile - 2, 1), Srgba::GREEN);
        assert_eq!(pixel(1, tile - 2), Srgba::BLUE);
        assert_eq!(pixel(0, 0), Srgba::RED, "the border repeats the edge");
        assert_eq!(pixel(tile + 10, 10), Srgba::WHITE);
    }

    #[test]
    fn displacement_moves_the_refined_surface_along_its_normals() {
        let directory = tempfile::tempdir().unwrap();
        let color = directory.path().join("color.ptx");
        let height = directory.path().join("height.ptx");
        std::fs::write(
            &color,
            crate::read::ptex::tests::ptex_texels(&[[[90; 3]; 4]]),
        )
        .unwrap();
        std::fs::write(
            &height,
            crate::read::ptex::tests::ptex_texels(&[[[255; 3]; 4]]),
        )
        .unwrap();
        let root = directory.path().join("scene.usda");
        std::fs::write(
            &root,
            format!(
                r#"#usda 1.0
def Mesh "Ground" (prepend apiSchemas = ["MaterialBindingAPI"]) {{
    point3f[] points = [(0,0,0),(1,0,0),(1,0,1),(0,0,1)]
    int[] faceVertexCounts = [4]
    int[] faceVertexIndices = [0, 1, 2, 3]
    rel material:binding = </Mat>
}}
def Material "Mat" {{
    token outputs:ri:surface.connect = </Mat/Disney.outputs:bxdf_out>
    token outputs:ri:displacement.connect = </Mat/Displace.outputs:displace>
    def Shader "Disney" {{
        uniform token info:id = "PxrDisneyBsdf"
        color3f inputs:baseColor.connect = </Mat/Color.outputs:resultRGB>
        token outputs:bxdf_out
    }}
    def Shader "Color" {{
        uniform token info:id = "PxrPtexture"
        asset inputs:filename = @{}@
        color3f outputs:resultRGB
    }}
    def Shader "Displace" {{
        uniform token info:id = "PxrDisplace"
        float inputs:dispAmount = 2
        float inputs:dispScalar.connect = </Mat/Remap.outputs:resultF>
        token outputs:displace
    }}
    def Shader "Remap" {{
        uniform token info:id = "PxrDispTransform"
        int inputs:dispRemapMode = 2
        float inputs:dispDepth = 0.5
        float inputs:dispHeight = 0.5
        float inputs:dispScalar.connect = </Mat/Height.outputs:resultR>
        float outputs:resultF
    }}
    def Shader "Height" {{
        uniform token info:id = "PxrPtexture"
        asset inputs:filename = @{}@
        float outputs:resultR
    }}
}}
"#,
                color.display(),
                height.display()
            ),
        )
        .unwrap();
        let source = crate::UsdSource::from_file(&root).unwrap();
        let live = crate::live::LiveStage::new(source.open_stage().unwrap());
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdPtexAtlas {
            min_face_meters: 0.0,
            ..default()
        });
        let mut map = crate::live::PrimEntities::default();
        crate::live::project_stage(&mut world, &live, &mut map);
        let entity = map.entity("/Ground").unwrap();
        let mesh = world
            .resource::<Assets<Mesh>>()
            .get(&world.get::<Mesh3d>(entity).unwrap().0)
            .unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions")
        };
        // A full texel lifts by the remapped height, 0.5, times the amount,
        // 2, along the face's normal, which points down.
        for position in positions {
            assert!((position[1] + 1.0).abs() < 1e-5, "{position:?}");
        }
    }

    #[test]
    fn tiles_shrink_to_fit_the_budget_and_the_largest_side() {
        let settings = UsdPtexAtlas::default();
        let small = Grid::new(4637, &settings).unwrap();
        assert_eq!((small.side, small.columns), (64, 69));
        assert_eq!((small.width, small.height), (69 * 66, 68 * 66));
        let large = Grid::new(57700, &settings).unwrap();
        assert_eq!(large.side, 16);
        assert!(large.width <= LARGEST_SIDE && large.height <= LARGEST_SIDE);
    }

    #[test]
    fn face_coordinates_land_inside_their_tiles_borders() {
        let grid = Grid::new(5, &UsdPtexAtlas::default()).unwrap();
        assert_eq!((grid.side, grid.columns), (64, 3));
        let texel = |[x, y]: [f32; 2]| {
            [x * grid.width as f32, (1.0 - y) * grid.height as f32].map(|value| value.round())
        };
        assert_eq!(texel(grid.uv(4, [0.0, 0.0])), [67.0, 67.0]);
        assert_eq!(texel(grid.uv(4, [1.0, 1.0])), [131.0, 131.0]);
        assert_eq!(texel(grid.uv(2, [1.0, 0.0])), [197.0, 1.0]);
    }
}
