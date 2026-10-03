//! Deterministic block timelines sharing the network runner's state within each block.

mod execution;
mod scheduled;

use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use xelis_common::{
    block::Block,
    contract::{ContractModule, ScheduledExecutionKind},
    crypto::Hash,
};

use crate::{ExecutionStorage, JsonStorage, RunOptions, RunResult};

use scheduled::run_scheduled;

/// Backend operations needed to advance a deterministic simulation between blocks.
#[async_trait::async_trait]
pub trait BlockStorage: ExecutionStorage {
    /// Return the current block header context, including version and miner.
    fn block(&self) -> Result<Block>;

    /// Return the hash of the last block in the fixture.
    fn block_hash(&self) -> Hash;

    /// Find the next pending execution strictly between two declared blocks.
    fn next_scheduled_topoheight(&self, _after: u64, _before: u64) -> Result<Option<u64>> {
        anyhow::bail!("backend does not support scheduled executions")
    }

    /// Consume and run scheduled calls in the selected network phase.
    async fn run_scheduled(
        _state: &mut Self::State,
        _block_end: bool,
    ) -> Result<Vec<ScheduledCallReport>> {
        anyhow::bail!("backend does not support scheduled executions")
    }

    /// Set block context before beginning its isolated execution state.
    fn set_block(&mut self, topoheight: u64, hash: Hash, block: Block) -> Result<()>;
}

#[async_trait::async_trait]
impl BlockStorage for JsonStorage {
    fn next_scheduled_topoheight(&self, after: u64, before: u64) -> Result<Option<u64>> {
        Ok(self
            .scheduled_executions
            .iter()
            .filter_map(|execution| match execution.kind {
                ScheduledExecutionKind::TopoHeight {
                    execution_topoheight,
                    ..
                } if execution_topoheight > after && execution_topoheight < before => {
                    Some(execution_topoheight)
                }
                _ => None,
            })
            .min())
    }

    async fn run_scheduled(
        state: &mut Self::State,
        block_end: bool,
    ) -> Result<Vec<ScheduledCallReport>> {
        run_scheduled(state, block_end).await
    }

    fn block(&self) -> Result<Block> {
        Ok(self.begin()?.block)
    }

    fn block_hash(&self) -> Hash {
        self.block_hash.clone()
    }

    fn set_block(&mut self, topoheight: u64, hash: Hash, block: Block) -> Result<()> {
        self.topoheight = topoheight;
        self.block_hash = hash;
        self.block = Some(block);

        Ok(())
    }
}

/// Caller-supplied contract modules and a deterministic block timeline.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playbook {
    #[serde(default)]
    pub contracts: Vec<PlaybookContract>,

    /// Enable network scheduled-call phases and insert due blocks into gaps.
    #[serde(default)]
    pub scheduled_executions: bool,

    pub blocks: Vec<PlaybookBlock>,
}

/// A module registered once before running the timeline, preserving existing data/balances.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybookContract {
    pub contract: Hash,
    pub module: ContractModule,
}

/// One simulated block. Calls execute sequentially in their declared order.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybookBlock {
    pub topoheight: u64,

    /// Defaults to the topoheight in this linear simulation.
    #[serde(default)]
    pub height: Option<u64>,

    /// Milliseconds; defaults to the previous timestamp plus the topoheight delta.
    #[serde(default)]
    pub timestamp: Option<u64>,

    #[serde(default)]
    pub calls: Vec<PlaybookCall>,
}

/// One contract invocation with the same typed options used by the library.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybookCall {
    pub contract: Hash,

    #[serde(default)]
    pub execution: RunOptions,
}

/// Ordered execution results with the block context visible to each contract.
#[derive(Serialize)]
pub struct PlaybookReport {
    pub blocks: Vec<BlockReport>,
}

#[derive(Serialize)]
pub struct BlockReport {
    pub topoheight: u64,
    pub height: u64,
    pub timestamp: u64,
    pub hash: Hash,
    pub calls: Vec<CallReport>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scheduled_calls: Vec<ScheduledCallReport>,
}

#[derive(Serialize)]
pub struct CallReport {
    pub contract: Hash,
    pub result: RunResult,
}

/// A consumed scheduled call, including skipped registrations and missing modules.
#[derive(Serialize)]
pub struct ScheduledCallReport {
    pub hash: Hash,
    pub contract: Hash,
    pub block_end: bool,
    pub result: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Final backend and execution report; callers choose whether to persist the result.
pub struct PlaybookOutcome<Storage> {
    pub storage: Storage,
    pub report: PlaybookReport,
}

impl Playbook {
    /// Reject ambiguous registry entries and unordered block declarations before execution.
    pub fn validate(&self) -> Result<()> {
        let mut contracts = HashSet::new();

        for contract in &self.contracts {
            ensure!(
                contracts.insert(&contract.contract),
                "duplicate registered contract {}",
                contract.contract
            );
        }

        for blocks in self.blocks.windows(2) {
            ensure!(
                blocks[0].topoheight < blocks[1].topoheight,
                "playbook blocks must have strictly increasing topoheights"
            );
        }

        Ok(())
    }
}
