//! Contract execution using caller-supplied modules, options, and storage.

mod types;

use std::{borrow::Cow, collections::HashMap};

use anyhow::{Context, Result, ensure};
use indexmap::IndexMap;
use xelis_common::{
    api::RPCContractLog,
    contract::{
        ContractModule,
        vm::{ContractCaller, invoke_contract},
    },
    crypto::Hash,
    transaction::verify::{BlockchainApplyState, BlockchainVerificationState},
};
use xelis_vm::ModuleValidator;

use crate::{
    contract::{compile_source_with_storage, environment_for},
    storage::{ExecutionStorage, JsonStorage},
};

pub use types::{Deposits, Invocation, RunOptions, RunResult, TransactionCaller};

/// Runs contracts with a user-supplied storage backend and chain-state implementation.
#[derive(Debug, Clone)]
pub struct Simulator<Storage = JsonStorage> {
    storage: Storage,
}

impl<Storage: ExecutionStorage> Simulator<Storage> {
    /// Validate and own a backend without opening files or creating an async runtime.
    pub fn new(storage: Storage) -> Result<Self> {
        storage.validate()?;

        Ok(Self { storage })
    }

    /// Inspect the backend after an execution.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Return the backend to the caller for further configuration or persistence.
    pub fn into_storage(self) -> Storage {
        self.storage
    }

    /// Validate and register a module without calling its constructor.
    pub fn register(&mut self, hash: Hash, module: ContractModule) -> Result<()> {
        ModuleValidator::new(&module.module, &environment_for::<Storage>(module.version))
            .verify()
            .context("module failed validation")?;

        self.storage.register(hash, module)
    }

    /// Compile, register, and invoke source against this backend.
    pub async fn run_source(
        &mut self,
        hash: Hash,
        source: &str,
        options: RunOptions,
    ) -> Result<RunResult> {
        let (module, _) = compile_source_with_storage::<Storage>(source)?;
        self.run_module(hash, module, options).await
    }

    /// Register and execute a caller-supplied module.
    pub async fn run_module(
        &mut self,
        hash: Hash,
        module: ContractModule,
        options: RunOptions,
    ) -> Result<RunResult> {
        self.register(hash.clone(), module)?;
        self.run(&hash, options).await
    }

    /// Validate serializable options and forward them to the upstream runner.
    /// Private deposits require `invoke` so the caller can provide decompressed proofs.
    pub async fn run(&mut self, hash: &Hash, options: RunOptions) -> Result<RunResult> {
        let mut state = self.storage.begin()?;
        let result = Self::run_in_state(&mut state, hash, options).await?;
        self.storage.commit(state)?;

        Ok(result)
    }

    /// Execute a call inside an existing block state, preserving caches between calls.
    pub async fn run_in_state(
        state: &mut Storage::State,
        hash: &Hash,
        options: RunOptions,
    ) -> Result<RunResult> {
        options.validate()?;

        ensure!(
            state.load_contract_module(Cow::Owned(hash.clone())).await?,
            "contract module not registered"
        );

        let (module, environment) = state.get_contract_module_with_environment(hash).await?;
        let invoke = options.resolve_invoke(module, environment)?;

        let caller = match (&options.transaction, options.caller) {
            (Some(tx), _) => ContractCaller::Transaction(&tx.hash, &tx.transaction),
            (_, Some(address)) => ContractCaller::Impersonate(Cow::Owned(address.to_public_key())),
            _ => ContractCaller::System,
        };

        let mut gas_sources = IndexMap::new();

        for source in options.gas_sources {
            ensure!(
                gas_sources.insert(source.source, source.amount).is_none(),
                "duplicate gas source"
            );
        }

        Self::invoke_in_state(
            state,
            Invocation {
                caller,
                contract: hash.clone(),
                deposits: Some((&options.deposits, &HashMap::new())),
                arguments: options.arguments,
                gas_sources,
                max_gas: options.gas_limit,
                invoke,
                permission: options.permission,
                post_execution: true,
            },
        )
        .await
    }

    /// Invoke `xelis_common::contract::vm::invoke_contract` with unchanged runner inputs.
    /// The backend controls state isolation and atomic persistence; errors before commit do not commit.
    pub async fn invoke(&mut self, invocation: Invocation<'_>) -> Result<RunResult> {
        let mut state = self.storage.begin()?;
        let result = Self::invoke_in_state(&mut state, invocation).await?;
        self.storage.commit(state)?;

        Ok(result)
    }

    /// Forward a full invocation into an existing block state without committing it.
    pub async fn invoke_in_state(
        state: &mut Storage::State,
        invocation: Invocation<'_>,
    ) -> Result<RunResult> {
        ensure!(
            state
                .load_contract_module(Cow::Owned(invocation.contract.clone()))
                .await?,
            "contract module not registered"
        );

        let caller_hash = invocation.caller.get_hash().into_owned();
        let mainnet = state.is_mainnet();
        let previous_logs = Storage::logs(state, &caller_hash).len();

        let execution = invoke_contract(
            invocation.caller,
            state,
            Cow::Owned(invocation.contract),
            invocation.deposits,
            invocation.arguments.into_iter(),
            invocation.gas_sources,
            invocation.max_gas,
            invocation.invoke,
            Cow::Owned(invocation.permission),
            invocation.post_execution,
        )
        .await
        .map_err(|error| anyhow::anyhow!("contract runner failed: {error}"))?;

        let logs = Storage::logs(state, &caller_hash)
            .into_iter()
            .skip(previous_logs)
            .map(|log| RPCContractLog::from_owned(log, mainnet))
            .collect();

        Ok(RunResult { execution, logs })
    }
}
