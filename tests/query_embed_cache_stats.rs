//! `GET /api/v2/operations/stats` reports the search query-embedding cache
//! (`query_embed_cache`) through the real router over an embedded SurrealDB
//! engine. Searches go through `POST /api/v1/search`, the real hybrid-search
//! route, so the counters prove the production path uses the cache.

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use surreal_memory::{EmbeddingService, Memory, MemoryStorage, SurrealStorage};
use surreal_memory_server::api::build_router;
use tower::ServiceExt;

const DIMENSIONS: usize = 8;
const USER: &str = "stats-user";

struct AxisEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for AxisEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut vector = vec![0.0f32; DIMENSIONS];
        vector[if text == "seed memory" { 0 } else { 1 }] = 1.0;
        Ok(vector)
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed(&text).await?);
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        DIMENSIONS
    }
}

async fn fixture(capacity: usize) -> Router {
    let embedder: Arc<dyn EmbeddingService> = Arc::new(AxisEmbedder);
    let storage = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&embedder))
            .await
            .expect("embedded storage")
            .with_query_embed_cache_capacity(capacity),
    );
    storage
        .add_memory(Memory::new(
            "seed memory".to_owned(),
            Some(USER.to_owned()),
            None,
            None,
            vec![],
        ))
        .await
        .expect("seed memory");
    build_router(storage as Arc<dyn MemoryStorage>, embedder)
}

async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn search(router: &Router, query: &str) {
    let (status, body) = call(
        router,
        "POST",
        "/api/v1/search",
        Some(json!({"query": query, "user_id": USER, "limit": 3})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:#}");
}

async fn cache_stats(router: &Router) -> Value {
    let (status, stats) = call(router, "GET", "/api/v2/operations/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{stats:#}");
    stats["query_embed_cache"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stats_report_query_embedding_cache_hits_and_misses() {
    let router = fixture(512).await;
    let initial = cache_stats(&router).await;
    assert_eq!(initial["capacity"], 512, "{initial:#}");
    assert_eq!(initial["hits"], 0);
    assert_eq!(initial["misses"], 0);

    for _ in 0..3 {
        search(&router, "recall the release plan").await;
    }
    search(&router, "a different question").await;

    let stats = cache_stats(&router).await;
    assert_eq!(stats["misses"], 2, "{stats:#}");
    assert_eq!(stats["hits"], 2, "{stats:#}");
    assert_eq!(stats["entries"], 2, "{stats:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_cache_reports_capacity_zero_and_counts_every_search_as_a_miss() {
    let router = fixture(0).await;
    for _ in 0..3 {
        search(&router, "recall the release plan").await;
    }
    let stats = cache_stats(&router).await;
    assert_eq!(stats["capacity"], 0, "{stats:#}");
    assert_eq!(stats["hits"], 0, "{stats:#}");
    assert_eq!(stats["misses"], 3, "{stats:#}");
}
