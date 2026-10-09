//! A Metal device fault in the embedding child (candle's poisoned lock after an
//! interrupted `MTLCompilerService`) must not leave the daemon failing every
//! later embed. The supervisor restarts the child pinned to the CPU device and
//! replays the request, for interactive and operation-scoped callers alike.

use std::{path::PathBuf, time::Duration};

use surreal_memory::{EmbeddingService, embeddings::ExecutorEventKind};
use surreal_memory_server::executor::{ExecutorIdentity, SupervisedEmbeddingService};

const FAULT_TEXT: &str = "metal-fault";

/// Built with the fixture's real identity so the supervisor enforces cache
/// identity exactly as it does for Candle: a replacement child on a different
/// device publishes a different namespace.
fn supervised_with_metal_fault() -> SupervisedEmbeddingService {
    SupervisedEmbeddingService::with_child_env_and_identity(
        PathBuf::from(env!("CARGO_BIN_EXE_surreal-memory-server")),
        2,
        Duration::from_secs(10),
        vec![
            ("SURREAL_EXECUTOR_FIXTURE".to_owned(), "1".to_owned()),
            (
                "SURREAL_EXECUTOR_METAL_FAULT_ON".to_owned(),
                FAULT_TEXT.to_owned(),
            ),
        ],
        Some(ExecutorIdentity {
            backend: "fixture".to_owned(),
            model_id: "fixture".to_owned(),
            model_revision: "fixture".to_owned(),
            dimensions: 2,
        }),
    )
}

#[tokio::test]
async fn interactive_embed_recovers_on_a_cpu_child_after_a_metal_fault() {
    let executor = supervised_with_metal_fault();
    let mut events = executor.subscribe_executor_events().unwrap();

    // A healthy child publishes a cache namespace.
    executor.embed("warm").await.unwrap();
    assert!(executor.cache_namespace().is_some());

    // Generation 1 reports the fault; generation 2 runs on the CPU device and
    // answers the replayed request.
    assert_eq!(executor.embed(FAULT_TEXT).await.unwrap(), vec![12.0, 1.0]);
    let after_recovery = executor.executor_snapshot().unwrap();
    assert!(
        after_recovery.generation >= 2,
        "a fresh child must serve it"
    );
    assert_eq!(after_recovery.exit_count, 1);
    assert_eq!(
        executor.cache_namespace(),
        None,
        "the device changed, so the query cache must be bypassed, not mixed"
    );

    let mut saw_fault = false;
    let mut saw_restart = false;
    while let Ok(event) = events.try_recv() {
        saw_fault |= event.kind == ExecutorEventKind::Error
            && event
                .message
                .as_deref()
                .is_some_and(|message| message.contains("poisoned lock"));
        saw_restart |= event.kind == ExecutorEventKind::Exited
            && event
                .message
                .as_deref()
                .is_some_and(|message| message.contains("restarting on cpu"));
    }
    assert!(saw_fault, "the Metal fault is reported, not swallowed");
    assert!(saw_restart, "the restart onto cpu is reported");

    // The CPU child cannot hit the fault again, so the same text now succeeds
    // in place without another restart.
    assert_eq!(executor.embed(FAULT_TEXT).await.unwrap(), vec![12.0, 1.0]);
    let steady = executor.executor_snapshot().unwrap();
    assert_eq!(steady.generation, after_recovery.generation);
    assert_eq!(steady.exit_count, after_recovery.exit_count);
    executor.terminate_idle_executor().await.unwrap();
}

#[tokio::test]
async fn operation_scoped_embed_is_replayed_instead_of_pausing_the_operation() {
    let executor = supervised_with_metal_fault();

    // Operation-scoped callers normally observe a failure and are resumed by
    // the operations layer. A Metal fault happens before any ledger write, so
    // the supervisor replays it itself and the operation never pauses.
    let embedding = executor
        .embed_for_operation("metal-fault-operation", 0, FAULT_TEXT)
        .await
        .unwrap();
    assert_eq!(embedding, vec![12.0, 1.0]);
    assert!(executor.executor_snapshot().unwrap().generation >= 2);
    executor.terminate_idle_executor().await.unwrap();
}
