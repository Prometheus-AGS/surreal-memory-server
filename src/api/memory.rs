//! Memory REST API routes.
//! POST/GET/PUT/DELETE /api/v1/memory

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use surreal_memory::Memory;

use crate::contracts::{AddMemoryRequest, UpdateMemoryRequest};

use super::{ApiFailure, AppState, api_error, bad_request, not_found};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(add_memory))
        .route("/", get(get_all_memories))
        .route("/", delete(delete_all_memories))
        .route("/{id}", get(get_memory))
        .route("/{id}", put(update_memory))
        .route("/{id}", delete(delete_memory))
        .route("/{id}/history", get(get_memory_history))
}

#[derive(Deserialize)]
struct ScopeQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
}

async fn add_memory(
    State(state): State<AppState>,
    Json(body): Json<AddMemoryRequest>,
) -> Result<(StatusCode, Json<Memory>), ApiFailure> {
    body.validate().map_err(|e| bad_request(e.to_string()))?;
    let created = state
        .storage
        .add_memory(body.into_memory())
        .await
        .map_err(api_error)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_all_memories(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<Memory>>, ApiFailure> {
    let mems = state
        .storage
        .get_all_memories(
            q.user_id.as_deref(),
            q.agent_id.as_deref(),
            q.session_id.as_deref(),
        )
        .await
        .map_err(api_error)?;
    Ok(Json(mems))
}

async fn get_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Memory>, ApiFailure> {
    let mem = state
        .storage
        .get_memory(&id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Memory not found"))?;
    Ok(Json(mem))
}

async fn update_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMemoryRequest>,
) -> Result<Json<Memory>, ApiFailure> {
    body.validate().map_err(|e| bad_request(e.to_string()))?;
    let updated = state
        .storage
        .update_memory(&id, body.content)
        .await
        .map_err(api_error)?;
    Ok(Json(updated))
}

async fn delete_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiFailure> {
    state
        .storage
        .get_memory(&id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| not_found("Memory not found"))?;
    state.storage.delete_memory(&id).await.map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_all_memories(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let count = state
        .storage
        .delete_all_memories(
            q.user_id.as_deref(),
            q.agent_id.as_deref(),
            q.session_id.as_deref(),
        )
        .await
        .map_err(api_error)?;
    Ok(Json(serde_json::json!({ "deleted": count })))
}

async fn get_memory_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let history = state
        .storage
        .get_memory_history(&id)
        .await
        .map_err(api_error)?;
    Ok(Json(serde_json::json!(history)))
}

#[cfg(all(test, feature = "embedded"))]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
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
            .nest("/api/v1/memory", router())
            .with_state(AppState {
                storage,
                embedding_service: Arc::new(NoOpEmbedder),
                operations,
            })
    }

    #[tokio::test]
    async fn get_memory_returns_404_for_missing_record() {
        let router = router_with_storage(make_storage().await);
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/memory/missing-memory")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
