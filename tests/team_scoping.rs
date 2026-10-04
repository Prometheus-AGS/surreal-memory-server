//! Team-aware scoping through the real HTTP router over an embedded SurrealDB
//! engine (no external server): lean search responses, category filtering on
//! both hybrid legs and in KNN search, and the loopback-only re-key operation.
//!
//! Design: prometheus-skill-system docs/design/team-aware-learning-memory.md §2, §8.

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::{EmbeddingService, Memory, MemoryStorage, SurrealStorage, TaskStream};
use surreal_memory_server::api::build_router;
use tower::ServiceExt;

const DIMENSIONS: usize = 8;
const QUERY: &str = "learning query";

/// Unit vector `degrees` from the query (axis 0) towards `axis`. Two vectors on
/// different axes have similarity cos(a)·cos(b), which keeps every seed below
/// `add_memory`'s 0.92 near-duplicate threshold so none are merged.
fn at(axis: usize, degrees: f32) -> Vec<f32> {
    let radians = degrees.to_radians();
    let mut vector = vec![0.0f32; DIMENSIONS];
    vector[0] = radians.cos();
    vector[axis] = radians.sin();
    vector
}

struct TableEmbedder(HashMap<String, Vec<f32>>);

#[async_trait::async_trait]
impl EmbeddingService for TableEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.0
            .get(text)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no test vector for {text:?}"))
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

/// (content, agent_id, categories, vector).
/// Seven untagged memories sit close to the query; three tagged
/// `aud:role:ui-dev` sit far away, outside the unfiltered top-k, so a filtered
/// search can only return them if the filter is applied inside the
/// nearest-neighbour search.
fn seed() -> Vec<(String, Option<&'static str>, Vec<&'static str>, Vec<f32>)> {
    let mut rows = Vec::new();
    for index in 0..7 {
        rows.push((
            format!("near-{index}"),
            Some("@project"),
            vec!["kind:lesson"],
            at(index + 1, 17.0 + index as f32),
        ));
    }
    for (index, degrees) in [120.0f32, 130.0, 140.0].into_iter().enumerate() {
        rows.push((
            format!("far-ui-{index}"),
            Some("@project"),
            vec!["kind:lesson", "aud:role:ui-dev"],
            at(index + 1, degrees),
        ));
    }
    rows
}

async fn fixture() -> (Router, Arc<SurrealStorage>) {
    let mut vectors: HashMap<String, Vec<f32>> = seed()
        .into_iter()
        .map(|(content, _, _, vector)| (content, vector))
        .collect();
    vectors.insert(QUERY.to_owned(), at(1, 0.0));
    // Far from every seeded row: an add_memory without agent_id looks for
    // near-duplicates across ALL agents in the user scope (> 0.92 similarity)
    // and would otherwise overwrite another agent's memory.
    for (index, degrees) in [80.0f32, 84.0, 88.0].into_iter().enumerate() {
        vectors.insert(format!("unattributed-{index}"), at(index + 4, degrees));
    }
    vectors.insert("attributed-kept".to_owned(), at(7, 89.0));
    let embedder: Arc<dyn EmbeddingService> = Arc::new(TableEmbedder(vectors));
    let storage = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&embedder))
            .await
            .expect("embedded storage"),
    );
    for (content, agent, categories, _) in seed() {
        storage
            .add_memory(Memory::new(
                content,
                Some("project:fixture".to_owned()),
                agent.map(str::to_owned),
                None,
                categories.into_iter().map(str::to_owned).collect(),
            ))
            .await
            .expect("seed memory");
    }
    let router = build_router(Arc::clone(&storage) as Arc<dyn MemoryStorage>, embedder);
    (router, storage)
}

async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    peer: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn operation(id: &str, payload: Value) -> Value {
    let hash = Sha256::digest(serde_json::to_vec(&payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({
        "operation_id": id,
        "schema_version": 2,
        "kind": "rekey_agent_id",
        "dependencies": [],
        "payload_hash": hash,
        "payload": payload
    })
}

async fn committed_result(router: &Router, id: &str) -> Value {
    for _ in 0..200 {
        let (status, receipt) = call(
            router,
            "GET",
            &format!("/api/v2/operations/{id}"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{receipt:#}");
        match receipt["state"].as_str() {
            Some("committed") => return receipt["result"].clone(),
            Some("failed") | Some("rejected") => panic!("operation {id} failed: {receipt:#}"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("operation {id} never committed");
}

fn contents(results: &Value) -> Vec<String> {
    results
        .as_array()
        .unwrap()
        .iter()
        .map(|memory| memory["content"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_responses_omit_embeddings_unless_requested() {
    let (router, _) = fixture().await;
    let (status, lean) = call(
        &router,
        "POST",
        "/api/v1/search",
        Some(json!({"query": QUERY, "user_id": "project:fixture", "limit": 3})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lean:#}");
    let lean = lean.as_array().unwrap();
    assert!(!lean.is_empty(), "lean search returned nothing");
    for memory in lean {
        assert!(
            memory.get("embedding").is_none(),
            "embedding leaked: {memory:#}"
        );
        assert!(memory["content"].is_string());
    }

    let (_, full) = call(
        &router,
        "POST",
        "/api/v1/search",
        Some(json!({"query": QUERY, "user_id": "project:fixture", "limit": 3, "include_embeddings": true})),
        None,
    )
    .await;
    assert!(
        full[0]["embedding"].is_array(),
        "include_embeddings did not restore the vector: {full:#}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn category_filter_applies_on_both_hybrid_legs_and_inside_knn() {
    let (router, storage) = fixture().await;
    let categories = vec!["aud:role:ui-dev".to_owned()];

    // REST hybrid: only tagged rows, and all three even though every one lies
    // outside the unfiltered top-3.
    let (status, filtered) = call(
        &router,
        "POST",
        "/api/v1/search",
        Some(json!({"query": QUERY, "user_id": "project:fixture", "limit": 3, "categories": categories})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{filtered:#}");
    let mut names = contents(&filtered);
    names.sort();
    assert_eq!(names, ["far-ui-0", "far-ui-1", "far-ui-2"], "{filtered:#}");

    // Unfiltered control: the tagged rows are not in the top-3.
    let (_, unfiltered) = call(
        &router,
        "POST",
        "/api/v1/search",
        Some(json!({"query": QUERY, "user_id": "project:fixture", "limit": 3})),
        None,
    )
    .await;
    assert!(
        contents(&unfiltered)
            .iter()
            .all(|name| name.starts_with("near-")),
        "control expected only near rows: {unfiltered:#}"
    );

    // The KNN search used by the MCP search_memories tool honours categories
    // (it used to accept and ignore them).
    let knn = storage
        .search_memories(
            QUERY,
            Some("project:fixture"),
            None,
            None,
            Some(&categories),
            3,
        )
        .await
        .expect("knn search");
    let mut knn_names: Vec<_> = knn.iter().map(|memory| memory.content.clone()).collect();
    knn_names.sort();
    assert_eq!(knn_names, ["far-ui-0", "far-ui-1", "far-ui-2"]);

    // An empty category list means no filter, not "match nothing".
    let empty: Vec<String> = Vec::new();
    let all = storage
        .hybrid_search_memories(
            QUERY,
            Some("project:fixture"),
            None,
            None,
            Some(&empty),
            3,
            0.7,
            0.3,
        )
        .await
        .expect("hybrid search");
    assert_eq!(all.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rekey_dry_run_then_apply_then_idempotent_replay() {
    let (router, storage) = fixture().await;
    // Unattributed memories: absent agent_id and the empty string.
    for (index, agent) in [None, Some(""), None].into_iter().enumerate() {
        storage
            .add_memory(Memory::new(
                format!("unattributed-{index}"),
                Some("project:fixture".to_owned()),
                agent.map(str::to_owned),
                None,
                vec![],
            ))
            .await
            .expect("seed unattributed");
    }
    storage
        .add_memory(Memory::new(
            "attributed-kept",
            Some("project:fixture".to_owned()),
            Some("tlm-fixture/api-dev".to_owned()),
            None,
            vec![],
        ))
        .await
        .expect("seed attributed");
    // Unattributed task stream (stored with agent_id "").
    storage
        .create_task_stream(TaskStream::new(
            "legacy-stream".to_owned(),
            None,
            None,
            Some("project:fixture".to_owned()),
        ))
        .await
        .expect("seed stream");

    let dry = operation(
        "rekey-dry",
        json!({"to_agent_id": "@project", "user_id": "project:fixture", "dry_run": true}),
    );
    let (status, receipt) = call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(dry),
        Some("127.0.0.1:50000"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{receipt:#}");
    let report = committed_result(&router, "rekey-dry").await;
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["memories"], 3, "{report:#}");
    assert_eq!(report["task_streams"], 1, "{report:#}");
    assert!(
        storage
            .get_all_memories(Some("project:fixture"), Some("@project"), None)
            .await
            .unwrap()
            .iter()
            .all(|memory| !memory.content.starts_with("unattributed-")),
        "dry run must not write"
    );

    let apply_body = operation(
        "rekey-apply",
        json!({"to_agent_id": "@project", "user_id": "project:fixture"}),
    );
    let (status, _) = call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(apply_body.clone()),
        Some("[::1]:50001"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let report = committed_result(&router, "rekey-apply").await;
    assert_eq!(report["memories"], 3, "{report:#}");
    assert_eq!(report["task_streams"], 1, "{report:#}");
    let rekeyed: Vec<String> = storage
        .get_all_memories(Some("project:fixture"), Some("@project"), None)
        .await
        .unwrap()
        .into_iter()
        .map(|memory| memory.content)
        .filter(|content| content.starts_with("unattributed-"))
        .collect();
    assert_eq!(rekeyed.len(), 3, "{rekeyed:?}");
    let kept = storage
        .get_all_memories(Some("project:fixture"), Some("tlm-fixture/api-dev"), None)
        .await
        .unwrap();
    assert_eq!(kept.len(), 1, "an attributed memory must not be re-keyed");

    // Replaying the same operation id is idempotent: 200, not a second run.
    let (status, replay) = call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(apply_body),
        Some("127.0.0.1:50002"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay:#}");
    let after = storage
        .get_all_memories(Some("project:fixture"), Some("@project"), None)
        .await
        .unwrap()
        .len();
    let second_dry = operation(
        "rekey-dry-2",
        json!({"to_agent_id": "@project", "user_id": "project:fixture", "dry_run": true}),
    );
    call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(second_dry),
        Some("127.0.0.1:50003"),
    )
    .await;
    let report = committed_result(&router, "rekey-dry-2").await;
    assert_eq!(
        report["memories"], 0,
        "nothing left unattributed: {report:#}"
    );
    assert!(after >= 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rekey_is_refused_from_non_loopback_or_unknown_peers() {
    let (router, _) = fixture().await;
    let body = operation("rekey-remote", json!({"to_agent_id": "@project"}));
    let (status, error) = call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(body.clone()),
        Some("10.0.0.5:4242"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error:#}");
    let (status, _) = call(&router, "POST", "/api/v2/operations", Some(body), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "no connect-info must fail closed"
    );
    // Other operation kinds are unaffected by the peer check.
    let memory = json!({"content": "near-0", "user_id": "project:fixture", "agent_id": "@project"});
    let hash = Sha256::digest(serde_json::to_vec(&memory).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let (status, receipt) = call(
        &router,
        "POST",
        "/api/v2/operations",
        Some(json!({"operation_id": "plain-add", "schema_version": 2, "kind": "add_memory", "dependencies": [], "payload_hash": hash, "payload": memory})),
        Some("10.0.0.5:4242"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{receipt:#}");
}
