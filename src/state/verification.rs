//! Provider-backed account and module access at the contract runner boundary.

use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
};

use anyhow::{Context, Error, Result, bail};
use async_trait::async_trait;
use xelis_common::{
    account::Nonce,
    block::BlockVersion,
    contract::{ContractMetadata, ContractModule, ContractProvider, ContractVersion},
    crypto::{Hash, PublicKey, elgamal::Ciphertext},
    transaction::{MultiSigPayload, Reference, Transaction, verify::BlockchainVerificationState},
    versioned::VersionedState,
};
use xelis_vm::{Environment, Module};

use super::{Account, ChainState};

#[async_trait]
impl<'a, Storage: for<'ty> ContractProvider<'ty> + Send + Sync>
    BlockchainVerificationState<'a, Error> for ChainState<Storage>
{
    /// Calculate the unused transaction fee budget with checked arithmetic.
    async fn handle_tx_fee<'b>(&'b mut self, tx: &Transaction, _: &Hash) -> Result<u64> {
        tx.get_fee_limit()
            .checked_sub(tx.get_fee())
            .context("fee exceeds limit")
    }

    /// Reject transaction verification: this adapter executes already prepared calls only.
    async fn pre_verify_tx<'b>(&'b mut self, _: &Transaction) -> Result<()> {
        bail!("snapshot chain state does not verify transactions")
    }

    /// Load receiver funds through the provider, using zero only for absent accounts/assets.
    async fn get_receiver_balance<'b>(
        &'b mut self,
        account: Cow<'a, PublicKey>,
        asset: Cow<'a, Hash>,
    ) -> Result<&'b mut Ciphertext> {
        let account_state = self
            .accounts
            .entry(account.as_ref().clone())
            .or_insert_with(|| Account {
                balances: HashMap::new(),
                nonce: 0,
            });

        let balance = match account_state.balances.entry(asset.into_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let balance = match self
                    .provider
                    .get_account_balance_for_asset(&account, entry.key(), self.topoheight)
                    .await?
                {
                    Some((_, mut balance)) => balance.computable()?.clone(),
                    None => Ciphertext::zero(),
                };

                entry.insert(balance)
            }
        };

        Ok(balance)
    }

    /// Return an existing sender balance; transaction funding is prepared by the caller.
    async fn get_sender_balance<'b>(
        &'b mut self,
        account: &'a PublicKey,
        asset: &'a Hash,
        _: &Reference,
    ) -> Result<&'b mut Ciphertext> {
        self.accounts
            .get_mut(account)
            .and_then(|account| account.balances.get_mut(asset))
            .context("Sender account or balance not found")
    }

    /// Reject transaction verification outputs outside the contract runner boundary.
    async fn add_sender_output(
        &mut self,
        _: &'a PublicKey,
        _: &'a Hash,
        _: Ciphertext,
    ) -> Result<()> {
        bail!("snapshot chain state does not verify sender outputs")
    }

    /// Read a configured account nonce.
    async fn get_account_nonce(&mut self, account: &'a PublicKey) -> Result<Nonce> {
        self.accounts
            .get(account)
            .map(|account| account.nonce)
            .context("Account not found")
    }

    /// Update an account already loaded into this state.
    async fn update_account_nonce(
        &mut self,
        account: &'a PublicKey,
        new_nonce: Nonce,
    ) -> Result<()> {
        self.accounts
            .get_mut(account)
            .map(|account| account.nonce = new_nonce)
            .context("Account not found")
    }

    /// Use the supplied block version for network rule selection.
    fn get_block_version(&self) -> BlockVersion {
        self.block.get_version()
    }

    /// Update the execution-local multisig cache required by the upstream state trait.
    async fn set_multisig_state(
        &mut self,
        account: &'a PublicKey,
        multisig: &MultiSigPayload,
    ) -> Result<()> {
        self.multisig.insert(account.clone(), multisig.clone());

        Ok(())
    }

    /// Read an execution-local multisig record.
    async fn get_multisig_state(
        &mut self,
        account: &'a PublicKey,
    ) -> Result<Option<&MultiSigPayload>> {
        Ok(self.multisig.get(account))
    }

    /// Return native functions bound to this provider and contract version.
    async fn get_environment(
        &mut self,
        version: ContractVersion,
    ) -> Result<&Environment<ContractMetadata>> {
        Ok(&self.environments[&version])
    }

    /// Record a module update while preserving its versioned-state marker.
    async fn set_contract_module(
        &mut self,
        hash: &'a Hash,
        module: &'a ContractModule,
    ) -> Result<()> {
        match self.contracts.entry(Cow::Owned(hash.clone())) {
            Entry::Occupied(mut o) => match o.get_mut() {
                Some((state, m)) => {
                    state.mark_updated();
                    *m = Some(Cow::Owned(module.clone()));
                }

                None => {
                    o.insert(Some((
                        VersionedState::New,
                        Some(Cow::Owned(module.clone())),
                    )));
                }
            },
            Entry::Vacant(v) => {
                v.insert(Some((
                    VersionedState::New,
                    Some(Cow::Owned(module.clone())),
                )));
            }
        };

        Ok(())
    }

    /// Load a module lazily through the custom provider, caching misses and deletions.
    async fn load_contract_module(&mut self, hash: Cow<'a, Hash>) -> Result<bool> {
        if !self.contracts.contains_key(&hash) {
            let module = self
                .provider
                .load_contract_module(&hash, self.topoheight)
                .await?;
            self.contracts.insert(
                Cow::Owned(hash.as_ref().clone()),
                module.map(|(height, module)| {
                    (VersionedState::FetchedAt(height), module.map(Cow::Owned))
                }),
            );
        }

        Ok(self
            .contracts
            .get(&hash)
            .and_then(|value| value.as_ref())
            .is_some_and(|(_, module)| module.is_some()))
    }

    /// Determine whether a module was newly registered in this execution state.
    async fn is_contract_module_new(&mut self, hash: Cow<'a, Hash>) -> Result<bool> {
        Ok(self
            .contracts
            .get(&hash)
            .and_then(|contract| contract.as_ref())
            .is_some_and(|(state, module)| state.is_new() && module.is_some()))
    }

    /// Pair loaded bytecode with the matching versioned native environment.
    async fn get_contract_module_with_environment(
        &self,
        contract: &'a Hash,
    ) -> Result<(&Module, &Environment<ContractMetadata>)> {
        let module = self.internal_load_contract_module(contract)?;

        Ok((&module.module, &self.environments[&module.version]))
    }
}
