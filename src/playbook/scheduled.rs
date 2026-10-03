//! Scheduled-call phases matching the upstream block processor.

use std::{borrow::Cow, collections::HashMap, mem::take, sync::Arc};

use anyhow::{Result, ensure};
use xelis_common::{
    block::BlockVersion,
    contract::{
        InterContractPermission, ScheduledExecution, ScheduledExecutionKind, Source,
        vm::{ContractCaller, InvokeContract, refund_gas_sources},
    },
    crypto::Hash,
    transaction::verify::{BlockchainContractState, BlockchainVerificationState},
};

use crate::{Invocation, JsonStorage, Simulator, state::ChainState};

use super::ScheduledCallReport;

/// Drain topoheight calls in hash order, or block-end calls in registration order.
pub(crate) async fn run_scheduled(
    state: &mut ChainState,
    block_end: bool,
) -> Result<Vec<ScheduledCallReport>> {
    let mut reports = Vec::new();

    loop {
        let hashes = pending_hashes(state, block_end);

        if hashes.is_empty() {
            break;
        }

        for hash in hashes {
            let execution = state
                .executions
                .executions
                .remove(&hash)
                .ok_or_else(|| anyhow::anyhow!("scheduled execution not found: {hash}"))?;
            state
                .executions
                .at_topoheight
                .retain(|_, pending| pending != &hash);
            reports.push(execute_scheduled(state, execution, block_end).await?);
        }

        // Future-height scheduling may continue; the current topoheight phase is a fixed batch.
        if !block_end {
            break;
        }
    }

    Ok(reports)
}

fn pending_hashes(state: &mut ChainState, block_end: bool) -> Vec<Arc<Hash>> {
    if block_end {
        let mut hashes = take(&mut state.executions.block_end);
        // Fixtures may contain block-end calls carried over from a standalone run.
        let mut persisted: Vec<_> = state
            .executions
            .executions
            .values()
            .filter(|execution| {
                matches!(execution.kind, ScheduledExecutionKind::BlockEnd)
                    && !hashes.contains(&execution.hash)
            })
            .map(|execution| execution.hash.clone())
            .collect();
        persisted.sort();
        hashes.extend(persisted);
        hashes
    } else {
        let mut hashes: Vec<_> = state
            .executions
            .executions
            .values()
            .filter(|execution| {
                matches!(execution.kind, ScheduledExecutionKind::TopoHeight {
                    execution_topoheight, ..
                } if execution_topoheight == state.topoheight)
            })
            .map(|execution| execution.hash.clone())
            .collect();
        hashes.sort();
        hashes
    }
}

async fn execute_scheduled(
    state: &mut ChainState,
    execution: ScheduledExecution,
    block_end: bool,
) -> Result<ScheduledCallReport> {
    let mut report = ScheduledCallReport {
        hash: execution.hash.as_ref().clone(),
        contract: execution.contract.clone(),
        block_end,
        result: None,
        skipped: None,
    };

    if !prepare_execution(state, &execution).await? {
        report.skipped = Some("invalid or legacy-unmarked gas sources".into());
    } else if !state
        .load_contract_module(Cow::Owned(execution.contract.clone()))
        .await?
    {
        refund_gas_sources(state, execution.gas_sources, 0, execution.max_gas).await?;
        report.skipped = Some("contract module not registered; gas refunded".into());
    } else {
        report.result = Some(
            Simulator::<JsonStorage>::invoke_in_state(
                state,
                Invocation {
                    caller: ContractCaller::Scheduled(
                        Cow::Owned(execution.hash.as_ref().clone()),
                        Cow::Owned(execution.contract.clone()),
                    ),
                    contract: execution.contract,
                    deposits: None,
                    arguments: execution.params,
                    gas_sources: execution.gas_sources,
                    max_gas: execution.max_gas,
                    invoke: InvokeContract::Chunk(execution.chunk_id, false),
                    permission: InterContractPermission::All,
                    post_execution: true,
                },
            )
            .await?,
        );
    }

    Ok(report)
}

/// Apply the pinned upstream V7 checks for reserved scheduled-call gas sources.
async fn prepare_execution(state: &mut ChainState, execution: &ScheduledExecution) -> Result<bool> {
    if state.block.get_version() < BlockVersion::V7 {
        return Ok(true);
    }

    let mut total = 0u64;
    let mut legacy_account = false;
    let mut refunds = HashMap::new();

    for (source, gas) in &execution.gas_sources {
        let Some(sum) = total.checked_add(*gas) else {
            return Ok(false);
        };
        total = sum;

        match source {
            Source::Account(_) => legacy_account = true,
            Source::AccountBalance(_) => {}
            Source::Contract(contract) => {
                if contract != &execution.contract {
                    return Ok(false);
                }
                let refund = refunds.entry(contract.clone()).or_insert(0u64);
                let Some(sum) = refund.checked_add(*gas) else {
                    return Ok(false);
                };
                *refund = sum;
            }
            Source::ContractBalance(_) => return Ok(false),
        }
    }

    if total != execution.max_gas {
        return Ok(false);
    }
    if !legacy_account {
        return Ok(true);
    }

    // Old unmarked account gas cannot execute or refund; return backed contract gas only.
    for (contract, refund) in &refunds {
        let (_, balance) = state.get_contract_balance_for_gas(contract).await?;
        if balance.checked_add(*refund).is_none() {
            return Ok(false);
        }
    }

    for (contract, refund) in refunds {
        let (version, balance) = state.get_contract_balance_for_gas(&contract).await?;
        version.mark_updated();
        ensure!(balance.checked_add(refund).is_some(), "gas refund overflow");
        *balance += refund;
    }

    Ok(false)
}
