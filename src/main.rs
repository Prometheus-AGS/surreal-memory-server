use anyhow::{Context, Result};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod embeddings;
mod mcp;
mod storage;

use config::Config;
use embeddings::{EmbeddingService, create_embedding_service};
use mcp::MemoryMcpServer;
use storage::{MemoryStorage, surreal::SurrealStorage};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging (stderr only, stdout is reserved for MCP protocol)
    init_logging();

    tracing::info!(
        "🚀 Starting Rust Memory MCP Server v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Load configuration
    let config = load_config().await?;

    // Initialize services
    let embedding_service = init_embedding_service(&config).await?;
    let storage = init_storage(config, embedding_service).await?;

    // Run server with graceful shutdown
    run_server(storage).await?;

    tracing::info!("👋 MCP server shut down gracefully");
    Ok(())
}

fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Default to info level, can be overridden with RUST_LOG env var
                "rust_memory_mcp=info,surreal=warn".into()
            }),
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

    tracing::info!("✅ Configuration loaded:");
    tracing::info!("   SurrealDB mode: {:?}", config.surreal_mode);
    tracing::info!("   Namespace: {}", config.surreal_namespace);
    tracing::info!("   Database: {}", config.surreal_database);

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

    let storage = SurrealStorage::new(&config, embedding_service)
        .await
        .context("Failed to initialize SurrealDB storage")?;

    tracing::info!("✅ Storage initialized successfully");

    Ok(Arc::new(storage))
}

async fn run_server(storage: Arc<dyn MemoryStorage>) -> Result<()> {
    tracing::info!("🎯 Starting MCP server on stdio transport...");
    tracing::info!("📡 Server is ready to accept requests");

    let server = MemoryMcpServer::new(storage);

    // Set up graceful shutdown on Ctrl+C
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("🛑 Received shutdown signal");
                let _ = shutdown_tx.send(()).await;
            }
            Err(err) => {
                tracing::error!("Failed to listen for shutdown signal: {}", err);
            }
        }
    });

    // Run server
    tokio::select! {
        result = server.run() => {
            result.context("MCP server error")?;
        }
        _ = shutdown_rx.recv() => {
            tracing::info!("Initiating graceful shutdown...");
        }
    }

    Ok(())
}
