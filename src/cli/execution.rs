//! Command dispatch; files are decoded before calling the library.

use anyhow::{Result, bail};
use silex_cli::{
    JsonStorage, Simulator,
    contract::{assemble, compile_source, decompile, disassemble, generate_abi},
    program::{parse_argument, run_program},
    xelis_common::{
        contract::vm::ExitValue, crypto::hash, serializer::Serializer,
        transaction::mock::MockStorageProvider,
    },
};

use super::{
    args::SubCommands,
    io::{
        load_execution, load_fixture, load_module, load_playbook, read_file, read_module,
        save_fixture, write_module, write_text,
    },
};

pub(super) async fn run(command: SubCommands) -> Result<()> {
    match command {
        SubCommands::Compile(config) => {
            let source = read_file(&config.input)?;
            let (module, _) = compile_source(&source)?;
            let output = config.output_path();

            write_module(&module, &output, config.format)?;
        }

        SubCommands::Asm(config) => {
            let source = read_file(&config.input)?;
            let module = assemble(&source)?;
            let output = config.output_path();

            write_module(&module, &output, config.format)?;
        }

        SubCommands::Disasm(config) => {
            let module = read_module(&config.input)?;
            let dump = disassemble(&module)?;

            println!("{dump}");
        }

        SubCommands::Decompile(config) => {
            let module = read_module(&config.input)?;
            let source = decompile(&module)?;

            print!("{source}");
        }

        SubCommands::Abi(config) => {
            let source = read_file(&config.input)?;
            let abi = generate_abi(&source)?;
            let output = config.output_path();
            write_text(&output, &format!("{abi}\n"))?;
        }

        SubCommands::Run(config) => {
            let arguments = config
                .arguments
                .iter()
                .map(|argument| parse_argument(argument))
                .collect::<Result<Vec<_>>>()?;

            if let Some(path) = &config.fixture {
                let mut fixture = load_fixture(path)?;
                let mut options = config
                    .execution
                    .as_deref()
                    .map(load_execution)
                    .transpose()?
                    .unwrap_or_default();

                if let Some(entry) = config.entry {
                    options.entry = Some(entry);
                    options.hook = None;
                }

                if let Some(limit) = config.gas_limit {
                    options.gas_limit = limit;
                }

                if !arguments.is_empty() {
                    options.arguments = arguments;
                }

                let module = load_module::<JsonStorage>(&config.input)?;
                let contract = config.contract.unwrap_or_else(|| hash(&module.to_bytes()));
                let mut simulator = Simulator::new(fixture)?;
                println!("Contract hash: {contract}");
                let result = simulator.run_module(contract, module, options).await?;
                fixture = simulator.into_storage();

                if config.update_fixture {
                    save_fixture(&fixture, path)?;
                }

                match result.execution.exit_value {
                    ExitValue::ExitCode(value) => println!("{value}"),
                    ExitValue::Payload(value) => println!("{value}"),
                    ExitValue::Error(error) => bail!("{error}"),
                }
            } else {
                let module = load_module::<MockStorageProvider>(&config.input)?;
                let contract = config.contract.unwrap_or_else(|| hash(&module.to_bytes()));
                println!("Contract hash: {contract}");
                let value = run_program::<MockStorageProvider>(
                    &module,
                    config.entry,
                    arguments,
                    config.gas_limit,
                )?;
                println!("{value}");
            }
        }

        SubCommands::Playbook(config) => {
            let loaded = load_playbook(&config.input)?;
            let mut fixture = loaded.fixture;
            let outcome = loaded.playbook.execute(fixture).await?;
            fixture = outcome.storage;

            if config.update_fixture {
                save_fixture(&fixture, &loaded.fixture_path)?;
            }

            println!("{}", serde_json::to_string_pretty(&outcome.report)?);
        }
    }

    Ok(())
}
