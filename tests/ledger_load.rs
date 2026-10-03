//! Ledger load harness for #36 (10 s operation-ledger timeouts).
//!
//! Drives the durable-operation ledger in server mode with concurrent
//! submitters, receipt pollers (the learning-worker pattern) and general
//! storage reads, under a deliberately tight ledger deadline so some queries
//! overrun it. It reports the client-visible error rate, receipt latency
//! and how often the ledger connection was replaced after a deadline.
//!
//! `#[ignore]`-gated; needs the `surreal` CLI on PATH:
//!
//! ```bash
//! cargo test --test ledger_load --features embedded,metal --release -- --ignored --nocapture
//! ```
//!
//! `LEDGER_LOAD_TIMEOUT_MS` (default 25) sets the ledger deadline. Each run
//! appends a row to `tests/ledger_load_baseline.md`.

use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, header},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::{
    EmbeddingService, MemoryStorage, RetryConfig, SurrealConfig, SurrealStorage,
    storage::surreal::SurrealMode,
};
use surreal_memory_server::api;
use tower::ServiceExt;
use tracing_subscriber::{Layer, layer::SubscriberExt};

const SUBMITTERS: usize = 32;
const OPERATIONS_PER_SUBMITTER: usize = 10;
const POLLERS: usize = 16;
const READERS: usize = 8;

struct NoOpEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for NoOpEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0; 1536])
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.into_iter().map(|_| vec![0.0; 1536]).collect())
    }

    fn dimensions(&self) -> usize {
        1536
    }
}

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Default)]
struct LedgerCounters {
    replaced: AtomicUsize,
    replacement_failed: AtomicUsize,
}

/// Counts the ledger's connection-recovery log events by message.
struct CountingLayer(Arc<LedgerCounters>);

struct MessageVisitor(Option<String>);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for CountingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = MessageVisitor(None);
        event.record(&mut visitor);
        let Some(message) = visitor.0 else { return };
        let counter = match message.as_str() {
            "operation database connection replaced" => &self.0.replaced,
            "operation database connection replacement failed"
            | "operation database connection replacement timed out" => &self.0.replacement_failed,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

async fn start_server() -> (ServerGuard, String) {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let child = Command::new("surreal")
        .args([
            "start",
            "--no-banner",
            "--unauthenticated",
            "--allow-all",
            "--bind",
            &address.to_string(),
            "memory",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("installed surreal CLI starts the load fixture");
    let guard = ServerGuard(child);
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::net::TcpStream::connect(address).await.is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("load fixture becomes reachable");
    (guard, format!("ws://{address}"))
}

fn operation_request(operation_id: &str) -> Value {
    let payload = json!({
        "name": operation_id,
        "description": "ledger load fixture",
        "agent_id": null,
        "user_id": "load"
    });
    let payload_hash = Sha256::digest(serde_json::to_vec(&payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({
        "operation_id": operation_id,
        "schema_version": 2,
        "kind": "create_task_stream",
        "dependencies": [],
        "payload_hash": payload_hash,
        "payload": payload
    })
}

async fn call(router: &Router, request: Request<Body>) -> (bool, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let ok = response.status().is_success();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (ok, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "load harness; requires the surreal CLI"]
async fn ledger_under_mixed_load() {
    let counters = Arc::new(LedgerCounters::default());
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(CountingLayer(Arc::clone(&counters))),
    )
    .unwrap();
    let ledger_timeout = Duration::from_millis(
        std::env::var("LEDGER_LOAD_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(25),
    );

    let (_server, endpoint) = start_server().await;
    let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
    let storage = Arc::new(
        SurrealStorage::new(
            &SurrealConfig {
                auth_level: Default::default(),
                mode: SurrealMode::Server,
                endpoint: Some(endpoint),
                embedded_path: None,
                username: None,
                password: None,
                namespace: format!("ledger_load_{}", uuid::Uuid::new_v4().simple()),
                database: "operations".to_owned(),
                retry: RetryConfig {
                    max_connect_retries: 0,
                    query_timeout_ms: 10_000,
                    ..RetryConfig::default()
                },
            },
            Arc::clone(&embedder),
        )
        .await
        .expect("server-mode SurrealStorage"),
    );
    let router = api::build_router_with_query_timeout(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        embedder,
        ledger_timeout,
    );

    let total = SUBMITTERS * OPERATIONS_PER_SUBMITTER;
    let ids: Arc<Vec<String>> = Arc::new((0..total).map(|i| format!("load-{i:05}")).collect());
    let started = Instant::now();
    let deadline = started + Duration::from_secs(120);
    let ledger_errors = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));

    let submitters = (0..SUBMITTERS).map(|worker| {
        let router = router.clone();
        let ids = Arc::clone(&ids);
        let ledger_errors = Arc::clone(&ledger_errors);
        let requests = Arc::clone(&requests);
        tokio::spawn(async move {
            for index in 0..OPERATIONS_PER_SUBMITTER {
                let id = &ids[worker * OPERATIONS_PER_SUBMITTER + index];
                let body = serde_json::to_vec(&operation_request(id)).unwrap();
                // Resubmission is idempotent, so retry until accepted.
                while Instant::now() < deadline {
                    let request = Request::builder()
                        .method("POST")
                        .uri("/api/v2/operations")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap();
                    requests.fetch_add(1, Ordering::Relaxed);
                    if call(&router, request).await.0 {
                        break;
                    }
                    ledger_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    });
    let submitters = futures_util::future::join_all(submitters.collect::<Vec<_>>());

    let pollers = (0..POLLERS).map(|worker| {
        let router = router.clone();
        let ids = Arc::clone(&ids);
        let ledger_errors = Arc::clone(&ledger_errors);
        let requests = Arc::clone(&requests);
        tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut committed = 0usize;
            let mut cursor = worker;
            while Instant::now() < deadline {
                let id = &ids[cursor % ids.len()];
                let request = Request::builder()
                    .uri(format!("/api/v2/operations/{id}"))
                    .body(Body::empty())
                    .unwrap();
                requests.fetch_add(1, Ordering::Relaxed);
                let begun = Instant::now();
                let (ok, body) = call(&router, request).await;
                latencies.push(begun.elapsed());
                if !ok
                    && body["error"]
                        .as_str()
                        .is_some_and(|e| e.contains("timed out"))
                {
                    ledger_errors.fetch_add(1, Ordering::Relaxed);
                }
                if ok && body["state"] == "committed" {
                    committed += 1;
                    if committed >= ids.len() / POLLERS {
                        break;
                    }
                }
                cursor += POLLERS;
            }
            latencies
        })
    });
    let pollers = futures_util::future::join_all(pollers.collect::<Vec<_>>());

    let readers = (0..READERS).map(|_| {
        let storage = Arc::clone(&storage);
        tokio::spawn(async move {
            while Instant::now() < deadline - Duration::from_secs(110) {
                let _ = storage.health_check().await;
            }
        })
    });
    let readers = futures_util::future::join_all(readers.collect::<Vec<_>>());

    let (_, poller_results, _) = tokio::join!(submitters, pollers, readers);
    let mut latencies: Vec<Duration> = poller_results
        .into_iter()
        .flat_map(|result| result.unwrap())
        .collect();
    latencies.sort_unstable();

    // Drain: every operation must eventually commit.
    let mut uncommitted = 0usize;
    for id in ids.iter() {
        let mut done = false;
        while Instant::now() < deadline {
            let request = Request::builder()
                .uri(format!("/api/v2/operations/{id}"))
                .body(Body::empty())
                .unwrap();
            let (ok, body) = call(&router, request).await;
            if ok && body["state"] == "committed" {
                done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        uncommitted += usize::from(!done);
    }
    let elapsed = started.elapsed();

    let row = format!(
        "| {} | {}ms | {} | {} | {} ({:.2}%) | {} | {} | {:?} | {:?} | {:.1}s | {} |",
        chrono::Utc::now().format("%Y-%m-%dT%H:%MZ"),
        ledger_timeout.as_millis(),
        total,
        requests.load(Ordering::Relaxed),
        ledger_errors.load(Ordering::Relaxed),
        100.0 * ledger_errors.load(Ordering::Relaxed) as f64
            / requests.load(Ordering::Relaxed).max(1) as f64,
        counters.replaced.load(Ordering::Relaxed),
        counters.replacement_failed.load(Ordering::Relaxed),
        percentile(&latencies, 0.5),
        percentile(&latencies, 0.99),
        elapsed.as_secs_f64(),
        uncommitted,
    );
    println!("{row}");
    let baseline = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ledger_load_baseline.md");
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(baseline) {
        let _ = writeln!(file, "{row}");
    }
    assert_eq!(uncommitted, 0, "every operation must commit");
}
