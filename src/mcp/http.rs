//! HTTP/SSE MCP transport — wraps `MemoryMcpServer` in rmcp's `StreamableHttpService`
//! and exposes it as an Axum route at `POST/GET/DELETE /mcp`.
//!
//! This enables any HTTP client (e.g., remote agents, web apps) to speak the
//! MCP Streamable HTTP transport specification in addition to the stdio transport.

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::{mcp::MemoryMcpServer, storage::MemoryStorage};

/// Build an `axum::Router` that handles the streamable-http MCP transport at `prefix`.
pub fn mcp_http_router(storage: Arc<dyn MemoryStorage>, prefix: &str) -> axum::Router {
    let storage = Arc::clone(&storage);

    // Session manager — tracks open SSE connections keyed by Mcp-Session-Id header.
    let session_manager = Arc::new(LocalSessionManager::default());

    // Use the Default config — keeps all fields populated correctly regardless
    // of which patch of rmcp we compile against.
    let config = StreamableHttpServerConfig {
        stateful_mode: true,
        ..StreamableHttpServerConfig::default()
    };

    // Service factory — invoked once per new client initialize request.
    let http_service = StreamableHttpService::new(
        move || -> Result<MemoryMcpServer, std::io::Error> {
            Ok(MemoryMcpServer::new(Arc::clone(&storage)))
        },
        Arc::clone(&session_manager),
        config,
    );

    // `StreamableHttpService` implements `tower::Service`, so Axum can route to it
    // directly via `route_service`. It handles GET (SSE), POST, and DELETE on one path.
    axum::Router::new().route_service(prefix, http_service)
}
