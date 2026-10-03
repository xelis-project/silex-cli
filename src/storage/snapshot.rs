//! Conversion between JSON snapshots and the upstream chain-state interfaces.

use super::{
    AccountBalance, AccountState, AssetState, CallbackState, ExecutionStorage, GasSource,
    JsonStorage, StorageEntry,
};

use crate::{
    contract::environment,
    state::{Account, ChainState},
};

use anyhow::{Context, Result};
use std::collections::HashMap;
use xelis_common::{
    contract::{
        AssetChanges, ContractLogs, ContractModule, EventCallbackRegistration,
        ScheduledExecutionKind,
    },
    crypto::{Address, Hash},
    serializer::Serializer,
    versioned::VersionedState,
};

use xelis_vm::ModuleValidator;

impl ExecutionStorage for JsonStorage {
    type State = ChainState;

    /// Use the configured JSON snapshot height for provider reads.
    fn topoheight(&self) -> u64 {
        self.topoheight
    }

    /// Validate addresses and typed storage keys before creating an execution state.
    fn validate(&self) -> Result<()> {
        JsonStorage::validate(self)
    }

    /// Install a module while preserving its configured balances and data.
    fn register(&mut self, hash: Hash, module: ContractModule) -> Result<()> {
        self.contracts.entry(hash).or_default().module = Some(module);

        Ok(())
    }

    /// Load a JSON snapshot into an isolated chain state for the upstream runner.
    fn begin(&self) -> Result<ChainState> {
        let mut state = ChainState::new(self.clone());
        state.mainnet = self.mainnet;
        state.topoheight = self.topoheight;
        state.block_hash = self.block_hash.clone();

        if let Some(block) = &self.block {
            state.block = block.clone();
        }

        for (hash, contract) in &self.contracts {
            if let Some(module) = &contract.module {
                ModuleValidator::new(&module.module, &environment(module.version))
                    .verify()
                    .with_context(|| format!("invalid registered module {hash}"))?;
                state.internal_set_contract_module(hash.clone(), module.clone());
            }

            for (asset, balance) in &contract.balances {
                state.set_contract_balance(hash, asset, *balance);
            }
        }

        for (hash, asset) in &self.assets {
            state.assets.insert(
                hash.clone(),
                Some(AssetChanges {
                    data: (
                        VersionedState::FetchedAt(self.topoheight),
                        asset.data.clone(),
                    ),
                    circulating_supply: (
                        VersionedState::FetchedAt(self.topoheight),
                        asset.circulating_supply,
                    ),
                }),
            );
        }

        for (address, account) in &self.accounts {
            state.accounts.insert(
                Address::from_string(address)?.to_public_key(),
                Account {
                    nonce: account.nonce,
                    balances: account
                        .balances
                        .iter()
                        .map(|(h, b)| (h.clone(), b.ciphertext()))
                        .collect(),
                },
            );
        }

        for callback in &self.callbacks {
            state
                .events_listeners
                .entry((callback.contract.clone(), callback.event_id))
                .or_default()
                .push((
                    callback.listener.clone(),
                    EventCallbackRegistration {
                        chunk_id: callback.chunk_id,
                        max_gas: callback.max_gas,
                        gas_sources: callback
                            .gas_sources
                            .iter()
                            .map(|s| (s.source.clone(), s.amount))
                            .collect(),
                    },
                ));
        }

        for execution in &self.scheduled_executions {
            if matches!(execution.kind, ScheduledExecutionKind::BlockEnd) {
                state.executions.block_end.push(execution.hash.clone());
            }
            state
                .executions
                .executions
                .insert(execution.hash.clone(), execution.clone());
        }

        Ok(state)
    }

    /// Persist the state produced by the upstream runner, including failed-call gas effects.
    fn commit(&mut self, state: Self::State) -> Result<()> {
        *self = self.snapshot(&state)?;

        Ok(())
    }

    /// Return logs attributed to this invocation's caller hash.
    fn logs(state: &Self::State, caller: &Hash) -> ContractLogs {
        state.contract_logs.get(caller).cloned().unwrap_or_default()
    }
}

impl JsonStorage {
    /// Apply caches and public transfers to a new snapshot before replacing the old one.
    fn snapshot(&self, state: &ChainState) -> Result<JsonStorage> {
        let mut storage = self.clone();

        for (hash, module) in &state.contracts {
            storage
                .contracts
                .entry(hash.as_ref().clone())
                .or_default()
                .module = module
                .as_ref()
                .and_then(|(_, m)| m.as_ref().map(|m| m.as_ref().clone()));
        }

        for hash in self.contracts.keys() {
            if !state.contracts.contains_key(hash) {
                storage.contracts.get_mut(hash).unwrap().module = None;
            }
        }

        for (hash, cache) in &state.contract_caches {
            let contract = storage.contracts.entry(hash.clone()).or_default();

            for (asset, balance) in &cache.balances {
                if let Some((_, amount)) = balance {
                    contract.balances.insert(asset.clone(), *amount);
                }
            }

            for (key, entry) in &cache.storage {
                if let Some((_, value)) = entry {
                    contract.data.retain(|e| &e.key != key);

                    if let Some(value) = value {
                        contract.data.push(StorageEntry {
                            key: key.clone(),
                            value: value.clone(),
                        });
                    }
                }
            }
        }

        for (hash, asset) in &state.assets {
            if let Some(asset) = asset {
                storage.assets.insert(
                    hash.clone(),
                    AssetState {
                        data: asset.data.1.clone(),
                        circulating_supply: asset.circulating_supply.1,
                    },
                );
            }
        }

        // The runner aggregates public transfers; the enclosing blockchain normally applies them.
        let mut accounts = state.accounts.clone();

        for (key, transfers) in &state.tracker.aggregated_transfers {
            let account = accounts.entry(key.clone()).or_insert_with(|| Account {
                nonce: 0,
                balances: HashMap::new(),
            });

            for (asset, amount) in transfers {
                *account
                    .balances
                    .entry(asset.clone())
                    .or_insert_with(xelis_common::crypto::elgamal::Ciphertext::zero) += *amount;
            }
        }

        for (key, account) in accounts {
            let address =
                Address::new(storage.mainnet, Default::default(), key.clone()).to_string();

            let old = storage.accounts.get(&address);
            let balances = account
                .balances
                .into_iter()
                .map(|(asset, ciphertext)| {
                    let amount = match old.and_then(|a| a.balances.get(&asset)) {
                        Some(AccountBalance::Amount(n)) => Some(*n),
                        None => Some(0),
                        _ => None,
                    }
                    .map(|n| {
                        n.checked_add(
                            state
                                .tracker
                                .aggregated_transfers
                                .get(&key)
                                .and_then(|a| a.get(&asset))
                                .copied()
                                .unwrap_or(0),
                        )
                        .context("account balance overflow")
                    })
                    .transpose()?;

                    let balance = match amount {
                        Some(n) if AccountBalance::Amount(n).ciphertext() == ciphertext => {
                            AccountBalance::Amount(n)
                        }
                        _ => AccountBalance::Ciphertext(Box::new(ciphertext)),
                    };

                    Ok((asset, balance))
                })
                .collect::<Result<_>>()?;
            storage.accounts.insert(
                address,
                AccountState {
                    nonce: account.nonce,
                    balances,
                },
            );
        }
        storage.callbacks = state
            .events_listeners
            .iter()
            .flat_map(|((contract, event_id), listeners)| {
                listeners
                    .iter()
                    .map(move |(listener, registration)| CallbackState {
                        contract: contract.clone(),
                        event_id: *event_id,
                        listener: listener.clone(),
                        chunk_id: registration.chunk_id,
                        max_gas: registration.max_gas,
                        gas_sources: registration
                            .gas_sources
                            .iter()
                            .map(|(source, amount)| GasSource {
                                source: source.clone(),
                                amount: *amount,
                            })
                            .collect(),
                    })
            })
            .collect();
        storage.scheduled_executions = state.executions.executions.values().cloned().collect();

        // HashMap iteration must not affect saved fixtures or deterministic replay comparisons.
        for contract in storage.contracts.values_mut() {
            contract
                .data
                .sort_by_cached_key(|entry| entry.key.to_bytes());
        }

        storage.callbacks.sort_by(|left, right| {
            (&left.contract, left.event_id).cmp(&(&right.contract, right.event_id))
        });
        storage
            .scheduled_executions
            .sort_by(|left, right| left.hash.cmp(&right.hash));

        // Persist block-end registrations in their execution order, including standalone runs.
        storage.scheduled_executions.sort_by_key(|execution| {
            state
                .executions
                .block_end
                .iter()
                .position(|hash| hash == &execution.hash)
                .map(|position| (0, position))
                .unwrap_or((1, 0))
        });

        storage.validate()?;

        Ok(storage)
    }
}
