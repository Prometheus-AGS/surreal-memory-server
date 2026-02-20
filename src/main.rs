use anyhow::{Context, Result};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod config;
mod embeddings;
mod mcp;
mod storage;
mod workers;

use config::Config;
use embeddings::{EmbeddingService, create_embedding_service};
use mcp::MemoryMcpServer;
use storage::{MemoryStorage, create_storage};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    tracing::info!(
        "🚀 Starting Rust Memory MCP Server v{}",
        env!("CARGO_PKG_VERSION")
    );

    let config = load_config().await?;
    let embedding_service = init_embedding_service(&config).await?;

    let api_port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);

    let storage = init_storage(config, embedding_service).await?;

    // Graceful shutdown channel — send `true` to stop all workers.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn Ctrl+C handler that broadcasts the shutdown signal.
    let shutdown_tx2 = shutdown_tx.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("🛑 Received shutdown signal");
                let _ = shutdown_tx2.send(true);
            }
            Err(e) => tracing::error!("Failed to listen for shutdown signal: {}", e),
        }
    });

    // ── Background TTL worker ─────────────────────────────────────────────────
    let ttl_storage = Arc::clone(&storage);
    let ttl_rx = shutdown_rx.clone();
    let ttl_handle =
        tokio::spawn(async move { workers::ttl::run_ttl_worker(ttl_storage, ttl_rx).await });

    // ── Axum REST API + HTTP/SSE MCP ─────────────────────────────────────────
    let api_storage = Arc::clone(&storage);
    let api_handle = tokio::spawn(async move { run_api_server(api_storage, api_port).await });

    // ── MCP stdio ─────────────────────────────────────────────────────────────
    let mcp_handle = tokio::spawn(async move { run_mcp_server(storage).await });

    tokio::select! {
        result = api_handle => {
            match result {
                Ok(Ok(())) => tracing::info!("REST API server stopped"),
                Ok(Err(e)) => tracing::error!("REST API server error: {}", e),
                Err(e) => tracing::error!("REST API task panic: {}", e),
            }
        }
        result = mcp_handle => {
            match result {
                Ok(Ok(())) => tracing::info!("MCP server stopped"),
                Ok(Err(e)) => tracing::error!("MCP server error: {}", e),
                Err(e) => tracing::error!("MCP task panic: {}", e),
            }
        }
        result = ttl_handle => {
            match result {
                Ok(Ok(())) => tracing::info!("TTL worker stopped"),
                Ok(Err(e)) => tracing::error!("TTL worker error: {}", e),
                Err(e) => tracing::error!("TTL worker panic: {}", e),
            }
        }
    }

    // Broadcast shutdown to any remaining tasks.
    let _ = shutdown_tx.send(true);
    tracing::info!("👋 Server shut down gracefully");
    Ok(())
}

fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "surreal_memory_server=info,surreal=warn".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
        .init();
}

async fn load_config() -> Result<Config> {
    tracing::info!("📋 Loading configuration...");
    let config = Config::from_env().context("Failed to load configuration")?;
    tracing::info!(
        "✅ Configuration loaded — SurrealDB mode: {:?}",
        config.surreal_mode
    );
    match &config.embedding_provider {
        embeddings::EmbeddingProvider::OpenAI { model, .. } => {
            tracing::info!("   Embedding: OpenAI ({})", model);
        }
        embeddings::EmbeddingProvider::Cohere { model, .. } => {
            tracing::info!("   Embedding: Cohere ({})", model);
        }
        embeddings::EmbeddingProvider::Local { model_id, .. } => {
            tracing::info!("   Embedding: Local ({})", model_id);
        }
    }
    Ok(config)
}

async fn init_embedding_service(config: &Config) -> Result<Arc<dyn EmbeddingService>> {
    tracing::info!("🧠 Initializing embedding service...");
    let service = create_embedding_service(config.embedding_provider.clone())
        .await
        .context("Failed to create embedding service")?;
    tracing::info!(
        "✅ Embedding service ready ({} dimensions)",
        service.dimensions()
    );
    Ok(Arc::from(service))
}

async fn init_storage(
    config: Config,
    embedding_service: Arc<dyn EmbeddingService>,
) -> Result<Arc<dyn MemoryStorage>> {
    tracing::info!("💾 Initializing storage...");
    let storage = create_storage(&config, embedding_service)
        .await
        .context("Failed to initialize SurrealDB storage")?;
    tracing::info!("✅ Storage initialized successfully");
    Ok(storage)
}

async fn run_mcp_server(storage: Arc<dyn MemoryStorage>) -> Result<()> {
    tracing::info!("🎯 Starting MCP server on stdio transport...");
    let server = MemoryMcpServer::new(storage);
    server.run().await.context("MCP server error")
}

async fn run_api_server(storage: Arc<dyn MemoryStorage>, port: u16) -> Result<()> {
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("🌐 Starting REST API + HTTP MCP server on http://{}", addr);
    let router = api::build_router(storage);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind REST API port")?;
    axum::serve(listener, router)
        .await
        .context("REST API server error")
}
