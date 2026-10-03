//! Read-only access to the caller's snapshot for upstream contract syscalls.

use anyhow::{Context, Result};
use async_trait::async_trait;
use silex_types::ValueCell;
use xelis_common::{
    account::CiphertextCache,
    asset::AssetData,
    block::TopoHeight,
    contract::{ContractModule, ContractProvider, ContractStorage, ScheduledExecutionKind},
    crypto::{Hash, PublicKey},
};

use super::JsonStorage;

#[async_trait]
impl ContractStorage for JsonStorage {
    /// Look up a typed key in the configured snapshot, without historical version lookup.
    async fn load_data(
        &self,
        contract: &Hash,
        key: &ValueCell,
        _: TopoHeight,
    ) -> Result<Option<(TopoHeight, Option<ValueCell>)>> {
        Ok(self
            .contracts
            .get(contract)
            .and_then(|c| c.data.iter().find(|v| &v.key == key))
            .map(|v| (self.topoheight, Some(v.value.clone()))))
    }

    /// Return this snapshot's height when the requested key exists.
    async fn load_data_latest_topoheight(
        &self,
        contract: &Hash,
        key: &ValueCell,
        topoheight: TopoHeight,
    ) -> Result<Option<TopoHeight>> {
        Ok(self
            .load_data(contract, key, topoheight)
            .await?
            .map(|(height, _)| height))
    }

    /// Report whether this snapshot contains executable code for the contract.
    async fn has_contract(&self, contract: &Hash, _: TopoHeight) -> Result<bool> {
        Ok(self
            .contracts
            .get(contract)
            .is_some_and(|c| c.module.is_some()))
    }
}

#[async_trait]
impl<'ty> ContractProvider<'ty> for JsonStorage {
    /// Read the public contract balance for a configured asset.
    async fn get_contract_balance_for_asset(
        &self,
        contract: &Hash,
        asset: &Hash,
        _: TopoHeight,
    ) -> Result<Option<(TopoHeight, u64)>> {
        Ok(self
            .contracts
            .get(contract)
            .and_then(|c| c.balances.get(asset))
            .map(|n| (self.topoheight, *n)))
    }

    /// Expose configured account funds in the network's ciphertext representation.
    async fn get_account_balance_for_asset(
        &self,
        key: &PublicKey,
        asset: &Hash,
        _: TopoHeight,
    ) -> Result<Option<(TopoHeight, CiphertextCache)>> {
        Ok(self
            .account(key)
            .and_then(|a| a.balances.get(asset))
            .map(|b| {
                (
                    self.topoheight,
                    CiphertextCache::Decompressed(None, b.ciphertext()),
                )
            }))
    }

    /// Check configured pending executions for a contract and target height.
    async fn has_scheduled_execution_at_topoheight(
        &self,
        contract: &Hash,
        topoheight: TopoHeight,
    ) -> Result<bool> {
        Ok(self.scheduled_executions.iter().any(|execution| {
            &execution.contract == contract
                && matches!(
                    execution.kind,
                    ScheduledExecutionKind::TopoHeight { execution_topoheight, .. }
                        if execution_topoheight == topoheight
                )
        }))
    }

    /// Report whether the asset is registered in the snapshot.
    async fn asset_exists(&self, asset: &Hash, _: TopoHeight) -> Result<bool> {
        Ok(self.assets.contains_key(asset))
    }

    /// Return configured asset metadata together with the snapshot height.
    async fn load_asset_data(
        &self,
        asset: &Hash,
        _: TopoHeight,
    ) -> Result<Option<(TopoHeight, AssetData)>> {
        Ok(self
            .assets
            .get(asset)
            .map(|a| (self.topoheight, a.data.clone())))
    }

    /// Read the configured supply, returning an error for an unknown asset.
    async fn load_asset_circulating_supply(
        &self,
        asset: &Hash,
        _: TopoHeight,
    ) -> Result<(TopoHeight, u64)> {
        Ok((
            self.topoheight,
            self.assets
                .get(asset)
                .context("asset does not exist")?
                .circulating_supply,
        ))
    }

    /// Check registration independently of whether the account has a balance.
    async fn account_exists(&self, key: &PublicKey, _: TopoHeight) -> Result<bool> {
        Ok(self.account(key).is_some())
    }

    /// Load the registered module or its explicit deletion marker.
    async fn load_contract_module(
        &self,
        contract: &Hash,
        _: TopoHeight,
    ) -> Result<Option<(TopoHeight, Option<ContractModule>)>> {
        Ok(self
            .contracts
            .get(contract)
            .map(|c| (self.topoheight, c.module.clone())))
    }

    /// Check whether the listener is registered for the contract's event.
    async fn has_contract_callback_for_event(
        &self,
        contract: &Hash,
        event_id: u64,
        listener: &Hash,
        _: TopoHeight,
    ) -> Result<bool> {
        Ok(self
            .callbacks
            .iter()
            .any(|c| &c.contract == contract && c.event_id == event_id && &c.listener == listener))
    }
}
