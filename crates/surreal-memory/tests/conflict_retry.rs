//! Storage writes must survive SurrealDB 3.3 retryable transaction conflicts
//! (surrealdb-3x-connection-model, c3).
//!
//! A 3.3 server or embedded engine can reject a write to an HNSW-indexed table
//! with `Transaction conflict: Resource busy. This transaction can be retried`
//! (`QueryError::TransactionConflict`). These tests drive enough HNSW writes to
//! hit that, and assert no conflict reaches the caller and no write is lost or
//! duplicated.
//!
//! Each test also counts the storage layer's "retrying write after
//! transaction conflict" debug events. A run that saw zero conflicts proves
//! nothing, so the server stress test treats it as inconclusive.
//!
//! ```bash
//! # Embedded concurrency test (no external server):
//! cargo test -p surreal-memory --test conflict_retry -- --nocapture
//! # Server stress test against a scratch 3.3.0 server, never :28000:
//! TEST_SURREAL_ENDPOINT=ws://127.0.0.1:<port> \
//!   cargo test -p surreal-memory --test conflict_retry -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::entity::Entity;
use surreal_memory::storage::surreal::{RetryConfig, SurrealConfig, SurrealMode};
use surreal_memory::{Memory, MemoryStorage, SurrealStorage};
use tracing_subscriber::layer::SubscriberExt;

const DIMENSIONS: usize = 64;
const RETRY_MESSAGE: &str = "retrying write after transaction conflict";

static RETRIES: AtomicUsize = AtomicUsize::new(0);

/// Counts the storage layer's conflict-retry events across all threads.
struct RetryCounter;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RetryCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Visitor(bool);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" && format!("{value:?}").contains(RETRY_MESSAGE) {
                    self.0 = true;
                }
            }
        }
        let mut visitor = Visitor(false);
        event.record(&mut visitor);
        if visitor.0 {
            RETRIES.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn install_retry_counter() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let subscriber = tracing_subscriber::registry().with(RetryCounter);
        tracing::subscriber::set_global_default(subscriber).expect("install retry counter");
    });
}

/// Deterministic pseudo-random unit vector per text, so HNSW does real graph
/// maintenance and distinct texts stay far below the 0.92 duplicate threshold.
fn vector_for(text: &str) -> Vec<f32> {
    let mut state = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    let mut vector: Vec<f32> = (0..DIMENSIONS)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
        })
        .collect();
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    vector.iter_mut().for_each(|v| *v /= norm);
    vector
}

struct HashEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for HashEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vector_for(text))
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| vector_for(t)).collect())
    }

    fn dimensions(&self) -> usize {
        DIMENSIONS
    }
}

fn record_id_string(id: &surrealdb::types::RecordId) -> String {
    use surrealdb::types::RecordIdKey;
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        RecordIdKey::Number(value) => value.to_string(),
        RecordIdKey::Uuid(value) => value.to_string(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

async fn count(storage: &SurrealStorage, sql: &str) -> u64 {
    let rows: Vec<serde_json::Value> = storage
        .db()
        .expect("db handle")
        .query(sql)
        .await
        .expect("count query")
        .take(0)
        .expect("count rows");
    rows.first()
        .and_then(|row| row["count"].as_u64())
        .unwrap_or(0)
}

/// Concurrent writers on one embedded store: every write must succeed exactly
/// once. Before c3, a conflict either surfaced as an error or silently dropped
/// the history row.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_embedded_writes_survive_conflicts_without_loss_or_duplicates() {
    install_retry_counter();
    let retries_before = RETRIES.load(Ordering::SeqCst);
    let embedder: Arc<dyn EmbeddingService> = Arc::new(HashEmbedder);
    let storage = Arc::new(
        SurrealStorage::new_mem(embedder)
            .await
            .expect("embedded SurrealStorage"),
    );

    const TASKS: usize = 16;
    const PER_TASK: usize = 50;
    let mut handles = Vec::new();
    for task in 0..TASKS {
        let storage = Arc::clone(&storage);
        handles.push(tokio::spawn(async move {
            let mut failures = Vec::new();
            for i in 0..PER_TASK {
                let memory = Memory::new(
                    format!("conflict-add-{task}-{i}"),
                    Some("conflict-user".into()),
                    Some("conflict-agent".into()),
                    None,
                    vec![],
                );
                if let Err(error) = storage.add_memory(memory).await {
                    failures.push(format!("{error:#}"));
                }
            }
            failures
        }));
    }
    let mut failures = Vec::new();
    for handle in handles {
        failures.extend(handle.await.expect("writer task"));
    }
    assert!(failures.is_empty(), "add_memory failures: {failures:?}");

    assert_eq!(
        count(&storage, "SELECT count() FROM memory GROUP ALL").await,
        (TASKS * PER_TASK) as u64,
        "every add_memory must store exactly one memory"
    );
    assert_eq!(
        count(
            &storage,
            "SELECT count() FROM memory_history WHERE change_type = 'created' GROUP ALL"
        )
        .await,
        (TASKS * PER_TASK) as u64,
        "every stored memory must have exactly one 'created' history row"
    );

    // Concurrent updates of one memory.
    let target = storage
        .add_memory(Memory::new(
            "conflict-update-target",
            Some("conflict-user".into()),
            Some("conflict-agent".into()),
            None,
            vec![],
        ))
        .await
        .expect("update target");
    let target_id = record_id_string(target.id.as_ref().expect("target id"));
    let mut handles = Vec::new();
    for task in 0..TASKS {
        let storage = Arc::clone(&storage);
        let target_id = target_id.clone();
        handles.push(tokio::spawn(async move {
            storage
                .update_memory(&target_id, format!("conflict-update-{task}"))
                .await
                .map(|_| ())
                .map_err(|error| format!("{error:#}"))
        }));
    }
    for handle in handles {
        handle
            .await
            .expect("updater task")
            .expect("update_memory must survive conflicts");
    }
    assert_eq!(
        count(
            &storage,
            "SELECT count() FROM memory_history WHERE change_type = 'updated' GROUP ALL"
        )
        .await,
        TASKS as u64,
        "every update_memory must record exactly one 'updated' history row"
    );

    eprintln!(
        "conflict retries observed: {}",
        RETRIES.load(Ordering::SeqCst) - retries_before
    );
}

/// Sequential add/delete and entity create/delete pairs against a scratch
/// 3.3.0 server. Zero surfaced errors is required; zero observed retries makes
/// the run inconclusive rather than a pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires TEST_SURREAL_ENDPOINT pointing at a scratch SurrealDB 3.3 server"]
async fn server_write_stress_surfaces_no_conflicts() {
    install_retry_counter();
    let retries_before = RETRIES.load(Ordering::SeqCst);
    let endpoint =
        std::env::var("TEST_SURREAL_ENDPOINT").expect("TEST_SURREAL_ENDPOINT must be set");
    let embedder: Arc<dyn EmbeddingService> = Arc::new(HashEmbedder);
    let storage = SurrealStorage::new(
        &SurrealConfig {
            auth_level: Default::default(),
            mode: SurrealMode::Server,
            endpoint: Some(endpoint),
            embedded_path: None,
            username: Some("root".into()),
            password: Some("root".into()),
            namespace: format!("conflict_{}", uuid::Uuid::new_v4().simple()),
            database: "main".into(),
            retry: RetryConfig::default(),
        },
        embedder,
    )
    .await
    .expect("server SurrealStorage");

    const MEMORY_PAIRS: usize = 1_500;
    const ENTITY_PAIRS: usize = 500;
    let mut failures = Vec::new();
    for i in 0..MEMORY_PAIRS {
        let memory = Memory::new(
            format!("stress-memory-{i}"),
            Some("stress-user".into()),
            Some("stress-agent".into()),
            None,
            vec![],
        );
        match storage.add_memory(memory).await {
            Ok(stored) => {
                let id = record_id_string(stored.id.as_ref().expect("stored id"));
                if let Err(error) = storage.delete_memory(&id).await {
                    failures.push(format!("delete_memory {i}: {error:#}"));
                }
            }
            Err(error) => failures.push(format!("add_memory {i}: {error:#}")),
        }
    }
    for i in 0..ENTITY_PAIRS {
        let name = format!("stress-entity-{i}");
        let entity = Entity::new(name.clone(), "stress".into(), vec!["observation".into()]);
        if let Err(error) = storage.create_entity(entity).await {
            failures.push(format!("create_entity {i}: {error:#}"));
            continue;
        }
        if let Err(error) = storage.delete_entity(&name).await {
            failures.push(format!("delete_entity {i}: {error:#}"));
        }
    }

    let retries = RETRIES.load(Ordering::SeqCst) - retries_before;
    eprintln!("conflict retries observed: {retries}");
    assert!(failures.is_empty(), "surfaced write failures: {failures:?}");
    assert_eq!(
        count(&storage, "SELECT count() FROM memory GROUP ALL").await,
        0,
        "every added memory was deleted"
    );
    assert!(
        retries > 0,
        "inconclusive: no transaction conflict occurred in this run, so retry handling was not exercised"
    );
}
