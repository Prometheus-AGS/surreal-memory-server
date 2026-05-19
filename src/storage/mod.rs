//! Storage module for surreal-memory-server binary.
//!
//! Re-exports the `MemoryStorage` trait and all types from the `surreal-memory`
//! library crate. The binary's `SurrealStorage` wraps the library's impl and
//! bridges the server's `Config` to the library's `SurrealConfig`.

// Re-export the trait and all types from the library for use in MCP handlers.
// Allow unused_imports: these are intentional public re-exports for the full API surface.
#[allow(unused_imports)]
pub use surreal_memory::storage::MemoryStorage;
#[allow(unused_imports)]
pub use surreal_memory::storage::surreal::{RetryConfig, SurrealConfig, SurrealMode};
#[allow(unused_imports)]
pub use surreal_memory::{
    ContextWindow, Entity, KnowledgeGraph, Memory, MemoryHistory, MemoryScope, MemoryType,
    Relation, SemanticSearchResult, SurrealStorage, TaskStep, TaskStepStatus, TaskStream,
    TaskStreamStatus,
};

// Bridge: create SurrealStorage from the server's Config
use crate::{
    config::{Config, SurrealMode as ServerMode},
    embeddings::EmbeddingService,
};
use anyhow::Result;
use std::sync::Arc;

pub async fn create_storage(
    config: &Config,
    embedding_service: Arc<dyn EmbeddingService>,
    retry_config: RetryConfig,
) -> Result<Arc<dyn MemoryStorage>> {
    let surreal_config = SurrealConfig {
        mode: match config.surreal_mode {
            ServerMode::Embedded => SurrealMode::Embedded,
            ServerMode::Server => SurrealMode::Server,
        },
        endpoint: config.surreal_endpoint.clone(),
        embedded_path: config.embedded_path.clone(),
        username: config.surreal_username.clone(),
        password: config.surreal_password.clone(),
        namespace: config.surreal_namespace.clone(),
        database: config.surreal_database.clone(),
        retry: retry_config,
    };

    let storage = SurrealStorage::new(&surreal_config, embedding_service).await?;
    Ok(Arc::new(storage))
}
