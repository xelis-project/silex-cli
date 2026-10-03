use silex_cli::playbook::{PlaybookBlock, PlaybookCall, PlaybookContract};
use silex_cli::xelis_common::crypto::Hash;
use silex_cli::{JsonStorage, Playbook, Primitive, RunOptions, Simulator, compile_source};

#[tokio::test]
async fn caller_supplied_modules_and_playbooks_execute_without_paths() {
    let hash = Hash::zero();
    let (module, _) = compile_source(
        r#"
        fn increment(n: u64) -> u64 { return n + 1u64; }
        entry main() {
            let s = Storage::new();
            let n: u64 = s.load(b"count").unwrap_or(0u64);
            s.store(b"count", increment(n));
            return 0;
        }
        "#,
    )
    .unwrap();

    let mut simulator = Simulator::new(JsonStorage::default()).unwrap();
    let result = simulator
        .run_module(hash.clone(), module.clone(), RunOptions::default())
        .await
        .unwrap();
    assert!(result.execution.is_success());

    // A private function precedes the entry. Explicitly selecting it must fail
    // before changing storage, while default selection finds the entry.
    let before = serde_json::to_value(simulator.storage()).unwrap();
    assert!(
        simulator
            .run(
                &hash,
                RunOptions {
                    entry: Some(0),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(before, serde_json::to_value(simulator.storage()).unwrap());

    let playbook = Playbook {
        contracts: vec![PlaybookContract {
            contract: hash.clone(),
            module,
        }],
        scheduled_executions: false,
        blocks: vec![PlaybookBlock {
            topoheight: 2,
            height: None,
            timestamp: None,
            calls: vec![PlaybookCall {
                contract: hash.clone(),
                execution: RunOptions::default(),
            }],
        }],
    };
    // The caller owns encoding and decoding the typed request.
    let encoded = serde_json::to_vec(&playbook).unwrap();
    let restored: Playbook = serde_json::from_slice(&encoded).unwrap();
    let outcome = restored.execute(simulator.into_storage()).await.unwrap();
    assert!(
        outcome.report.blocks[0].calls[0]
            .result
            .execution
            .is_success()
    );
    assert_eq!(
        outcome.storage.contracts[&hash].data[0].value,
        Primitive::U64(2).into()
    );
}
