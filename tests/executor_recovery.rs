use std::{path::PathBuf, sync::Arc, time::Duration};

use futures_util::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::embeddings::ExecutorSnapshot;
use surreal_memory::{
    EmbeddingService, MemoryStorage, SurrealStorage, embeddings::ExecutorEventKind,
};
use surreal_memory_server::{
    executor::SupervisedEmbeddingService,
    operations::{OperationRequest, OperationService, OperationState},
};

fn payload_hash(payload: &Value) -> String {
    Sha256::digest(serde_json::to_vec(payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn request(id: &str, kind: &str, dependencies: Vec<&str>, payload: Value) -> OperationRequest {
    OperationRequest {
        operation_id: id.to_owned(),
        schema_version: 2,
        kind: kind.to_owned(),
        dependencies: dependencies.into_iter().map(str::to_owned).collect(),
        payload_hash: payload_hash(&payload),
        payload,
    }
}

#[tokio::test]
async fn child_kills_recover_across_acceptance_and_every_persisted_part() {
    let marker_dir = std::env::temp_dir().join(format!(
        "surreal-executor-recovery-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&marker_dir).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_surreal-memory-server"));
    let executor = Arc::new(SupervisedEmbeddingService::with_child_env(
        executable,
        2,
        Duration::from_secs(10),
        vec![
            ("SURREAL_EXECUTOR_FIXTURE".to_owned(), "1".to_owned()),
            (
                "SURREAL_EXECUTOR_EXIT_MARKER_DIR".to_owned(),
                marker_dir.display().to_string(),
            ),
            (
                "SURREAL_EXECUTOR_EXIT_ON".to_owned(),
                "alpha,beta,gamma".to_owned(),
            ),
        ],
    ));

    // Kill generation 1 before an operation is accepted. The next request
    // must start a new generation without guessing whether any write landed.
    executor.embed("pre-acceptance probe").await.unwrap();
    executor.terminate_idle_executor().await.unwrap();

    let embedding_service: Arc<dyn EmbeddingService> = executor.clone();
    let storage = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&embedding_service))
            .await
            .unwrap(),
    );
    let operations = OperationService::start(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        embedding_service,
    );
    let memory_payload = json!({"content":"three-part logical memory","user_id":"test"});
    let memory_request = request(
        "executor-kill-memory",
        "add_memory",
        vec!["executor-kill-prerequisite"],
        memory_payload,
    );
    operations.submit(memory_request.clone()).await.unwrap();
    let mut events = operations
        .event_stream("executor-kill-memory", 0)
        .await
        .unwrap();
    loop {
        let event = events.next().await.unwrap().unwrap();
        if event.to_state == OperationState::Blocked.as_str() {
            break;
        }
    }

    // Kill another healthy generation after durable acceptance while the
    // operation remains explicitly blocked on its prerequisite.
    executor.embed("post-acceptance probe").await.unwrap();
    executor.terminate_idle_executor().await.unwrap();

    let prerequisite_payload = json!({
        "name":"executor-kill-prerequisite",
        "description":"unblocks supervised executor recovery",
        "agent_id":null,
        "user_id":"test"
    });
    operations
        .submit(request(
            "executor-kill-prerequisite",
            "create_task_stream",
            vec![],
            prerequisite_payload,
        ))
        .await
        .unwrap();

    let mut paused = 0usize;
    loop {
        let event = events.next().await.unwrap().unwrap();
        if event
            .detail
            .as_ref()
            .and_then(|detail| detail.get("paused"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            paused += 1;
            operations.submit(memory_request.clone()).await.unwrap();
        }
        if event.to_state == OperationState::Committed.as_str() {
            break;
        }
    }

    assert_eq!(paused, 3, "each planned part killed one child generation");
    let receipt = operations
        .get("executor-kill-memory")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.state, OperationState::Committed);
    assert_eq!(receipt.executor_exit_count, 3);
    assert!(receipt.executor_generation >= 5);
    assert!(receipt.executor_progress_seq > 0);
    assert!(receipt.executor_last_exit.is_some());
    assert!(receipt.executor_error.is_some());

    let search_results = storage
        .search_memories(
            "three-part logical memory",
            Some("test"),
            None,
            None,
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(search_results.len(), 1, "search exposes one logical memory");

    // A final child kill after the memory commit cannot change the durable
    // outcome, even though it happens before this test returns its response.
    executor.terminate_idle_executor().await.unwrap();
    assert_eq!(
        operations
            .get("executor-kill-memory")
            .await
            .unwrap()
            .unwrap()
            .state,
        OperationState::Committed
    );

    let executor_rows: Vec<Value> = storage
        .db()
        .unwrap()
        .query("SELECT * FROM memory_executor_event WHERE operation_id = $id")
        .bind(("id", "executor-kill-memory".to_owned()))
        .await
        .unwrap()
        .check()
        .unwrap()
        .take(0)
        .unwrap();
    assert!(
        executor_rows.len() >= 3,
        "executor progress and exits are append-only durable records"
    );
    std::fs::remove_dir_all(marker_dir).unwrap();
}

#[tokio::test]
async fn watchdog_restarts_only_a_child_without_progress() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_surreal-memory-server"));
    let executor = SupervisedEmbeddingService::with_child_env(
        executable,
        2,
        Duration::from_secs(3),
        vec![
            ("SURREAL_EXECUTOR_FIXTURE".to_owned(), "1".to_owned()),
            ("SURREAL_EXECUTOR_FREEZE_ON".to_owned(), "frozen".to_owned()),
            (
                "SURREAL_EXECUTOR_SLOW_ON".to_owned(),
                "slow-but-responsive".to_owned(),
            ),
        ],
    );
    let mut events = executor.subscribe_executor_events().unwrap();

    let error = executor.embed("frozen").await.unwrap_err();
    assert!(error.to_string().contains("nonresponsive"));
    loop {
        let event = events.recv().await.unwrap();
        if event.kind == ExecutorEventKind::Nonresponsive {
            break;
        }
    }

    // This request exceeds the watchdog interval but emits child progress, so
    // it must complete in the same generation without a watchdog restart.
    let before = executor.executor_snapshot().unwrap();
    assert_eq!(
        executor.embed("slow-but-responsive").await.unwrap(),
        vec![20.0, 1.0]
    );
    let after = executor.executor_snapshot().unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.exit_count, before.exit_count);
    executor.terminate_idle_executor().await.unwrap();
}

#[tokio::test]
async fn server_restart_advances_the_persisted_executor_generation() {
    let marker_dir = std::env::temp_dir().join(format!(
        "surreal-executor-generation-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&marker_dir).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_surreal-memory-server"));
    let child_env = vec![
        ("SURREAL_EXECUTOR_FIXTURE".to_owned(), "1".to_owned()),
        (
            "SURREAL_EXECUTOR_EXIT_MARKER_DIR".to_owned(),
            marker_dir.display().to_string(),
        ),
        ("SURREAL_EXECUTOR_EXIT_ON".to_owned(), "alpha".to_owned()),
    ];
    let first_executor = Arc::new(SupervisedEmbeddingService::with_child_env(
        executable.clone(),
        2,
        Duration::from_secs(10),
        child_env.clone(),
    ));
    let first_embedding: Arc<dyn EmbeddingService> = first_executor.clone();
    let storage = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&first_embedding))
            .await
            .unwrap(),
    );
    let first = OperationService::start(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        first_embedding,
    );
    let payload = json!({"content":"three-part generation recovery","user_id":"test"});
    let operation = request("generation-restart", "add_memory", vec![], payload);
    first.submit(operation).await.unwrap();
    let mut first_events = first.event_stream("generation-restart", 0).await.unwrap();
    loop {
        let event = first_events.next().await.unwrap().unwrap();
        if event
            .detail
            .as_ref()
            .and_then(|detail| detail.get("paused"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            break;
        }
    }
    let interrupted = first.get("generation-restart").await.unwrap().unwrap();
    assert_eq!(interrupted.executor_exit_count, 1);
    first_executor.terminate_idle_executor().await.unwrap();

    let second_executor = Arc::new(SupervisedEmbeddingService::with_child_env(
        executable,
        2,
        Duration::from_secs(10),
        child_env,
    ));
    let second_embedding: Arc<dyn EmbeddingService> = second_executor.clone();
    let second = OperationService::start(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        second_embedding,
    );
    let mut second_events = second
        .event_stream("generation-restart", interrupted.progress_seq)
        .await
        .unwrap();
    loop {
        let event = second_events.next().await.unwrap().unwrap();
        if event.to_state == OperationState::Committed.as_str() {
            break;
        }
    }
    let completed = second.get("generation-restart").await.unwrap().unwrap();
    assert!(completed.executor_generation > interrupted.executor_generation);
    assert_eq!(completed.executor_exit_count, 1);
    second_executor.terminate_idle_executor().await.unwrap();
    std::fs::remove_dir_all(marker_dir).unwrap();
}

#[tokio::test]
async fn durable_generation_adopts_the_pre_warmed_child_without_restarting_it() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_surreal-memory-server"));
    let executor = SupervisedEmbeddingService::with_child_env(
        executable,
        2,
        Duration::from_secs(10),
        vec![("SURREAL_EXECUTOR_FIXTURE".to_owned(), "1".to_owned())],
    );

    executor.embed("warmup").await.unwrap();
    let warm = executor.executor_snapshot().unwrap();
    let persisted = ExecutorSnapshot {
        generation: warm.generation + 10,
        progress_seq: warm.progress_seq,
        exit_count: warm.exit_count,
        last_exit: None,
        error: None,
    };

    executor
        .prepare_operation("resume-with-warm-child", &persisted)
        .await
        .unwrap();
    let adopted = executor.executor_snapshot().unwrap();
    assert!(adopted.generation > persisted.generation);
    assert_eq!(adopted.exit_count, warm.exit_count);

    executor.embed("still-warm").await.unwrap();
    let after = executor.executor_snapshot().unwrap();
    assert_eq!(after.generation, adopted.generation);
    assert_eq!(after.exit_count, warm.exit_count);
    executor.terminate_idle_executor().await.unwrap();
}
