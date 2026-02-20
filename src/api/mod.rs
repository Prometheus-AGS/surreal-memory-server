//! REST API module — Axum-based HTTP server for surreal-memory-server.
//!
//! Runs alongside the MCP stdio server on a configurable port.
//! Routes: /health, /api/v1/memory, /api/v1/entities, /api/v1/mindmaps, /api/v1/search
//! A2A: /agent.json, /a2a/tasks/*

pub mod a2a;
pub mod entities;
pub mod memory;
pub mod mindmaps;
pub mod search;

use axum::{Router, routing::get};
use serde_json::json;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::storage::MemoryStorage;

/// Shared application state passed to every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn MemoryStorage>,
}

/// Build the full Axum router.
pub fn build_router(storage: Arc<dyn MemoryStorage>) -> Router {
    let state = AppState {
        storage: Arc::clone(&storage),
    };

    // The MCP HTTP service captures storage in its factory closure, returning Router<()>.
    // Convert it to Router<AppState> via .with_state(()) before merging.
    let mcp_sub: Router<AppState> = crate::mcp::http::mcp_http_router(Arc::clone(&storage), "/mcp")
        .with_state(());

    Router::new()
        .route("/health", get(health_handler))
        .nest("/api/v1/memory", memory::router())
        .nest("/api/v1/entities", entities::router())
        .nest("/api/v1/mindmaps", mindmaps::router())
        .nest("/api/v1/search", search::router())
        .merge(a2a::router())
        .merge(mcp_sub)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "surreal-memory-server"
    }))
}
