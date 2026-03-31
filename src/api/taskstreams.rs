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

use super::AppState;

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

fn internal_err(e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

fn not_found(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": message })),
    )
}

async fn create_task_stream(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskStreamBody>,
) -> Result<(StatusCode, Json<TaskStream>), (StatusCode, Json<serde_json::Value>)> {
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name cannot be empty" })),
        ));
    }

    let stream = TaskStream::new(body.name, body.description, body.agent_id, body.user_id);
    let created = state
        .storage
        .create_task_stream(stream)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_task_streams(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<TaskStream>>, (StatusCode, Json<serde_json::Value>)> {
    let streams = state
        .storage
        .list_task_streams(q.agent_id.as_deref(), q.user_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(Json(streams))
}

async fn get_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, (StatusCode, Json<serde_json::Value>)> {
    let stream = state
        .storage
        .get_task_stream(&name)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| not_found("Task stream not found"))?;
    Ok(Json(stream))
}

async fn delete_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, (StatusCode, Json<serde_json::Value>)> {
    let archived = state
        .storage
        .archive_task_stream(&name)
        .await
        .map_err(internal_err)?;
    Ok(Json(archived))
}

async fn archive_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, (StatusCode, Json<serde_json::Value>)> {
    let archived = state
        .storage
        .archive_task_stream(&name)
        .await
        .map_err(internal_err)?;
    Ok(Json(archived))
}

async fn pause_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TaskStream>, (StatusCode, Json<serde_json::Value>)> {
    let paused = state
        .storage
        .pause_task_stream(&name)
        .await
        .map_err(internal_err)?;
    Ok(Json(paused))
}

async fn add_memory_to_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<AddToTaskStreamBody>,
) -> Result<(StatusCode, Json<Memory>), (StatusCode, Json<serde_json::Value>)> {
    let mut memory = Memory::new(
        body.content,
        body.user_id,
        body.agent_id,
        None,
        body.categories.unwrap_or_default(),
    );
    memory.token_count = Some(memory.content.split_whitespace().count() as u32);

    let stored = state
        .storage
        .add_to_task_stream(&name, memory)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

async fn get_context_for_task(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ContextQuery>,
) -> Result<Json<ContextWindow>, (StatusCode, Json<serde_json::Value>)> {
    let context = state
        .storage
        .get_context_for_task(
            &name,
            q.model_name.as_deref().unwrap_or("default"),
            q.max_tokens,
        )
        .await
        .map_err(internal_err)?;
    Ok(Json(context))
}

async fn auto_summarize_task_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<AutoSummarizeBody>>,
) -> Result<Json<AutoSummarizeResponse>, (StatusCode, Json<serde_json::Value>)> {
    let body = body.map(|Json(body)| body).unwrap_or(AutoSummarizeBody {
        user_id: None,
        agent_id: None,
        model_id: None,
    });

    let stream = state
        .storage
        .get_task_stream(&name)
        .await
        .map_err(internal_err)?
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
        .map_err(internal_err)?;

    let updated = state
        .storage
        .get_task_stream(&name)
        .await
        .map_err(internal_err)?
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
            .nest("/api/v1/taskstreams", router())
            .with_state(AppState { storage })
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
            .uri("/api/v1/taskstreams/")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"feature-x","description":"Investigate feature x","agent_id":"agent-1","user_id":"user-1"}"#,
            ))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_response(response).await;
        assert_eq!(body["name"], "feature-x");
        assert_eq!(body["agent_id"], "agent-1");
        assert_eq!(body["user_id"], "user-1");
    }

    #[tokio::test]
    async fn task_stream_routes_cover_add_context_and_summarize() {
        let router = router_with_storage(make_storage().await);

        let create_request = Request::builder()
            .method("POST")
            .uri("/api/v1/taskstreams/")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"research","user_id":"user-2","agent_id":"agent-2"}"#,
            ))
            .unwrap();
        let create_response = router.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);

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

        let summarize_request = Request::builder()
            .method("POST")
            .uri("/api/v1/taskstreams/research/summarize")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let summarize_response = router.clone().oneshot(summarize_request).await.unwrap();
        assert_eq!(summarize_response.status(), StatusCode::OK);
        let summarize_body = json_response(summarize_response).await;
        assert_eq!(summarize_body["stream"]["summary_count"], 1);
        assert_eq!(summarize_body["summary"]["categories"][0], "auto_summary");

        let archive_request = Request::builder()
            .method("DELETE")
            .uri("/api/v1/taskstreams/research")
            .body(Body::empty())
            .unwrap();
        let archive_response = router.clone().oneshot(archive_request).await.unwrap();
        assert_eq!(archive_response.status(), StatusCode::OK);
        let archive_body = json_response(archive_response).await;
        assert_eq!(archive_body["status"], "archived");

        let get_request = Request::builder()
            .method("GET")
            .uri("/api/v1/taskstreams/research")
            .body(Body::empty())
            .unwrap();
        let get_response = router.oneshot(get_request).await.unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = json_response(get_response).await;
        assert_eq!(get_body["status"], "archived");
    }
}
