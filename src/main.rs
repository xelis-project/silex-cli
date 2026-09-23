mod cli;
mod contract;
mod runtime;

use std::{fs, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use silex_abi::abi_from_silex;
use silex_assembler::{Assembler, Disassembler};
use silex_decompiler::Decompiler;
use xelis_common::{
    contract::{ContractModule, ContractVersion, build_environment},
    transaction::mock::MockStorageProvider,
};

use cli::{Config, SubCommands};
use contract::{compile_source, read_file, write_module};

fn main() -> Result<()> {
    let config = Config::parse();

    match config.command {
        SubCommands::Compile(config) => {
            let source = read_file(&config.input)?;
            let (module, _) = compile_source(&source)?;
            let output = config.output_path();
            write_module(&module, &output, config.format)?;
        }
        SubCommands::Asm(config) => {
            let source = read_file(&config.input)?;
            let module = Assembler::new(&source)
                .assemble()
                .map_err(|error| anyhow::anyhow!("failed to assemble source: {error}"))?;
            let module = ContractModule {
                version: ContractVersion::V1,
                module: Arc::new(module),
            };
            let output = config.output_path();
            write_module(&module, &output, config.format)?;
        }
        SubCommands::Disasm(config) => {
            let module = contract::read_module(&config.input)?;
            let dump = Disassembler::new(&module.module)
                .disasemble()
                .context("failed to disassemble bytecode module")?;
            println!("{dump}");
        }
        SubCommands::Decompile(config) => {
            let module = contract::read_module(&config.input)?;
            let environment = build_environment::<MockStorageProvider>(module.version);
            let source = Decompiler::new(&module.module, &environment)
                .decompile()
                .context("failed to decompile bytecode module")?;
            print!("{source}");
        }
        SubCommands::Abi(config) => {
            let source = read_file(&config.input)?;
            let abi = abi_from_silex(
                &source,
                build_environment::<MockStorageProvider>(ContractVersion::V1),
            )
            .context("failed to generate ABI")?;
            let output = config.output_path();
            fs::write(&output, format!("{abi}\n"))
                .with_context(|| format!("failed to write {}", output.display()))?;
        }
        SubCommands::Run(config) => runtime::run(config)?,
    }

    Ok(())
}
