use silex_cli::storage::{AccountBalance, AccountState, ContractState, StorageEntry};
use silex_cli::xelis_common::{
    config::XELIS_ASSET,
    contract::{ContractProvider, ContractStorage, InterContractPermission, vm::ExitValue},
    crypto::{Hash, KeyPair},
    transaction::ContractDeposit,
};
use silex_cli::{JsonStorage, Primitive, RunOptions, Simulator, ValueCell, compile_source};

fn hash(n: u8) -> Hash {
    Hash::new([n; 32])
}
fn funded() -> JsonStorage {
    let mut storage = JsonStorage::default();
    storage
        .assets
        .get_mut(&XELIS_ASSET)
        .unwrap()
        .circulating_supply = 1_000_000_000;
    storage.contracts.insert(
        hash(1),
        ContractState {
            balances: [(XELIS_ASSET, 100_000_000)].into(),
            ..Default::default()
        },
    );
    storage
}

#[tokio::test]
async fn storage_persists_across_json_round_trip_and_failure_rolls_back() {
    let mut simulator = Simulator::new(funded()).unwrap();
    let source = r#"
        entry write() { let s = Storage::new(); s.store(b"count", 42u64); return 0; }
        entry read() { let s = Storage::new(); let n: u64 = s.load(b"count").unwrap(); require(n == 42, "persisted"); return 0; }
        entry fail() { let s = Storage::new(); s.store(b"count", 99u64); return 1; }
        entry remove() { let s = Storage::new(); s.delete(b"count"); return 0; }
    "#;
    assert!(
        simulator
            .run_source(hash(1), source, RunOptions::default())
            .await
            .unwrap()
            .execution
            .is_success()
    );
    let json = serde_json::to_string(simulator.storage()).unwrap();
    let mut simulator =
        Simulator::new(serde_json::from_str::<JsonStorage>(&json).unwrap()).unwrap();
    assert!(
        simulator
            .run(
                &hash(1),
                RunOptions {
                    entry: Some(1),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .execution
            .is_success()
    );
    assert!(
        !simulator
            .run(
                &hash(1),
                RunOptions {
                    entry: Some(2),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .execution
            .is_success()
    );
    let value = simulator
        .storage()
        .load_data(&hash(1), &ValueCell::Bytes(b"count".to_vec()), 1)
        .await
        .unwrap()
        .unwrap()
        .1
        .unwrap();
    assert_eq!(value, Primitive::U64(42).into());
    assert!(
        simulator
            .run(
                &hash(1),
                RunOptions {
                    entry: Some(3),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .execution
            .is_success()
    );
    assert!(simulator.storage().contracts[&hash(1)].data.is_empty());
}

#[tokio::test]
async fn registered_contracts_can_call_each_other() {
    let mut simulator = Simulator::new(funded()).unwrap();
    simulator
        .register(
            hash(2),
            compile_source("pub fn answer() -> u64 { return 42; }")
                .unwrap()
                .0,
        )
        .unwrap();
    let source = format!(
        r#"
        entry main() {{
            let c = Contract::new(Hash::from_hex("{}")).unwrap();
            let deposits: map<Hash, u64> = {{}};
            let n: u64 = c.call(0u16, [], deposits);
            require(n == 42, "callee result");
            return 0;
        }}
    "#,
        hash(2)
    );
    let result = simulator
        .run_source(
            hash(1),
            &source,
            RunOptions {
                permission: InterContractPermission::All,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.execution.is_success(), "{:?}", result.execution);
}

#[tokio::test]
async fn arguments_and_gas_are_checked() {
    let mut simulator = Simulator::new(funded()).unwrap();
    simulator
        .register(
            hash(1),
            compile_source("entry main(n: u64) { return n; }")
                .unwrap()
                .0,
        )
        .unwrap();
    let before = serde_json::to_string(simulator.storage()).unwrap();
    assert!(
        simulator
            .run(&hash(1), RunOptions::default())
            .await
            .is_err()
    );
    assert_eq!(before, serde_json::to_string(simulator.storage()).unwrap());
    let result = simulator
        .run(
            &hash(1),
            RunOptions {
                arguments: vec![Primitive::U64(0).into()],
                gas_limit: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(result.execution.exit_value, ExitValue::Error(_)));
}

#[tokio::test]
async fn json_provider_exposes_assets_accounts_and_typed_storage() {
    let key = KeyPair::new().get_public_key().compress();
    let address = key.as_address(false).to_string();
    let mut storage = funded();
    storage.accounts.insert(
        address,
        AccountState {
            balances: [(XELIS_ASSET, AccountBalance::Amount(123))].into(),
            ..Default::default()
        },
    );
    storage
        .contracts
        .get_mut(&hash(1))
        .unwrap()
        .data
        .push(StorageEntry {
            key: Primitive::String("key".into()).into(),
            value: Primitive::U64(7).into(),
        });
    let storage =
        serde_json::from_str::<JsonStorage>(&serde_json::to_string(&storage).unwrap()).unwrap();
    assert!(storage.account_exists(&key, 1).await.unwrap());
    assert!(storage.asset_exists(&XELIS_ASSET, 1).await.unwrap());
    assert_eq!(
        storage
            .get_contract_balance_for_asset(&hash(1), &XELIS_ASSET, 1)
            .await
            .unwrap()
            .unwrap()
            .1,
        100_000_000
    );
    assert!(
        storage
            .get_account_balance_for_asset(&key, &XELIS_ASSET, 1)
            .await
            .unwrap()
            .is_some()
    );
}

#[test]
fn invalid_snapshots_are_rejected() {
    assert!(serde_json::from_str::<JsonStorage>(r#"{"acounts":{}}"#).is_err());
    let mut storage = funded();
    let entry = StorageEntry {
        key: Primitive::U64(1).into(),
        value: Primitive::U64(2).into(),
    };
    storage.contracts.get_mut(&hash(1)).unwrap().data = vec![entry.clone(), entry];
    assert!(Simulator::new(storage).is_err());
}

#[tokio::test]
async fn prepaid_deposits_follow_upstream_impersonated_caller_semantics() {
    let caller = KeyPair::new().get_public_key().compress().as_address(false);
    let mut storage = funded();
    storage.accounts.insert(
        caller.to_string(),
        AccountState {
            balances: [(XELIS_ASSET, AccountBalance::Amount(1000))].into(),
            ..Default::default()
        },
    );
    let mut simulator = Simulator::new(storage).unwrap();
    simulator
        .register(
            hash(1),
            compile_source("entry main(code: u64) { return code; }")
                .unwrap()
                .0,
        )
        .unwrap();
    for code in [1, 0] {
        let result = simulator
            .run(
                &hash(1),
                RunOptions {
                    caller: Some(caller.clone()),
                    deposits: [(XELIS_ASSET, ContractDeposit::Public(100))].into(),
                    arguments: vec![Primitive::U64(code).into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.execution.is_success(), code == 0);
        let expected = 1000; // Funding happens before the runner; impersonation has no transaction refund.
        assert!(
            matches!(simulator.storage().accounts[&caller.to_string()].balances[&XELIS_ASSET], AccountBalance::Amount(n) if n == expected)
        );
    }
    assert_eq!(
        simulator.storage().contracts[&hash(1)].balances[&XELIS_ASSET],
        100_000_100
    );
}

#[tokio::test]
async fn transfers_credit_accounts_once_across_repeated_calls() {
    let recipient = KeyPair::new().get_public_key().compress().as_address(false);
    let source = format!(
        r#"entry main() {{ require(transfer(Address::from_string("{recipient}"), 10u64, Hash::from_hex("{XELIS_ASSET}")), "transfer"); return 0; }}"#
    );
    let mut simulator = Simulator::new(funded()).unwrap();
    simulator
        .register(hash(1), compile_source(&source).unwrap().0)
        .unwrap();
    for expected in [10, 20] {
        let result = simulator
            .run(&hash(1), RunOptions::default())
            .await
            .unwrap();
        assert!(result.execution.is_success(), "{:?}", result.execution);
        assert!(
            matches!(simulator.storage().accounts[&recipient.to_string()].balances[&XELIS_ASSET], AccountBalance::Amount(n) if n == expected)
        );
    }
    assert_eq!(
        simulator.storage().contracts[&hash(1)].balances[&XELIS_ASSET],
        99_999_980
    );
}

#[tokio::test]
async fn constructor_hook_uses_runner_and_persists_state() {
    let mut simulator = Simulator::new(funded()).unwrap();
    let source = r#"
        hook constructor() -> u64 { Storage::new().store(b"initialized", true); return 0; }
        entry main() { let initialized: bool = Storage::new().load(b"initialized").unwrap(); require(initialized, "constructor"); return 0; }
    "#;
    let result = simulator
        .run_source(
            hash(1),
            source,
            RunOptions {
                hook: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.execution.is_success(), "{:?}", result.execution);
    assert!(
        simulator
            .run(&hash(1), RunOptions::default())
            .await
            .unwrap()
            .execution
            .is_success()
    );
}
