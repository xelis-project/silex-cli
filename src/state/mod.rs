//! Chain-state adapter for snapshot backends.
//! Execution, rollback, refunds, and gas accounting use the upstream contract runner.

mod apply;
mod callbacks;
mod contracts;
mod verification;

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque, hash_map::Entry},
    sync::Arc,
};

use anyhow::{Context, Result};
use indexmap::IndexSet;
use xelis_common::{
    account::Nonce,
    block::{Block, BlockHeader, BlockVersion, EXTRA_NONCE_SIZE},
    contract::{
        AssetChanges, CallbackEvent, ContractCache, ContractEnvironments, ContractEventTracker,
        ContractLogs, ContractModule, ContractProvider, ContractVersion, EventCallbackRegistration,
        ExecutionsChanges, build_environment,
    },
    crypto::{
        Hash, PublicKey,
        elgamal::{Ciphertext, CompressedPublicKey},
    },
    serializer::Serializer,
    transaction::MultiSigPayload,
    versioned::VersionedState,
};

use crate::storage::JsonStorage;

#[derive(Debug, Clone)]
pub struct Account {
    pub balances: HashMap<Hash, Ciphertext>,
    pub nonce: Nonce,
}

type ModuleCache =
    HashMap<Cow<'static, Hash>, Option<(VersionedState, Option<Cow<'static, ContractModule>>)>>;

#[derive(Debug, Clone)]
pub struct ChainState<Storage = JsonStorage> {
    pub assets: HashMap<Hash, Option<AssetChanges>>,
    pub tracker: ContractEventTracker,
    pub events: VecDeque<CallbackEvent>,
    pub events_listeners: HashMap<(Hash, u64), Vec<(Hash, EventCallbackRegistration)>>,

    pub accounts: HashMap<PublicKey, Account>,
    pub multisig: HashMap<PublicKey, MultiSigPayload>,
    pub contracts: ModuleCache,
    pub contract_logs: HashMap<Hash, ContractLogs>,

    pub burned_coins: HashMap<Hash, u64>,
    pub gas_fee: u64,
    pub burned_fee: u64,

    pub environments: ContractEnvironments,
    pub provider: Storage,

    pub mainnet: bool,
    pub topoheight: u64,
    pub block_hash: Hash,
    pub block: Block,

    pub contract_caches: HashMap<Hash, ContractCache>,
    pub executions: ExecutionsChanges,
}

impl<Storage: for<'ty> ContractProvider<'ty> + Send + Sync> ChainState<Storage> {
    /// Create an empty snapshot adapter with native functions bound to `Storage`.
    pub fn with(provider: Storage, version: BlockVersion) -> Self {
        let header = BlockHeader::new(
            version,
            0,
            0,
            IndexSet::new(),
            [0u8; EXTRA_NONCE_SIZE],
            CompressedPublicKey::from_bytes(&[0; 32]).expect("identity public key"),
            IndexSet::new(),
        );

        Self {
            assets: HashMap::new(),
            tracker: Default::default(),
            events: VecDeque::new(),
            events_listeners: HashMap::new(),
            accounts: HashMap::new(),
            multisig: HashMap::new(),
            contracts: HashMap::new(),
            contract_logs: HashMap::new(),
            burned_coins: HashMap::new(),
            gas_fee: 0,
            burned_fee: 0,
            environments: ContractVersion::variants()
                .into_iter()
                .map(|version| {
                    (
                        version,
                        Arc::new(build_environment::<Storage>(version).build()),
                    )
                })
                .collect(),
            provider,
            mainnet: false,
            topoheight: 1,
            block_hash: Hash::zero(),
            block: Block::new(header, Vec::new()),
            contract_caches: HashMap::new(),
            executions: ExecutionsChanges::default(),
        }
    }

    /// Create an empty adapter using the current V7 block rules.
    pub fn new(provider: Storage) -> Self {
        Self::with(provider, BlockVersion::V7)
    }

    /// Seed or replace a public contract balance in the execution cache.
    pub fn set_contract_balance(&mut self, contract: &Hash, asset: &Hash, new_balance: u64) {
        let cache = self.contract_caches.entry(contract.clone()).or_default();

        match cache.balances.entry(asset.clone()) {
            Entry::Occupied(mut o) => match o.get_mut() {
                Some((state, balance)) => {
                    state.mark_updated();
                    *balance = new_balance;
                }

                None => {
                    o.insert(Some((VersionedState::New, new_balance)));
                }
            },
            Entry::Vacant(v) => {
                v.insert(Some((VersionedState::New, new_balance)));
            }
        }
    }

    /// Return an already loaded module or an error for an absent/deleted contract.
    fn internal_load_contract_module(&self, hash: &Hash) -> Result<&ContractModule> {
        self.contracts
            .get(hash)
            .context("Contract module not found")?
            .as_ref()
            .context("Contract module not loaded")?
            .1
            .as_ref()
            .context("Contract module not available")
            .map(|m| m.as_ref())
    }

    /// Seed a module cache entry at the configured snapshot height.
    pub fn internal_set_contract_module(&mut self, hash: Hash, module: ContractModule) {
        self.contracts.insert(
            Cow::Owned(hash),
            Some((
                VersionedState::FetchedAt(self.topoheight),
                Some(Cow::Owned(module)),
            )),
        );
    }
}
