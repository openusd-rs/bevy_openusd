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
