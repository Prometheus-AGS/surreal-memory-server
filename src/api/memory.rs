//! Memory REST API routes.
//! POST/GET/PUT/DELETE /api/v1/memory

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use surreal_memory::Memory;

use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(add_memory))
        .route("/", get(get_all_memories))
        .route("/", delete(delete_all_memories))
        .route("/:id", get(get_memory))
        .route("/:id", put(update_memory))
        .route("/:id", delete(delete_memory))
        .route("/:id/history", get(get_memory_history))
}

#[derive(Deserialize)]
struct ScopeQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct AddMemoryBody {
    content: String,
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
    categories: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct UpdateMemoryBody {
    content: String,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

fn internal_err(e: impl ToString) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            error: e.to_string(),
        }),
    )
}

async fn add_memory(
    State(state): State<AppState>,
    Json(body): Json<AddMemoryBody>,
) -> Result<(StatusCode, Json<Memory>), (StatusCode, Json<ApiError>)> {
    let memory = Memory::new(
        body.content,
        body.user_id,
        body.agent_id,
        body.session_id,
        body.categories.unwrap_or_default(),
    );
    let created = state
        .storage
        .add_memory(memory)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_all_memories(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<Memory>>, (StatusCode, Json<ApiError>)> {
    let mems = state
        .storage
        .get_all_memories(
            q.user_id.as_deref(),
            q.agent_id.as_deref(),
            q.session_id.as_deref(),
        )
        .await
        .map_err(internal_err)?;
    Ok(Json(mems))
}

async fn get_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<Memory>>, (StatusCode, Json<ApiError>)> {
    let mem = state.storage.get_memory(&id).await.map_err(internal_err)?;
    Ok(Json(mem))
}

async fn update_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMemoryBody>,
) -> Result<Json<Memory>, (StatusCode, Json<ApiError>)> {
    let updated = state
        .storage
        .update_memory(&id, body.content)
        .await
        .map_err(internal_err)?;
    Ok(Json(updated))
}

async fn delete_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    state
        .storage
        .delete_memory(&id)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_all_memories(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let count = state
        .storage
        .delete_all_memories(
            q.user_id.as_deref(),
            q.agent_id.as_deref(),
            q.session_id.as_deref(),
        )
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "deleted": count })))
}

async fn get_memory_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let history = state
        .storage
        .get_memory_history(&id)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!(history)))
}
