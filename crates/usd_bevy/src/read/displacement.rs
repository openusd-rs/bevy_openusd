//! RenderMan displacement behind a material's `outputs:ri:displacement`.
//!
//! Reads the scalar chains Ptex-painted terrain uses: `PxrDisplace` scales a
//! value that `PxrDispTransform` remaps from Ptex files a `PxrBlend`
//! multiplies together.

use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use super::util::{connections_at, read_token_or_string};

/// How far a surface moves along its normal: `amount` times the remapped
/// product of the Ptex files' first channels, in the mesh's units.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadDisplacement {
    pub amount: f32,
    pub textures: Vec<String>,
    pub remap: Remap,
}

/// How a texture value in 0..=1 becomes a signed displacement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Remap {
    /// Used as it is.
    None,
    /// Shifted so `center` stays in place.
    Centered { center: f32 },
    /// Below `center` reaches down to `-depth`, above it up to `height`.
    DepthHeight {
        center: f32,
        depth: f32,
        height: f32,
    },
}

impl Remap {
    pub fn apply(self, value: f32) -> f32 {
        match self {
            Self::None => value,
            Self::Centered { center } => value - center,
            Self::DepthHeight {
                center,
                depth,
                height,
            } => {
                if value < center {
                    -depth * (center - value) / center.max(f32::EPSILON)
                } else {
                    height * (value - center) / (1.0 - center).max(f32::EPSILON)
                }
            }
        }
    }
}

/// The displacement `material` applies, when its RenderMan displacement is
/// a chain this reader follows.
pub fn read_displacement(
    stage: &Stage,
    material: &Path,
) -> anyhow::Result<Option<ReadDisplacement>> {
    let output = material.append_property("outputs:ri:displacement")?;
    let Some(displace) = connections_at(stage, &output)?
        .into_iter()
        .next()
        .map(|target| target.prim_path())
    else {
        return Ok(None);
    };
    if read_token_or_string(stage, &displace, "info:id")?.as_deref() != Some("PxrDisplace") {
        return Ok(None);
    }
    let amount = input_value(stage, &displace, "dispAmount")?.unwrap_or(1.0);
    let mut remap = Remap::None;
    let mut textures = Vec::new();
    let mut pending = vec![displace.append_property("inputs:dispScalar")?];
    while let Some(input) = pending.pop() {
        let Some(node) = connections_at(stage, &input)?
            .into_iter()
            .next()
            .map(|target| target.prim_path())
        else {
            return Ok(None);
        };
        let value = |name: &str, default: f32| -> anyhow::Result<f32> {
            Ok(input_value(stage, &node, name)?.unwrap_or(default))
        };
        match read_token_or_string(stage, &node, "info:id")?.as_deref() {
            Some("PxrDispTransform") if remap == Remap::None => {
                remap = match input_value(stage, &node, "dispRemapMode")?.unwrap_or(0.0) as i32 {
                    0 => Remap::None,
                    1 => Remap::Centered {
                        center: value("dispCenter", 0.5)?,
                    },
                    2 => Remap::DepthHeight {
                        center: value("dispCenter", 0.5)?,
                        depth: value("dispDepth", 1.0)?,
                        height: value("dispHeight", 1.0)?,
                    },
                    _ => return Ok(None),
                };
                pending.push(node.append_property("inputs:dispScalar")?);
            }
            // Operation 18 multiplies the top layer by the bottom one.
            Some("PxrBlend") if input_value(stage, &node, "operation")? == Some(18.0) => {
                pending.push(node.append_property("inputs:topRGB")?);
                pending.push(node.append_property("inputs:bottomRGB")?);
            }
            Some("PxrPtexture") => match super::shade::read_texture_file(stage, &node, None)? {
                Some(file) => textures.push(file),
                None => return Ok(None),
            },
            _ => return Ok(None),
        }
    }
    Ok((!textures.is_empty()).then_some(ReadDisplacement {
        amount,
        textures,
        remap,
    }))
}

/// The number an input of `node` holds, read through any connection to its
/// material's interface.
fn input_value(stage: &Stage, node: &Path, name: &str) -> anyhow::Result<Option<f32>> {
    let Some(attribute) = super::shade::texture_input_attribute(stage, node, name)? else {
        return Ok(None);
    };
    Ok(match attribute.get_at::<Value>(None)? {
        Some(Value::Float(value)) => Some(value),
        Some(Value::Double(value)) => Some(value as f32),
        Some(Value::Int(value)) => Some(value as f32),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_ptex_displacement_reads_as_one_remapped_product() {
        let stage = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0
def Material "Soil" {
    token outputs:ri:displacement.connect = </Soil/Displace.outputs:displace>
    def Shader "Displace" {
        uniform token info:id = "PxrDisplace"
        float inputs:dispAmount = 5
        float inputs:dispScalar.connect = </Soil/Transform.outputs:resultF>
        token outputs:displace
    }
    def Shader "Transform" {
        uniform token info:id = "PxrDispTransform"
        float inputs:dispCenter = 0.5
        float inputs:dispDepth = 0.35
        float inputs:dispHeight = 0.35
        int inputs:dispRemapMode = 2
        float inputs:dispScalar.connect = </Soil/Mask.outputs:resultR>
        float outputs:resultF
    }
    def Shader "Mask" {
        uniform token info:id = "PxrBlend"
        int inputs:operation = 18
        color3f inputs:topRGB.connect = </Soil/Detail.outputs:resultRGB>
        color3f inputs:bottomRGB.connect = </Soil/Side.outputs:resultRGB>
        float outputs:resultR
    }
    def Shader "Detail" {
        uniform token info:id = "PxrPtexture"
        asset inputs:filename = @detail.ptx@
        color3f outputs:resultRGB
    }
    def Shader "Side" {
        uniform token info:id = "PxrPtexture"
        asset inputs:filename = @side.ptx@
        color3f outputs:resultRGB
    }
}
def Material "Plain" {
}
def Material "Interface" {
    float inputs:dispScale = 6.25
    float inputs:dispOffset = 0.25
    int inputs:dispRemapMode = 2
    asset inputs:displacementMap = @height.ptx@
    token outputs:ri:displacement.connect = </Interface/Displace.outputs:displace>
    def Shader "Displace" {
        uniform token info:id = "PxrDisplace"
        float inputs:dispAmount.connect = </Interface.inputs:dispScale>
        float inputs:dispScalar = 0
        float inputs:dispScalar.connect = </Interface/Transform.outputs:resultF>
        token outputs:displace
    }
    def Shader "Transform" {
        uniform token info:id = "PxrDispTransform"
        float inputs:dispCenter.connect = </Interface.inputs:dispOffset>
        int inputs:dispRemapMode = 0
        int inputs:dispRemapMode.connect = </Interface.inputs:dispRemapMode>
        float inputs:dispScalar.connect = </Interface/Height.outputs:resultR>
        float outputs:resultF
    }
    def Shader "Height" {
        uniform token info:id = "PxrPtexture"
        asset inputs:filename.connect = </Interface.inputs:displacementMap>
        float outputs:resultR
    }
}
"#,
        )
        .open_stage()
        .unwrap();
        let soil = read_displacement(&stage, &openusd::sdf::path("/Soil").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(soil.amount, 5.0);
        assert_eq!(soil.textures.len(), 2);
        assert!(
            soil.textures
                .iter()
                .any(|file| file.ends_with("detail.ptx"))
        );
        assert!(soil.textures.iter().any(|file| file.ends_with("side.ptx")));
        assert!((soil.remap.apply(0.0) + 0.35).abs() < 1e-6);
        assert_eq!(soil.remap.apply(0.5), 0.0);
        assert!((soil.remap.apply(1.0) - 0.35).abs() < 1e-6);
        let plain = read_displacement(&stage, &openusd::sdf::path("/Plain").unwrap()).unwrap();
        assert!(plain.is_none());
        let interface = read_displacement(&stage, &openusd::sdf::path("/Interface").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(interface.amount, 6.25);
        assert!(interface.textures[0].ends_with("height.ptx"));
        assert_eq!(
            interface.remap,
            Remap::DepthHeight {
                center: 0.25,
                depth: 1.0,
                height: 1.0
            }
        );
    }
}
