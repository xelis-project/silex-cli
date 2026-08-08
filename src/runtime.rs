use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::from_str;
use xelis_bytecode::Access;
use xelis_common::{
    contract::{ContractMetadata, ContractModule, ContractVersion, ModuleMetadata},
    crypto::Hash,
};
use xelis_environment::Environment;
use xelis_types::{Primitive, TypePacked, ValueCell};
use xelis_vm::{ModuleValidator, VM};

use crate::{
    cli::RunConfig,
    contract::{compile_source, environment, read_file, read_module},
};

pub(crate) fn run(config: RunConfig) -> Result<()> {
    let (module, environment) = load_module(&config.input)?;
    ModuleValidator::new(&module.module, &environment)
        .verify()
        .context("module failed validation")?;

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
    validate_arguments(&module, entry, &arguments)?;

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

fn validate_arguments(module: &ContractModule, entry: u16, arguments: &[ValueCell]) -> Result<()> {
    let chunk = &module.module.chunks()[entry as usize];
    let expected = chunk.access.parameters().map_or(&[][..], Vec::as_slice);

    if expected.len() != arguments.len() {
        bail!(
            "entry chunk {entry} expects {} argument(s) [{}], got {} [{}]",
            expected.len(),
            format_expected_types(expected),
            arguments.len(),
            format_received_types(arguments),
        );
    }

    for (index, (expected, argument)) in expected.iter().zip(arguments).enumerate() {
        if !expected.check(argument) {
            bail!(
                "argument {index} for entry chunk {entry} has the wrong type: expected {expected:?}, got {}",
                value_type(argument),
            );
        }
    }

    Ok(())
}

fn format_expected_types(types: &[TypePacked]) -> String {
    types
        .iter()
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_received_types(values: &[ValueCell]) -> String {
    values.iter().map(value_type).collect::<Vec<_>>().join(", ")
}

fn value_type(value: &ValueCell) -> String {
    match value {
        ValueCell::Primitive(primitive) => TypePacked::from_primitive(primitive)
            .map(|value| format!("{value:?}"))
            .unwrap_or_else(|| "primitive".to_owned()),
        ValueCell::Bytes(_) => "Bytes".to_owned(),
        ValueCell::Object(_) => "Object".to_owned(),
        ValueCell::Map(_) => "Map".to_owned(),
    }
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
    fn validates_entry_arguments() {
        let (module, _) = compile_source("entry main(value: u64) -> u64 { return value; }")
            .expect("source should compile");
        let arguments = vec![Primitive::U64(7).into()];

        validate_arguments(&module, 0, &arguments).expect("argument should match");
    }

    #[test]
    fn reports_argument_count_and_types() {
        let (module, _) = compile_source("entry main(value: u64) -> u64 { return value; }")
            .expect("source should compile");
        let error = validate_arguments(&module, 0, &[]).expect_err("argument should be required");

        assert!(error.to_string().contains("expects 1 argument(s)"));
        assert!(error.to_string().contains("got 0"));

        let arguments = vec![Primitive::String("wrong".to_owned()).into()];
        let error = validate_arguments(&module, 0, &arguments).expect_err("type should mismatch");

        assert!(error.to_string().contains("wrong type"));
        assert!(error.to_string().contains("Number(U64)"));
        assert!(error.to_string().contains("String"));
    }
}
