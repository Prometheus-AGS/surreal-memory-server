//! TaskStream REST API routes.
//! POST/GET/DELETE /api/v1/taskstreams

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use surreal_memory::{ContextWindow, Memory, TaskStream};

use super::{ApiFailure, AppState, api_error, bad_request, not_found};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_task_stream))
        .route("/", get(list_task_streams))
        .route("/{name}", get(get_task_stream))
        .route("/{name}", delete(delete_task_stream))
        .route("/{name}/archive", post(archive_task_stream))
        .route("/{name}/pause", post(pause_task_stream))
        .route("/{name}/memories", post(add_memory_to_task_stream))
        .route("/{name}/context", get(get_context_for_task))
        .route("/{name}/summarize", post(auto_summarize_task_stream))
}

#[derive(Deserialize)]
struct ScopeQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
}

#[derive(Deserialize)]
struct ContextQuery {
    model_name: Option<String>,
    max_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct CreateTaskStreamBody {
    name: String,
    description: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
    model_id: Option<String>,
    auto_summarize: Option<bool>,
}

#[derive(Deserialize)]
struct AddToTaskStreamBody {
    content: String,
    user_id: Option<String>,
    agent_id: Option<String>,
    categories: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AutoSummarizeBody {
    user_id: Option<String>,
    agent_id: Option<String>,
    model_id: Option<String>,
}

#[derive(Serialize)]
struct AutoSummarizeResponse {
    stream: TaskStream,
    summary: Option<Memory>,
}

async fn create_task_stream(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskStreamBody>,
) -> Result<(StatusCode, Json<TaskStream>), ApiFailure> {
    if body.name.trim().is_empty() {
        return Err(bad_request("name cannot be empty"));
    }

    let mut stream = TaskStream::new(body.name, body.description, body.agent_id, body.user_id);
    stream.model_id = body.model_id;
    if let Some(auto_summarize) = body.auto_summarize {
        stream.auto_summarize = auto_summarize;
    }
    let created = state
        .storage
        .create_task_stream(stream)
        .await
        .map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_task_streams(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<TaskStream>>, ApiFailure> {
    let streams = state
        .storage
        .list_task_streams(q.agent_id.as_deref(), q.user_id.as_deref())
        .await
        .map_err(api_error)?;
    Ok(Json(streams))
}

async fn get_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, ApiFailure> {
    let stream = state
        .storage
        .get_task_stream(&name, None, None)
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Task stream not found"))?;
    Ok(Json(stream))
}

async fn delete_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiFailure> {
    state
        .storage
        .delete_task_stream(&name, None, None)
        .await
        .map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn archive_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, ApiFailure> {
    let archived = state
        .storage
        .archive_task_stream(&name, None, None)
        .await
        .map_err(api_error)?;
    Ok(Json(archived))
}

async fn pause_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, ApiFailure> {
    let paused = state
        .storage
        .pause_task_stream(&name, None, None)
        .await
        .map_err(api_error)?;
    Ok(Json(paused))
}

async fn add_memory_to_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<AddToTaskStreamBody>,
) -> Result<(StatusCode, Json<Memory>), ApiFailure> {
    let memory = Memory::new(
        body.content,
        body.user_id,
        body.agent_id,
        None,
        body.categories.unwrap_or_default(),
    );

    let stored = state
        .storage
        .add_to_task_stream(&name, None, None, memory)
        .await
        .map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

async fn get_context_for_task(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ContextQuery>,
) -> Result<Json<ContextWindow>, ApiFailure> {
    let context = state
        .storage
        .get_context_for_task(
            &name,
            None,
            None,
            q.model_name.as_deref().unwrap_or("default"),
            q.max_tokens,
        )
        .await
        .map_err(api_error)?;
    Ok(Json(context))
}

async fn auto_summarize_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<AutoSummarizeBody>>,
) -> Result<Json<AutoSummarizeResponse>, ApiFailure> {
    let body = body.map(|Json(body)| body).unwrap_or(AutoSummarizeBody {
        user_id: None,
        agent_id: None,
        model_id: None,
    });

    let stream = state
        .storage
        .get_task_stream(&name, None, None)
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Task stream not found"))?;

    let summary = state
        .storage
        .auto_summarize_task_stream(
            &name,
            body.user_id.as_deref().or(stream.user_id.as_deref()),
            body.agent_id.as_deref().or(stream.agent_id.as_deref()),
            body.model_id
                .as_deref()
                .or(stream.model_id.as_deref())
                .unwrap_or("default"),
        )
        .await
        .map_err(api_error)?;

    let updated = state
        .storage
        .get_task_stream(&name, None, None)
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Task stream not found"))?;

    Ok(Json(AutoSummarizeResponse {
        stream: updated,
        summary,
    }))
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
        let operations = crate::operations::OperationService::start(
            Arc::clone(&storage),
            Arc::new(NoOpEmbedder),
        );
        Router::new()
            .nest("/api/v1/taskstreams", router())
            .with_state(AppState {
                storage,
                embedding_service: Arc::new(NoOpEmbedder),
                operations,
            })
    }

    async fn json_response(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn create_task_stream_rest_route_returns_201() {
        let router = router_with_storage(make_storage().await);
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/taskstreams")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"feature-x","description":"Investigate feature x","agent_id":"agent-1","user_id":"user-1","model_id":"gpt-4o","auto_summarize":false}"#,
            ))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_response(response).await;
        assert_eq!(body["name"], "feature-x");
        assert_eq!(body["agent_id"], "agent-1");
        assert_eq!(body["user_id"], "user-1");
        assert_eq!(body["model_id"], "gpt-4o");
        assert_eq!(body["auto_summarize"], false);
    }

    #[tokio::test]
    async fn task_stream_routes_cover_add_context_and_get() {
        let router = router_with_storage(make_storage().await);

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/v1/taskstreams")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"research","user_id":"user-2","agent_id":"agent-2","model_id":"default","auto_summarize":true}"#,
            ))
            .unwrap();
        let create_response = router.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let create_body = json_response(create_response).await;
        assert_eq!(create_body["model_id"], "default");
        assert_eq!(create_body["auto_summarize"], true);

        for content in ["step one", "step two"] {
            let add_request = Request::builder()
                .method("POST")
                .uri("/api/v1/taskstreams/research/memories")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"content":"{content}","user_id":"user-2","agent_id":"agent-2"}}"#
                )))
                .unwrap();
            let add_response = router.clone().oneshot(add_request).await.unwrap();
            assert_eq!(add_response.status(), StatusCode::CREATED);
        }

        let context_request = Request::builder()
            .method("GET")
            .uri("/api/v1/taskstreams/research/context?model_name=gpt-4o&max_tokens=20")
            .body(Body::empty())
            .unwrap();
        let context_response = router.clone().oneshot(context_request).await.unwrap();
        assert_eq!(context_response.status(), StatusCode::OK);
        let context_body = json_response(context_response).await;
        assert_eq!(context_body["model_name"], "gpt-4o");
        assert_eq!(context_body["memories"].as_array().unwrap().len(), 2);

        let get_request = Request::builder()
            .method("GET")
            .uri("/api/v1/taskstreams/research")
            .body(Body::empty())
            .unwrap();
        let get_response = router.oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = json_response(get_response).await;
        assert_eq!(get_body["status"], "active");
        assert_eq!(get_body["model_id"], "default");
    }

    #[tokio::test]
    async fn delete_task_stream_route_removes_stream_memories_and_detaches_mindmaps() {
        let storage = make_storage().await;
        let stream = storage
            .create_task_stream(TaskStream::new(
                "delete-me",
                Some("cleanup test".to_string()),
                Some("agent-9".to_string()),
                Some("user-9".to_string()),
            ))
            .await
            .expect("create task stream");
        storage
            .add_to_task_stream(
                "delete-me",
                None,
                None,
                Memory::new(
                    "important step".to_string(),
                    Some("user-9".to_string()),
                    Some("agent-9".to_string()),
                    None,
                    vec!["task".to_string()],
                ),
            )
            .await
            .expect("add task memory");

        let mut mindmap = surreal_memory::MindMap::new(
            "linked-map",
            surreal_memory::MapType::Radial,
            "Root",
            None,
            Some("agent-9".to_string()),
            Some("user-9".to_string()),
        );
        mindmap.task_stream_id = stream.id.clone();
        storage
            .create_mindmap(mindmap)
            .await
            .expect("create mindmap");

        let router = router_with_storage(Arc::clone(&storage));
        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/taskstreams/delete-me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        assert!(
            storage
                .get_task_stream("delete-me", None, None)
                .await
                .expect("get task stream")
                .is_none()
        );
        assert!(
            storage
                .get_all_memories(Some("user-9"), Some("agent-9"), None)
                .await
                .expect("list memories")
                .is_empty()
        );
        let mindmap = storage
            .get_mindmap("linked-map", Some("user-9"), Some("agent-9"))
            .await
            .expect("get mindmap")
            .expect("mindmap should survive delete");
        assert!(mindmap.task_stream_id.is_none());
    }

    #[tokio::test]
    async fn archive_and_pause_missing_task_stream_return_404() {
        let router = router_with_storage(make_storage().await);

        for uri in [
            "/api/v1/taskstreams/missing/archive",
            "/api/v1/taskstreams/missing/pause",
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn adding_memory_to_paused_task_stream_returns_400() {
        let storage = make_storage().await;
        storage
            .create_task_stream(TaskStream::new(
                "paused-task",
                None,
                Some("agent-3".to_string()),
                Some("user-3".to_string()),
            ))
            .await
            .expect("create task stream");
        storage
            .pause_task_stream("paused-task", None, None)
            .await
            .expect("pause task stream");

        let router = router_with_storage(storage);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/taskstreams/paused-task/memories")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"content":"should fail","user_id":"user-3","agent_id":"agent-3"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
