//! Compare our state adapter with the independent upstream mock chain state.
use indexmap::IndexMap;
use silex_cli::storage::{ContractState, StorageEntry};
use silex_cli::xelis_common::{
    api::RPCContractLog,
    block::BlockVersion,
    config::XELIS_ASSET,
    contract::{
        InterContractPermission,
        vm::{ContractCaller, InvokeContract, invoke_contract},
    },
    crypto::Hash,
    serializer::Serializer,
    transaction::mock::MockChainState,
};
use silex_cli::{Invocation, JsonStorage, Primitive, Simulator, ValueCell, compile_source};
use std::{borrow::Cow, collections::HashMap};

#[tokio::test]
async fn results_logs_and_state_match_upstream_mock_chain_state() {
    for (exit_code, gas_limit, initial) in [
        (0, 100_000, None),
        (0, 100_000, Some(7)),
        (1, 100_000, Some(7)),
        (0, 0, Some(7)),
    ] {
        let hash = Hash::new([1; 32]);
        let mut storage = JsonStorage::default();
        storage.contracts.insert(
            hash.clone(),
            ContractState {
                module: Some(
                    compile_source(
                        r#"entry main(code: u64) {
                let s = Storage::new();
                let previous: u64 = s.load("key").unwrap_or(0u64);
                s.store("key", previous + 42u64); return code;
            }"#,
                    )
                    .unwrap()
                    .0,
                ),
                ..Default::default()
            },
        );
        let key: ValueCell = Primitive::String("key".into()).into();
        if let Some(value) = initial {
            storage
                .contracts
                .get_mut(&hash)
                .unwrap()
                .data
                .push(StorageEntry {
                    key: key.clone(),
                    value: Primitive::U64(value).into(),
                });
        }
        let deposits = IndexMap::new();
        let decompressed = HashMap::new();
        let parameters = vec![Primitive::U64(exit_code).into()];
        let caller = ContractCaller::System;
        let gas_sources: IndexMap<_, _> = IndexMap::new();
        let mut direct_state = MockChainState::with(BlockVersion::V7);
        direct_state.internal_set_contract_module(
            hash.clone(),
            storage.contracts[&hash].module.clone().unwrap(),
        );
        if let Some(value) = initial {
            direct_state
                .provider
                .contracts
                .entry(hash.clone())
                .or_default()
                .insert(key.clone(), (0, Some(Primitive::U64(value).into())));
        }
        let direct = invoke_contract(
            caller.clone(),
            &mut direct_state,
            Cow::Owned(hash.clone()),
            Some((&deposits, &decompressed)),
            parameters.clone().into_iter(),
            gas_sources.clone(),
            gas_limit,
            InvokeContract::Entry(0),
            Cow::Owned(InterContractPermission::default()),
            true,
        )
        .await
        .unwrap();
        let direct_logs: Vec<_> = direct_state
            .contract_logs
            .get(&caller.get_hash())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|log| RPCContractLog::from_owned(log, false))
            .collect();

        let mut simulator = Simulator::new(storage.clone()).unwrap();
        let actual = simulator
            .invoke(Invocation {
                caller,
                contract: hash.clone(),
                deposits: Some((&deposits, &decompressed)),
                arguments: parameters,
                gas_sources,
                max_gas: gas_limit,
                invoke: InvokeContract::Entry(0),
                permission: InterContractPermission::default(),
                post_execution: true,
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual.execution).unwrap(),
            serde_json::to_value(direct).unwrap()
        );
        assert_eq!(
            serde_json::to_value(actual.logs).unwrap(),
            serde_json::to_value(direct_logs).unwrap()
        );
        let actual_contract = &simulator.storage().contracts[&hash];
        assert_eq!(
            actual_contract
                .balances
                .get(&XELIS_ASSET)
                .copied()
                .unwrap_or(0),
            direct_state.get_contract_balance(&hash, &XELIS_ASSET)
        );
        let mut direct_data: HashMap<_, _> = direct_state
            .provider
            .contracts
            .get(&hash)
            .into_iter()
            .flat_map(|data| data.iter())
            .filter_map(|(key, (_, value))| {
                value.as_ref().map(|value| (key.to_bytes(), value.clone()))
            })
            .collect();
        if let Some(cache) = direct_state.contract_caches.get(&hash) {
            for (key, entry) in &cache.storage {
                if let Some(value) = entry.as_ref().and_then(|(_, value)| value.as_ref()) {
                    direct_data.insert(key.to_bytes(), value.clone());
                } else {
                    direct_data.remove(&key.to_bytes());
                }
            }
        }
        let actual_data: HashMap<_, _> = actual_contract
            .data
            .iter()
            .map(|entry| (entry.key.to_bytes(), entry.value.clone()))
            .collect();
        assert_eq!(actual_data, direct_data);
        if exit_code == 0 && gas_limit > 0 {
            assert_eq!(actual_data.len(), 1);
            assert_eq!(
                actual_data.values().next().unwrap().as_u64().unwrap(),
                initial.unwrap_or(0) + 42
            );
        } else {
            assert_eq!(
                actual_data
                    .get(&key.to_bytes())
                    .map(|value| value.as_u64().unwrap()),
                initial,
                "Failed calls must preserve the initial state"
            );
        }
    }
}
