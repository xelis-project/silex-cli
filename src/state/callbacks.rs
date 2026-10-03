//! Event callback dispatch after a successful contract invocation.

use std::borrow::Cow;

use anyhow::{Result, bail};
use log::warn;
use xelis_common::{
    contract::{
        ContractProvider, InterContractPermission,
        vm::{ContractCaller, InvokeContract, invoke_contract},
    },
    crypto::Hash,
    transaction::verify::BlockchainVerificationState,
};

use super::ChainState;

impl<Storage: for<'ty> ContractProvider<'ty> + Send + Sync> ChainState<Storage> {
    /// Drain emitted events and invoke their registered callbacks through the network runner.
    pub async fn on_post_execution(&mut self, caller: &Hash) -> Result<()> {
        while let Some(event) = self.events.pop_front() {
            let contract_key = (event.contract.clone(), event.event_id);

            if let Some(listeners) = self.events_listeners.remove(&contract_key) {
                for (contract, callback) in listeners {
                    if !self
                        .load_contract_module(Cow::Owned(contract.clone()))
                        .await?
                    {
                        bail!("contract module {contract} not found for event callback");
                    }

                    if let Err(e) = invoke_contract(
                        ContractCaller::EventCallback(
                            Cow::Owned(caller.clone()),
                            Cow::Owned(event.contract.clone()),
                        ),
                        self,
                        Cow::Owned(contract.clone()),
                        None,
                        event.params.iter().map(|p| p.deep_clone()),
                        callback.gas_sources,
                        callback.max_gas,
                        InvokeContract::Chunk(callback.chunk_id, false),
                        Cow::Owned(InterContractPermission::All),
                        false,
                    )
                    .await
                    {
                        warn!(
                            "failed to process execution of contract {} with caller {}: {}",
                            contract, caller, e
                        );
                    }
                }
            }
        }

        Ok(())
    }
}
