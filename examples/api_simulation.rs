use anyhow::{Result, ensure};
use silex_cli::{JsonStorage, RunOptions, Simulator, xelis_common::crypto::Hash};

#[tokio::main]
async fn main() -> Result<()> {
    // The caller supplies source and storage; the library never opens files.
    let mut simulator = Simulator::new(JsonStorage::default())?;
    let contract = Hash::zero();
    let source = include_str!("../fixtures/counter.slx");
    let result = simulator
        .run_source(contract.clone(), source, RunOptions::default())
        .await?;
    ensure!(result.execution.is_success(), "{:?}", result.execution);
    println!("{}", serde_json::to_string_pretty(&result)?);

    // Serialization and persistence are the caller's responsibility.
    let snapshot = serde_json::to_string(simulator.storage())?;
    let storage: JsonStorage = serde_json::from_str(&snapshot)?;
    let mut restored = Simulator::new(storage)?;
    let result = restored.run(&contract, RunOptions::default()).await?;
    ensure!(result.execution.is_success(), "{:?}", result.execution);
    println!("{}", serde_json::to_string_pretty(restored.storage())?);
    Ok(())
}
