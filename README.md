
## Silex CLI

The `silex` CLI is the main entry point for working with Silex programs.

It can:

- compile `.slx` source files into bytecode modules;
- run either source files or compiled modules;
- disassemble bytecode modules into readable assembly;
- assemble textual bytecode back into modules;
- generate JSON ABIs from Silex source.

Compiled modules are written as binary `.slxc` files by default, next to the input path. For inspection or tooling, pass `--format json` to write a human-readable bytecode representation instead.

| Command | Description |
| --- | --- |
| `compile` | Compile a `.slx` source file into a bytecode module. |
| `run` | Execute a `.slx` source file or compiled module. Uses the first entry chunk by default; pass `--entry ID` to choose another one. |
| `disasm` | Print a binary `.slxc` module, or a JSON bytecode module, as assembly. |
| `asm` | Assemble textual bytecode into a module. |
| `abi` | Generate a JSON ABI from `.slx` source. |

```sh
# Install the CLI
cargo install --path silex-cli

# Available commands
silex compile examples/factorial.slx -o factorial.slxc
silex run examples/factorial.slx 5
silex run factorial.slxc 5
silex disasm factorial.slxc
silex asm program.asm -o program.slxc
silex abi examples/factorial.slx
```

For compiled bytecode, prefer `.slxc`. If JSON bytecode is needed for inspection or external tooling, pass `--format json` and choose an explicit JSON output path. ABI files are JSON documents and default to `.abi.json`.

`run` accepts `null`, booleans, unsigned integers, and strings as positional arguments. Pass the JSON representation of a `ValueCell` for other values or an explicit numeric type.
