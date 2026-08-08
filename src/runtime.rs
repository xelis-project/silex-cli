use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::from_str;
use xelis_bytecode::Access;
use xelis_common::{
    contract::{ContractMetadata, ContractModule, ContractVersion, ModuleMetadata},
    crypto::Hash,
};
use xelis_environment::Environment;
use xelis_types::{Primitive, ValueCell};
use xelis_vm::{ModuleValidator, VM};

use crate::{
    cli::RunConfig,
    contract::{compile_source, environment, read_file, read_module},
};

pub(crate) fn run(config: RunConfig) -> Result<()> {
    let (module, environment) = load_module(&config.input)?;
    let validator = ModuleValidator::new(&module.module, &environment);
    validator.verify().context("module failed validation")?;

    let entry = config
        .entry
        .or_else(|| {
            module
                .module
                .chunks()
                .iter()
                .position(|chunk| matches!(chunk.access, Access::Entry { .. }))
                .and_then(|id| u16::try_from(id).ok())
        })
        .context("program does not define an entry chunk")?;
    if !module.module.is_entry_chunk(entry as usize) {
        bail!("chunk {entry} is not an entry chunk");
    }

    let arguments = config
        .arguments
        .iter()
        .map(|argument| parse_argument(argument))
        .collect::<Result<Vec<_>>>()?;
    validator
        .verify_invoke_chunk(entry as usize, arguments.iter())
        .with_context(|| format!("invalid arguments for entry chunk {entry}: {arguments:?}"))?;

    let mut vm = VM::<ContractMetadata>::default();
    let metadata = runtime_metadata(module.version);
    vm.append_module(ModuleMetadata {
        module: module.module.as_ref().into(),
        metadata: (&metadata).into(),
        environment: (&environment).into(),
    })?;
    if let Some(gas_limit) = config.gas_limit {
        vm.context_mut().set_gas_limit(gas_limit);
    }
    vm.invoke_chunk_with_args(entry, arguments.into_iter())?;
    println!("{}", vm.run_blocking()?);

    Ok(())
}

fn load_module(path: &Path) -> Result<(ContractModule, Environment<ContractMetadata>)> {
    if path.extension().is_some_and(|extension| extension == "slx") {
        return compile_source(&read_file(path)?);
    }

    let module = read_module(path)?;
    let environment = environment(module.version);
    Ok((module, environment))
}

fn runtime_metadata(version: ContractVersion) -> ContractMetadata {
    ContractMetadata {
        contract_executor: Hash::zero(),
        contract_caller: None,
        contract_version: version,
        deposits: Default::default(),
    }
}

fn parse_argument(argument: &str) -> Result<ValueCell> {
    if let Ok(value) = from_str(argument) {
        return Ok(value);
    }

    let primitive = match argument {
        "null" => Primitive::Null,
        "true" => Primitive::Boolean(true),
        "false" => Primitive::Boolean(false),
        _ => match argument.parse() {
            Ok(value) => Primitive::U64(value),
            Err(_) => Primitive::String(argument.to_owned()),
        },
    };

    Ok(primitive.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::contract::compile_source;

    #[test]
    fn parses_primitive_arguments() {
        assert_eq!(parse_argument("null").unwrap(), Primitive::Null.into());
        assert_eq!(
            parse_argument("true").unwrap(),
            Primitive::Boolean(true).into()
        );
        assert_eq!(parse_argument("42").unwrap(), Primitive::U64(42).into());
        assert_eq!(
            parse_argument("hello").unwrap(),
            Primitive::String("hello".to_owned()).into()
        );
    }

    #[test]
    fn parses_json_value_cell_arguments() {
        let value =
            parse_argument(r#"{"type":"object","value":[]}"#).expect("JSON ValueCell should parse");

        assert_eq!(
            value,
            serde_json::from_str(r#"{"type":"object","value":[]}"#).unwrap()
        );
    }

    #[test]
    fn runtime_metadata_uses_requested_contract_version() {
        let metadata = runtime_metadata(ContractVersion::V1);

        assert_eq!(metadata.contract_version, ContractVersion::V1);
        assert_eq!(metadata.contract_executor, Hash::zero());
        assert!(metadata.contract_caller.is_none());
        assert!(metadata.deposits.is_empty());
    }

    #[test]
    fn validator_rejects_extra_entry_arguments() {
        let (module, environment) =
            compile_source("entry main(value: u64) -> u64 { return value; }")
                .expect("source should compile");
        let arguments = [Primitive::U64(1).into(), Primitive::U64(2).into()];
        let validator = ModuleValidator::new(&module.module, &environment);
        let error = validator
            .verify_invoke_chunk(0, arguments.iter())
            .expect_err("extra arguments should fail");

        assert!(error.to_string().contains("expected 1, got 2"));
    }
}
