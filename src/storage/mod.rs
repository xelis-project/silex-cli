//! Pluggable storage, caller-owned snapshots, and validation.

mod provider;
mod snapshot;
mod types;

use std::collections::HashSet;

use anyhow::{Context, Error, Result, ensure};
use xelis_common::{
    block::TopoHeight,
    contract::{ContractLogs, ContractModule, ContractProvider, check_storage_entry_size},
    crypto::{Address, Hash, PublicKey},
    transaction::verify::BlockchainApplyState,
};

pub use types::{
    AccountBalance, AccountState, AssetState, CallbackState, ContractState, GasSource, JsonStorage,
    StorageEntry,
};

/// Persistence boundary around the network runner.
///
/// `begin` must create an isolated state. `commit` must apply its changes atomically.
/// Custom backends provide their own upstream `BlockchainApplyState` implementation;
/// they do not need to serialize through JSON or use the bundled fixture state.
pub trait ExecutionStorage: for<'ty> ContractProvider<'ty> + Sized {
    type State: for<'a, 'ty> BlockchainApplyState<'a, 'ty, Self, Error>;

    /// Validate backend configuration before accepting executions.
    fn validate(&self) -> Result<()>;

    /// Return the snapshot height used when loading registered modules.
    fn topoheight(&self) -> TopoHeight;

    /// Register a module without executing it or altering its data and balances.
    fn register(&mut self, hash: Hash, module: ContractModule) -> Result<()>;

    /// Create an isolated chain state exposing this provider to contract syscalls.
    fn begin(&self) -> Result<Self::State>;

    /// Atomically persist the state after the upstream runner returns a result.
    fn commit(&mut self, state: Self::State) -> Result<()>;

    /// Retrieve logs recorded by the chain state for the supplied caller hash.
    fn logs(state: &Self::State, caller: &Hash) -> ContractLogs;
}

impl JsonStorage {
    /// Reject malformed addresses, mismatched networks, and duplicate or invalid storage keys.
    pub fn validate(&self) -> Result<()> {
        let mut keys = HashSet::new();

        for address in self.accounts.keys() {
            let address = Address::from_string(address).context("invalid account address")?;

            ensure!(
                address.is_normal(),
                "account address must be a normal address"
            );

            ensure!(
                address.is_mainnet() == self.mainnet,
                "account address network differs from environment"
            );

            ensure!(
                keys.insert(address.to_public_key()),
                "duplicate account public key"
            );
        }

        for contract in self.contracts.values() {
            // Validated hashable keys are only borrowed here and never mutated in this set.
            #[allow(clippy::mutable_key_type)]
            let mut keys = HashSet::new();

            for entry in &contract.data {
                ensure!(entry.key.is_hashable(), "storage key must be hashable");
                check_storage_entry_size(&entry.key, &entry.value)?;

                ensure!(keys.insert(&entry.key), "duplicate contract storage key");
            }
        }

        Ok(())
    }

    /// Resolve a provider public key to its configured account address.
    fn account(&self, key: &PublicKey) -> Option<&AccountState> {
        self.accounts.iter().find_map(|(address, account)| {
            Address::from_string(address)
                .ok()
                .filter(|a| a.get_public_key() == key)
                .map(|_| account)
        })
    }
}
