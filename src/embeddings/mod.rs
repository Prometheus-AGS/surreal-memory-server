//! Embeddings module for surreal-memory-server binary.
//!
//! Re-exports the `EmbeddingService` trait and providers from the
//! `surreal-memory` library crate. This ensures that the same trait object
//! type is used throughout the binary and library, preventing trait mismatch
//! errors when passing `Arc<dyn EmbeddingService>` across the crate boundary.

pub use surreal_memory::embeddings::{
    EmbeddingProvider, EmbeddingService, ExecutorEvent, ExecutorEventKind, ExecutorSnapshot,
    create_embedding_service,
};
