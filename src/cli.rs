use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "silex",
    about = "Compile, inspect, assemble, and run Silex programs"
)]
pub(crate) struct Config {
    #[command(subcommand)]
    pub(crate) command: SubCommands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SubCommands {
    /// Compile a Silex source program into a bytecode module.
    Compile(CompileConfig),
    /// Run an entry chunk from a Silex source or compiled bytecode module.
    Run(RunConfig),
    /// Print a bytecode module as assembly.
    Disasm(DisasmConfig),
    /// Assemble textual bytecode into a bytecode module.
    Asm(AsmConfig),
    /// Generate a JSON ABI from a Silex source program.
    Abi(AbiConfig),
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Binary,
    Json,
}

impl OutputFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Binary => "slxc",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct CompileConfig {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(
        short,
        long,
        value_name = "OUTPUT",
        help = "Output module path (defaults to INPUT with a .slxc extension)"
    )]
    pub(crate) output: Option<PathBuf>,
    #[arg(short, long, value_enum, default_value_t, help = "Output format")]
    pub(crate) format: OutputFormat,
}

impl CompileConfig {
    pub(crate) fn output_path(&self) -> PathBuf {
        output_path(&self.input, self.output.as_ref(), self.format)
    }
}

#[derive(Debug, Args)]
pub(crate) struct RunConfig {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(
        short,
        long,
        value_name = "ID",
        help = "Entry chunk ID to invoke (defaults to the first entry chunk)"
    )]
    pub(crate) entry: Option<u16>,
    #[arg(
        long,
        value_name = "GAS",
        help = "Maximum gas available to the program"
    )]
    pub(crate) gas_limit: Option<u64>,
    #[arg(
        value_name = "ARG",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Arguments: null, bool, unsigned integer, string, or a JSON ValueCell"
    )]
    pub(crate) arguments: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct DisasmConfig {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct AsmConfig {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(
        short,
        long,
        value_name = "OUTPUT",
        help = "Output module path (defaults to INPUT with a .slxc extension)"
    )]
    pub(crate) output: Option<PathBuf>,
    #[arg(short, long, value_enum, default_value_t, help = "Output format")]
    pub(crate) format: OutputFormat,
}

impl AsmConfig {
    pub(crate) fn output_path(&self) -> PathBuf {
        output_path(&self.input, self.output.as_ref(), self.format)
    }
}

#[derive(Debug, Args)]
pub(crate) struct AbiConfig {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(
        short,
        long,
        value_name = "OUTPUT",
        help = "Output ABI JSON path (defaults to INPUT with a .abi.json extension)"
    )]
    pub(crate) output: Option<PathBuf>,
}

impl AbiConfig {
    pub(crate) fn output_path(&self) -> PathBuf {
        self.output
            .clone()
            .unwrap_or_else(|| self.input.with_extension("abi.json"))
    }
}

fn output_path(input: &Path, output: Option<&PathBuf>, format: OutputFormat) -> PathBuf {
    output
        .cloned()
        .unwrap_or_else(|| input.with_extension(format.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_output_defaults_to_slxc() {
        let config = CompileConfig {
            input: PathBuf::from("examples/factorial.slx"),
            output: None,
            format: OutputFormat::Binary,
        };

        assert_eq!(
            config.output_path(),
            PathBuf::from("examples/factorial.slxc")
        );
    }

    #[test]
    fn json_output_uses_json_extension() {
        let config = AsmConfig {
            input: PathBuf::from("program.asm"),
            output: None,
            format: OutputFormat::Json,
        };

        assert_eq!(config.output_path(), PathBuf::from("program.json"));
    }

    #[test]
    fn abi_output_appends_abi_json() {
        let config = AbiConfig {
            input: PathBuf::from("program.slx"),
            output: None,
        };

        assert_eq!(config.output_path(), PathBuf::from("program.abi.json"));
    }
}
