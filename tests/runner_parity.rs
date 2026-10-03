//! Verify that the library forwards runner inputs and commits its resulting state.
use indexmap::IndexMap;
use silex_cli::storage::ContractState;
use silex_cli::xelis_common::{
    api::RPCContractLog,
    config::XELIS_ASSET,
    contract::{
        InterContractPermission, Source,
        vm::{ContractCaller, InvokeContract, invoke_contract},
    },
    crypto::Hash,
    transaction::ContractDeposit,
};
use silex_cli::{ExecutionStorage, Invocation, JsonStorage, Primitive, Simulator, compile_source};
use std::{borrow::Cow, collections::HashMap};

#[tokio::test]
async fn library_boundary_matches_direct_runner() {
    for (exit_code, gas_limit, scheduled) in [
        (0, 100_000, false),
        (1, 100_000, false),
        (0, 0, false),
        (0, 100_000, true),
    ] {
        let hash = Hash::new([1; 32]);
        let mut storage = JsonStorage::default();
        storage.contracts.insert(
            hash.clone(),
            ContractState {
                module: Some(
                    compile_source(
                        r#"entry main(code: u64) {
                Storage::new().store(b"key", 42u64); return code;
            }"#,
                    )
                    .unwrap()
                    .0,
                ),
                balances: [(XELIS_ASSET, 10_000_000)].into(),
                ..Default::default()
            },
        );
        let deposits = [(XELIS_ASSET, ContractDeposit::Public(100))].into();
        let decompressed = HashMap::new();
        let parameters = vec![Primitive::U64(exit_code).into()];
        let caller = if scheduled {
            ContractCaller::Scheduled(Cow::Owned(Hash::new([2; 32])), Cow::Owned(hash.clone()))
        } else {
            ContractCaller::System
        };
        let gas_sources: IndexMap<_, _> = if scheduled {
            [(Source::Contract(hash.clone()), gas_limit)].into()
        } else {
            IndexMap::new()
        };
        let mut direct_state = storage.begin().unwrap();
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
        let direct_logs: Vec<_> = JsonStorage::logs(&direct_state, &caller.get_hash())
            .into_iter()
            .map(|log| RPCContractLog::from_owned(log, false))
            .collect();

        let mut simulator = Simulator::new(storage.clone()).unwrap();
        let actual = simulator
            .invoke(Invocation {
                caller,
                contract: hash,
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
        storage.commit(direct_state).unwrap();
        assert_eq!(
            serde_json::to_value(actual.execution).unwrap(),
            serde_json::to_value(direct).unwrap()
        );
        assert_eq!(
            serde_json::to_value(actual.logs).unwrap(),
            serde_json::to_value(direct_logs).unwrap()
        );
        assert_eq!(
            serde_json::to_value(simulator.storage()).unwrap(),
            serde_json::to_value(storage).unwrap()
        );
    }
}
