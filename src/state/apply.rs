//! Checked accumulation of contract fees and burned assets.

use anyhow::{Context, Error, Result};
use async_trait::async_trait;
use xelis_common::{
    contract::ContractProvider, crypto::Hash, transaction::verify::BlockchainApplyState,
};

use super::ChainState;

#[async_trait]
impl<'a, 'ty, Storage: for<'p> ContractProvider<'p> + Send + Sync>
    BlockchainApplyState<'a, 'ty, Storage, Error> for ChainState<Storage>
{
    /// Accumulate burned asset amounts with overflow checking.
    async fn add_burned_coins(&mut self, asset: &Hash, amount: u64) -> Result<()> {
        let total = self.burned_coins.entry(asset.clone()).or_insert(0);
        *total = total.checked_add(amount).context("burned coins overflow")?;

        Ok(())
    }

    /// Accumulate miner gas fees with overflow checking.
    async fn add_gas_fee(&mut self, amount: u64) -> Result<()> {
        self.gas_fee = self
            .gas_fee
            .checked_add(amount)
            .context("gas fee overflow")?;

        Ok(())
    }

    /// Accumulate burned gas fees with overflow checking.
    async fn add_burned_fee(&mut self, amount: u64) -> Result<()> {
        self.burned_fee = self
            .burned_fee
            .checked_add(amount)
            .context("burned fee overflow")?;

        Ok(())
    }

    /// Expose the configured network flag to upstream state operations.
    fn is_mainnet(&self) -> bool {
        self.mainnet
    }
}
