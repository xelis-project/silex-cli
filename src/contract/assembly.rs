use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use silex_assembler::{Assembler, Disassembler};
use silex_bytecode::Access;
use silex_types::ValueCell;
use xelis_common::contract::{ContractModule, ContractVersion};

// The upstream instruction dump omits constants and parameter types. Keep them
// in a comment so instruction edits remain authoritative and the dump is lossless.
const METADATA_PREFIX: &str = "// @silex-module ";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssemblyMetadata {
    version: ContractVersion,
    constants: Vec<ValueCell>,
    access: Vec<Access>,
}

/// Assemble instructions, restoring metadata when produced by `disassemble`.
pub fn assemble(source: &str) -> Result<ContractModule> {
    let mut metadata = None;
    for line in source.lines() {
        if let Some(json) = line.trim().strip_prefix(METADATA_PREFIX) {
            ensure!(metadata.is_none(), "duplicate assembly metadata");
            metadata = Some(
                serde_json::from_str::<AssemblyMetadata>(json)
                    .context("invalid assembly metadata")?,
            );
        }
    }
    let mut module = Assembler::new(source)
        .assemble()
        .map_err(|error| anyhow::anyhow!("failed to assemble source: {error}"))?;
    let version = if let Some(metadata) = metadata {
        ensure!(
            metadata.access.len() == module.chunks().len(),
            "assembly metadata chunk count does not match instructions"
        );
        for (index, access) in metadata.access.into_iter().enumerate() {
            let chunk = module
                .get_chunk_at_mut(index)
                .context("missing assembly chunk")?;
            ensure!(
                chunk.access.to_string() == access.to_string(),
                "assembly metadata access does not match chunk {index}"
            );
            chunk.access = access;
        }
        for (index, value) in metadata.constants.into_iter().enumerate() {
            ensure!(
                module.add_constant(value) == index,
                "duplicate assembly constant"
            );
        }
        metadata.version
    } else {
        ContractVersion::V1
    };
    Ok(ContractModule {
        version,
        module: Arc::new(module),
    })
}

/// Render instructions and the constants, access types, and version they need.
pub fn disassemble(module: &ContractModule) -> Result<String> {
    let instructions = Disassembler::new(&module.module)
        .disasemble()
        .context("failed to disassemble bytecode module")?
        .to_string();
    let metadata = AssemblyMetadata {
        version: module.version,
        constants: module.module.constants().iter().cloned().collect(),
        access: module
            .module
            .chunks()
            .iter()
            .map(|chunk| chunk.access.clone())
            .collect(),
    };
    Ok(format!(
        "{METADATA_PREFIX}{}\n{instructions}",
        serde_json::to_string(&metadata)?
    ))
}

#[cfg(test)]
mod tests {
    use super::{assemble, disassemble};
    use crate::{JsonStorage, Primitive, compile_source, run_program};
    use xelis_common::serializer::Serializer;

    #[test]
    fn assembly_round_trip_preserves_metadata_and_execution() {
        let (module, _) = compile_source(
            "hook constructor() -> u64 { return 0; }\nfn double(n: u64) -> u64 { return n * 2u64; }\nentry main(n: u64) -> u64 { return double(n); }"
        ).unwrap();
        let assembly = disassemble(&module).unwrap();
        let restored = assemble(&assembly).unwrap();
        assert_eq!(restored.to_bytes(), module.to_bytes());
        let result =
            run_program::<JsonStorage>(&restored, None, vec![Primitive::U64(21).into()], None)
                .unwrap();
        assert_eq!(result.as_u64().unwrap(), 42);
        let edited = assemble(&assembly.replace("MUL", "ADD")).unwrap();
        let result =
            run_program::<JsonStorage>(&edited, None, vec![Primitive::U64(21).into()], None)
                .unwrap();
        assert_eq!(
            result.as_u64().unwrap(),
            23,
            "Assembly edits must change execution"
        );
        assert!(
            run_program::<JsonStorage>(
                &restored,
                None,
                vec![Primitive::String("21".into()).into()],
                None
            )
            .is_err()
        );
        assert!(assemble(&format!("{assembly}\n{}", assembly.lines().next().unwrap())).is_err());
        assert!(assemble(&assembly.replace(" entry\n", " internal\n")).is_err());
    }
}
