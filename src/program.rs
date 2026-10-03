//! Standalone program execution preserving the original CLI return-value semantics.

use anyhow::{Context, Result, ensure};
use silex_bytecode::Access;
use silex_types::{Primitive, ValueCell};
use xelis_common::{
    contract::{ContractMetadata, ContractModule, ContractProvider, ModuleMetadata},
    crypto::Hash,
};
use xelis_vm::{Module, ModuleValidator, VM};

use crate::contract::environment_for;

/// Execute an entry as a standalone program and return its value verbatim.
/// Provider-backed network calls use `Simulator`; this preserves unrestricted program return values.
pub fn run_program<Storage: for<'ty> ContractProvider<'ty>>(
    module: &ContractModule,
    entry: Option<u16>,
    arguments: Vec<ValueCell>,
    gas_limit: Option<u64>,
) -> Result<ValueCell> {
    let environment = environment_for::<Storage>(module.version);
    let validator = ModuleValidator::new(&module.module, &environment);
    validator.verify().context("module failed validation")?;

    let entry = resolve_entry(&module.module, entry)?;
    validator
        .verify_invoke_chunk(entry as usize, arguments.iter())
        .with_context(|| format!("invalid arguments for entry chunk {entry}: {arguments:?}"))?;

    let metadata = ContractMetadata {
        contract_executor: Hash::zero(),
        contract_caller: None,
        contract_version: module.version,
        deposits: Default::default(),
    };

    let mut vm = VM::<ContractMetadata>::default();
    vm.append_module(ModuleMetadata {
        module: module.module.as_ref().into(),
        metadata: (&metadata).into(),
        environment: (&environment).into(),
    })?;

    if let Some(limit) = gas_limit {
        vm.context_mut().set_gas_limit(limit);
    }

    vm.invoke_chunk_with_args(entry, arguments.into_iter())?;
    vm.run_blocking().context("program execution failed")
}

/// Resolve an explicit or default entry and reject non-entry chunks.
pub(crate) fn resolve_entry(module: &Module, requested: Option<u16>) -> Result<u16> {
    let entry = requested
        .or_else(|| {
            module
                .chunks()
                .iter()
                .position(|chunk| matches!(chunk.access, Access::Entry { .. }))
                .and_then(|id| u16::try_from(id).ok())
        })
        .context("program does not define an entry chunk")?;

    ensure!(
        module.is_entry_chunk(entry as usize),
        "chunk {entry} is not an entry chunk"
    );
    Ok(entry)
}

/// Parse the original positional argument syntax, including explicit JSON ValueCells.
pub fn parse_argument(argument: &str) -> Result<ValueCell> {
    if let Ok(value) = serde_json::from_str(argument) {
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
