use std::{fs, path::Path, sync::Arc};

use anyhow::{Context, Result};
use xelis_common::{
    contract::{ContractMetadata, ContractModule, ContractVersion, build_environment},
    serializer::Serializer,
    transaction::mock::MockStorageProvider,
};
use xelis_compiler::Compiler;
use xelis_environment::Environment;
use xelis_lexer::Lexer;
use xelis_parser::Parser;

use crate::cli::OutputFormat;

pub(crate) fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub(crate) fn write_module(
    module: &ContractModule,
    path: &Path,
    format: OutputFormat,
) -> Result<()> {
    let bytes = match format {
        OutputFormat::Binary => module.to_bytes(),
        OutputFormat::Hex => module.to_hex().into_bytes(),
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(module)
                .context("failed to serialize contract module as JSON")?;
            format!("{json}\n").into_bytes()
        }
    };

    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn read_module(path: &Path) -> Result<ContractModule> {
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        let source = read_file(path)?;
        return serde_json::from_str(&source)
            .with_context(|| format!("failed to parse JSON contract module {}", path.display()));
    }

    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    ContractModule::from_bytes(&bytes)
        .with_context(|| format!("failed to parse binary bytecode module {}", path.display()))
}

pub(crate) fn compile_source(
    source: &str,
) -> Result<(ContractModule, Environment<ContractMetadata>)> {
    let environment = build_environment::<MockStorageProvider>(ContractVersion::V1);
    let tokens = Lexer::new(source)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| anyhow::anyhow!("failed to lex Silex source: {error}"))?;
    let (program, _) = Parser::with(tokens.into_iter(), &environment)
        .parse()
        .map_err(|error| anyhow::anyhow!("failed to parse Silex source: {error}"))?;
    let module = Compiler::new(&program, environment.environment())
        .with_enforce_public_parameters(true)
        .compile()
        .context("failed to compile Silex source")?;
    let module = ContractModule {
        version: ContractVersion::V1,
        module: Arc::new(module),
    };

    Ok((module, environment.build()))
}

pub(crate) fn environment(version: ContractVersion) -> Environment<ContractMetadata> {
    build_environment::<MockStorageProvider>(version).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn main() -> u64 { return 10; }";

    #[test]
    fn source_compiles_to_v1_contract_module() {
        let (module, _) = compile_source(SOURCE).expect("source should compile");

        assert_eq!(module.version, ContractVersion::V1);
        assert!(!module.module.chunks().is_empty());
    }

    #[test]
    fn contract_module_round_trips_as_binary() {
        let (module, _) = compile_source(SOURCE).expect("source should compile");
        let encoded = module.to_bytes();
        let decoded = ContractModule::from_bytes(&encoded).expect("binary should decode");

        assert_eq!(decoded.version, module.version);
        assert_eq!(decoded.module.chunks().len(), module.module.chunks().len());
        assert_eq!(
            decoded.module.constants().len(),
            module.module.constants().len()
        );
    }

    #[test]
    fn contract_module_round_trips_as_json() {
        let (module, _) = compile_source(SOURCE).expect("source should compile");
        let encoded = serde_json::to_string(&module).expect("JSON should encode");
        let decoded: ContractModule = serde_json::from_str(&encoded).expect("JSON should decode");

        assert_eq!(decoded.version, ContractVersion::V1);
        assert_eq!(decoded.module.chunks().len(), module.module.chunks().len());
    }
}
