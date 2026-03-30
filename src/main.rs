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

/// Parse retry configuration from environment variables with sensible defaults.
fn parse_retry_config_from_env() -> surreal_memory::RetryConfig {
    use std::env;

    let max_connect_retries = env::var("SURREAL_MAX_CONNECT_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let max_operation_retries = env::var("SURREAL_MAX_OPERATION_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let base_retry_delay_ms = env::var("SURREAL_BASE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);

    let max_retry_delay_ms = env::var("SURREAL_MAX_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);

    let jitter_factor = env::var("SURREAL_RETRY_JITTER_FACTOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.25);

    surreal_memory::RetryConfig {
        max_connect_retries,
        max_operation_retries,
        base_retry_delay_ms,
        max_retry_delay_ms,
        jitter_factor,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    tracing::info!(
        "🚀 Starting Rust Memory MCP Server v{}",
        env!("CARGO_PKG_VERSION")
    );

    let config = load_config().await?;
    let embedding_service = init_embedding_service(&config).await?;
    let retry_config = parse_retry_config_from_env();

    let api_port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);

    let storage = init_storage(config, embedding_service, retry_config).await?;

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
    let enable_stdio_mcp = std::env::var("MCP_STDIO")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase()
        != "false";

    let mcp_handle = tokio::spawn(async move {
        if enable_stdio_mcp {
            run_mcp_server(storage).await
        } else {
            // Keep the future pending forever so tokio::select! doesn't exit
            std::future::pending().await
        }
    });

    tokio::select! {
        result = api_handle => {
            match result {
                Ok(Ok(())) => tracing::info!("REST API server stopped"),
                Ok(Err(e)) => tracing::error!("REST API server error: {}", e),
                Err(e) => tracing::error!("REST API task panic: {}", e),
            }
        }
        result = mcp_handle => {
            if enable_stdio_mcp {
                match result {
                    Ok(Ok(())) => tracing::info!("MCP server stopped"),
                    Ok(Err(e)) => tracing::error!("MCP server error: {}", e),
                    Err(e) => tracing::error!("MCP task panic: {}", e),
                }
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
    retry_config: surreal_memory::RetryConfig,
) -> Result<Arc<dyn MemoryStorage>> {
    tracing::info!("💾 Initializing storage...");
    let storage = create_storage(&config, embedding_service, retry_config)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Global lock to ensure tests don't run concurrently and mutate env vars
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_retry_config_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Clear any environment variables
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10);
        assert_eq!(config.max_operation_retries, 3);
        assert_eq!(config.base_retry_delay_ms, 100);
        assert_eq!(config.max_retry_delay_ms, 5000);
        assert_eq!(config.jitter_factor, 0.25);
    }

    #[test]
    fn test_retry_config_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();

        unsafe {
            std::env::set_var("SURREAL_MAX_CONNECT_RETRIES", "20");
            std::env::set_var("SURREAL_MAX_OPERATION_RETRIES", "5");
            std::env::set_var("SURREAL_BASE_RETRY_DELAY_MS", "200");
            std::env::set_var("SURREAL_MAX_RETRY_DELAY_MS", "10000");
            std::env::set_var("SURREAL_RETRY_JITTER_FACTOR", "0.5");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 20);
        assert_eq!(config.max_operation_retries, 5);
        assert_eq!(config.base_retry_delay_ms, 200);
        assert_eq!(config.max_retry_delay_ms, 10000);
        assert_eq!(config.jitter_factor, 0.5);

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }
    }

    #[test]
    fn test_retry_config_partial_env() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Set only some variables, others should use defaults
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::set_var("SURREAL_BASE_RETRY_DELAY_MS", "500");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10); // default
        assert_eq!(config.max_operation_retries, 3); // default
        assert_eq!(config.base_retry_delay_ms, 500); // custom
        assert_eq!(config.max_retry_delay_ms, 5000); // default
        assert_eq!(config.jitter_factor, 0.25); // default

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
        }
    }

    #[test]
    fn test_retry_config_invalid_values() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Clear all first
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        // Invalid values should fall back to defaults
        unsafe {
            std::env::set_var("SURREAL_MAX_CONNECT_RETRIES", "not_a_number");
            std::env::set_var("SURREAL_RETRY_JITTER_FACTOR", "invalid");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10); // default due to parse error
        assert_eq!(config.jitter_factor, 0.25); // default due to parse error

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }
    }
}
