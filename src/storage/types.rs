//! Serializable, caller-owned simulation state.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use silex_types::ValueCell;
use xelis_common::{
    asset::{AssetData, AssetOwner, MaxSupplyMode},
    block::{Block, TopoHeight},
    config::XELIS_ASSET,
    contract::{ContractModule, ScheduledExecution, Source},
    crypto::{Hash, elgamal::Ciphertext},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JsonStorage {
    pub mainnet: bool,
    pub topoheight: TopoHeight,
    pub block_hash: Hash,

    /// Optional complete block; omitted means a deterministic empty V7 block.
    pub block: Option<Block>,

    pub assets: BTreeMap<Hash, AssetState>,
    pub accounts: BTreeMap<String, AccountState>,
    pub contracts: BTreeMap<Hash, ContractState>,

    pub scheduled_executions: Vec<ScheduledExecution>,
    pub callbacks: Vec<CallbackState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetState {
    pub data: AssetData,
    #[serde(default)]
    pub circulating_supply: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AccountState {
    pub nonce: u64,
    pub balances: BTreeMap<Hash, AccountBalance>,
}

/// Plain amounts are converted to zero-blinding ciphertexts for local simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AccountBalance {
    Amount(u64),
    Ciphertext(Box<Ciphertext>),
}

impl AccountBalance {
    /// Convert a fixture amount or stored ciphertext into the account balance representation.
    pub fn ciphertext(&self) -> Ciphertext {
        match self {
            Self::Amount(n) => Ciphertext::zero() + *n,
            Self::Ciphertext(c) => c.as_ref().clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ContractState {
    pub module: Option<ContractModule>,
    pub balances: BTreeMap<Hash, u64>,

    /// Typed keys cannot be JSON object keys, so storage uses key/value records.
    pub data: Vec<StorageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageEntry {
    pub key: ValueCell,
    pub value: ValueCell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackState {
    pub contract: Hash,
    pub event_id: u64,
    pub listener: Hash,
    pub chunk_id: u16,
    pub max_gas: u64,
    pub gas_sources: Vec<GasSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasSource {
    pub source: Source,
    pub amount: u64,
}

xelis_vm::tid!(JsonStorage);

impl Default for JsonStorage {
    fn default() -> Self {
        Self {
            mainnet: false,
            topoheight: 1,
            block_hash: Hash::zero(),
            block: None,
            assets: [(
                XELIS_ASSET,
                AssetState {
                    data: AssetData::new(
                        8,
                        "XELIS".into(),
                        "XELIS".into(),
                        MaxSupplyMode::None,
                        AssetOwner::None,
                    ),
                    circulating_supply: 0,
                },
            )]
            .into(),
            accounts: BTreeMap::new(),
            contracts: BTreeMap::new(),
            scheduled_executions: vec![],
            callbacks: vec![],
        }
    }
}
