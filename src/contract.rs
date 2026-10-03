use std::sync::Arc;

use anyhow::{Context, Result};
use silex_compiler::Compiler;
use silex_decompiler::Decompiler;
use silex_environment::Environment;
use silex_lexer::Lexer;
use silex_parser::Parser;
use xelis_common::contract::{
    ContractMetadata, ContractModule, ContractProvider, ContractVersion, build_environment,
};

use crate::JsonStorage;

/// Compile source with the bundled JSON provider and V1 contract environment.
pub fn compile_source(source: &str) -> Result<(ContractModule, Environment<ContractMetadata>)> {
    compile_source_with_storage::<JsonStorage>(source)
}

/// Compile source with native functions bound to the application's storage provider.
pub fn compile_source_with_storage<Storage: for<'ty> ContractProvider<'ty>>(
    source: &str,
) -> Result<(ContractModule, Environment<ContractMetadata>)> {
    let environment = build_environment::<Storage>(ContractVersion::V1);
    let tokens = Lexer::new(source)
        .collect::<Result<Vec<_>, _>>()
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

/// Build the JSON provider environment for the requested contract version.
pub fn environment(version: ContractVersion) -> Environment<ContractMetadata> {
    environment_for::<JsonStorage>(version)
}

/// Build versioned native functions for the specified provider type.
pub fn environment_for<Storage: for<'ty> ContractProvider<'ty>>(
    version: ContractVersion,
) -> Environment<ContractMetadata> {
    build_environment::<Storage>(version).build()
}

mod assembly;
pub use assembly::{assemble, disassemble};

/// Recover Silex source using the module's versioned native function definitions.
pub fn decompile(module: &ContractModule) -> Result<String> {
    Decompiler::new(
        &module.module,
        &build_environment::<JsonStorage>(module.version),
    )
    .decompile()
    .context("failed to decompile bytecode module")
}

/// Generate a JSON ABI using the same native definitions as contract compilation.
pub fn generate_abi(source: &str) -> Result<String> {
    silex_abi::abi_from_silex(
        source,
        build_environment::<JsonStorage>(ContractVersion::V1),
    )
    .context("failed to generate ABI")
}

#[cfg(test)]
mod tests {
    use xelis_common::{
        contract::{ContractModule, ContractVersion},
        serializer::Serializer,
    };

    use super::compile_source;

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

        assert_eq!(decoded.to_bytes(), module.to_bytes());
    }

    #[test]
    fn contract_module_round_trips_as_json() {
        let (module, _) = compile_source(SOURCE).expect("source should compile");
        let encoded = serde_json::to_string(&module).expect("JSON should encode");
        let decoded: ContractModule = serde_json::from_str(&encoded).expect("JSON should decode");

        assert_eq!(decoded.to_bytes(), module.to_bytes());
    }
}
