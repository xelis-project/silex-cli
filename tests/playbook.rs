use std::{borrow::Cow, sync::Arc};

use silex_cli::storage::ContractState;
use silex_cli::xelis_common::{
    config::XELIS_ASSET,
    contract::{
        InterContractPermission, ScheduledExecution, ScheduledExecutionKind, Source,
        vm::{ContractCaller, InvokeContract},
    },
    crypto::Hash,
};
use silex_cli::{
    ExecutionStorage, Invocation, JsonStorage, Playbook, Primitive, Simulator, ValueCell,
    compile_source,
};

fn value(storage: &JsonStorage, contract: u8, key: &[u8]) -> ValueCell {
    storage.contracts[&Hash::new([contract; 32])]
        .data
        .iter()
        .find(|entry| entry.key == ValueCell::Bytes(key.to_vec()))
        .unwrap()
        .value
        .clone()
}

#[tokio::test]
async fn scheduled_failures_match_runner_and_execute_before_explicit_calls() {
    let contract = Hash::new([7; 32]);
    let hash = Hash::new([8; 32]);
    let mut storage = JsonStorage::default();
    storage.contracts.insert(
        contract.clone(),
        ContractState {
            module: Some(
                compile_source(
                    r#"
            pub fn failed() -> u64 {
                Storage::new().store(b"failed", 99u64);
                return 1;
            }
            entry verify() {
                require(!Storage::new().has(b"failed"), "failed write must roll back");
                return 0;
            }
        "#,
                )
                .unwrap()
                .0,
            ),
            balances: [(XELIS_ASSET, 10000000)].into(),
            ..Default::default()
        },
    );
    let execution = ScheduledExecution {
        hash: Arc::new(hash.clone()),
        contract: contract.clone(),
        chunk_id: 0,
        params: vec![],
        max_gas: 1000000,
        gas_sources: [(Source::Contract(contract.clone()), 1000000)].into(),
        kind: ScheduledExecutionKind::TopoHeight {
            execution_topoheight: 5,
            registration_topoheight: 1,
        },
    };
    storage.scheduled_executions.push(execution.clone());
    let playbook: Playbook = serde_json::from_value(serde_json::json!({
        "scheduled_executions": true,
        "blocks": [{ "topoheight": 5, "calls": [{ "contract": contract }] }]
    }))
    .unwrap();
    let outcome = playbook.execute(storage.clone()).await.unwrap();
    let block = &outcome.report.blocks[0];
    assert_eq!(block.scheduled_calls.len(), 1);
    assert!(
        !block.scheduled_calls[0]
            .result
            .as_ref()
            .unwrap()
            .execution
            .is_success()
    );
    assert!(block.calls[0].result.execution.is_success());
    assert!(outcome.storage.scheduled_executions.is_empty());

    storage.topoheight = 5;
    storage.block = outcome.storage.block.clone();
    storage.block_hash = outcome.storage.block_hash.clone();
    storage.scheduled_executions.clear();
    let mut simulator = Simulator::new(storage).unwrap();
    let direct = simulator
        .invoke(Invocation {
            caller: ContractCaller::Scheduled(Cow::Owned(hash), Cow::Owned(contract.clone())),
            contract: contract.clone(),
            deposits: None,
            arguments: vec![],
            gas_sources: execution.gas_sources,
            max_gas: execution.max_gas,
            invoke: InvokeContract::Chunk(0, false),
            permission: InterContractPermission::All,
            post_execution: true,
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(direct).unwrap(),
        serde_json::to_value(block.scheduled_calls[0].result.as_ref().unwrap()).unwrap()
    );
    simulator.run(&contract, Default::default()).await.unwrap();
    assert_eq!(
        serde_json::to_string(simulator.storage()).unwrap(),
        serde_json::to_string(&outcome.storage).unwrap()
    );
    outcome.storage.validate().unwrap();
}

#[tokio::test]
async fn missing_scheduled_modules_are_consumed_and_reserved_gas_is_refunded() {
    let contract = Hash::new([9; 32]);
    let mut storage = JsonStorage::default();
    storage
        .contracts
        .insert(contract.clone(), ContractState::default());
    storage.scheduled_executions.push(ScheduledExecution {
        hash: Arc::new(Hash::new([10; 32])),
        contract: contract.clone(),
        chunk_id: 0,
        params: vec![],
        max_gas: 1000000,
        gas_sources: [(Source::Contract(contract.clone()), 1000000)].into(),
        kind: ScheduledExecutionKind::TopoHeight {
            execution_topoheight: 5,
            registration_topoheight: 1,
        },
    });
    let playbook: Playbook = serde_json::from_str(
        r#"{
        "scheduled_executions":true,
        "blocks":[{"topoheight":5}]
    }"#,
    )
    .unwrap();
    let outcome = playbook.execute(storage).await.unwrap();
    let report = &outcome.report.blocks[0].scheduled_calls[0];
    assert!(report.result.is_none());
    assert!(report.skipped.as_ref().unwrap().contains("refunded"));
    assert!(outcome.storage.scheduled_executions.is_empty());
    assert_eq!(
        outcome.storage.contracts[&contract].balances[&XELIS_ASSET],
        1000000
    );
}

#[tokio::test]
async fn persisted_block_end_calls_keep_registration_order() {
    let contract = Hash::new([4; 32]);
    let mut storage = JsonStorage::default();
    storage.contracts.insert(
        contract.clone(),
        ContractState {
            module: Some(
                compile_source(
                    r#"
            pub fn write(n: u64) -> u64 {
                Storage::new().store(b"order", n);
                return 0;
            }
            entry main() { return 0; }
        "#,
                )
                .unwrap()
                .0,
            ),
            balances: [(XELIS_ASSET, 10000000)].into(),
            ..Default::default()
        },
    );
    for n in [2u8, 1] {
        storage.scheduled_executions.push(ScheduledExecution {
            hash: Arc::new(Hash::new([n; 32])),
            contract: contract.clone(),
            chunk_id: 0,
            params: vec![Primitive::U64(n as u64).into()],
            max_gas: 1000000,
            gas_sources: [(Source::Contract(contract.clone()), 1000000)].into(),
            kind: ScheduledExecutionKind::BlockEnd,
        });
    }
    // A standalone state commit must also preserve the registration order.
    storage.commit(storage.begin().unwrap()).unwrap();
    assert_eq!(
        storage.scheduled_executions[0].hash.as_ref(),
        &Hash::new([2; 32])
    );
    let playbook: Playbook = serde_json::from_str(
        r#"{
        "scheduled_executions":true,
        "blocks":[{"topoheight":5}]
    }"#,
    )
    .unwrap();
    let outcome = playbook.execute(storage).await.unwrap();
    assert_eq!(
        outcome.report.blocks[0]
            .scheduled_calls
            .iter()
            .map(|call| call.hash.clone())
            .collect::<Vec<_>>(),
        [Hash::new([2; 32]), Hash::new([1; 32])]
    );
    assert_eq!(
        value(&outcome.storage, 4, b"order"),
        Primitive::U64(1).into()
    );
}
