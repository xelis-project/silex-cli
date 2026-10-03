//! Timeline validation and block execution with shared state inside each block.

use std::collections::VecDeque;

use anyhow::{Context, Result, ensure};
use indexmap::IndexSet;
use xelis_common::{
    block::{Block, BlockHeader, EXTRA_NONCE_SIZE},
    crypto::{Hash, Hashable},
};
use xelis_vm::ModuleValidator;

use crate::{Simulator, contract::environment_for};

use super::{
    BlockReport, BlockStorage, CallReport, Playbook, PlaybookCall, PlaybookOutcome, PlaybookReport,
};

/// Validated block metadata; calls remain borrowed from the original playbook.
struct PreparedBlock<'a> {
    topoheight: u64,
    height: u64,
    timestamp: u64,
    calls: &'a [PlaybookCall],
}

/// Context used to build the next declared or generated block.
struct BlockContext {
    topoheight: u64,
    block: Block,
    hash: Hash,
}

impl BlockContext {
    fn from_storage<Storage: BlockStorage>(storage: &Storage) -> Result<Self> {
        Ok(Self {
            topoheight: storage.topoheight(),
            block: storage.block()?,
            hash: storage.block_hash(),
        })
    }

    fn make_block(&self, step: &PreparedBlock<'_>) -> Block {
        make_block(
            &self.block,
            self.hash.clone(),
            step.height,
            step.timestamp,
            step.calls,
        )
    }

    fn scheduled_block(&self, topoheight: u64, next: &PreparedBlock<'_>) -> Result<Block> {
        let distance = topoheight - self.topoheight;
        let timestamp = self
            .block
            .get_timestamp()
            .checked_add(distance)
            .context("block timestamp overflow")?;
        ensure!(
            timestamp <= next.timestamp,
            "generated block timestamp exceeds next declared block"
        );

        let height = self
            .block
            .get_height()
            .checked_add(distance)
            .context("block height overflow")?;
        ensure!(
            height <= next.height,
            "generated block height exceeds next declared block"
        );

        Ok(make_block(
            &self.block,
            self.hash.clone(),
            height,
            timestamp,
            &[],
        ))
    }
}

impl Playbook {
    /// Run listed blocks with deterministic headers and shared state for each block's calls.
    /// Failed contract exits are recorded and rolled back by the network runner; later calls continue.
    pub async fn execute<Storage: BlockStorage>(
        &self,
        mut storage: Storage,
    ) -> Result<PlaybookOutcome<Storage>> {
        self.validate()?;
        storage.validate()?;
        self.register_contracts(&mut storage)?;

        // Validate all declarations before executing any calls or changing block context.
        let mut pending = self.prepare_blocks(&storage).await?;
        let mut context = BlockContext::from_storage(&storage)?;
        let mut report = PlaybookReport { blocks: Vec::new() };

        while let Some(next) = pending.front() {
            let due = if self.scheduled_executions {
                storage.next_scheduled_topoheight(context.topoheight, next.topoheight)?
            } else {
                None
            };

            let (topoheight, block, calls) = match due {
                Some(topoheight) => (
                    topoheight,
                    context.scheduled_block(topoheight, next)?,
                    &[][..],
                ),
                None => {
                    let step = pending.pop_front().context("missing prepared block")?;
                    (step.topoheight, context.make_block(&step), step.calls)
                }
            };
            let hash = block.get_header().hash();
            let block_report = self
                .execute_block(&mut storage, topoheight, &block, &hash, calls)
                .await?;
            context = BlockContext {
                topoheight,
                block,
                hash,
            };
            report.blocks.push(block_report);
        }

        Ok(PlaybookOutcome { storage, report })
    }

    fn register_contracts<Storage: BlockStorage>(&self, storage: &mut Storage) -> Result<()> {
        for contract in &self.contracts {
            let module = &contract.module;
            ModuleValidator::new(&module.module, &environment_for::<Storage>(module.version))
                .verify()
                .with_context(|| format!("invalid module for {}", contract.contract))?;
            storage.register(contract.contract.clone(), module.clone())?;
        }
        Ok(())
    }

    async fn prepare_blocks<Storage: BlockStorage>(
        &self,
        storage: &Storage,
    ) -> Result<VecDeque<PreparedBlock<'_>>> {
        let initial = storage.block()?;
        let mut previous_topoheight = storage.topoheight();
        let mut previous_height = initial.get_height();
        let mut previous_timestamp = initial.get_timestamp();
        let mut blocks = VecDeque::with_capacity(self.blocks.len());

        for step in &self.blocks {
            ensure!(
                step.topoheight >= previous_topoheight,
                "block {} precedes fixture topoheight {}",
                step.topoheight,
                previous_topoheight
            );
            let height = step.height.unwrap_or(step.topoheight);
            let timestamp = match step.timestamp {
                Some(timestamp) => timestamp,
                None => previous_timestamp
                    .checked_add(step.topoheight - previous_topoheight)
                    .context("block timestamp overflow")?,
            };
            ensure!(height >= previous_height, "block height cannot decrease");
            ensure!(
                timestamp >= previous_timestamp,
                "block timestamp cannot decrease"
            );

            for call in &step.calls {
                ensure!(
                    storage
                        .load_contract_module(&call.contract, storage.topoheight())
                        .await?
                        .and_then(|(_, module)| module)
                        .is_some(),
                    "contract {} is not registered",
                    call.contract
                );
            }

            blocks.push_back(PreparedBlock {
                topoheight: step.topoheight,
                height,
                timestamp,
                calls: &step.calls,
            });
            previous_topoheight = step.topoheight;
            previous_height = height;
            previous_timestamp = timestamp;
        }
        Ok(blocks)
    }

    async fn execute_block<Storage: BlockStorage>(
        &self,
        storage: &mut Storage,
        topoheight: u64,
        block: &Block,
        hash: &Hash,
        explicit_calls: &[PlaybookCall],
    ) -> Result<BlockReport> {
        storage.set_block(topoheight, hash.clone(), block.clone())?;
        let mut state = storage.begin()?;
        let mut calls = Vec::with_capacity(explicit_calls.len());
        let mut scheduled_calls = if self.scheduled_executions {
            Storage::run_scheduled(&mut state, false).await?
        } else {
            Vec::new()
        };

        for call in explicit_calls {
            let result = Simulator::<Storage>::run_in_state(
                &mut state,
                &call.contract,
                call.execution.clone(),
            )
            .await
            .with_context(|| format!("block {topoheight} contract {}", call.contract))?;
            calls.push(CallReport {
                contract: call.contract.clone(),
                result,
            });
        }
        if self.scheduled_executions {
            scheduled_calls.extend(Storage::run_scheduled(&mut state, true).await?);
        }
        storage.commit(state)?;

        Ok(BlockReport {
            topoheight,
            height: block.get_height(),
            timestamp: block.get_timestamp(),
            hash: hash.clone(),
            calls,
            scheduled_calls,
        })
    }
}

/// Build deterministic headers while retaining explicitly supplied height and timestamp.
fn make_block(
    previous: &Block,
    tip: Hash,
    height: u64,
    timestamp: u64,
    calls: &[PlaybookCall],
) -> Block {
    let transactions = calls
        .iter()
        .filter_map(|call| call.execution.transaction.as_ref());
    let header = BlockHeader::new(
        previous.get_version(),
        height,
        timestamp,
        [tip].into_iter().collect::<IndexSet<_>>(),
        [0; EXTRA_NONCE_SIZE],
        previous.get_miner().clone(),
        transactions
            .clone()
            .map(|tx| tx.hash.clone())
            .collect::<IndexSet<_>>(),
    );
    Block::new(
        header,
        transactions.map(|tx| tx.transaction.clone()).collect(),
    )
}
