//! Latency of one query embedding through the real supervised executor
//! (change-tlh-05, task 1). Ignored by default: it needs the local MLX
//! executor and its cached model, and it measures wall-clock time.
//!
//! Run explicitly, on a quiet machine:
//!
//! ```text
//! cargo test --test query_embedding_latency -- --ignored --nocapture
//! ```
//!
//! The executor and model come from the same environment variables the
//! installed service uses (`LOCAL_EMBEDDING_*`, `MODEL_CACHE_DIR`,
//! `HF_HUB_CACHE`), falling back to the values of the shipped launchd plist.
//! It prints `IDLE_P50_SECONDS=`, `IDLE_P95_SECONDS=` and
//! `LOADED_P95_SECONDS=` lines for `docs/perf/query-embedding-latency.md`.

use std::{path::PathBuf, sync::Arc, time::Duration, time::Instant};

use surreal_memory::EmbeddingService;
use surreal_memory_server::executor::SupervisedEmbeddingService;

const SAMPLES: usize = 30;
const WRITERS: usize = 8;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn summarise(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (percentile(&samples, 50.0), percentile(&samples, 95.0))
}

async fn measure(service: &SupervisedEmbeddingService, label: &str) -> Vec<f64> {
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Distinct text per sample, so no layer can serve a repeat.
        let text = format!("{label} query {i}: how does recall scope pk entries for role delivery?");
        let started = Instant::now();
        service.embed(&text).await.expect("query embedding");
        out.push(started.elapsed().as_secs_f64());
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the local MLX executor and model; run explicitly on a quiet machine"]
async fn query_embedding_latency_idle_and_loaded() {
    let executor = PathBuf::from(env_or(
        "LOCAL_EMBEDDING_EXECUTOR",
        "/usr/local/bin/surreal-memory-mlx-executor",
    ));
    assert!(executor.exists(), "executor not found: {}", executor.display());
    let dimensions: usize = env_or("LOCAL_EMBEDDING_DIMENSIONS", "384").parse().unwrap();
    let home = std::env::var("HOME").unwrap_or_default();
    let child_env = vec![
        ("EMBEDDING_PROVIDER".to_owned(), env_or("EMBEDDING_PROVIDER", "local")),
        ("LOCAL_EMBEDDING_MODEL".to_owned(), env_or("LOCAL_EMBEDDING_MODEL", "BAAI/bge-small-en-v1.5")),
        (
            "LOCAL_EMBEDDING_MODEL_REVISION".to_owned(),
            env_or("LOCAL_EMBEDDING_MODEL_REVISION", "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"),
        ),
        ("LOCAL_EMBEDDING_DIMENSIONS".to_owned(), dimensions.to_string()),
        ("LOCAL_EMBEDDING_BACKEND".to_owned(), env_or("LOCAL_EMBEDDING_BACKEND", "mlx")),
        ("LOCAL_EMBEDDING_DEVICE".to_owned(), env_or("LOCAL_EMBEDDING_DEVICE", "auto")),
        ("MODEL_CACHE_DIR".to_owned(), env_or("MODEL_CACHE_DIR", &format!("{home}/.cache/huggingface"))),
        ("HF_HUB_CACHE".to_owned(), env_or("HF_HUB_CACHE", &format!("{home}/.cache/huggingface/hub"))),
        ("SURREAL_EXECUTOR_STARTUP_MS".to_owned(), env_or("SURREAL_EXECUTOR_STARTUP_MS", "300000")),
    ];
    let service = Arc::new(SupervisedEmbeddingService::with_child_env(
        executor,
        dimensions,
        Duration::from_secs(60),
        child_env,
    ));

    // Warm the model once so the measurement excludes the cold load.
    let warm = Instant::now();
    service.embed("warm-up").await.expect("warm-up embedding");
    println!("WARMUP_SECONDS={:.3}", warm.elapsed().as_secs_f64());

    let (idle_p50, idle_p95) = summarise(measure(&service, "idle").await);

    // Loaded: writers embed continuously through the same executor while the
    // queries are measured, as memory writes do during a SubagentStart recall.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut writers = Vec::new();
    for w in 0..WRITERS {
        let service = service.clone();
        let stop = stop.clone();
        writers.push(tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let text = format!("writer {w} memory {n}: a lesson recorded during the phase");
                let _ = service.embed(&text).await;
                n += 1;
            }
        }));
    }
    let (loaded_p50, loaded_p95) = summarise(measure(&service, "loaded").await);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for writer in writers {
        let _ = writer.await;
    }

    println!("SAMPLES={SAMPLES}");
    println!("WRITERS={WRITERS}");
    println!("IDLE_P50_SECONDS={idle_p50:.3}");
    println!("IDLE_P95_SECONDS={idle_p95:.3}");
    println!("LOADED_P50_SECONDS={loaded_p50:.3}");
    println!("LOADED_P95_SECONDS={loaded_p95:.3}");
}
