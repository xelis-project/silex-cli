//! Proves that native functions use the caller's provider type, without JSON conversion.
use anyhow::Result;
use async_trait::async_trait;
use silex_cli::state::ChainState;
use silex_cli::storage::{AccountBalance, AccountState, ContractState, StorageEntry};
use silex_cli::xelis_common::{
    account::CiphertextCache,
    asset::AssetData,
    block::TopoHeight,
    config::XELIS_ASSET,
    contract::{ContractLogs, ContractModule, ContractProvider, ContractStorage},
    crypto::{Hash, KeyPair, PublicKey},
    transaction::verify::BlockchainVerificationState,
};
use silex_cli::{ExecutionStorage, JsonStorage, Primitive, RunOptions, Simulator, ValueCell};
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct CustomStorage {
    inner: JsonStorage,
    reads: Arc<AtomicUsize>,
    commits: usize,
}
xelis_vm::tid!(CustomStorage);

#[async_trait]
impl ContractStorage for CustomStorage {
    async fn load_data(
        &self,
        contract: &Hash,
        key: &ValueCell,
        topoheight: TopoHeight,
    ) -> Result<Option<(TopoHeight, Option<ValueCell>)>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.load_data(contract, key, topoheight).await
    }
    async fn load_data_latest_topoheight(
        &self,
        contract: &Hash,
        key: &ValueCell,
        topoheight: TopoHeight,
    ) -> Result<Option<TopoHeight>> {
        self.inner
            .load_data_latest_topoheight(contract, key, topoheight)
            .await
    }
    async fn has_contract(&self, contract: &Hash, topoheight: TopoHeight) -> Result<bool> {
        self.inner.has_contract(contract, topoheight).await
    }
}

#[async_trait]
impl<'ty> ContractProvider<'ty> for CustomStorage {
    async fn get_contract_balance_for_asset(
        &self,
        contract: &Hash,
        asset: &Hash,
        topoheight: TopoHeight,
    ) -> Result<Option<(TopoHeight, u64)>> {
        self.inner
            .get_contract_balance_for_asset(contract, asset, topoheight)
            .await
    }
    async fn get_account_balance_for_asset(
        &self,
        key: &PublicKey,
        asset: &Hash,
        topoheight: TopoHeight,
    ) -> Result<Option<(TopoHeight, CiphertextCache)>> {
        self.inner
            .get_account_balance_for_asset(key, asset, topoheight)
            .await
    }
    async fn has_scheduled_execution_at_topoheight(
        &self,
        contract: &Hash,
        topoheight: TopoHeight,
    ) -> Result<bool> {
        self.inner
            .has_scheduled_execution_at_topoheight(contract, topoheight)
            .await
    }
    async fn asset_exists(&self, asset: &Hash, topoheight: TopoHeight) -> Result<bool> {
        self.inner.asset_exists(asset, topoheight).await
    }
    async fn load_asset_data(
        &self,
        asset: &Hash,
        topoheight: TopoHeight,
    ) -> Result<Option<(TopoHeight, AssetData)>> {
        self.inner.load_asset_data(asset, topoheight).await
    }
    async fn load_asset_circulating_supply(
        &self,
        asset: &Hash,
        topoheight: TopoHeight,
    ) -> Result<(TopoHeight, u64)> {
        self.inner
            .load_asset_circulating_supply(asset, topoheight)
            .await
    }
    async fn account_exists(&self, key: &PublicKey, topoheight: TopoHeight) -> Result<bool> {
        self.inner.account_exists(key, topoheight).await
    }
    async fn load_contract_module(
        &self,
        contract: &Hash,
        topoheight: TopoHeight,
    ) -> Result<Option<(TopoHeight, Option<ContractModule>)>> {
        self.inner.load_contract_module(contract, topoheight).await
    }
    async fn has_contract_callback_for_event(
        &self,
        contract: &Hash,
        event_id: u64,
        listener: &Hash,
        topoheight: TopoHeight,
    ) -> Result<bool> {
        self.inner
            .has_contract_callback_for_event(contract, event_id, listener, topoheight)
            .await
    }
}

impl ExecutionStorage for CustomStorage {
    type State = ChainState<Self>;
    fn validate(&self) -> Result<()> {
        self.inner.validate()
    }
    fn topoheight(&self) -> u64 {
        self.inner.topoheight
    }
    fn register(&mut self, hash: Hash, module: ContractModule) -> Result<()> {
        self.inner.contracts.entry(hash).or_default().module = Some(module);
        Ok(())
    }
    fn begin(&self) -> Result<Self::State> {
        let mut state = ChainState::new(self.clone());
        state.topoheight = self.inner.topoheight;
        Ok(state)
    }
    fn commit(&mut self, state: Self::State) -> Result<()> {
        for (hash, cache) in state.contract_caches {
            for (key, entry) in cache.storage {
                if let Some((_, value)) = entry {
                    let contract = self.inner.contracts.entry(hash.clone()).or_default();
                    contract.data.retain(|entry| entry.key != key);
                    if let Some(value) = value {
                        contract.data.push(StorageEntry { key, value });
                    }
                }
            }
        }
        self.commits += 1;
        Ok(())
    }
    fn logs(state: &Self::State, caller: &Hash) -> ContractLogs {
        state.contract_logs.get(caller).cloned().unwrap_or_default()
    }
}

#[tokio::test]
async fn execution_uses_custom_storage_and_its_commit_boundary() {
    let hash = Hash::zero();
    let mut inner = JsonStorage::default();
    inner.contracts.insert(
        hash.clone(),
        ContractState {
            data: vec![StorageEntry {
                key: ValueCell::Bytes(b"answer".to_vec()),
                value: Primitive::U64(42).into(),
            }],
            ..Default::default()
        },
    );
    let reads = Arc::new(AtomicUsize::new(0));
    let mut simulator = Simulator::new(CustomStorage {
        inner,
        reads: reads.clone(),
        commits: 0,
    })
    .unwrap();
    let result = simulator
        .run_source(
            hash,
            r#"
        entry main() { let answer: u64 = Storage::new().load(b"answer").unwrap();
            require(answer == 42, "custom provider"); return 0; }
    "#,
            RunOptions::default(),
        )
        .await
        .unwrap();
    assert!(result.execution.is_success(), "{:?}", result.execution);
    assert!(reads.load(Ordering::Relaxed) > 0);
    assert_eq!(simulator.storage().commits, 1);
}

#[tokio::test]
async fn lazy_receiver_loading_preserves_custom_provider_funds() {
    let key = KeyPair::new().get_public_key().compress();
    let mut inner = JsonStorage::default();
    inner.accounts.insert(
        key.as_address(false).to_string(),
        AccountState {
            balances: [(XELIS_ASSET, AccountBalance::Amount(123))].into(),
            ..Default::default()
        },
    );
    let storage = CustomStorage {
        inner,
        reads: Arc::new(AtomicUsize::new(0)),
        commits: 0,
    };
    let mut state = storage.begin().unwrap();
    let balance = state
        .get_receiver_balance(Cow::Owned(key), Cow::Owned(XELIS_ASSET))
        .await
        .unwrap();
    assert_eq!(*balance, AccountBalance::Amount(123).ciphertext());
}
