//! Hybrid search REST API route.
//! POST /api/v1/search

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use surreal_memory::Memory;

use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", post(hybrid_search))
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_vector_weight")]
    vector_weight: f32,
    #[serde(default = "default_bm25_weight")]
    bm25_weight: f32,
}
fn default_limit() -> usize {
    10
}
fn default_vector_weight() -> f32 {
    0.7
}
fn default_bm25_weight() -> f32 {
    0.3
}

async fn hybrid_search(
    State(state): State<AppState>,
    Json(body): Json<SearchBody>,
) -> Result<Json<Vec<Memory>>, (StatusCode, Json<serde_json::Value>)> {
    let results = state
        .storage
        .hybrid_search_memories(
            &body.query,
            body.user_id.as_deref(),
            body.agent_id.as_deref(),
            body.session_id.as_deref(),
            body.limit,
            body.vector_weight,
            body.bm25_weight,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(results))
}
