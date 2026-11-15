//! Paged memory listing on the REST and MCP surfaces (issue #62).
//!
//! Embedded storage and synthetic data only. Both surfaces return one page
//! object (`memories`, `next_cursor`, `has_more`), never a bare array.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rmcp::model::{CallToolResult, ErrorCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::{
    MEMORY_PAGE_BYTE_BUDGET, Memory, MemoryStorage, embeddings::EmbeddingService,
    storage::surreal::SurrealStorage,
};
use surreal_memory_server::{
    api,
    mcp::handlers::{GeneratePersonaMindmapParams, GetAllMemoriesParams, MemoryHandler},
};
use tower::ServiceExt;

const DIM: usize = 64;

/// A stable pseudo-random unit vector per text, so distinct synthetic
/// memories never trip the near-duplicate merge.
struct HashEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for HashEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut raw = Vec::with_capacity(DIM);
        let mut block = 0u32;
        while raw.len() < DIM {
            let mut hasher = Sha256::new();
            hasher.update(block.to_le_bytes());
            hasher.update(text.as_bytes());
            raw.extend(hasher.finalize().iter().map(|b| f32::from(*b) - 127.5));
            block += 1;
        }
        raw.truncate(DIM);
        let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
        Ok(raw.into_iter().map(|v| v / norm).collect())
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed(&text).await?);
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        DIM
    }
}

async fn storage() -> Arc<dyn MemoryStorage> {
    Arc::new(
        SurrealStorage::new_mem(Arc::new(HashEmbedder))
            .await
            .expect("in-memory SurrealStorage"),
    )
}

/// `n` synthetic memories for `user` in `category`.
async fn seed(storage: &Arc<dyn MemoryStorage>, user: &str, category: &str, n: usize) {
    for i in 0..n {
        let memory = Memory::new(
            format!("synthetic {category} note {i} {}", uuid::Uuid::new_v4()),
            Some(user.to_string()),
            None,
            None,
            vec![category.to_string()],
        );
        storage.add_memory(memory).await.expect("add memory");
    }
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tool_json(result: CallToolResult) -> Value {
    let content = serde_json::to_value(&result.content).expect("serialisable content");
    let text = content[0]["text"].as_str().expect("text content");
    serde_json::from_str(text).expect("JSON tool output")
}

fn ids(page: &Value) -> Vec<String> {
    page["memories"]
        .as_array()
        .expect("memories array")
        .iter()
        .map(|m| m["id"].to_string())
        .collect()
}

#[tokio::test]
async fn rest_list_returns_page_objects_that_follow_to_completion() {
    let storage = storage().await;
    seed(&storage, "u1", "work", 12).await;
    seed(&storage, "u2", "work", 3).await;
    let router = api::build_router(Arc::clone(&storage), Arc::new(HashEmbedder));

    let mut uri = "/api/v1/memory?user_id=u1&limit=5".to_string();
    let mut seen = Vec::new();
    let mut pages = 0;
    loop {
        let (status, page) = send(&router, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert!(page.is_object(), "a page object, not a bare array");
        assert!(page["memories"].as_array().unwrap().len() <= 5);
        for memory in page["memories"].as_array().unwrap() {
            assert_eq!(memory["user_id"], json!("u1"));
            assert!(memory["embedding"].is_null(), "no embeddings by default");
        }
        seen.extend(ids(&page));
        pages += 1;
        match page["next_cursor"].as_str() {
            Some(cursor) => {
                assert_eq!(page["has_more"], json!(true));
                uri = format!("/api/v1/memory?user_id=u1&limit=5&cursor={cursor}");
            }
            None => {
                assert_eq!(page["has_more"], json!(false));
                break;
            }
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), 12);
    assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 12, "no repeats");

    let (_, embedded) = send(
        &router,
        "GET",
        "/api/v1/memory?user_id=u1&limit=2&include_embeddings=true",
        None,
    )
    .await;
    for memory in embedded["memories"].as_array().unwrap() {
        assert_eq!(memory["embedding"].as_array().map(Vec::len), Some(DIM));
    }

    for bad in [
        "/api/v1/memory?user_id=u1&limit=201",
        "/api/v1/memory?user_id=u1&limit=0",
        "/api/v1/memory?user_id=u1&limit=26&include_embeddings=true",
        "/api/v1/memory?user_id=u1&cursor=not-a-cursor",
    ] {
        let (status, body) = send(&router, "GET", bad, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}

#[tokio::test]
async fn mcp_get_all_memories_accepts_string_parameters_and_returns_a_page() {
    let storage = storage().await;
    seed(&storage, "u1", "work", 30).await;
    let handler = MemoryHandler::new(Arc::clone(&storage));

    let params: GetAllMemoriesParams = serde_json::from_value(json!({
        "user_id": "u1",
        "limit": "25",
        "include_embeddings": "false",
    }))
    .expect("string-encoded parameters are accepted");
    let result = handler.get_all_memories(params).await.expect("tool call");
    let text_len = serde_json::to_value(&result.content).expect("content")[0]["text"]
        .as_str()
        .expect("text")
        .len();
    // The memories array is within the budget; the envelope adds a few bytes.
    assert!(
        text_len <= MEMORY_PAGE_BYTE_BUDGET + 256,
        "compact page text is {text_len} bytes"
    );
    let page = tool_json(result);
    assert_eq!(page["memories"].as_array().unwrap().len(), 25);
    assert_eq!(page["has_more"], json!(true));
    let cursor = page["next_cursor"].as_str().expect("cursor").to_string();
    assert!(page["memories"][0]["embedding"].is_null());

    let rest = tool_json(
        handler
            .get_all_memories(GetAllMemoriesParams {
                user_id: Some("u1".to_string()),
                cursor: Some(cursor),
                ..GetAllMemoriesParams::default()
            })
            .await
            .expect("second page"),
    );
    assert_eq!(rest["memories"].as_array().unwrap().len(), 5);
    assert_eq!(rest["has_more"], json!(false));
    assert!(rest["next_cursor"].is_null());

    let error = handler
        .get_all_memories(GetAllMemoriesParams {
            limit: Some(500),
            ..GetAllMemoriesParams::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);

    for bad in ["not-a-cursor", "entity:abc", "memory:"] {
        let error = handler
            .get_all_memories(GetAllMemoriesParams {
                user_id: Some("u1".to_string()),
                cursor: Some(bad.to_string()),
                ..GetAllMemoriesParams::default()
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS, "cursor {bad:?}");
    }
}

#[tokio::test]
async fn mcp_page_text_stays_within_the_byte_budget_for_large_memories() {
    let storage = storage().await;
    for i in 0..6 {
        let memory = Memory::new(
            format!("synthetic large {i} {}", "filler ".repeat(40 * 1024)),
            Some("big".to_string()),
            None,
            None,
            Vec::<String>::new(),
        );
        storage.add_memory(memory).await.expect("add memory");
    }
    let handler = MemoryHandler::new(Arc::clone(&storage));
    let result = handler
        .get_all_memories(GetAllMemoriesParams {
            user_id: Some("big".to_string()),
            ..GetAllMemoriesParams::default()
        })
        .await
        .expect("tool call");
    let text_len = serde_json::to_value(&result.content).expect("content")[0]["text"]
        .as_str()
        .expect("text")
        .len();
    assert!(
        text_len <= MEMORY_PAGE_BYTE_BUDGET + 256,
        "compact page text is {text_len} bytes"
    );
    let page = tool_json(result);
    assert_eq!(
        page["has_more"],
        json!(true),
        "the budget shortened the page"
    );
}

fn branches(mindmap: &Value) -> Vec<(String, usize)> {
    let nodes = mindmap["nodes"].as_array().expect("nodes");
    let mut out: Vec<(String, usize)> = nodes
        .iter()
        .filter(|n| n["node_type"] == json!("branch"))
        .map(|branch| {
            let leaves = nodes
                .iter()
                .filter(|n| n["node_type"] == json!("leaf") && n["parent_id"] == branch["id"])
                .count();
            (branch["label"].as_str().unwrap().to_string(), leaves)
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn persona_mindmaps_read_every_page_through_mcp_and_rest() {
    let storage = storage().await;
    // 63 memories: more than one default page (50).
    seed(&storage, "persona", "alpha", 50).await;
    seed(&storage, "persona", "beta", 3).await;
    seed(&storage, "persona", "gamma", 10).await;
    let expected = vec![
        ("alpha".to_string(), 5),
        ("beta".to_string(), 3),
        ("gamma".to_string(), 5),
    ];

    let handler = MemoryHandler::new(Arc::clone(&storage));
    let mcp = tool_json(
        handler
            .generate_persona_mindmap(GeneratePersonaMindmapParams {
                user_id: "persona".to_string(),
                name: "persona-mcp".to_string(),
            })
            .await
            .expect("mcp persona"),
    );
    assert_eq!(branches(&mcp), expected);
    assert_eq!(mcp["description"], json!("Auto-generated from 63 memories"));

    let router = api::build_router(Arc::clone(&storage), Arc::new(HashEmbedder));
    let (status, rest) = send(
        &router,
        "POST",
        "/api/v1/mindmaps/generate/persona",
        Some(json!({"user_id": "persona", "name": "persona-rest"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rest}");
    assert_eq!(branches(&rest), expected);
    assert_eq!(
        rest["description"],
        json!("Auto-generated from 63 memories")
    );
}

#[test]
fn the_get_all_memories_tool_schema_lists_the_paging_parameters() {
    let schema =
        serde_json::to_value(rmcp::schemars::schema_for!(GetAllMemoriesParams)).expect("schema");
    let properties = schema["properties"].as_object().expect("properties");
    for name in [
        "user_id",
        "agent_id",
        "session_id",
        "limit",
        "cursor",
        "include_embeddings",
    ] {
        assert!(
            properties.contains_key(name),
            "schema lists {name}: {schema}"
        );
    }
}
