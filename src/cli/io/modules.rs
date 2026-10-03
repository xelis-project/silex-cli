use std::{fs, path::Path};

use anyhow::{Context, Result};
use silex_cli::{
    contract::compile_source_with_storage,
    xelis_common::{
        contract::{ContractModule, ContractProvider},
        serializer::Serializer,
    },
};

use super::super::args::OutputFormat;

pub(crate) fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub(crate) fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
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
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse JSON contract module {}", path.display())),
        Some("hex") => {
            ContractModule::from_hex(str::from_utf8(&bytes)?.trim()).context("invalid hex module")
        }
        _ => ContractModule::from_bytes(&bytes)
            .with_context(|| format!("failed to parse binary bytecode module {}", path.display())),
    }
}

pub(crate) fn load_module<Storage: for<'ty> ContractProvider<'ty>>(
    path: &Path,
) -> Result<ContractModule> {
    if path.extension().is_some_and(|extension| extension == "slx") {
        Ok(compile_source_with_storage::<Storage>(&read_file(path)?)?.0)
    } else {
        read_module(path)
    }
}
