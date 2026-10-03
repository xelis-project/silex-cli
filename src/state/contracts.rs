//! Contract caches, prepaid deposits, and successful state changes.

use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
    marker::PhantomData,
};

use anyhow::{Context, Error, Result};
use async_trait::async_trait;
use indexmap::IndexMap;
use log::Level;
use xelis_common::{
    block::BlockVersion,
    config::XELIS_ASSET,
    contract::{
        ChainState as ContractChainState, ChainStateChanges, ContractLogs, ContractModule,
        ContractProvider, ExecutionsChanges, ExecutionsManager, InterContractPermission,
        vm::ContractCaller,
    },
    crypto::Hash,
    transaction::{
        ContractDeposit,
        verify::{BlockchainApplyState, BlockchainContractState, ContractEnvironment},
    },
    versioned::VersionedState,
};

use super::ChainState;

#[async_trait]
impl<'a, 'ty, Storage: for<'p> ContractProvider<'p> + Send + Sync>
    BlockchainContractState<'a, 'ty, Storage, Error> for ChainState<Storage>
{
    /// Append runner logs under the transaction, scheduled-call, or system caller hash.
    async fn set_contract_logs(
        &mut self,
        caller: ContractCaller<'a>,
        logs: ContractLogs,
    ) -> Result<()> {
        let hash = caller.get_hash().into_owned();

        match self.contract_logs.entry(hash) {
            Entry::Occupied(mut o) => {
                o.get_mut().extend(logs);
            }
            Entry::Vacant(e) => {
                e.insert(logs);
            }
        };

        Ok(())
    }

    /// Prepare versioned caches and prepaid deposits exactly at the network runner boundary.
    async fn get_contract_environment_for<'b>(
        &'b mut self,
        contract: Cow<'b, Hash>,
        deposits: Option<&'b IndexMap<Hash, ContractDeposit>>,
        caller: ContractCaller<'b>,
        permission: Cow<'b, InterContractPermission>,
    ) -> Result<(
        ContractEnvironment<'b, 'ty, Storage>,
        ContractChainState<'b>,
    )> {
        let contract_module = self.internal_load_contract_module(&contract)?;

        let mut cache = self
            .contract_caches
            .get(&contract)
            .map(|cache| cache.clone_with(self.block.get_version() < BlockVersion::V6))
            .unwrap_or_default();

        if let Some(deposits) = deposits {
            for (asset, deposit) in deposits.iter() {
                match deposit {
                    ContractDeposit::Public(amount) => match cache.balances.entry(asset.clone()) {
                        Entry::Occupied(mut o) => match o.get_mut() {
                            Some((state, balance)) => {
                                state.mark_updated();
                                *balance = balance
                                    .checked_add(*amount)
                                    .context("Overflow while applying contract deposit")?;
                            }

                            None => {
                                o.insert(Some((VersionedState::New, *amount)));
                            }
                        },
                        Entry::Vacant(e) => {
                            let (mut version, balance) = self
                                .provider
                                .get_contract_balance_for_asset(&contract, asset, self.topoheight)
                                .await?
                                .map(|(height, amount)| (VersionedState::FetchedAt(height), amount))
                                .unwrap_or((VersionedState::New, 0));
                            version.mark_updated();
                            e.insert(Some((
                                version,
                                balance
                                    .checked_add(*amount)
                                    .context("contract deposit overflow")?,
                            )));
                        }
                    },
                    ContractDeposit::Private { .. } => {}
                }
            }
        }

        let environment = ContractEnvironment {
            environment: &self.environments[&contract_module.version],
            module: &contract_module.module,
            version: contract_module.version,
            provider: &self.provider,
            _phantom: PhantomData,
        };

        let mut caches = HashMap::new();
        caches.insert(contract.clone().into_owned(), cache);

        let chain_state = ContractChainState {
            log_level: Level::Info,
            mainnet: self.mainnet,
            entry_contract: contract,
            topoheight: self.topoheight,
            block_hash: &self.block_hash,
            block: &self.block,
            block_version: self.block.get_version(),
            caller,
            logs: ContractLogs::default(),
            global_caches: &self.contract_caches,
            global_modules: &self.contracts,
            injected_gas: IndexMap::new(),
            executions: ExecutionsManager {
                allow_executions: true,
                global_executions: &self.executions.executions,
                changes: Default::default(),
            },
            changes: ChainStateChanges {
                caches,
                tracker: self.tracker.clone(),
                assets: self.assets.clone(),
                ..Default::default()
            },
            permission,
            gas_fee_allowance: 0,
            environments: Cow::Borrowed(&self.environments),
            loaded_modules: Default::default(),
        };

        Ok((environment, chain_state))
    }

    /// Retain modules loaded by the runner, including absent/deleted modules.
    async fn set_modules_cache(
        &mut self,
        modules: HashMap<Hash, Option<(VersionedState, Option<ContractModule>)>>,
    ) -> Result<()> {
        for (hash, value) in modules {
            self.contracts.insert(
                Cow::Owned(hash),
                value.map(|(state, module)| (state, module.map(Cow::Owned))),
            );
        }

        Ok(())
    }

    /// Merge only changes that the upstream runner accepted as successful.
    async fn merge_contract_changes(
        &mut self,
        changes: ChainStateChanges,
        mut executions_changes: ExecutionsChanges,
    ) -> Result<()> {
        for (contract, mut cache) in changes.caches {
            cache.clean_up();

            match self.contract_caches.entry(contract) {
                Entry::Occupied(mut o) => {
                    let current = o.get_mut();
                    *current = cache;
                }
                Entry::Vacant(e) => {
                    e.insert(cache);
                }
            };
        }

        self.assets = changes.assets;
        self.tracker = changes.tracker;
        self.events.extend(changes.events);

        for (key, mut listeners) in changes.events_listeners {
            match self.events_listeners.entry(key) {
                Entry::Occupied(mut o) => {
                    o.get_mut().append(&mut listeners);
                }
                Entry::Vacant(e) => {
                    e.insert(listeners);
                }
            };
        }

        for (hash, execution) in executions_changes.executions {
            self.executions.executions.insert(hash, execution);
        }

        self.executions
            .at_topoheight
            .append(&mut executions_changes.at_topoheight);
        self.executions
            .block_end
            .append(&mut executions_changes.block_end);

        self.add_gas_fee(changes.extra_gas_fee).await
    }

    /// Load the native-asset balance used by upstream gas injection and refund handling.
    async fn get_contract_balance_for_gas<'b>(
        &'b mut self,
        contract: &'b Hash,
    ) -> Result<&'b mut (VersionedState, u64)> {
        let cache = self.contract_caches.entry(contract.clone()).or_default();

        if let Entry::Vacant(entry) = cache.balances.entry(XELIS_ASSET) {
            let balance = self
                .provider
                .get_contract_balance_for_asset(contract, &XELIS_ASSET, self.topoheight)
                .await?
                .map(|(height, amount)| (VersionedState::FetchedAt(height), amount))
                .unwrap_or((VersionedState::New, 0));
            entry.insert(Some(balance));
        }
        cache
            .balances
            .get_mut(&XELIS_ASSET)
            .and_then(Option::as_mut)
            .context("Contract balance for gas not found")
    }

    /// Retain a deletion marker so later lookups cannot resurrect code from the provider.
    async fn remove_contract_module(&mut self, hash: &'a Hash) -> Result<()> {
        let mut version = VersionedState::FetchedAt(self.topoheight);
        version.mark_updated();
        self.contracts
            .insert(Cow::Owned(hash.clone()), Some((version, None)));

        Ok(())
    }

    /// Dispatch pending event callbacks after a successful upstream invocation.
    async fn post_contract_execution(
        &mut self,
        caller: &ContractCaller<'a>,
        _: &Hash,
    ) -> Result<()> {
        self.on_post_execution(caller.get_hash().as_ref()).await
    }
}
