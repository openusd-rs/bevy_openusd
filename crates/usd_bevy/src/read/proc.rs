//! `UsdProc` — procedural prim metadata, read from the stage via openusd.

use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::read_token_or_string;

#[derive(Debug, Clone, Default)]
pub struct ReadProcedural {
    pub procedural_type: Option<String>,
    pub procedural_system: Option<String>,
}

pub fn read_procedural(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadProcedural>> {
    let procedural_type = read_token_or_string(stage, prim, "info:procedural:type")?;
    let procedural_system = read_token_or_string(stage, prim, "proceduralSystem")?;
    if procedural_type.is_none() && procedural_system.is_none() {
        return Ok(None);
    }
    Ok(Some(ReadProcedural { procedural_type, procedural_system }))
}
