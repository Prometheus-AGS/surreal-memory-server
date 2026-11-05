//! Expired memories are hidden on the exposed REST and MCP surfaces (issue #52).
//!
//! Both surfaces delegate to the same storage methods, so this proves they
//! inherit the live-memory view without a TTL sweep having run.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use surreal_memory::{
    Memory, MemoryStorage, embeddings::EmbeddingService, storage::surreal::SurrealStorage,
};
use surreal_memory_server::{
    api,
    mcp::handlers::{MemoryHandler, MemoryIdParams},
};
use surrealdb::types::{Datetime, RecordIdKey};
use tower::ServiceExt;

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

fn id_of(memory: &Memory) -> String {
    let id = memory.id.as_ref().expect("stored memory has an id");
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

async fn add(storage: &Arc<dyn MemoryStorage>, content: &str, valid_until: Option<&str>) -> String {
    let mut memory = Memory::new(
        content.to_string(),
        Some("u1".to_string()),
        None,
        None,
        Vec::<String>::new(),
    );
    memory.valid_until = valid_until.map(|v| v.parse::<Datetime>().expect("timestamp"));
    id_of(&storage.add_memory(memory).await.expect("add memory"))
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn rest_and_mcp_hide_expired_memories_without_a_sweep() {
    let storage: Arc<dyn MemoryStorage> = Arc::new(
        SurrealStorage::new_mem(Arc::new(NoOpEmbedder))
            .await
            .expect("in-memory SurrealStorage"),
    );
    let expired = add(&storage, "expired note", Some("2000-01-01T00:00:00Z")).await;
    let future = add(&storage, "future note", Some("2999-01-01T00:00:00Z")).await;
    let never = add(&storage, "never expires", None).await;
    let router = api::build_router(Arc::clone(&storage), Arc::new(NoOpEmbedder));

    // REST: by id.
    let (status, _) = get(&router, &format!("/api/v1/memory/{expired}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expired memory is hidden by id"
    );
    for live in [&future, &never] {
        let (status, body) = get(&router, &format!("/api/v1/memory/{live}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["content"].is_string());
    }

    // REST: list.
    let (status, body) = get(&router, "/api/v1/memory?user_id=u1").await;
    assert_eq!(status, StatusCode::OK);
    let mut contents: Vec<&str> = body
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect();
    contents.sort_unstable();
    assert_eq!(contents, vec!["future note", "never expires"]);

    // MCP: the handler returns the stored value as JSON text, `null` when absent.
    let handler = MemoryHandler::new(Arc::clone(&storage));
    let text = |result: rmcp::model::CallToolResult| {
        serde_json::to_string(&result.content).expect("serialisable content")
    };
    let hidden = handler
        .get_memory(MemoryIdParams { id: expired })
        .await
        .expect("handler call");
    assert!(
        text(hidden).contains("null"),
        "expired memory is hidden over MCP"
    );
    let visible = handler
        .get_memory(MemoryIdParams { id: never })
        .await
        .expect("handler call");
    assert!(text(visible).contains("never expires"));
}
