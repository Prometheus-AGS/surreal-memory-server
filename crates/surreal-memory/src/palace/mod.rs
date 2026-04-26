//! MemPalace integration — memory-palace spatial metaphor over SurrealDB.
//!
//! This module bridges mempalace-core's 4-layer memory stack with surreal-memory's
//! existing `SurrealStorage` backend, reusing the same `Surreal<Any>` connection.
//!
//! # Key types
//!
//! - [`PalaceAdapter`] — `StorageBackend` implementation over SurrealDB
//! - [`PalaceContext`] — high-level facade (ingest, compress, search)
//! - [`FastEmbedService`] — `EmbeddingService` backed by fastembed (384-dim)
//! - [`PalaceStorage`] — trait defining the async palace API surface

pub mod adapter;
pub mod context;
pub mod embedding;

pub use adapter::PalaceAdapter;
pub use context::PalaceContext;
pub use embedding::FastEmbedService;

use crate::memory::{MemoryScope, MemoryType};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Types ────────────────────────────────────────────────────────────────────

/// Summary of palace status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalaceStatus {
    pub total_drawers: usize,
    pub total_wings: usize,
    pub total_rooms: usize,
    pub identity_loaded: bool,
}

/// Source discriminator for hits returned by unified search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HitSource {
    /// Hit came from surreal-memory's existing `Memory` table.
    Memory {
        scope: MemoryScope,
        memory_type: MemoryType,
    },
    /// Hit came from the knowledge graph `Entity` table.
    Entity { entity_type: String },
    /// Hit came from the mempalace `drawers` table.
    Palace { wing: String, room: String },
}

/// A search hit from either the palace, the legacy memory system, or the
/// knowledge graph, used for RRF (Reciprocal Rank Fusion) merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedHit {
    pub id: String,
    pub content: String,
    pub source: HitSource,
    pub score: f32,
}

// ── PalaceStorage trait ──────────────────────────────────────────────────────

/// Async trait defining the palace-side storage API.
///
/// Implemented by `SurrealStorage` (via `PalaceContext`) so consumers can
/// call palace operations through the same storage handle they already hold.
#[async_trait]
pub trait PalaceStorage: Send + Sync {
    /// Wake-up context (L0 identity + L1 essential story).
    async fn palace_wake_up(&self, wing: Option<&str>) -> anyhow::Result<String>;

    /// On-demand recall (L2) filtered by wing/room.
    async fn palace_recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<String>;

    /// Deep search (L3) returning formatted context text from MemoryStack.
    async fn palace_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> anyhow::Result<String>;

    /// Ingest content into the palace. Returns drawer ID.
    /// Bails with an error on duplicate content.
    async fn palace_ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> anyhow::Result<String>;

    /// Delete a drawer by ID.
    async fn palace_delete(&self, id: &str) -> anyhow::Result<()>;

    /// Palace status summary.
    async fn palace_status(&self) -> anyhow::Result<PalaceStatus>;

    /// Compress text using AAAK Dialect.
    fn palace_compress(&self, text: &str) -> String;

    /// Hybrid search across memories, entities, and palace drawers.
    /// Results are merged via Reciprocal Rank Fusion.
    async fn palace_hybrid_search(
        &self,
        query: &str,
        scope: Option<MemoryScope>,
        wing: Option<&str>,
        n: usize,
    ) -> anyhow::Result<Vec<UnifiedHit>>;
}
