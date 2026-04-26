//! Memory Palace REST API routes.
//!
//! - `GET  /api/v1/palace/wake`            — load identity context
//! - `GET  /api/v1/palace/recall`          — on-demand recall
//! - `POST /api/v1/palace/search`          — deep semantic search
//! - `POST /api/v1/palace/ingest`          — store content
//! - `DELETE /api/v1/palace/drawer/{id}`    — delete a drawer
//! - `GET  /api/v1/palace/status`          — palace status summary
//! - `POST /api/v1/palace/hybrid-search`   — unified cross-store search

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use surreal_memory::PalaceStorage;

use super::{ApiFailure, AppState, api_error, bad_request};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/wake", get(wake_up))
        .route("/recall", get(recall))
        .route("/search", post(search))
        .route("/ingest", post(ingest))
        .route("/drawer/{id}", delete(delete_drawer))
        .route("/status", get(status))
        .route("/hybrid-search", post(hybrid_search))
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn get_palace(state: &AppState) -> Result<&surreal_memory::SurrealStorage, ApiFailure> {
    state
        .storage
        .as_any()
        .downcast_ref::<surreal_memory::SurrealStorage>()
        .ok_or_else(|| bad_request("Palace features require SurrealStorage backend"))
}

// ── query / body structs ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WakeQuery {
    wing: Option<String>,
    #[serde(default)]
    compress: bool,
}

#[derive(Deserialize)]
struct RecallQuery {
    wing: Option<String>,
    room: Option<String>,
    #[serde(default = "default_recall_limit")]
    limit: usize,
    #[serde(default)]
    compress: bool,
}
fn default_recall_limit() -> usize {
    20
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    wing: Option<String>,
    room: Option<String>,
    #[serde(default = "default_n")]
    n: usize,
    #[serde(default)]
    compress: bool,
}
fn default_n() -> usize {
    10
}

#[derive(Deserialize)]
struct IngestBody {
    content: String,
    wing: String,
    room: String,
    #[serde(default = "default_hall")]
    hall: String,
    #[serde(default = "default_importance")]
    importance: f32,
}
fn default_hall() -> String {
    "general".to_string()
}
fn default_importance() -> f32 {
    1.0
}

#[derive(Deserialize)]
struct HybridSearchBody {
    query: String,
    scope: Option<String>,
    wing: Option<String>,
    #[serde(default = "default_n")]
    n: usize,
    #[serde(default)]
    compress: bool,
}

#[derive(Serialize)]
struct TextResponse {
    result: String,
}

#[derive(Serialize)]
struct IngestResponse {
    drawer_id: String,
}

#[derive(Serialize)]
struct DeleteResponse {
    deleted: bool,
    id: String,
}

// ── handlers ────────────────────────────────────────────────────────────────

async fn wake_up(
    State(state): State<AppState>,
    Query(q): Query<WakeQuery>,
) -> Result<Json<TextResponse>, ApiFailure> {
    let storage = get_palace(&state)?;
    let result = storage
        .palace_wake_up(q.wing.as_deref())
        .await
        .map_err(api_error)?;
    let output = if q.compress {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(TextResponse { result: output }))
}

async fn recall(
    State(state): State<AppState>,
    Query(q): Query<RecallQuery>,
) -> Result<Json<TextResponse>, ApiFailure> {
    let storage = get_palace(&state)?;
    let result = storage
        .palace_recall(q.wing.as_deref(), q.room.as_deref(), q.limit)
        .await
        .map_err(api_error)?;
    let output = if q.compress {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(TextResponse { result: output }))
}

async fn search(
    State(state): State<AppState>,
    Json(body): Json<SearchBody>,
) -> Result<Json<TextResponse>, ApiFailure> {
    if body.query.trim().is_empty() {
        return Err(bad_request("query cannot be empty"));
    }
    let storage = get_palace(&state)?;
    let result = storage
        .palace_search(
            &body.query,
            body.wing.as_deref(),
            body.room.as_deref(),
            body.n,
        )
        .await
        .map_err(api_error)?;
    let output = if body.compress {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(TextResponse { result: output }))
}

async fn ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestBody>,
) -> Result<Json<IngestResponse>, ApiFailure> {
    if body.content.trim().is_empty() {
        return Err(bad_request("content cannot be empty"));
    }
    if body.wing.trim().is_empty() {
        return Err(bad_request("wing cannot be empty"));
    }
    if body.room.trim().is_empty() {
        return Err(bad_request("room cannot be empty"));
    }
    let storage = get_palace(&state)?;
    let drawer_id = storage
        .palace_ingest(
            &body.content,
            &body.wing,
            &body.room,
            &body.hall,
            body.importance,
        )
        .await
        .map_err(api_error)?;
    Ok(Json(IngestResponse { drawer_id }))
}

async fn delete_drawer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiFailure> {
    if id.trim().is_empty() {
        return Err(bad_request("id cannot be empty"));
    }
    let storage = get_palace(&state)?;
    storage.palace_delete(&id).await.map_err(api_error)?;
    Ok(Json(DeleteResponse { deleted: true, id }))
}

async fn status(
    State(state): State<AppState>,
) -> Result<Json<surreal_memory::PalaceStatus>, ApiFailure> {
    let storage = get_palace(&state)?;
    let status = storage.palace_status().await.map_err(api_error)?;
    Ok(Json(status))
}

async fn hybrid_search(
    State(state): State<AppState>,
    Json(body): Json<HybridSearchBody>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    if body.query.trim().is_empty() {
        return Err(bad_request("query cannot be empty"));
    }
    let storage = get_palace(&state)?;
    let scope = body.scope.as_deref().and_then(|s| match s {
        "global" => Some(surreal_memory::MemoryScope::Global),
        "agent" => Some(surreal_memory::MemoryScope::Agent),
        "user" => Some(surreal_memory::MemoryScope::User),
        "session" => Some(surreal_memory::MemoryScope::Session),
        _ => None,
    });
    let hits = storage
        .palace_hybrid_search(&body.query, scope, body.wing.as_deref(), body.n)
        .await
        .map_err(api_error)?;
    if body.compress {
        let json = serde_json::to_string_pretty(&hits).unwrap_or_default();
        let compressed = storage.palace_compress(&json);
        Ok(Json(serde_json::json!({ "result": compressed })))
    } else {
        Ok(Json(serde_json::to_value(&hits).unwrap_or_default()))
    }
}
