//! Load-repro harness — Change 1 of the
//! `surrealdb-connection-architecture` KBD phase.
//!
//! Goal: deterministic, repeatable measurement of `SurrealStorage` under
//! concurrent load to surface the symptoms the user reported (timeouts in
//! server mode, sync errors in embedded mode under load) BEFORE we change
//! any production code.
//!
//! Tests are `#[ignore]`-gated so `cargo test` stays free of external
//! dependencies. Run explicitly with:
//!
//! ```bash
//! # Bring up the docker-compose SurrealDB first:
//! docker-compose up -d surrealdb
//!
//! # Server mode (default endpoint = ws://127.0.0.1:28000 per docker-compose.yaml):
//! cargo test --test load_repro --features embedded,metal --release -- --ignored server
//!
//! # Embedded mode (uses a temp RocksDB path):
//! cargo test --test load_repro --features embedded,metal --release -- --ignored embedded
//! ```
//!
//! Override the server endpoint with `TEST_SURREAL_ENDPOINT=ws://host:port`
//! (matches the existing `integration_test.rs` convention).
//!
//! Each test appends a markdown row to
//! `crates/surreal-memory/tests/load_repro_baseline.md` so successive runs
//! are diffable.

use std::sync::Arc;
use std::time::{Duration, Instant};
use surreal_memory::{
    Memory, MemoryStorage,
    storage::surreal::{RetryConfig, SurrealConfig, SurrealMode, SurrealStorage},
};
use uuid::Uuid;

// ── NoOp embedder (mirrors integration_test.rs) ──────────────────────────────

struct NoOpEmbedder;

#[async_trait::async_trait]
impl surreal_memory::embeddings::EmbeddingService for NoOpEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0f32; 1536])
    }
    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0f32; 1536]).collect())
    }
    fn dimensions(&self) -> usize {
        1536
    }
}

// ── Storage builders ─────────────────────────────────────────────────────────

async fn server_storage(namespace: String) -> Arc<SurrealStorage> {
    let embedder: Arc<dyn surreal_memory::embeddings::EmbeddingService> = Arc::new(NoOpEmbedder);
    let endpoint = std::env::var("TEST_SURREAL_ENDPOINT")
        .unwrap_or_else(|_| "ws://127.0.0.1:28000".to_string());
    let config = SurrealConfig {
        mode: SurrealMode::Server,
        endpoint: Some(endpoint),
        embedded_path: None,
        username: Some(
            std::env::var("TEST_SURREAL_USERNAME").unwrap_or_else(|_| "root".to_string()),
        ),
        password: Some(
            std::env::var("TEST_SURREAL_PASSWORD").unwrap_or_else(|_| "root".to_string()),
        ),
        namespace,
        database: "main".to_string(),
        retry: RetryConfig::default(),
    };
    Arc::new(
        SurrealStorage::new(&config, embedder)
            .await
            .expect("server-mode SurrealStorage"),
    )
}

async fn embedded_storage(path: String) -> Arc<SurrealStorage> {
    let embedder: Arc<dyn surreal_memory::embeddings::EmbeddingService> = Arc::new(NoOpEmbedder);
    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        endpoint: None,
        embedded_path: Some(path),
        username: None,
        password: None,
        namespace: format!("test_{}", Uuid::new_v4().simple()),
        database: "main".to_string(),
        retry: RetryConfig::default(),
    };
    Arc::new(
        SurrealStorage::new(&config, embedder)
            .await
            .expect("embedded SurrealStorage"),
    )
}

// ── Workload primitives ──────────────────────────────────────────────────────

/// A single operation classification — kept narrow on purpose.
#[derive(Debug, Clone, Copy)]
enum Op {
    AddMemory,
    HybridSearch,
}

fn make_memory(i: usize) -> Memory {
    Memory::new(
        format!("load-repro memory entry #{i} — quick brown fox"),
        Some("anonymous".to_string()),
        Some("load-repro-agent".to_string()),
        None,
        vec!["load-repro".to_string()],
    )
}

async fn run_op(storage: &Arc<SurrealStorage>, op: Op, i: usize) -> Result<(), String> {
    match op {
        Op::AddMemory => storage
            .add_memory(make_memory(i))
            .await
            .map(|_| ())
            .map_err(|e| classify_error(&e)),
        Op::HybridSearch => storage
            .hybrid_search_memories(
                "quick brown fox",
                Some("anonymous"),
                Some("load-repro-agent"),
                None,
                10,
                0.6,
                0.4,
            )
            .await
            .map(|_| ())
            .map_err(|e| classify_error(&e)),
    }
}

fn classify_error(err: &anyhow::Error) -> String {
    let s = format!("{err:#}").to_lowercase();
    if s.contains("timeout") || s.contains("timed out") {
        "timeout".to_string()
    } else if s.contains("lock") {
        "lock".to_string()
    } else if s.contains("serialization") {
        "serialization".to_string()
    } else if s.contains("connection") || s.contains("closed") || s.contains("reset") {
        "connection".to_string()
    } else {
        "other".to_string()
    }
}

// ── Latency aggregation ──────────────────────────────────────────────────────

#[derive(Default)]
struct LatencyReport {
    samples_ms: Vec<u64>,
    errors: std::collections::BTreeMap<String, u64>,
}

impl LatencyReport {
    fn record(&mut self, dur: Duration, result: Result<(), String>) {
        self.samples_ms.push(dur.as_millis() as u64);
        if let Err(class) = result {
            *self.errors.entry(class).or_insert(0) += 1;
        }
    }

    fn percentile(&self, p: f64) -> u64 {
        if self.samples_ms.is_empty() {
            return 0;
        }
        let mut sorted = self.samples_ms.clone();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx]
    }

    fn error_total(&self) -> u64 {
        self.errors.values().sum()
    }

    fn summary_md_row(&self, label: &str) -> String {
        let p50 = self.percentile(0.50);
        let p95 = self.percentile(0.95);
        let p99 = self.percentile(0.99);
        let errs = self
            .errors
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let errs = if errs.is_empty() { "—".to_string() } else { errs };
        format!(
            "| {label} | {} | {p50} | {p95} | {p99} | {} | {errs} |",
            self.samples_ms.len(),
            self.error_total()
        )
    }
}

async fn run_workload(
    storage: Arc<SurrealStorage>,
    concurrency: usize,
    ops_per_task: usize,
    mix: &[Op],
) -> LatencyReport {
    let mut handles = Vec::with_capacity(concurrency);
    let start_offset = Instant::now().elapsed().as_nanos() as usize;

    for t in 0..concurrency {
        let storage = Arc::clone(&storage);
        let mix = mix.to_vec();
        handles.push(tokio::spawn(async move {
            let mut local: Vec<(Duration, Result<(), String>)> = Vec::with_capacity(ops_per_task);
            for i in 0..ops_per_task {
                let op = mix[(t * ops_per_task + i) % mix.len()];
                let item_idx = start_offset.wrapping_add(t * 10_000 + i);
                let started = Instant::now();
                let result = run_op(&storage, op, item_idx).await;
                local.push((started.elapsed(), result));
            }
            local
        }));
    }

    let mut report = LatencyReport::default();
    for h in handles {
        match h.await {
            Ok(samples) => {
                for (dur, res) in samples {
                    report.record(dur, res);
                }
            }
            Err(join_err) => {
                report.errors.insert(format!("join:{join_err}"), 1);
            }
        }
    }
    report
}

// ── Baseline file append ─────────────────────────────────────────────────────

fn append_baseline(section: &str, header: &str, rows: &[String]) {
    use std::fs::OpenOptions;
    use std::io::Write as _;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("load_repro_baseline.md");

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open baseline file");

    let ts = chrono::Utc::now().to_rfc3339();
    writeln!(file, "\n## {section} — {ts}\n").unwrap();
    writeln!(file, "{header}").unwrap();
    writeln!(
        file,
        "|---|---|---|---|---|---|---|"
    )
    .unwrap();
    for row in rows {
        writeln!(file, "{row}").unwrap();
    }
}

const HEADER: &str = "| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |";

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires docker-compose up -d surrealdb"]
async fn server_mixed_load() {
    let namespace = format!("loadrepro_{}", Uuid::new_v4().simple());
    let storage = server_storage(namespace).await;

    let read_only = run_workload(Arc::clone(&storage), 64, 25, &[Op::HybridSearch]).await;
    let write_only = run_workload(Arc::clone(&storage), 64, 25, &[Op::AddMemory]).await;
    let mixed = run_workload(
        Arc::clone(&storage),
        128,
        25,
        &[Op::HybridSearch, Op::AddMemory],
    )
    .await;

    append_baseline(
        "server-mode (pre-refactor baseline)",
        HEADER,
        &[
            read_only.summary_md_row("hybrid_search × 64"),
            write_only.summary_md_row("add_memory × 64"),
            mixed.summary_md_row("mixed 50/50 × 128"),
        ],
    );

    // Test does NOT fail on errors — this is a measurement run.
    // The assertion is informational, captured in the baseline file.
    eprintln!("server_mixed_load: see crates/surreal-memory/tests/load_repro_baseline.md");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "writes to a temp embedded RocksDB path"]
async fn embedded_mixed_load() {
    let temp = std::env::temp_dir().join(format!("loadrepro_{}", Uuid::new_v4().simple()));
    let storage = embedded_storage(temp.to_string_lossy().into_owned()).await;

    let read_only = run_workload(Arc::clone(&storage), 64, 25, &[Op::HybridSearch]).await;
    let write_only = run_workload(Arc::clone(&storage), 64, 25, &[Op::AddMemory]).await;
    let mixed = run_workload(
        Arc::clone(&storage),
        128,
        25,
        &[Op::HybridSearch, Op::AddMemory],
    )
    .await;

    append_baseline(
        "embedded mode (pre-refactor baseline)",
        HEADER,
        &[
            read_only.summary_md_row("hybrid_search × 64"),
            write_only.summary_md_row("add_memory × 64"),
            mixed.summary_md_row("mixed 50/50 × 128"),
        ],
    );

    eprintln!("embedded_mixed_load: see crates/surreal-memory/tests/load_repro_baseline.md");
    // Best-effort cleanup; ignore errors (RocksDB lock files may linger briefly).
    let _ = std::fs::remove_dir_all(&temp);
}
