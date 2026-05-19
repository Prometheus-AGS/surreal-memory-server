//! REST API module — Axum-based HTTP server for surreal-memory-server.
//!
//! Runs alongside the MCP stdio server on a configurable port.
//! Routes: /health, /api/v1/memory, /api/v1/entities, /api/v1/taskstreams, /api/v1/mindmaps, /api/v1/search
//! A2A: /agent.json, /a2a/tasks/*

pub mod a2a;
pub mod entities;
pub mod memory;
pub mod mindmaps;
#[cfg(feature = "palace")]
pub mod palace;
pub mod search;
pub mod taskstreams;

use axum::extract::State;
use axum::{Json, http::StatusCode};
use axum::{Router, routing::get};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::embeddings::EmbeddingService;
use crate::storage::MemoryStorage;

/// Shared application state passed to every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn MemoryStorage>,
    /// The embedding provider. `/health` queries `is_ready()` on it to
    /// distinguish "process up" from "write path actually ready" — accurate
    /// whether the model was loaded via startup warmup or lazily on first use.
    pub embedding_service: Arc<dyn EmbeddingService>,
}

#[derive(Serialize)]
pub(crate) struct ApiError {
    error: String,
}

pub(crate) type ApiFailure = (StatusCode, Json<ApiError>);

pub(crate) fn api_error(error: anyhow::Error) -> ApiFailure {
    let message = error.to_string();
    let lowered = message.to_lowercase();
    let status = if lowered.contains("not found") {
        StatusCode::NOT_FOUND
    } else if lowered.contains("invalid")
        || lowered.contains("unknown ")
        || lowered.contains("already exists")
        || lowered.contains("duplicate")
        || lowered.contains("references unknown")
        || lowered.contains("cannot be empty")
        || lowered.contains("is not active")
        || lowered.contains("expected a record")
        || lowered.contains("record id")
        || lowered.contains("parse")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiError { error: message }))
}

pub(crate) fn bad_request(message: impl Into<String>) -> ApiFailure {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            error: message.into(),
        }),
    )
}

pub(crate) fn not_found(message: impl Into<String>) -> ApiFailure {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: message.into(),
        }),
    )
}

/// Build the full Axum router.
pub fn build_router(
    storage: Arc<dyn MemoryStorage>,
    embedding_service: Arc<dyn EmbeddingService>,
) -> Router {
    let state = AppState {
        storage: Arc::clone(&storage),
        embedding_service,
    };

    // The MCP HTTP service captures storage in its factory closure, returning Router<()>.
    // Convert it to Router<AppState> via .with_state(()) before merging.
    let mcp_sub: Router<AppState> =
        crate::mcp::http::mcp_http_router(Arc::clone(&storage), "/mcp").with_state(());

    let router = Router::new()
        .route("/health", get(health_handler))
        .nest("/api/v1/memory", memory::router())
        .nest("/api/v1/entities", entities::router())
        .nest("/api/v1/taskstreams", taskstreams::router())
        .nest("/api/v1/mindmaps", mindmaps::router())
        .nest("/api/v1/search", search::router());

    #[cfg(feature = "palace")]
    let router = router.nest("/api/v1/palace", palace::router());

    router
        .merge(a2a::router())
        .merge(mcp_sub)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health_handler(State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    let embedding_ready = state.embedding_service.is_ready();
    axum::Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "surreal-memory-server",
        "embedding_ready": embedding_ready,
        "write_path_ready": embedding_ready
    }))
}
