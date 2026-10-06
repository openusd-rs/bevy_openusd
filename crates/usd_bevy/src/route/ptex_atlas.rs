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
use crate::read::geom::{Interpolation, ReadMesh, SubdivScheme};
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
    let tiles = ptex_faces(&read.face_vertex_counts);
    let image = atlas_image(world, path, srgb, grid, &tiles)?;
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
    debug!(
        "{}: Ptex atlas {}x{} at {} texels a face, {} faces",
        ctx.prim_str(),
        grid.width,
        grid.height,
        grid.side,
        refined.face_vertex_counts.len()
    );
    Some((assemble(&refined, &coordinates, &grid), image))
}

/// Atlases already built, by Ptex file, color space and tiles.
#[derive(Resource, Default)]
struct AtlasImages(bevy::platform::collections::HashMap<(String, bool, Grid), Handle<Image>>);

/// The atlas of the Ptex file at `path` in `grid`'s tiles, built once and
/// shared by every mesh drawing that file the same way.
fn atlas_image(
    world: &mut World,
    path: &str,
    srgb: bool,
    grid: Grid,
    tiles: &[std::ops::Range<usize>],
) -> Option<Handle<Image>> {
    let key = (path.to_string(), srgb, grid);
    world.init_resource::<AtlasImages>();
    if let Some(image) = world.resource::<AtlasImages>().0.get(&key) {
        return Some(image.clone());
    }
    let (layout, level) = super::ptex::read_level(world, path, |layout| {
        let mut picks: Vec<usize> = (0..layout.faces.len())
            .map(|face| layout.level_for(face, grid.side))
            .collect();
        picks.sort_unstable();
        picks.get(picks.len() / 2).copied().unwrap_or(0)
    })
    .inspect_err(|error| warn!("Ptex {path}: {error}"))
    .ok()?;
    if layout.triangles || layout.faces.len() != tiles.last().map_or(0, |tile| tile.end) {
        return None;
    }
    let compress = world
        .get_resource::<bevy::image::CompressedImageFormatSupport>()
        .is_some_and(|support| support.0.contains(bevy::image::CompressedImageFormats::BC));
    let image = grid.image(tiles, &layout, &level, srgb, compress);
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let mut cache = world.resource_mut::<AtlasImages>();
    cache
        .0
        .retain(|_, image| super::cache::externally_owned(image));
    cache.0.insert(key, image.clone());
    Some(image)
}

/// A triangle mesh of `refined` that samples `grid`: the corners of one
/// control face share vertices, and points on the edges between control
/// faces split into one vertex per face, as their tiles differ.
fn assemble(
    refined: &ReadMesh,
    coordinates: &crate::subdivision::FaceCoordinates,
    grid: &Grid,
) -> Mesh {
    let normal = |corner: usize, point: usize| -> Option<[f32; 3]> {
        let normals = refined.normals.as_ref()?;
        let at = match normals.interpolation {
            Interpolation::FaceVarying => corner,
            _ => point,
        };
        let at = normals.indices.get(at).map_or(at, |&index| index as usize);
        normals.values.get(at).copied()
    };
    let mut vertices = bevy::platform::collections::HashMap::<(u32, u32), u32>::new();
    let (mut positions, mut normals, mut uvs) = (Vec::new(), Vec::new(), Vec::new());
    let corners: Vec<u32> = refined
        .face_vertex_indices
        .iter()
        .enumerate()
        .map(|(corner, &point)| {
            let face = coordinates.faces[corner / 4];
            *vertices
                .entry((point as u32, face as u32))
                .or_insert_with(|| {
                    positions.push(refined.points[point as usize]);
                    normals.push(normal(corner, point as usize));
                    uvs.push(grid.uv(face, coordinates.corners[corner]));
                    (positions.len() - 1) as u32
                })
        })
        .collect();
    let mut indices = Vec::with_capacity(corners.len() / 4 * 6);
    for quad in corners.chunks_exact(4) {
        let [a, b, c, d] = [quad[0], quad[1], quad[2], quad[3]];
        match refined.orientation {
            crate::read::geom::Orientation::LeftHanded => indices.extend([a, c, b, a, d, c]),
            crate::read::geom::Orientation::RightHanded => indices.extend([a, b, c, a, c, d]),
        }
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    match normals.into_iter().collect::<Option<Vec<_>>>() {
        Some(normals) => mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals),
        None => mesh.compute_smooth_normals(),
    }
    mesh
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

/// The Ptex faces of each mesh face in a quad Ptex file: one for a quad,
/// one per corner for any other face.
fn ptex_faces(counts: &[i32]) -> Vec<std::ops::Range<usize>> {
    let mut next = 0;
    counts
        .iter()
        .map(|&count| {
            let subfaces = if count == 4 { 1 } else { count.max(0) as usize };
            next += subfaces;
            next - subfaces..next
        })
        .collect()
}

/// Texels around each tile that repeat its edge, so filtering stays inside
/// the tile and compressed blocks never straddle two tiles.
const BORDER: usize = 2;

/// Square tiles of `side` texels and a border, in rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
            let tile = side + 2 * BORDER;
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

    fn tile(&self) -> usize {
        self.side + 2 * BORDER
    }

    fn origin(&self, face: usize) -> (usize, usize) {
        let tile = self.tile();
        ((face % self.columns) * tile, (face / self.columns) * tile)
    }

    /// Where face coordinates `uv` of mesh face `face` land in the texture,
    /// from its top-left corner.
    fn uv(&self, face: usize, [u, v]: [f32; 2]) -> [f32; 2] {
        let (left, top) = self.origin(face);
        let inset = BORDER as f32;
        let x = (left as f32 + inset + u * self.side as f32) / self.width as f32;
        let y = (top as f32 + inset + v * self.side as f32) / self.height as f32;
        [x, y]
    }

    /// Fills each mesh face's tile from its Ptex texels, or with its
    /// average where the level holds none; `compress` stores it as BC1.
    fn image(
        &self,
        tiles: &[std::ops::Range<usize>],
        layout: &PtexLayout,
        level: &PtexLevel,
        srgb: bool,
        compress: bool,
    ) -> Image {
        let tile = self.tile();
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
            let at = |texel: usize| (texel as f32 - BORDER as f32 + 0.5) / self.side as f32;
            for y in 0..tile {
                for x in 0..tile {
                    let color = texels.map_or(average, |texels| texels.sample(at(x), at(y)));
                    let at = ((top + y) * self.width + left + x) * 4;
                    for channel in 0..3 {
                        data[at + channel] = (color[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
        let format = match (compress, srgb) {
            (true, true) => TextureFormat::Bc1RgbaUnormSrgb,
            (true, false) => TextureFormat::Bc1RgbaUnorm,
            (false, true) => TextureFormat::Rgba8UnormSrgb,
            (false, false) => TextureFormat::Rgba8Unorm,
        };
        if compress {
            data = bc1(&data, self.width, self.height);
        }
        let mut image = Image::new(
            Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::linear();
        image
    }
}

/// RGBA8 texels, `width` by `height` in multiples of four, as opaque BC1
/// blocks: each four by four block keeps two colors spanning its texels'
/// range, slightly inset, and picks one of four blends of them per texel.
fn bc1(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let pack = |color: [u8; 3]| {
        (u16::from(color[0]) >> 3) << 11
            | (u16::from(color[1]) >> 2) << 5
            | u16::from(color[2]) >> 3
    };
    let unpack = |packed: u16| {
        let (r, g, b) = (packed >> 11 & 31, packed >> 5 & 63, packed & 31);
        [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2].map(i32::from)
    };
    let mut blocks = Vec::with_capacity(width * height / 2);
    for top in (0..height).step_by(4) {
        for left in (0..width).step_by(4) {
            let texels: [[u8; 3]; 16] = std::array::from_fn(|texel| {
                let at = ((top + texel / 4) * width + left + texel % 4) * 4;
                [rgba[at], rgba[at + 1], rgba[at + 2]]
            });
            let (mut low, mut high) = ([u8::MAX; 3], [0u8; 3]);
            for texel in &texels {
                for channel in 0..3 {
                    low[channel] = low[channel].min(texel[channel]);
                    high[channel] = high[channel].max(texel[channel]);
                }
            }
            for channel in 0..3 {
                let inset = (high[channel] - low[channel]) >> 4;
                low[channel] += inset;
                high[channel] -= inset;
            }
            let (first, second) = (pack(high).max(pack(low)), pack(high).min(pack(low)));
            let mut indices = 0u32;
            if first != second {
                let (a, b) = (unpack(first), unpack(second));
                let palette: [[i32; 3]; 4] = [
                    a,
                    b,
                    std::array::from_fn(|channel| (2 * a[channel] + b[channel]) / 3),
                    std::array::from_fn(|channel| (a[channel] + 2 * b[channel]) / 3),
                ];
                for (texel, color) in texels.iter().enumerate() {
                    let distance = |entry: &[i32; 3]| -> i32 {
                        (0..3)
                            .map(|channel| (entry[channel] - i32::from(color[channel])).pow(2))
                            .sum()
                    };
                    let best = (0..4)
                        .min_by_key(|&entry| distance(&palette[entry]))
                        .unwrap();
                    indices |= (best as u32) << (2 * texel);
                }
            }
            blocks.extend(first.to_le_bytes());
            blocks.extend(second.to_le_bytes());
            blocks.extend(indices.to_le_bytes());
        }
    }
    blocks
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
def Mesh "Raised" (prepend apiSchemas = ["MaterialBindingAPI"]) {{
    point3f[] points = [(0,1,0),(1,1,0),(2,1,0),(0,1,1),(1,1,1),(2,1,1)]
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
        let raised = map.entity("/Raised").unwrap();
        assert_eq!(
            world.get::<PtexAtlasImage>(raised).unwrap().0,
            atlas,
            "meshes on one Ptex file share its atlas"
        );
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
        // Two faces refined three times into nine by nine points each; the
        // points on the edge they share split, one for each face's tile.
        assert_eq!(mesh.count_vertices(), 2 * 9 * 9);
        let image = world.resource::<Assets<Image>>().get(&atlas).unwrap();
        assert!(image.texture_descriptor.format.is_srgb());
        let pixel = |x: u32, y: u32| image.get_color_at(x, y).unwrap().to_srgba();
        let tile = 64 + 2 * BORDER as u32;
        assert_eq!(pixel(2, 2), Srgba::RED);
        assert_eq!(pixel(tile - 3, 2), Srgba::GREEN);
        assert_eq!(pixel(2, tile - 3), Srgba::BLUE);
        assert_eq!(pixel(0, 0), Srgba::RED, "the border repeats the edge");
        assert_eq!(pixel(tile + 10, 10), Srgba::WHITE);
    }

    #[test]
    fn bc1_blocks_keep_flat_colors_and_follow_gradients() {
        let (width, height) = (8, 4);
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|texel| {
                let x = texel % width;
                if x < 4 {
                    [200, 120, 40, 255]
                } else {
                    let ramp = (x - 4) as u8 * 80;
                    [ramp, ramp, ramp, 255]
                }
            })
            .collect();
        let blocks = bc1(&rgba, width, height);
        assert_eq!(blocks.len(), 2 * 8);
        let decode = |block: &[u8], texel: usize| -> [i32; 3] {
            let unpack = |packed: u16| {
                let (r, g, b) = (packed >> 11 & 31, packed >> 5 & 63, packed & 31);
                [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2].map(i32::from)
            };
            let a = unpack(u16::from_le_bytes([block[0], block[1]]));
            let b = unpack(u16::from_le_bytes([block[2], block[3]]));
            let index = u32::from_le_bytes(block[4..8].try_into().unwrap()) >> (2 * texel) & 3;
            std::array::from_fn(|channel| match index {
                0 => a[channel],
                1 => b[channel],
                2 => (2 * a[channel] + b[channel]) / 3,
                _ => (a[channel] + 2 * b[channel]) / 3,
            })
        };
        for texel in 0..16 {
            let flat = decode(&blocks[..8], texel);
            assert!(
                (flat[0] - 200).abs() <= 8 && (flat[1] - 120).abs() <= 4,
                "{flat:?}"
            );
            let ramp = decode(&blocks[8..], texel)[1];
            let expected = (texel % 4) as i32 * 80;
            assert!(
                (ramp - expected).abs() <= 24,
                "texel {texel}: {ramp} vs {expected}"
            );
        }
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
        assert_eq!((small.width, small.height), (69 * 68, 68 * 68));
        let large = Grid::new(57700, &settings).unwrap();
        assert_eq!(large.side, 16);
        assert!(large.width <= LARGEST_SIDE && large.height <= LARGEST_SIDE);
    }

    #[test]
    fn face_coordinates_land_inside_their_tiles_borders() {
        let grid = Grid::new(5, &UsdPtexAtlas::default()).unwrap();
        assert_eq!((grid.side, grid.columns), (64, 3));
        let texel = |[x, y]: [f32; 2]| {
            [x * grid.width as f32, y * grid.height as f32].map(|value| value.round())
        };
        assert_eq!(texel(grid.uv(4, [0.0, 0.0])), [70.0, 70.0]);
        assert_eq!(texel(grid.uv(4, [1.0, 1.0])), [134.0, 134.0]);
        assert_eq!(texel(grid.uv(2, [1.0, 0.0])), [202.0, 2.0]);
    }
}
