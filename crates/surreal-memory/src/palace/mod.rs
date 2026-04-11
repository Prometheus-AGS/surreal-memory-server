//! MemPalace integration — memory-palace spatial metaphor over SurrealDB.
//!
//! This module bridges mempalace-core's 4-layer memory stack with surreal-memory's
//! existing `SurrealStorage` backend, reusing the same `Surreal<Any>` connection.
//!
//! # Key types
//!
//! - [`PalaceAdapter`] — `StorageBackend` implementation over SurrealDB
//! - [`PalaceContext`] — high-level façade (ingest, compress, search)
//! - [`FastEmbedService`] — `EmbeddingService` backed by fastembed (384-dim)
//! - [`PalaceStorage`] — trait defining the async palace API surface

pub mod adapter;
pub mod context;
pub mod embedding;

pub use adapter::PalaceAdapter;
pub use context::PalaceContext;
pub use embedding::FastEmbedService;

use async_trait::async_trait;
use mempalace_core::storage::types::{DrawerHit, RoomStats, WingStats};
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HitSource {
    /// Hit came from surreal-memory's existing `Memory` table.
    Memory,
    /// Hit came from the mempalace `drawers` table.
    Palace,
}

/// A search hit from either the palace or the legacy memory system,
/// used for RRF (Reciprocal Rank Fusion) merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedHit {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub source: HitSource,
    /// Spatial metadata (palace only).
    pub wing: Option<String>,
    pub room: Option<String>,
}

// ── PalaceStorage trait ──────────────────────────────────────────────────────

/// Async trait defining the palace-side storage API.
///
/// Implemented by `SurrealStorage` (via `PalaceContext`) so consumers can
/// call palace operations through the same storage handle they already hold.
#[async_trait]
pub trait PalaceStorage: Send + Sync {
    /// Ingest content into the palace. Returns drawer ID, or `None` if duplicate.
    async fn palace_ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        source_file: Option<&str>,
        date: Option<&str>,
        importance: f32,
    ) -> anyhow::Result<Option<String>>;

    /// Compress text using AAAK Dialect.
    fn palace_compress(
        &self,
        text: &str,
        wing: Option<&str>,
        room: Option<&str>,
        date: Option<&str>,
        source_file: Option<&str>,
    ) -> anyhow::Result<String>;

    /// Structured vector search returning hits with similarity scores.
    async fn palace_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> anyhow::Result<Vec<DrawerHit>>;

    /// Wake-up context (L0 identity + L1 essential story).
    async fn palace_wake_up(&self, wing: Option<&str>) -> anyhow::Result<String>;

    /// On-demand recall (L2) filtered by wing/room.
    async fn palace_recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<String>;

    /// Palace status summary.
    async fn palace_status(&self) -> anyhow::Result<PalaceStatus>;

    /// List all wings with drawer counts.
    async fn palace_list_wings(&self) -> anyhow::Result<Vec<WingStats>>;

    /// List rooms, optionally filtered by wing.
    async fn palace_list_rooms(&self, wing: Option<&str>) -> anyhow::Result<Vec<RoomStats>>;
}
