//! Mindmap REST API routes.
//! GET/POST/DELETE /api/v1/mindmaps

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;
use surreal_memory::{MapType, MindMap, MindMapEdge, MindMapNode};
use surrealdb::types::RecordId;

use super::{ApiFailure, AppState, api_error, bad_request, not_found};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/generate/persona", post(generate_persona_mindmap))
        .route("/generate/ideation", post(generate_ideation_mindmap))
        .route("/", post(create_mindmap))
        .route("/", get(list_mindmaps))
        .route("/{name}", get(get_mindmap))
        .route("/{name}", delete(delete_mindmap))
        .route("/{name}/nodes", post(add_node))
        .route("/{name}/nodes/{node_id}", delete(delete_node))
        .route("/{name}/edges", post(add_edge))
        .route("/{name}/export", get(export_mindmap))
}

#[derive(Deserialize)]
struct ScopeQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
}

#[derive(Deserialize)]
struct ExportQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}
fn default_format() -> String {
    "json".to_string()
}

#[derive(Deserialize)]
struct CreateMindmapBody {
    name: String,
    map_type: Option<String>,
    root_label: String,
    description: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
    task_stream_id: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AddNodeBody {
    node_id: String,
    label: String,
    parent_id: Option<String>,
    node_type: Option<String>,
    color: Option<String>,
    metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct AddEdgeBody {
    from_id: String,
    to_id: String,
    label: Option<String>,
    #[serde(default)]
    directed: bool,
}

#[derive(Deserialize)]
struct GeneratePersonaMindmapBody {
    user_id: String,
    name: String,
}

#[derive(Deserialize)]
struct GenerateIdeationMindmapBody {
    topic: String,
    map_type: String,
    context: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
}

fn parse_map_type(raw: &str) -> Result<MapType, ApiFailure> {
    MapType::parse_str(raw).map_err(|_| bad_request(format!("invalid map_type '{}'", raw)))
}

async fn create_mindmap(
    State(state): State<AppState>,
    Json(body): Json<CreateMindmapBody>,
) -> Result<(StatusCode, Json<MindMap>), ApiFailure> {
    let map_type = parse_map_type(body.map_type.as_deref().unwrap_or("radial"))?;
    let mut mm = MindMap::new(
        body.name,
        map_type,
        body.root_label,
        body.description,
        body.agent_id,
        body.user_id,
    );
    if let Some(task_stream_id) = body.task_stream_id {
        mm.task_stream_id = Some(
            RecordId::parse_simple(&task_stream_id)
                .map_err(|e| bad_request(format!("invalid task_stream_id: {e}")))?,
        );
    }
    mm.tags = body.tags.unwrap_or_default();
    let created = state.storage.create_mindmap(mm).await.map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_mindmaps(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<MindMap>>, ApiFailure> {
    let maps = state
        .storage
        .list_mindmaps(q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?;
    Ok(Json(maps))
}

async fn get_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<MindMap>, ApiFailure> {
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Mindmap not found"))?;
    Ok(Json(mm))
}

async fn delete_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<StatusCode, ApiFailure> {
    state
        .storage
        .get_mindmap(&name, q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Mindmap not found"))?;
    state
        .storage
        .delete_mindmap(&name, q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_node(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddNodeBody>,
) -> Result<Json<MindMap>, ApiFailure> {
    let node = MindMapNode {
        id: body.node_id,
        label: body.label,
        parent_id: body.parent_id,
        node_type: body.node_type,
        color: body.color,
        metadata: body.metadata,
    };
    let mm = state
        .storage
        .add_mindmap_node(&name, q.user_id.as_deref(), q.agent_id.as_deref(), node)
        .await
        .map_err(api_error)?;
    Ok(Json(mm))
}

async fn delete_node(
    State(state): State<AppState>,
    Path((name, node_id)): Path<(String, String)>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<MindMap>, ApiFailure> {
    let mindmap = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Mindmap not found"))?;
    if !mindmap.nodes.iter().any(|node| node.id == node_id) {
        return Err(not_found(format!("Mindmap node '{}' not found", node_id)));
    }
    let mm = state
        .storage
        .delete_mindmap_node(&name, q.user_id.as_deref(), q.agent_id.as_deref(), &node_id)
        .await
        .map_err(api_error)?;
    Ok(Json(mm))
}

async fn add_edge(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddEdgeBody>,
) -> Result<Json<MindMap>, ApiFailure> {
    let edge = MindMapEdge {
        from_id: body.from_id,
        to_id: body.to_id,
        label: body.label,
        directed: body.directed,
    };
    let mm = state
        .storage
        .add_mindmap_edge(&name, q.user_id.as_deref(), q.agent_id.as_deref(), edge)
        .await
        .map_err(api_error)?;
    Ok(Json(mm))
}

async fn export_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<String, ApiFailure> {
    use surreal_memory::ExportFormat;
    let fmt = match q.format.to_lowercase().as_str() {
        "mermaid" => ExportFormat::Mermaid,
        "markdown" | "md" => ExportFormat::Markdown,
        _ => ExportFormat::Json,
    };
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Mindmap not found"))?;
    Ok(mm.export(&fmt))
}

async fn generate_persona_mindmap(
    State(state): State<AppState>,
    Json(body): Json<GeneratePersonaMindmapBody>,
) -> Result<(StatusCode, Json<MindMap>), ApiFailure> {
    use std::collections::HashMap;

    // Fetch all memories for this user and cluster by category
    let memories = state
        .storage
        .get_all_memories(Some(&body.user_id), None, None)
        .await
        .map_err(api_error)?;

    let mut mm = MindMap::new(
        body.name.clone(),
        MapType::Radial,
        format!("Persona: {}", body.user_id),
        Some(format!("Auto-generated from {} memories", memories.len())),
        None,
        Some(body.user_id.clone()),
    );

    // Cluster memories by first category tag, default to "general"
    let mut clusters: HashMap<String, Vec<String>> = HashMap::new();
    for mem in &memories {
        let cat = mem
            .categories
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        clusters.entry(cat).or_default().push(mem.content.clone());
    }

    for (cat, items) in &clusters {
        let branch_id = cat.replace(' ', "_");
        mm.nodes.push(MindMapNode {
            id: branch_id.clone(),
            label: cat.clone(),
            parent_id: Some("root".to_string()),
            node_type: Some("branch".to_string()),
            color: None,
            metadata: None,
        });
        for (i, item) in items.iter().take(5).enumerate() {
            let leaf_id = format!("{}_leaf_{}", branch_id, i);
            let short = if item.len() > 80 {
                format!("{}…", &item[..80])
            } else {
                item.clone()
            };
            mm.nodes.push(MindMapNode {
                id: leaf_id,
                label: short,
                parent_id: Some(branch_id.clone()),
                node_type: Some("leaf".to_string()),
                color: None,
                metadata: None,
            });
        }
    }

    let created = state.storage.create_mindmap(mm).await.map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn generate_ideation_mindmap(
    State(state): State<AppState>,
    Json(body): Json<GenerateIdeationMindmapBody>,
) -> Result<(StatusCode, Json<MindMap>), ApiFailure> {
    let map_type = parse_map_type(&body.map_type)?;
    let mm = MindMap::new(
        format!("ideation_{}", body.topic.replace(' ', "_").to_lowercase()),
        map_type,
        body.topic.clone(),
        body.context,
        body.agent_id,
        body.user_id,
    );
    let created = state.storage.create_mindmap(mm).await.map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[cfg(all(test, feature = "embedded"))]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use surreal_memory::{
        MemoryStorage, embeddings::EmbeddingService, storage::surreal::SurrealStorage,
    };
    use tower::ServiceExt;

    use super::*;

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

    async fn make_storage() -> Arc<dyn MemoryStorage> {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        Arc::new(
            SurrealStorage::new_mem(embedder)
                .await
                .expect("in-memory SurrealStorage"),
        )
    }

    fn router_with_storage(storage: Arc<dyn MemoryStorage>) -> Router {
        router().with_state(AppState { storage })
    }

    async fn json_response(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn mindmap_routes_cover_metadata_and_get() {
        let router = router_with_storage(make_storage().await);

        let request = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"test-map","root_label":"Root"}"#))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_generate_persona_mindmap() {
        let storage = make_storage().await;
        let router = router_with_storage(Arc::clone(&storage));

        // Add some memories first
        let mem1 = surreal_memory::Memory::new(
            "test content 1".to_string(),
            Some("user-persona".to_string()),
            None,
            None,
            vec!["work".to_string()],
        );
        let mem2 = surreal_memory::Memory::new(
            "test content 2".to_string(),
            Some("user-persona".to_string()),
            None,
            None,
            vec!["personal".to_string()],
        );
        storage.add_memory(mem1).await.unwrap();
        storage.add_memory(mem2).await.unwrap();

        let generate_request = Request::builder()
            .method("POST")
            .uri("/generate/persona")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"user_id":"user-persona","name":"my-persona"}"#,
            ))
            .unwrap();
        let generate_response = router.oneshot(generate_request).await.unwrap();
        assert_eq!(generate_response.status(), StatusCode::CREATED);
        let body = json_response(generate_response).await;
        assert_eq!(body["name"], "my-persona");
        assert_eq!(body["map_type"], "radial");
        assert!(body["nodes"].as_array().unwrap().len() > 1); // root + category nodes
    }

    #[tokio::test]
    async fn test_generate_ideation_mindmap() {
        let router = router_with_storage(make_storage().await);

        let generate_request = Request::builder()
            .method("POST")
            .uri("/generate/ideation")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"topic":"Feature Planning","map_type":"tree","context":"Planning for Q2","agent_id":"agent-1"}"#,
            ))
            .unwrap();
        let generate_response = router.oneshot(generate_request).await.unwrap();
        assert_eq!(generate_response.status(), StatusCode::CREATED);
        let body = json_response(generate_response).await;
        assert_eq!(body["map_type"], "tree");
        assert_eq!(body["nodes"][0]["label"], "Feature Planning");
    }

    #[tokio::test]
    async fn get_mindmap_returns_404_for_missing_record() {
        let router = router_with_storage(make_storage().await);
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/missing-map?user_id=user-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_mindmap_rejects_invalid_map_type() {
        let router = router_with_storage(make_storage().await);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"bad-map","root_label":"Root","map_type":"invalid"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ideation_mindmap_rejects_invalid_map_type() {
        let router = router_with_storage(make_storage().await);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/generate/ideation")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"topic":"Feature Planning","map_type":"invalid"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn mindmap_routes_cover_metadata_fields() {
        let router = router_with_storage(make_storage().await);

        // Create mindmap
        let create_request = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"persona-map","root_label":"Root","map_type":"radial","user_id":"user-1","task_stream_id":"task_stream:stream-1","tags":["persona","seed"]}"#,
            ))
            .unwrap();
        let create_response = router.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let create_body = json_response(create_response).await;
        assert_eq!(create_body["task_stream_id"]["table"], "task_stream");
        assert_eq!(create_body["task_stream_id"]["key"]["String"], "stream-1");
        assert_eq!(create_body["tags"][0], "persona");
        assert_eq!(create_body["tags"][1], "seed");

        let add_node_request = Request::builder()
            .method("POST")
            .uri("/persona-map/nodes?user_id=user-1")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"node_id":"beliefs","label":"Beliefs","parent_id":"root","metadata":{"source":"memory","details":{"confidence":0.9}}}"#,
            ))
            .unwrap();
        let add_node_response = router.clone().oneshot(add_node_request).await.unwrap();
        assert_eq!(add_node_response.status(), StatusCode::OK);
        let add_node_body = json_response(add_node_response).await;
        assert_eq!(
            add_node_body["nodes"][1]["metadata"]["details"]["confidence"],
            serde_json::json!(0.9)
        );
        let get_request = Request::builder()
            .method("GET")
            .uri("/persona-map?user_id=user-1")
            .body(Body::empty())
            .unwrap();
        let get_response = router.oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = json_response(get_response).await;
        assert_eq!(get_body["name"], "persona-map");
        assert_eq!(get_body["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(get_body["tags"][0], "persona");
    }
}
