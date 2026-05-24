//! `PalaceContext` — high-level facade over the mempalace-core `MemoryStack`,
//! `Dialect` compressor, and surreal-memory's `PalaceAdapter`.
//!
//! Provides `ingest` (embed + dedup + store), `compress` (AAAK dialect), and
//! structured search returning `DrawerHit` for downstream RRF fusion.

use crate::palace::adapter::PalaceAdapter;
use crate::palace::embedding::FastEmbedService;
use crate::storage::surreal::{ConnectionCell, SurrealStorage};
use anyhow::{Context, Result};
use mempalace_core::{
    dialect::compress::{Dialect, DialectConfig},
    embedder::Embedder,
    layers::{MemoryStack, StackStatus},
    storage::{
        StorageBackend,
        types::{Drawer, DrawerFilter, DrawerHit},
    },
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// High-level palace facade wrapping MemoryStack + Dialect + direct adapter access.
pub struct PalaceContext {
    /// The 4-layer memory stack from mempalace-core.
    stack: MemoryStack,
    /// Direct adapter access for operations not exposed by MemoryStack
    /// (add_drawer, delete_drawer, check_duplicate, search_drawers).
    adapter: Arc<PalaceAdapter>,
    /// Embedder kept separately since MemoryStack's fields are pub(super).
    embedder: Arc<dyn Embedder>,
    /// AAAK Dialect compressor.
    dialect: Dialect,
}

impl PalaceContext {
    /// Build a `PalaceContext` from an existing `SurrealStorage`.
    ///
    /// This reuses the same `Surreal<Any>` connection as the rest of
    /// surreal-memory, so there is no extra connection cost.
    pub async fn from_storage(storage: &SurrealStorage) -> Result<Self> {
        // Capture connection cell for the adapter closure — atomic load,
        // no lock, no contention with the rest of surreal-memory.
        let conn = storage.connection_arc();
        let adapter = Arc::new(PalaceAdapter::new(move || {
            let cell = conn.load();
            match &**cell {
                ConnectionCell::Connected(db) => Ok(db.clone()),
                ConnectionCell::Reconnecting => {
                    anyhow::bail!("Connection is reconnecting")
                }
                ConnectionCell::Failed(msg) => {
                    anyhow::bail!("Connection failed: {}", msg)
                }
            }
        }));

        // Create embedder
        let embed_service = FastEmbedService::new()
            .await
            .context("Failed to initialise FastEmbedService for PalaceContext")?;
        let embedder = embed_service.as_embedder();

        // Build the 4-layer MemoryStack
        let stack = MemoryStack::new(
            adapter.clone() as Arc<dyn StorageBackend>,
            Arc::clone(&embedder),
            None, // no custom identity path
        );

        let dialect = Dialect::new(DialectConfig::default());

        Ok(Self {
            stack,
            adapter,
            embedder,
            dialect,
        })
    }

    /// Build a `PalaceContext` with a no-op embedder (for tests).
    pub fn from_storage_noop(storage: &SurrealStorage) -> Self {
        let conn = storage.connection_arc();
        let adapter = Arc::new(PalaceAdapter::new(move || {
            let cell = conn.load();
            match &**cell {
                ConnectionCell::Connected(db) => Ok(db.clone()),
                ConnectionCell::Reconnecting => {
                    anyhow::bail!("Connection is reconnecting")
                }
                ConnectionCell::Failed(msg) => {
                    anyhow::bail!("Connection failed: {}", msg)
                }
            }
        }));

        let embed_service = FastEmbedService::noop();
        let embedder = embed_service.as_embedder();

        let stack = MemoryStack::new(
            adapter.clone() as Arc<dyn StorageBackend>,
            Arc::clone(&embedder),
            None,
        );

        let dialect = Dialect::new(DialectConfig::default());

        Self {
            stack,
            adapter,
            embedder,
            dialect,
        }
    }

    // ── Ingest ───────────────────────────────────────────────────────────────

    /// Embed content, check for duplicates via SHA-256 content hash, and store
    /// as a `Drawer`. Returns the drawer ID. Bails with an error on duplicate.
    pub async fn ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> Result<String> {
        // Dedup check
        let hash = sha256_hex(content);
        if self.adapter.check_duplicate(&hash).await? {
            anyhow::bail!("Duplicate content: drawer with identical content hash already exists");
        }

        // Embed
        let embedding = self.embedder.embed(content).await?;

        let drawer = Drawer {
            id: String::new(), // adapter assigns UUID
            content: content.to_string(),
            wing: wing.to_string(),
            room: room.to_string(),
            hall: hall.to_string(),
            source_file: None,
            date: None,
            importance,
            embedding: Some(embedding),
        };

        let id = self.adapter.add_drawer(drawer).await?;
        Ok(id)
    }

    // ── Compress ─────────────────────────────────────────────────────────────

    /// Compress text into AAAK Dialect format for token-efficient storage.
    pub fn compress(&self, text: &str) -> String {
        self.dialect.compress(text, None)
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    /// Delete a drawer by ID.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.adapter.delete_drawer(id).await
    }

    // ── Search ───────────────────────────────────────────────────────────────

    /// Structured vector search returning `DrawerHit` with similarity scores.
    /// Use this for RRF fusion with surreal-memory's hybrid search.
    pub async fn search_drawers_structured(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> Result<Vec<DrawerHit>> {
        let query_embedding = self.embedder.embed(query).await?;
        let filter = DrawerFilter {
            wing: wing.map(String::from),
            room: room.map(String::from),
            hall: None,
        };
        self.adapter
            .search_drawers(&query_embedding, filter, n)
            .await
    }

    // ── Delegated MemoryStack methods ────────────────────────────────────────

    /// Wake-up context: L0 (identity) + L1 (essential story).
    pub async fn wake_up(&self, wing: Option<&str>) -> Result<String> {
        self.stack.wake_up(wing).await
    }

    /// On-demand recall: L2 filtered by wing/room.
    pub async fn recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        self.stack.recall(wing, room, limit).await
    }

    /// Deep search: L3 semantic similarity (returns formatted string).
    pub async fn search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> Result<String> {
        self.stack.search(query, wing, room, n).await
    }

    /// Palace status summary.
    pub async fn status(&self) -> Result<StackStatus> {
        self.stack.status().await
    }
}

/// Compute SHA-256 content hash for deduplication.
fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}
