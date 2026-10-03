//! Typed invocation inputs and results shared by library clients and adapters.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use silex_types::ValueCell;
use xelis_common::{
    api::RPCContractLog,
    contract::{
        ContractMetadata, InterContractPermission, Source,
        vm::{ContractCaller, ExecutionResult, InvokeContract},
    },
    crypto::{Address, Hash},
    transaction::{ContractDeposit, Transaction, verify::DecompressedDepositCt},
};
use xelis_vm::{Environment, Module, ModuleValidator};

use crate::{program::resolve_entry, storage::GasSource};

/// A real transaction caller, including the hash used for logs and deterministic execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionCaller {
    pub hash: Hash,
    pub transaction: Arc<Transaction>,
}

/// Serializable invocation settings for a config file or library caller.
/// Deposits and gas sources must already be funded, as at the network runner boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunOptions {
    pub entry: Option<u16>,
    pub hook: Option<u8>,

    pub deposits: IndexMap<Hash, ContractDeposit>,
    pub gas_sources: Vec<GasSource>,
    pub gas_limit: u64,
    pub arguments: Vec<ValueCell>,

    /// An impersonated caller for fixtures; use `transaction` for transaction semantics.
    pub caller: Option<Address>,
    pub transaction: Option<TransactionCaller>,

    pub permission: InterContractPermission,
}

impl Default for RunOptions {
    /// Match the runner's system-call defaults, with a finite gas budget.
    fn default() -> Self {
        Self {
            entry: None,
            hook: None,
            deposits: IndexMap::new(),
            gas_sources: Vec::new(),
            gas_limit: 1_000_000,
            arguments: Vec::new(),
            caller: None,
            transaction: None,
            permission: InterContractPermission::default(),
        }
    }
}

impl RunOptions {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.entry.is_none() || self.hook.is_none(),
            "choose either entry or hook"
        );

        ensure!(
            self.caller.is_none() || self.transaction.is_none(),
            "choose either caller or transaction"
        );

        ensure!(
            self.deposits
                .values()
                .all(|d| matches!(d, ContractDeposit::Public(_))),
            "private deposits require Invocation with decompressed proofs"
        );

        Ok(())
    }

    pub(super) fn resolve_invoke(
        &self,
        module: &Module,
        environment: &Environment<ContractMetadata>,
    ) -> Result<InvokeContract> {
        let validator = ModuleValidator::new(module, environment);
        validator.verify().context("module failed validation")?;

        match self.hook {
            Some(hook) => {
                module
                    .get_chunk_id_of_hook(hook)
                    .context("hook not found")?;
                Ok(InvokeContract::Hook(hook))
            }

            None => {
                let entry = resolve_entry(module, self.entry)?;
                validator
                    .verify_invoke_chunk(entry as usize, self.arguments.iter())
                    .context("invalid entry arguments")?;
                Ok(InvokeContract::Entry(entry))
            }
        }
    }
}

/// Borrowed deposit inputs accepted by the upstream runner, including private deposit proofs.
pub type Deposits<'a> = (
    &'a IndexMap<Hash, ContractDeposit>,
    &'a HashMap<&'a Hash, DecompressedDepositCt>,
);

/// Full runner inputs for applications that supply transaction or scheduled-call context.
/// No fee reservation, deposit debit, refund, or exit-value interpretation is added here.
pub struct Invocation<'a> {
    pub caller: ContractCaller<'a>,
    pub contract: Hash,
    pub deposits: Option<Deposits<'a>>,
    pub arguments: Vec<ValueCell>,

    pub gas_sources: IndexMap<Source, u64>,
    pub max_gas: u64,

    pub invoke: InvokeContract,
    pub permission: InterContractPermission,
    pub post_execution: bool,
}

/// The upstream execution result together with the backend's logs for the caller.
#[derive(Serialize)]
pub struct RunResult {
    pub execution: ExecutionResult,
    pub logs: Vec<RPCContractLog<'static>>,
}
