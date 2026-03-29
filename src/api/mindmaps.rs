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

use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
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

fn internal_err(e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

async fn create_mindmap(
    State(state): State<AppState>,
    Json(body): Json<CreateMindmapBody>,
) -> Result<(StatusCode, Json<MindMap>), (StatusCode, Json<serde_json::Value>)> {
    let map_type = match body.map_type.as_deref().unwrap_or("radial") {
        "concept" => MapType::Concept,
        "argument" => MapType::Argument,
        "tree" => MapType::Tree,
        "temporal" => MapType::Temporal,
        _ => MapType::Radial,
    };
    let mm = MindMap::new(
        body.name,
        map_type,
        body.root_label,
        body.description,
        body.agent_id,
        body.user_id,
    );
    let created = state
        .storage
        .create_mindmap(mm)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_mindmaps(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<MindMap>>, (StatusCode, Json<serde_json::Value>)> {
    let maps = state
        .storage
        .list_mindmaps(q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(Json(maps))
}

async fn get_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Option<MindMap>>, (StatusCode, Json<serde_json::Value>)> {
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn delete_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    state
        .storage
        .delete_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_node(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddNodeBody>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
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
        .add_mindmap_node(&name, q.user_id.as_deref(), node)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn delete_node(
    State(state): State<AppState>,
    Path((name, node_id)): Path<(String, String)>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
    let mm = state
        .storage
        .delete_mindmap_node(&name, q.user_id.as_deref(), &node_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn add_edge(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddEdgeBody>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
    let edge = MindMapEdge {
        from_id: body.from_id,
        to_id: body.to_id,
        label: body.label,
        directed: body.directed,
    };
    let mm = state
        .storage
        .add_mindmap_edge(&name, q.user_id.as_deref(), edge)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn export_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    use surreal_memory::ExportFormat;
    let fmt = match q.format.to_lowercase().as_str() {
        "mermaid" => ExportFormat::Mermaid,
        "markdown" | "md" => ExportFormat::Markdown,
        _ => ExportFormat::Json,
    };
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "not found" })),
            )
        })?;
    Ok(mm.export(&fmt))
}

#[cfg(all(test, feature = "embedded"))]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use surreal_memory::{MemoryStorage, embeddings::EmbeddingService, storage::surreal::SurrealStorage};
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
        Router::new()
            .nest("/api/v1/mindmaps", router())
            .with_state(AppState { storage })
    }

    async fn json_response(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn mindmap_routes_cover_metadata_export_and_delete() {
        let router = router_with_storage(make_storage().await);

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/v1/mindmaps/")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"persona-map","root_label":"Root","map_type":"radial","user_id":"user-1"}"#,
            ))
            .unwrap();
        let create_response = router.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let add_node_request = Request::builder()
            .method("POST")
            .uri("/api/v1/mindmaps/persona-map/nodes?user_id=user-1")
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

        let add_edge_request = Request::builder()
            .method("POST")
            .uri("/api/v1/mindmaps/persona-map/edges?user_id=user-1")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"from_id":"root","to_id":"beliefs","label":"contains","directed":true}"#,
            ))
            .unwrap();
        let add_edge_response = router.clone().oneshot(add_edge_request).await.unwrap();
        assert_eq!(add_edge_response.status(), StatusCode::OK);
        let add_edge_body = json_response(add_edge_response).await;
        assert_eq!(add_edge_body["edges"][0]["label"], "contains");

        let export_request = Request::builder()
            .method("GET")
            .uri("/api/v1/mindmaps/persona-map/export?user_id=user-1&format=mermaid")
            .body(Body::empty())
            .unwrap();
        let export_response = router.clone().oneshot(export_request).await.unwrap();
        assert_eq!(export_response.status(), StatusCode::OK);
        let export_text = String::from_utf8(
            to_bytes(export_response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(export_text.contains("graph TD"));
        assert!(export_text.contains("root -->|contains| beliefs"));

        let delete_node_request = Request::builder()
            .method("DELETE")
            .uri("/api/v1/mindmaps/persona-map/nodes/beliefs?user_id=user-1")
            .body(Body::empty())
            .unwrap();
        let delete_node_response = router.clone().oneshot(delete_node_request).await.unwrap();
        assert_eq!(delete_node_response.status(), StatusCode::OK);
        let delete_node_body = json_response(delete_node_response).await;
        assert_eq!(delete_node_body["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(delete_node_body["edges"].as_array().unwrap().len(), 0);

        let delete_map_request = Request::builder()
            .method("DELETE")
            .uri("/api/v1/mindmaps/persona-map?user_id=user-1")
            .body(Body::empty())
            .unwrap();
        let delete_map_response = router.clone().oneshot(delete_map_request).await.unwrap();
        assert_eq!(delete_map_response.status(), StatusCode::NO_CONTENT);

        let get_request = Request::builder()
            .method("GET")
            .uri("/api/v1/mindmaps/persona-map?user_id=user-1")
            .body(Body::empty())
            .unwrap();
        let get_response = router.oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = json_response(get_response).await;
        assert!(get_body.is_null());
    }
}
