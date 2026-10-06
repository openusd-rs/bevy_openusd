//! Ptex per-face colors.
//!
//! A Ptex file stores, ahead of its texels, one constant value per face: the
//! face's average. Reading only the header and that block gives a mesh's
//! colors face by face without decoding any texel data.

use std::io::Read;

const HEADER: usize = 64;

/// The average color of every face in a Ptex file.
#[derive(Debug, Clone, PartialEq)]
pub struct PtexFaces {
    /// Triangle meshes map faces one to one; quad meshes split every
    /// non-quad face into one subface per corner.
    pub triangles: bool,
    pub colors: Vec<[f32; 3]>,
}

/// Reads the face averages from the start of a Ptex file.
pub fn read_face_colors(mut reader: impl Read) -> anyhow::Result<PtexFaces> {
    let mut header = [0u8; HEADER];
    reader.read_exact(&mut header)?;
    anyhow::ensure!(&header[0..4] == b"Ptex", "not a Ptex file");
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let half = |at: usize| u16::from_le_bytes(header[at..at + 2].try_into().unwrap());
    anyhow::ensure!(word(4) == 1, "unsupported Ptex version {}", word(4));
    let triangles = word(8) == 0;
    let datatype = word(12);
    let channels = usize::from(half(20));
    let faces = word(24) as usize;
    let (extended, face_info, constant) = (word(28), word(32), word(36));
    anyhow::ensure!(channels > 0, "Ptex file has no channels");
    let skip = u64::from(extended) + u64::from(face_info);
    std::io::copy(&mut (&mut reader).take(skip), &mut std::io::sink())?;
    let mut packed = vec![0u8; constant as usize];
    reader.read_exact(&mut packed)?;
    let data = miniz_oxide::inflate::decompress_to_vec_zlib(&packed)
        .map_err(|error| anyhow::anyhow!("Ptex constant data: {error:?}"))?;
    let size = match datatype {
        0 => 1,
        1 | 2 => 2,
        3 => 4,
        other => anyhow::bail!("unsupported Ptex data type {other}"),
    };
    anyhow::ensure!(
        data.len() >= faces * channels * size,
        "Ptex constant data is short"
    );
    let value = |at: usize| -> f32 {
        let bytes = &data[at * size..(at + 1) * size];
        match datatype {
            0 => f32::from(bytes[0]) / 255.0,
            1 => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65535.0,
            2 => half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32(),
            _ => f32::from_le_bytes(bytes.try_into().unwrap()),
        }
    };
    let colors = (0..faces)
        .map(|face| {
            let base = face * channels;
            std::array::from_fn(|channel| value(base + channel.min(channels - 1)))
        })
        .collect();
    Ok(PtexFaces { triangles, colors })
}

impl PtexFaces {
    /// The color of each mesh face, given the mesh's `faceVertexCounts`;
    /// `None` when the file was written for different topology.
    pub fn mesh_face_colors(&self, counts: &[i32]) -> Option<Vec<[f32; 3]>> {
        let mut colors = Vec::with_capacity(counts.len());
        let mut next = 0;
        for &count in counts {
            let subfaces = if self.triangles || count == 4 {
                1
            } else {
                count.max(0) as usize
            };
            let faces = self.colors.get(next..next + subfaces)?;
            let sum = faces.iter().fold([0.0; 3], |sum, color| {
                std::array::from_fn(|channel| sum[channel] + color[channel])
            });
            colors.push(sum.map(|value| value / subfaces.max(1) as f32));
            next += subfaces;
        }
        (next == self.colors.len()).then_some(colors)
    }
}

/// Where a Ptex file keeps each face's texels: the face resolutions and the
/// byte range of every resolution level.
///
/// Level 0 holds every face at full resolution; each further level halves
/// both sides and keeps only the faces still at least four texels on their
/// shorter side, ordered by that side, largest first.
#[derive(Debug, Clone)]
pub struct PtexLayout {
    pub triangles: bool,
    datatype: u32,
    channels: usize,
    /// Each face's side lengths as powers of two, and whether it is constant.
    pub faces: Vec<(u8, u8, bool)>,
    /// Each level's byte offset, byte size, header size and face count.
    levels: Vec<(u64, u64, usize, usize)>,
    /// Each face's place in the reduced levels.
    reduced: Vec<usize>,
    constant: Vec<[f32; 3]>,
}

/// One face's texels in rows of `width`, from its first corner along `u`.
#[derive(Debug, Clone, PartialEq)]
pub struct PtexTexels {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 3]>,
}

impl PtexTexels {
    /// The bilinearly filtered texel at face coordinates `u`, `v` in 0..=1.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let x = (u.clamp(0.0, 1.0) * self.width as f32 - 0.5).max(0.0);
        let y = (v.clamp(0.0, 1.0) * self.height as f32 - 0.5).max(0.0);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.texels[y * self.width + x];
        std::array::from_fn(|channel| {
            let top = at(x0, y0)[channel] * (1.0 - fx) + at(x1, y0)[channel] * fx;
            let bottom = at(x0, y1)[channel] * (1.0 - fx) + at(x1, y1)[channel] * fx;
            top * (1.0 - fy) + bottom * fy
        })
    }
}

const ENCODING_CONSTANT: u32 = 0;
const ENCODING_DIFFERENCED: u32 = 2;
const ENCODING_TILED: u32 = 3;

impl PtexLayout {
    /// How many leading bytes, given the 64-byte header, hold everything
    /// [`PtexLayout::parse`] reads.
    pub fn prefix_len(header: &[u8]) -> anyhow::Result<usize> {
        anyhow::ensure!(
            header.len() >= HEADER && &header[0..4] == b"Ptex",
            "not a Ptex file"
        );
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap()) as usize;
        Ok(HEADER + word(28) + word(32) + word(36) + word(40))
    }

    /// Reads the layout from a file's leading [`PtexLayout::prefix_len`] bytes.
    pub fn parse(head: &[u8]) -> anyhow::Result<Self> {
        let prefix = Self::prefix_len(head)?;
        anyhow::ensure!(head.len() >= prefix, "Ptex header is short");
        let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        anyhow::ensure!(word(4) == 1, "unsupported Ptex version {}", word(4));
        let channels = usize::from(u16::from_le_bytes([head[20], head[21]]));
        let levels = usize::from(u16::from_le_bytes([head[22], head[23]]));
        let count = word(24) as usize;
        let (extended, face_info, constant) =
            (word(28) as usize, word(32) as usize, word(36) as usize);
        anyhow::ensure!(channels > 0, "Ptex file has no channels");
        let info_start = HEADER + extended;
        let info = inflate(&head[info_start..info_start + face_info])?;
        anyhow::ensure!(info.len() >= count * 20, "Ptex face info is short");
        let faces: Vec<(u8, u8, bool)> = info
            .chunks_exact(20)
            .take(count)
            .map(|face| (face[0], face[1], face[3] & 1 != 0))
            .collect();
        let constant_data = read_face_colors(head)?.colors;
        let mut offset = (info_start + face_info + constant + levels * 16) as u64;
        let table = info_start + face_info + constant;
        let levels = (0..levels)
            .map(|level| {
                let at = table + level * 16;
                let size = u64::from_le_bytes(head[at..at + 8].try_into().unwrap());
                let header = word(at + 8) as usize;
                let faces = word(at + 12) as usize;
                let entry = (offset, size, header, faces);
                offset += size;
                entry
            })
            .collect();
        let mut order: Vec<usize> = (0..count).collect();
        let shorter = |face: usize| {
            let (u, v, constant) = faces[face];
            if constant { 1 } else { u.min(v) }
        };
        order.sort_by_key(|&face| std::cmp::Reverse(shorter(face)));
        let mut reduced = vec![0; count];
        for (place, &face) in order.iter().enumerate() {
            reduced[face] = place;
        }
        Ok(Self {
            triangles: word(8) == 0,
            datatype: word(12),
            channels,
            faces,
            levels,
            reduced,
            constant: constant_data,
        })
    }

    /// The coarsest level at which `face` still has at least `side` texels
    /// on its shorter side, or level 0.
    pub fn level_for(&self, face: usize, side: usize) -> usize {
        let (u, v, constant) = self.faces[face];
        if constant {
            return 0;
        }
        let shorter = usize::from(u.min(v));
        (1..self.levels.len())
            .rev()
            .find(|&level| {
                shorter >= level + 2
                    && (1usize << (shorter - level)) >= side
                    && self.reduced[face] < self.levels[level].3
            })
            .unwrap_or(0)
    }

    /// The average color of `face`.
    pub fn average(&self, face: usize) -> [f32; 3] {
        self.constant[face]
    }

    /// The byte range of `level` within the file.
    pub fn level_range(&self, level: usize) -> std::ops::Range<u64> {
        let (offset, size, ..) = self.levels[level];
        offset..offset + size
    }

    /// Decodes every face stored at `level` from that level's bytes.
    pub fn decode_level(&self, level: usize, bytes: &[u8]) -> anyhow::Result<PtexLevel> {
        let (_, _, header, count) = self.levels[level];
        let headers = inflate(
            bytes
                .get(..header)
                .ok_or_else(|| anyhow::anyhow!("short level"))?,
        )?;
        anyhow::ensure!(headers.len() >= count * 4, "Ptex level header is short");
        let ids: Vec<usize> = if level == 0 {
            (0..count).collect()
        } else {
            let mut order: Vec<usize> = (0..self.faces.len()).collect();
            order.sort_by_key(|&face| self.reduced[face]);
            order.truncate(count);
            order
        };
        let mut at = header;
        let mut faces = vec![None; self.faces.len()];
        for (slot, &face) in ids.iter().enumerate() {
            let entry = u32::from_le_bytes(headers[slot * 4..slot * 4 + 4].try_into().unwrap());
            let (size, encoding) = ((entry & 0x3fff_ffff) as usize, entry >> 30);
            let block = bytes
                .get(at..at + size)
                .ok_or_else(|| anyhow::anyhow!("Ptex face block is short"))?;
            at += size;
            let (u, v, constant) = self.faces[face];
            let (width, height) = if constant {
                (1, 1)
            } else {
                (1usize << (u - level as u8), 1usize << (v - level as u8))
            };
            faces[face] = Some(if constant {
                PtexTexels {
                    width,
                    height,
                    texels: vec![self.constant[face]],
                }
            } else {
                self.decode_face(block, encoding, width, height)?
            });
        }
        Ok(PtexLevel { faces })
    }

    fn texel_size(&self) -> usize {
        match self.datatype {
            0 => 1,
            1 | 2 => 2,
            _ => 4,
        }
    }

    fn decode_face(
        &self,
        block: &[u8],
        encoding: u32,
        width: usize,
        height: usize,
    ) -> anyhow::Result<PtexTexels> {
        if encoding != ENCODING_TILED {
            return self.decode_block(block, encoding, width, height);
        }
        anyhow::ensure!(block.len() >= 6, "Ptex tiled face is short");
        let (tile_width, tile_height) = (1usize << block[0], 1usize << block[1]);
        let header = u32::from_le_bytes(block[2..6].try_into().unwrap()) as usize;
        let headers = inflate(&block[6..6 + header])?;
        let (across, down) = (width.div_ceil(tile_width), height.div_ceil(tile_height));
        anyhow::ensure!(
            headers.len() >= across * down * 4,
            "Ptex tile header is short"
        );
        let mut texels = vec![[0.0; 3]; width * height];
        let mut at = 6 + header;
        for tile in 0..across * down {
            let entry = u32::from_le_bytes(headers[tile * 4..tile * 4 + 4].try_into().unwrap());
            let (size, encoding) = ((entry & 0x3fff_ffff) as usize, entry >> 30);
            let data = block
                .get(at..at + size)
                .ok_or_else(|| anyhow::anyhow!("Ptex tile is short"))?;
            at += size;
            let decoded = self.decode_block(data, encoding, tile_width, tile_height)?;
            let (left, top) = ((tile % across) * tile_width, (tile / across) * tile_height);
            for y in 0..tile_height.min(height - top) {
                for x in 0..tile_width.min(width - left) {
                    texels[(top + y) * width + left + x] = decoded.texels[y * tile_width + x];
                }
            }
        }
        Ok(PtexTexels {
            width,
            height,
            texels,
        })
    }

    /// A constant, zipped or differenced-and-zipped block; zipped texels are
    /// stored one channel plane after another.
    fn decode_block(
        &self,
        block: &[u8],
        encoding: u32,
        width: usize,
        height: usize,
    ) -> anyhow::Result<PtexTexels> {
        let size = self.texel_size();
        let count = width * height;
        let value = |bytes: &[u8]| -> f32 {
            match self.datatype {
                0 => f32::from(bytes[0]) / 255.0,
                1 => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65535.0,
                2 => half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32(),
                _ => f32::from_le_bytes(bytes.try_into().unwrap()),
            }
        };
        let texel = |read: &dyn Fn(usize) -> f32| -> [f32; 3] {
            std::array::from_fn(|channel| read(channel.min(self.channels - 1)))
        };
        if encoding == ENCODING_CONSTANT {
            anyhow::ensure!(
                block.len() >= self.channels * size,
                "Ptex constant block is short"
            );
            let color = texel(&|channel| value(&block[channel * size..(channel + 1) * size]));
            return Ok(PtexTexels {
                width,
                height,
                texels: vec![color; count],
            });
        }
        let mut data = inflate(block)?;
        anyhow::ensure!(
            data.len() >= count * self.channels * size,
            "Ptex block is short"
        );
        if encoding == ENCODING_DIFFERENCED {
            match self.datatype {
                0 => {
                    for at in 1..data.len() {
                        data[at] = data[at].wrapping_add(data[at - 1]);
                    }
                }
                1 => {
                    let mut previous = 0u16;
                    for pair in data.chunks_exact_mut(2) {
                        previous = previous.wrapping_add(u16::from_le_bytes([pair[0], pair[1]]));
                        pair.copy_from_slice(&previous.to_le_bytes());
                    }
                }
                _ => {}
            }
        }
        let plane = count * size;
        let texels = (0..count)
            .map(|at| texel(&|channel| value(&data[channel * plane + at * size..][..size])))
            .collect();
        Ok(PtexTexels {
            width,
            height,
            texels,
        })
    }
}

/// The texels of every face stored at one Ptex level, by face index.
#[derive(Debug, Clone, Default)]
pub struct PtexLevel {
    pub faces: Vec<Option<PtexTexels>>,
}

fn inflate(packed: &[u8]) -> anyhow::Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib(packed)
        .map_err(|error| anyhow::anyhow!("Ptex zip block: {error:?}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn ptex(datatype: u32, channels: u16, values: &[u8], triangles: bool) -> Vec<u8> {
        let faces = values.len() / (usize::from(channels) * [1, 2, 2, 4][datatype as usize]);
        let face_info = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0; faces * 20], 6);
        let constant = miniz_oxide::deflate::compress_to_vec_zlib(values, 6);
        let mut bytes = b"Ptex".to_vec();
        for word in [1, u32::from(!triangles), datatype, u32::MAX] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(channels.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        for word in [
            faces as u32,
            8,
            face_info.len() as u32,
            constant.len() as u32,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.resize(HEADER + 8, 0);
        bytes.extend(face_info);
        bytes.extend(constant);
        bytes.extend([0xAB; 32]);
        bytes
    }

    /// A quad Ptex file whose faces each hold two by two RGB texels, in rows
    /// along `u`.
    pub(crate) fn ptex_texels(faces: &[[[u8; 3]; 4]]) -> Vec<u8> {
        let zip = |data: &[u8]| miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        let info: Vec<u8> = faces
            .iter()
            .flat_map(|_| [1u8, 1, 0, 0].into_iter().chain([0xFF; 16]))
            .collect();
        let averages: Vec<u8> = faces
            .iter()
            .flat_map(|texels| {
                (0..3).map(move |channel| {
                    (texels
                        .iter()
                        .map(|texel| u32::from(texel[channel]))
                        .sum::<u32>()
                        / 4) as u8
                })
            })
            .collect();
        let (mut headers, mut blocks) = (Vec::new(), Vec::new());
        for texels in faces {
            let planar: Vec<u8> = (0..3)
                .flat_map(|channel| texels.iter().map(move |texel| texel[channel]))
                .collect();
            let block = zip(&planar);
            headers.extend((block.len() as u32 | 1 << 30).to_le_bytes());
            blocks.extend(block);
        }
        let (face_info, constant, level_header) = (zip(&info), zip(&averages), zip(&headers));
        let mut bytes = b"Ptex".to_vec();
        for word in [1, 1, 0, u32::MAX] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(3u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        for word in [
            faces.len() as u32,
            0,
            face_info.len() as u32,
            constant.len() as u32,
            16,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.resize(HEADER, 0);
        bytes.extend(face_info);
        bytes.extend(constant);
        bytes.extend(((level_header.len() + blocks.len()) as u64).to_le_bytes());
        bytes.extend((level_header.len() as u32).to_le_bytes());
        bytes.extend((faces.len() as u32).to_le_bytes());
        bytes.extend(level_header);
        bytes.extend(blocks);
        bytes
    }

    #[test]
    fn full_resolution_texels_decode_in_rows() {
        let bytes = ptex_texels(&[[[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]]);
        let layout = PtexLayout::parse(&bytes).unwrap();
        let range = layout.level_range(0);
        let level = layout
            .decode_level(0, &bytes[range.start as usize..range.end as usize])
            .unwrap();
        let texels = level.faces[0].as_ref().unwrap();
        assert_eq!((texels.width, texels.height), (2, 2));
        assert_eq!(texels.sample(0.0, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(texels.sample(1.0, 0.0), [0.0, 1.0, 0.0]);
        assert_eq!(texels.sample(0.0, 1.0), [0.0, 0.0, 1.0]);
        assert_eq!(
            layout.average(0),
            [127.0 / 255.0, 127.0 / 255.0, 127.0 / 255.0]
        );
    }

    #[test]
    fn face_averages_come_from_the_constant_block() {
        let bytes = ptex(0, 3, &[255, 0, 0, 0, 255, 0, 0, 0, 255], false);
        let faces = read_face_colors(bytes.as_slice()).unwrap();
        assert!(!faces.triangles);
        assert_eq!(
            faces.colors,
            vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        // A quad, then a triangle split into three subfaces in a quad file.
        let faces = PtexFaces {
            triangles: false,
            colors: vec![[1.0; 3], [0.0; 3], [0.3; 3], [0.6; 3]],
        };
        let colors = faces.mesh_face_colors(&[4, 3]).unwrap();
        assert_eq!(colors[0], [1.0; 3]);
        assert!((colors[1][0] - 0.3).abs() < 1e-6);
        assert!(faces.mesh_face_colors(&[4, 4]).is_none());
    }

    #[test]
    fn wide_data_types_and_single_channels_decode() {
        let half = half::f16::from_f32(0.25).to_le_bytes();
        let faces = read_face_colors(ptex(2, 1, &half, true).as_slice()).unwrap();
        assert!(faces.triangles);
        assert_eq!(faces.colors, vec![[0.25; 3]]);
        assert!(read_face_colors(&b"PNG!"[..]).is_err());
    }
}

#[cfg(test)]
mod dataset_checks {
    use super::*;

    #[test]
    #[ignore]
    fn reduced_levels_average_to_the_face_constants() {
        for (path, level) in [
            (
                "/pkg/datasets/moana/island/textures/isBeach/Color/beach_geo.ptx",
                3,
            ),
            (
                "/pkg/datasets/moana/island/textures/islandsun_cloudmap.ptx",
                3,
            ),
            (
                "/pkg/datasets/moana/island/textures/islandsun_cloudmap.ptx",
                0,
            ),
        ] {
            let bytes = std::fs::read(path).unwrap();
            let layout = PtexLayout::parse(&bytes).unwrap();
            let range = layout.level_range(level);
            let decoded = layout
                .decode_level(level, &bytes[range.start as usize..range.end as usize])
                .unwrap();
            let averages = read_face_colors(bytes.as_slice()).unwrap().colors;
            let (mut checked, mut worst) = (0, 0.0f32);
            for (face, texels) in decoded.faces.iter().enumerate() {
                let Some(texels) = texels else { continue };
                let mean = texels.texels.iter().fold([0.0f32; 3], |sum, texel| {
                    std::array::from_fn(|channel| sum[channel] + texel[channel])
                });
                let error = (0..3)
                    .map(|channel| {
                        (mean[channel] / texels.texels.len() as f32 - averages[face][channel]).abs()
                    })
                    .fold(0.0, f32::max);
                worst = worst.max(error);
                checked += 1;
            }
            println!("{path} level {level}: {checked} faces, worst mean error {worst}");
            assert!(checked > 0 && worst < 0.05);
        }
    }
}
