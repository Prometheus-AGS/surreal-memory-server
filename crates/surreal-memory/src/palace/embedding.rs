//! Bridge from mempalace-core's `Embedder` to surreal-memory's `EmbeddingService`.
//!
//! `FastEmbedService` wraps a `mempalace_core::embedder::FastEmbedder` (384-dim
//! all-MiniLM-L6-v2) and exposes it as an `EmbeddingService` so the rest of
//! surreal-memory can use it for HNSW indexing and hybrid search.

use crate::embeddings::{Embedding, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Upper bound on the cold-cache FastEmbed model download (~25-80 MB). Exceeding
/// it yields a typed error instead of an open-ended hang on the first Palace call.
const FASTEMBED_INIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Wraps a mempalace-core `Embedder` as a surreal-memory `EmbeddingService`.
pub struct FastEmbedService {
    inner: Arc<dyn mempalace_core::embedder::Embedder>,
}

impl FastEmbedService {
    /// Create a new service backed by the default FastEmbed model (all-MiniLM-L6-v2, 384 dims).
    /// Downloads the model on first call (~25 MB) then caches it locally.
    ///
    /// The download is bounded by `FASTEMBED_INIT_TIMEOUT` so a cold cache or
    /// unreachable network fails fast with a typed error instead of hanging the
    /// first Palace operation.
    pub async fn new() -> Result<Self> {
        let embedder = tokio::time::timeout(
            FASTEMBED_INIT_TIMEOUT,
            mempalace_core::embedder::FastEmbedder::new_default(),
        )
        .await
        .with_context(|| {
            format!(
                "FastEmbed model initialization exceeded {}s timeout",
                FASTEMBED_INIT_TIMEOUT.as_secs()
            )
        })?
        .context("Failed to initialize FastEmbed model")?;
        Ok(Self {
            inner: Arc::new(embedder),
        })
    }

    /// Create a no-op service that returns zero vectors (useful for tests).
    pub fn noop() -> Self {
        Self {
            inner: Arc::new(mempalace_core::embedder::NoOpEmbedder),
        }
    }

    /// Return the inner embedder as a trait object for passing to mempalace-core APIs.
    pub fn as_embedder(&self) -> Arc<dyn mempalace_core::embedder::Embedder> {
        Arc::clone(&self.inner)
    }
}

#[async_trait]
impl EmbeddingService for FastEmbedService {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.inner.embed_batch(&refs).await
    }

    fn dimensions(&self) -> usize {
        self.inner.dimension()
    }
}
