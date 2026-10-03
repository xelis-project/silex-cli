# Silex CLI

Compile and run Silex programs, simulate smart contracts, or use the toolchain as a Rust library.

```sh
cargo install --path .
```

## Run

```sh
silex-cli run examples/factorial.slx 5
# 120
```

`run INPUT [ARGUMENTS...]` prints the returned value. Arguments accept null, booleans, unsigned integers, strings, and JSON-encoded values. Use `--entry` to select an entry and `--gas-limit` to limit execution.

To run a contract with configured network state, provide a fixture:

```sh
silex-cli run --fixture fixtures/environment.json fixtures/counter.slx
silex-cli run --fixture fixtures/environment.json --update-fixture fixtures/counter.slx
```

The same environment can be used for different contract files:

```bash
silex-cli run --fixture fixtures/environment.json --contract <contract-hash> contract.slx
```

The fixture stays unchanged unless `--update-fixture` is set. Contract execution uses `xelis_common::contract::vm::invoke_contract`, including its gas accounting, permissions, and failure rollback.

## Fixtures

A fixture contains only chain state, using the same JSON format as `JsonStorage`:

```json
{
  "mainnet": false,
  "topoheight": 100,
  "contracts": {},
  "accounts": {},
  "assets": {}
}
```

The environment contains state for all contract hashes and has no source file path. Pass the source or compiled module to `run` separately. Select its hash with `--contract <64-character hex hash>`; when omitted, `run` computes the XELIS BLAKE3 hash of the serialized contract module, including its version. Source, binary, JSON, and hex representations of the same module use the same hash. The module replaces only that contract’s code, preserving data and balances for all contracts. Reuse the same environment to run different files and hashes.

| Storage field | Configures |
| --- | --- |
| `mainnet`, `topoheight` | Network and current height |
| `block`, `block_hash` | Block context |
| `assets` | Asset metadata and circulating supplies |
| `accounts` | Address nonces and asset balances |
| `contracts` | Registered modules, data, and balances |
| `scheduled_executions`, `callbacks` | Scheduled calls and event listeners |

An empty fixture `{}` uses an empty testnet environment at topoheight 1. Amounts use atomic units; the zero asset hash represents XELIS. Contract data and arguments use upstream JSON encoding for values.

Execution options are supplied separately through CLI flags or `--execution execution.json`:

```json
{
  "entry": 0,
  "gas_limit": 1000000
}
```

```sh
silex-cli run --fixture fixtures/environment.json --execution fixtures/execution.json contract.slx
```

The execution options file also supports `hook`, `arguments`, `deposits`, `gas_sources`, `permission`, and either `caller` or `transaction`. CLI entry, gas limit, and supplied arguments override this file. Without it, execution uses default options. Use `permission: "all"` to allow inter-contract calls and `hook: 0` for a constructor. Playbooks keep execution options on each call. Saving a fixture writes only chain state.

See [fixtures/environment.json](fixtures/environment.json) for a complete example. Deposit funds and gas budgets must already be reserved in the fixture, as expected by the network runner.

## Playbooks

Run an ordered sequence of contract calls across blocks:

```sh
silex-cli playbook fixtures/playbook.json
silex-cli playbook fixtures/playbook.json --update-fixture
```

A playbook references a fixture, registers contract files, and lists calls by block:

```json
{
  "fixture": "environment.json",
  "contracts": [
    { "contract": "<A hash>", "input": "a.slx" },
    { "contract": "<B hash>", "input": "b.slx" },
    { "contract": "<X hash>", "input": "x.slx" }
  ],
  "blocks": [
    {
      "topoheight": 100,
      "timestamp": 1000,
      "calls": [
        { "contract": "<A hash>" },
        { "contract": "<B hash>" }
      ]
    },
    {
      "topoheight": 105,
      "calls": [{ "contract": "<X hash>" }]
    }
  ]
}
```

Replace the placeholders with contract hashes, or use the runnable [example](fixtures/playbook.json). File paths resolve relative to the playbook. Each call can include its own `execution` settings.

Calls run in list order and share execution state within a block. Data and balances persist between blocks; shared memory resets. Topoheights must strictly increase. Height defaults to topoheight; omitted timestamps advance by the topoheight difference from the previous block. Replaying the same fixture and playbook produces the same results.

The command prints a JSON report with block metadata, results, and logs. Failed contract calls roll back their writes and the timeline continues. `--update-fixture` saves the final state after the playbook completes.

Set `"scheduled_executions": true` in the playbook to process pending scheduled calls. Topoheight calls run before explicit calls; block-end calls run afterward. Empty blocks are supported, and an explicitly declared block keeps its configured height and timestamp.

Calls due between declared blocks create deterministic intermediate blocks. Processing stops at the last declared block; later calls remain in the fixture. Reports include `scheduled_calls` with results or reasons for skipping an invalid registration. Scheduling is disabled by default.

See [fixtures/scheduled-playbook.json](fixtures/scheduled-playbook.json) for an example. Playbooks emulate contract execution, not transaction validation or consensus.

## Rust API

```rust,no_run
use silex_cli::{ExecutionStorage, RunOptions, RunResult, Simulator};
use silex_cli::xelis_common::crypto::Hash;

async fn run<Storage: ExecutionStorage>(storage: Storage) -> anyhow::Result<RunResult> {
    let mut simulator = Simulator::<Storage>::new(storage)?;

    simulator.run_source(
        Hash::zero(),
        "entry main() { return 0; }",
        RunOptions::default(),
    ).await
}
```

`Simulator` defaults to `JsonStorage`. Custom backends implement `ExecutionStorage` and the upstream storage traits; JSON serialization is not required. Playbooks accept custom backends through `BlockStorage`.

Use `run_module` with a compiled module or `run_source` with source text, `invoke` for a complete upstream invocation, and `run_program` for standalone programs. Results include execution status and logs; check `result.execution.is_success()` for contract success.

The library operates entirely in memory. Callers provide typed storage, modules, and execution options and handle serialization and persistence themselves. `JsonStorage` represents the environment; `RunOptions` represents a separate execution request. Each `PlaybookContract` contains a contract hash and compiled `module`; the library playbook has no fixture or file paths. The CLI handles file loading, relative paths, and updates using the environment, execution options, and playbook file formats.

The `contract` module exposes compilation, assembly, disassembly, decompilation, and ABI generation.

See the [API example](examples/api_simulation.rs) and [custom backend example](tests/custom_storage.rs).

```sh
cargo run --example api_simulation
cargo doc --open
```

## Browser API (WebAssembly)

Build the JavaScript module and TypeScript declarations with [wasm-pack](https://rustwasm.github.io/wasm-pack/):

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build --target web --no-default-features --features wasm
```

The generated `pkg/` directory contains the JavaScript bindings and WebAssembly module.

## Toolchain

```sh
silex-cli compile program.slx -o program.slxc
silex-cli compile program.slx --format json -o program.json
silex-cli disasm program.slxc
silex-cli decompile program.slxc
silex-cli asm program.asm -o assembled.slxc
silex-cli abi program.slx -o program.abi.json
```

Modules support binary `.slxc`, JSON, and `.hex` formats. Use `silex-cli --help` or `<command> --help` for options.

## Development

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
wasm-pack build --target web --dev --no-default-features --features wasm --locked
```
