# MemPalace Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate `mempalace-core`'s 4-layer memory model into the `surreal-memory` library crate via git dependency, exposing palace operations through a separate `PalaceStorage` trait, 7 MCP tools, and 7 REST endpoints.

**Architecture:** Feature-gated (`palace`) integration that shares the existing `Surreal<Any>` connection. `PalaceAdapter` implements mempalace-core's `StorageBackend` trait over SurrealQL. `PalaceContext` wraps `MemoryStack` for the 4-layer API. A separate `PalaceStorage` trait on `SurrealStorage` avoids touching the core `MemoryStorage` trait. RRF hybrid reranking merges results across memory, entity, and drawer tables.

**Tech Stack:** Rust, SurrealDB, mempalace-core (git dep), fastembed (via mempalace-core), axum, rmcp

**Spec:** `docs/superpowers/specs/2026-04-10-mempalace-integration-design.md`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `Cargo.toml` | Add `mempalace-core` workspace dep, `palace` feature |
| Modify | `crates/surreal-memory/Cargo.toml` | Add optional `mempalace-core` dep, `palace` feature |
| Modify | `crates/surreal-memory/src/lib.rs` | Conditional `palace` module + re-exports |
| Create | `crates/surreal-memory/src/palace/mod.rs` | `PalaceStorage` trait, `PalaceStatus`, `UnifiedHit`, `HitSource` |
| Create | `crates/surreal-memory/src/palace/adapter.rs` | `PalaceAdapter`: `StorageBackend` impl over shared `Surreal<Any>` |
| Create | `crates/surreal-memory/src/palace/context.rs` | `PalaceContext`: wraps `MemoryStack` + `Dialect` |
| Create | `crates/surreal-memory/src/palace/embedding.rs` | `FastEmbedService`: bridges `FastEmbedder` → `EmbeddingService` |
| Modify | `crates/surreal-memory/src/storage/surreal.rs` | `db()` method, `OnceCell<PalaceContext>`, `PalaceStorage` impl |
| Modify | `crates/surreal-memory/src/storage/migrations/mod.rs` | Migration v16 (drawers table) |
| Modify | `crates/surreal-memory/src/embeddings/mod.rs` | `EmbeddingProvider::Fast` variant |
| Modify | `src/mcp/mod.rs` | 7 palace tool definitions |
| Modify | `src/mcp/handlers.rs` | 7 palace handler methods + param structs |
| Create | `src/api/palace.rs` | REST endpoints for palace |
| Modify | `src/api/mod.rs` | Register palace router |
| Modify | `src/main.rs` | Log `Fast` embedding provider |

---

### Task 1: Cargo Dependencies & Feature Gates

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/surreal-memory/Cargo.toml`

- [ ] **Step 1: Add workspace dependency for mempalace-core**

In `Cargo.toml` (workspace root), add to `[workspace.dependencies]`:

```toml
mempalace-core = { git = "https://github.com/GQAdonis/mempalace-rs.git", branch = "main" }
```

In `Cargo.toml` (workspace root), add to `[features]`:

```toml
palace = ["surreal-memory/palace"]
```

In `Cargo.toml` (workspace root), add to `[dependencies]`:

```toml
mempalace-core = { workspace = true, optional = true }
```

- [ ] **Step 2: Add optional dependency and feature in library crate**

In `crates/surreal-memory/Cargo.toml`, add to `[dependencies]`:

```toml
mempalace-core = { workspace = true, optional = true }
```

In `crates/surreal-memory/Cargo.toml`, add to `[features]`:

```toml
palace = ["dep:mempalace-core"]
```

- [ ] **Step 3: Verify the dependency resolves**

Run: `cargo check --features palace -p surreal-memory 2>&1 | head -20`

Expected: Fetches from GitHub, compiles mempalace-core. May show warnings but no errors. If `sha2` version conflict appears, both 0.10 and 0.11 will compile as separate crate versions — this is expected and fine.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/surreal-memory/Cargo.toml Cargo.lock
git commit -m "feat(palace): add mempalace-core git dependency with palace feature gate"
```

---

### Task 2: Migration v16 — Drawers Table

**Files:**
- Modify: `crates/surreal-memory/src/storage/migrations/mod.rs`

- [ ] **Step 1: Add migration v16 SQL constant**

After the existing `MIGRATION_V15_SQL` constant (around line 308), add:

```rust
// ── v16: MemPalace drawers table ────────────────────────────────────────────

const MIGRATION_V16_SQL: &str = "
DEFINE TABLE IF NOT EXISTS drawers SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS content      ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS wing         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS room         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS hall         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS source_file  ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date         ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS importance   ON drawers TYPE float DEFAULT 1.0;
DEFINE FIELD IF NOT EXISTS embedding    ON drawers TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding.*  ON drawers TYPE float;
DEFINE FIELD IF NOT EXISTS content_hash ON drawers TYPE option<string>;

DEFINE INDEX IF NOT EXISTS drawer_embedding_idx
  ON drawers FIELDS embedding
  MTREE DIMENSION 384 DIST COSINE;

DEFINE INDEX IF NOT EXISTS drawer_content_idx
  ON drawers FIELDS content SEARCH ANALYZER unicode BM25;

DEFINE INDEX IF NOT EXISTS drawer_hash_idx
  ON drawers FIELDS content_hash;

DEFINE INDEX IF NOT EXISTS drawer_wing_idx ON drawers FIELDS wing;
DEFINE INDEX IF NOT EXISTS drawer_room_idx ON drawers FIELDS room;
";
```

- [ ] **Step 2: Register migration v16 in the MIGRATIONS array**

In the `MIGRATIONS` array (around line 93), add after the v15 entry:

```rust
    Migration::sql(16, "palace_drawers_table", MIGRATION_V16_SQL),
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p surreal-memory`

Expected: PASS (migration is just a constant + array entry, no feature gate needed since it's pure SQL).

- [ ] **Step 4: Commit**

```bash
git add crates/surreal-memory/src/storage/migrations/mod.rs
git commit -m "feat(palace): add migration v16 — drawers table with HNSW and BM25 indexes"
```

---

### Task 3: DB Handle Exposure on SurrealStorage

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs`

- [ ] **Step 1: Add `db()` method to SurrealStorage**

Add this method to the `impl SurrealStorage` block, near the existing `health_check()` method (around line 474):

```rust
    /// Clone the live `Surreal<Any>` handle for shared use by subsystems (e.g. PalaceAdapter).
    ///
    /// `Surreal<Any>` is internally `Arc`-wrapped — cloning is cheap.
    /// Returns `Err` if the connection is in `Reconnecting` or `Failed` state.
    pub fn db(&self) -> Result<Surreal<Any>> {
        let state = self.connection.read().expect("Connection lock poisoned");
        match &*state {
            ConnectionState::Connected(db) => Ok(db.clone()),
            ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
            ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
        }
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p surreal-memory`

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat(palace): expose db() handle on SurrealStorage for subsystem sharing"
```

---

### Task 4: FastEmbedService — Embedding Bridge

**Files:**
- Create: `crates/surreal-memory/src/palace/embedding.rs`
- Modify: `crates/surreal-memory/src/embeddings/mod.rs`

- [ ] **Step 1: Create the palace directory**

```bash
mkdir -p crates/surreal-memory/src/palace
```

- [ ] **Step 2: Create `embedding.rs`**

Create `crates/surreal-memory/src/palace/embedding.rs`:

```rust
//! Bridge from mempalace-core's FastEmbedder to surreal-memory's EmbeddingService.

use crate::embeddings::{Embedding, EmbeddingService};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Wraps a `mempalace_core::embedder::Embedder` to implement `EmbeddingService`.
///
/// Default construction uses `FastEmbedder` (all-MiniLM-L6-v2, 384 dims).
/// The model (~25 MB) is downloaded on first use and cached locally.
pub struct FastEmbedService {
    inner: Arc<dyn mempalace_core::embedder::Embedder>,
}

impl FastEmbedService {
    /// Create with the default FastEmbedder (all-MiniLM-L6-v2, 384 dims).
    pub async fn new() -> Result<Self> {
        let embedder = mempalace_core::embedder::FastEmbedder::new_default().await?;
        Ok(Self {
            inner: Arc::new(embedder),
        })
    }

    /// Create with the NoOpEmbedder (returns zero vectors). For tests.
    pub fn noop() -> Self {
        Self {
            inner: Arc::new(mempalace_core::embedder::NoOpEmbedder),
        }
    }

    /// Access the underlying `Embedder` as an `Arc` (needed by `MemoryStack`).
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
```

- [ ] **Step 3: Add `EmbeddingProvider::Fast` variant**

In `crates/surreal-memory/src/embeddings/mod.rs`, add the variant to the `EmbeddingProvider` enum:

```rust
    #[cfg(feature = "palace")]
    Fast,
```

Add this arm inside `create_embedding_service()`, before the `#[cfg(not(feature = "local-embeddings"))]` arm:

```rust
        #[cfg(feature = "palace")]
        EmbeddingProvider::Fast => {
            Ok(Box::new(crate::palace::embedding::FastEmbedService::new().await?))
        }
```

- [ ] **Step 4: Verify compilation (will fail — palace module not yet wired)**

This step will fail because `crate::palace` doesn't exist yet. That's expected. We'll wire it in Task 6.

- [ ] **Step 5: Commit**

```bash
git add crates/surreal-memory/src/palace/embedding.rs crates/surreal-memory/src/embeddings/mod.rs
git commit -m "feat(palace): add FastEmbedService bridging mempalace-core embedder to EmbeddingService"
```

---

### Task 5: PalaceAdapter — StorageBackend over Surreal\<Any\>

**Files:**
- Create: `crates/surreal-memory/src/palace/adapter.rs`

- [ ] **Step 1: Create `adapter.rs`**

Create `crates/surreal-memory/src/palace/adapter.rs`:

```rust
//! PalaceAdapter — implements mempalace-core's StorageBackend over a shared Surreal<Any>.

use anyhow::{Context, Result};
use async_trait::async_trait;
use mempalace_core::storage::types::{Drawer, DrawerFilter, DrawerHit, RoomStats, WingStats};
use mempalace_core::storage::StorageBackend;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;

/// Adapter that implements `StorageBackend` by executing SurrealQL against
/// the shared `Surreal<Any>` handle from `SurrealStorage`.
///
/// Each operation calls `db_fn()` fresh so it inherits reconnection state.
pub struct PalaceAdapter {
    db_fn: Arc<dyn Fn() -> Result<Surreal<Any>> + Send + Sync>,
}

impl PalaceAdapter {
    /// Create from a closure that yields a live `Surreal<Any>` handle.
    pub fn new(db_fn: impl Fn() -> Result<Surreal<Any>> + Send + Sync + 'static) -> Self {
        Self {
            db_fn: Arc::new(db_fn),
        }
    }

    fn db(&self) -> Result<Surreal<Any>> {
        (self.db_fn)()
    }
}

/// Helper struct for deserializing drawers from SurrealDB.
/// SurrealDB returns record IDs as `Thing` objects, so we need a separate
/// struct that handles the `id` field as a generic Value.
#[derive(serde::Deserialize)]
struct RawDrawer {
    id: serde_json::Value,
    content: String,
    wing: String,
    room: String,
    hall: String,
    source_file: Option<String>,
    date: Option<String>,
    importance: f32,
    embedding: Option<Vec<f32>>,
}

impl RawDrawer {
    fn into_drawer(self) -> Drawer {
        let id_str = match &self.id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(obj) => {
                // SurrealDB Thing format: { "tb": "drawers", "id": { "String": "..." } }
                obj.get("id")
                    .and_then(|v| match v {
                        serde_json::Value::String(s) => Some(s.clone()),
                        serde_json::Value::Object(inner) => {
                            inner.get("String").and_then(|s| s.as_str().map(String::from))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("{}", self.id))
            }
            other => format!("{}", other),
        };
        Drawer {
            id: id_str,
            content: self.content,
            wing: self.wing,
            room: self.room,
            hall: self.hall,
            source_file: self.source_file,
            date: self.date,
            importance: self.importance,
            embedding: self.embedding,
        }
    }
}

#[derive(serde::Deserialize)]
struct RawDrawerHit {
    #[serde(flatten)]
    drawer: RawDrawer,
    score: f32,
}

#[derive(serde::Deserialize)]
struct CountResult {
    count: usize,
}

#[derive(serde::Deserialize)]
struct RawWingStats {
    wing: String,
    count: usize,
}

#[derive(serde::Deserialize)]
struct RawRoomStats {
    room: String,
    wing: String,
    count: usize,
}

#[async_trait]
impl StorageBackend for PalaceAdapter {
    async fn add_drawer(&self, drawer: Drawer) -> Result<String> {
        let db = self.db()?;
        let content_hash = hex::encode(Sha256::digest(drawer.content.as_bytes()));
        let id = uuid::Uuid::new_v4().to_string();

        db.query(
            "CREATE type::thing('drawers', $id) CONTENT {
                content: $content,
                wing: $wing,
                room: $room,
                hall: $hall,
                source_file: $source_file,
                date: $date,
                importance: $importance,
                embedding: $embedding,
                content_hash: $content_hash
            }",
        )
        .bind(("id", id.clone()))
        .bind(("content", drawer.content))
        .bind(("wing", drawer.wing))
        .bind(("room", drawer.room))
        .bind(("hall", drawer.hall))
        .bind(("source_file", drawer.source_file))
        .bind(("date", drawer.date))
        .bind(("importance", drawer.importance))
        .bind(("embedding", drawer.embedding))
        .bind(("content_hash", content_hash))
        .await
        .context("Failed to create drawer")?;

        Ok(format!("drawers:{id}"))
    }

    async fn delete_drawer(&self, id: &str) -> Result<()> {
        let db = self.db()?;
        // Handle both "drawers:xyz" and bare "xyz" formats
        let record_id = if id.starts_with("drawers:") {
            id.to_string()
        } else {
            format!("drawers:{id}")
        };
        db.query(&format!("DELETE {record_id}"))
            .await
            .context("Failed to delete drawer")?;
        Ok(())
    }

    async fn get_drawer(&self, id: &str) -> Result<Option<Drawer>> {
        let db = self.db()?;
        let record_id = if id.starts_with("drawers:") {
            id.to_string()
        } else {
            format!("drawers:{id}")
        };
        let mut result = db
            .query(&format!("SELECT * FROM {record_id}"))
            .await
            .context("Failed to get drawer")?;
        let rows: Vec<RawDrawer> = result.take(0)?;
        Ok(rows.into_iter().next().map(RawDrawer::into_drawer))
    }

    async fn list_drawers(&self, filter: DrawerFilter, limit: usize) -> Result<Vec<Drawer>> {
        let db = self.db()?;
        let mut conditions = Vec::new();
        let mut bindings: Vec<(String, String)> = Vec::new();

        if let Some(wing) = &filter.wing {
            conditions.push("wing = $wing".to_string());
            bindings.push(("wing".to_string(), wing.clone()));
        }
        if let Some(room) = &filter.room {
            conditions.push("room = $room".to_string());
            bindings.push(("room".to_string(), room.clone()));
        }
        if let Some(hall) = &filter.hall {
            conditions.push("hall = $hall".to_string());
            bindings.push(("hall".to_string(), hall.clone()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            "SELECT * FROM drawers {where_clause} ORDER BY importance DESC LIMIT {limit}"
        );

        let mut stmt = db.query(&query);
        for (key, value) in &bindings {
            stmt = stmt.bind((key.as_str(), value.clone()));
        }

        let mut result = stmt.await.context("Failed to list drawers")?;
        let rows: Vec<RawDrawer> = result.take(0)?;
        Ok(rows.into_iter().map(RawDrawer::into_drawer).collect())
    }

    async fn search_drawers(
        &self,
        query_embedding: &[f32],
        filter: DrawerFilter,
        n: usize,
    ) -> Result<Vec<DrawerHit>> {
        let db = self.db()?;
        let embedding_vec: Vec<f32> = query_embedding.to_vec();

        let mut conditions = Vec::new();
        let mut bindings_str: Vec<(String, String)> = Vec::new();

        if let Some(wing) = &filter.wing {
            conditions.push("wing = $wing".to_string());
            bindings_str.push(("wing".to_string(), wing.clone()));
        }
        if let Some(room) = &filter.room {
            conditions.push("room = $room".to_string());
            bindings_str.push(("room".to_string(), room.clone()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("AND {}", conditions.join(" AND "))
        };

        let query = format!(
            "SELECT *, vector::similarity::cosine(embedding, $qvec) AS score \
             FROM drawers \
             WHERE embedding <|{n},384|> $qvec {where_clause} \
             ORDER BY score DESC \
             LIMIT {n}"
        );

        let mut stmt = db.query(&query).bind(("qvec", embedding_vec));
        for (key, value) in &bindings_str {
            stmt = stmt.bind((key.as_str(), value.clone()));
        }

        let mut result = stmt.await.context("Failed to search drawers")?;
        let rows: Vec<RawDrawerHit> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|r| DrawerHit {
                drawer: r.drawer.into_drawer(),
                similarity: r.score,
            })
            .collect())
    }

    async fn check_duplicate(&self, content_hash: &str) -> Result<bool> {
        let db = self.db()?;
        let mut result = db
            .query("SELECT count() AS count FROM drawers WHERE content_hash = $hash GROUP ALL")
            .bind(("hash", content_hash.to_string()))
            .await
            .context("Failed to check duplicate")?;
        let rows: Vec<CountResult> = result.take(0)?;
        Ok(rows.first().map(|r| r.count > 0).unwrap_or(false))
    }

    async fn list_wings(&self) -> Result<Vec<WingStats>> {
        let db = self.db()?;
        let mut result = db
            .query("SELECT wing, count() AS count FROM drawers GROUP BY wing")
            .await
            .context("Failed to list wings")?;
        let rows: Vec<RawWingStats> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|r| WingStats {
                name: r.wing,
                count: r.count,
            })
            .collect())
    }

    async fn list_rooms(&self, wing: Option<&str>) -> Result<Vec<RoomStats>> {
        let db = self.db()?;
        let (query, bind_wing) = match wing {
            Some(w) => (
                "SELECT room, wing, count() AS count FROM drawers WHERE wing = $wing GROUP BY room, wing",
                Some(w.to_string()),
            ),
            None => (
                "SELECT room, wing, count() AS count FROM drawers GROUP BY room, wing",
                None,
            ),
        };

        let mut stmt = db.query(query);
        if let Some(w) = &bind_wing {
            stmt = stmt.bind(("wing", w.clone()));
        }

        let mut result = stmt.await.context("Failed to list rooms")?;
        let rows: Vec<RawRoomStats> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|r| RoomStats {
                name: r.room,
                wing: r.wing,
                count: r.count,
            })
            .collect())
    }

    async fn drawer_count(&self) -> Result<usize> {
        let db = self.db()?;
        let mut result = db
            .query("SELECT count() AS count FROM drawers GROUP ALL")
            .await
            .context("Failed to count drawers")?;
        let rows: Vec<CountResult> = result.take(0)?;
        Ok(rows.first().map(|r| r.count).unwrap_or(0))
    }
}
```

- [ ] **Step 2: Add `hex` dependency to the library crate**

In `crates/surreal-memory/Cargo.toml`, add under `[dependencies]`:

```toml
hex = "0.4"
```

- [ ] **Step 3: Commit**

```bash
git add crates/surreal-memory/src/palace/adapter.rs crates/surreal-memory/Cargo.toml
git commit -m "feat(palace): add PalaceAdapter implementing StorageBackend over shared Surreal<Any>"
```

---

### Task 6: PalaceContext & Palace Module Root

**Files:**
- Create: `crates/surreal-memory/src/palace/context.rs`
- Create: `crates/surreal-memory/src/palace/mod.rs`
- Modify: `crates/surreal-memory/src/lib.rs`

- [ ] **Step 1: Create `context.rs`**

Create `crates/surreal-memory/src/palace/context.rs`:

```rust
//! PalaceContext — wraps MemoryStack + Dialect for the 4-layer API.

use crate::storage::surreal::SurrealStorage;
use anyhow::Result;
use mempalace_core::dialect::{Dialect, DialectConfig};
use mempalace_core::embedder::FastEmbedder;
use mempalace_core::layers::stack::{MemoryStack, StackStatus};
use std::sync::Arc;

use super::adapter::PalaceAdapter;

/// High-level palace context combining the 4-layer memory stack with
/// AAAK Dialect compression. Built from an existing `SurrealStorage`.
pub struct PalaceContext {
    stack: MemoryStack,
    #[allow(dead_code)]
    adapter: Arc<PalaceAdapter>,
    dialect: Dialect,
}

impl PalaceContext {
    /// Build a `PalaceContext` from an existing `SurrealStorage` instance.
    ///
    /// This downloads the FastEmbedder model (~25 MB) on first call.
    pub async fn from_storage(storage: &SurrealStorage) -> Result<Self> {
        // Capture a closure over the storage's db() method
        let connection = storage.connection_arc();
        let adapter = Arc::new(PalaceAdapter::new(move || {
            let state = connection.read().expect("Connection lock poisoned");
            match &*state {
                crate::storage::surreal::ConnectionState::Connected(db) => Ok(db.clone()),
                crate::storage::surreal::ConnectionState::Reconnecting => {
                    anyhow::bail!("Connection is reconnecting")
                }
                crate::storage::surreal::ConnectionState::Failed(msg) => {
                    anyhow::bail!("Connection failed: {}", msg)
                }
            }
        }));

        let embedder = Arc::new(FastEmbedder::new_default().await?);
        let stack = MemoryStack::new(adapter.clone() as Arc<dyn mempalace_core::storage::StorageBackend>, embedder, None);
        let dialect = Dialect::new(DialectConfig::default());

        Ok(Self {
            stack,
            adapter,
            dialect,
        })
    }

    /// L0 (identity) + L1 (essential story).
    pub async fn wake_up(&self, wing: Option<&str>) -> Result<String> {
        self.stack.wake_up(wing).await
    }

    /// L2 — on-demand recall filtered by wing/room.
    pub async fn recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        self.stack.recall(wing, room, limit).await
    }

    /// L3 — deep semantic search.
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

    /// Ingest content into a drawer with embedding + dedup.
    pub async fn ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> Result<String> {
        use mempalace_core::storage::StorageBackend;
        use mempalace_core::storage::types::Drawer;
        use sha2::{Digest, Sha256};

        // Check for duplicates
        let content_hash = hex::encode(Sha256::digest(content.as_bytes()));
        if self.adapter.check_duplicate(&content_hash).await? {
            anyhow::bail!("Duplicate content detected (hash: {content_hash})");
        }

        // Embed the content
        let embedding = self.stack.backend.embed(content).await.ok();

        let drawer = Drawer {
            id: String::new(), // assigned by adapter
            content: content.to_string(),
            wing: wing.to_string(),
            room: room.to_string(),
            hall: hall.to_string(),
            source_file: None,
            date: Some(chrono::Utc::now().to_rfc3339()),
            importance,
            embedding,
        };

        self.adapter.add_drawer(drawer).await
    }

    /// Delete a drawer by ID.
    pub async fn delete(&self, id: &str) -> Result<()> {
        use mempalace_core::storage::StorageBackend;
        self.adapter.delete_drawer(id).await
    }

    /// Compress text using AAAK Dialect pipeline.
    pub fn compress(&self, text: &str) -> String {
        self.dialect.compress(text, None)
    }
}
```

- [ ] **Step 2: Expose `connection` on SurrealStorage for PalaceContext**

In `crates/surreal-memory/src/storage/surreal.rs`, add a method and make `ConnectionState` pub(crate):

Change the `ConnectionState` visibility from private to `pub(crate)`:

```rust
pub(crate) enum ConnectionState {
```

Add this method to `impl SurrealStorage`:

```rust
    /// Expose the inner connection arc for subsystem sharing (e.g. PalaceAdapter).
    pub(crate) fn connection_arc(&self) -> Arc<std::sync::RwLock<ConnectionState>> {
        Arc::clone(&self.connection)
    }
```

- [ ] **Step 3: Create `palace/mod.rs`**

Create `crates/surreal-memory/src/palace/mod.rs`:

```rust
//! MemPalace integration — 4-layer spatial memory with taxonomy, semantic search,
//! AAAK Dialect compression, and RRF hybrid reranking.

pub mod adapter;
pub mod context;
pub mod embedding;

pub use adapter::PalaceAdapter;
pub use context::PalaceContext;
pub use embedding::FastEmbedService;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::memory::{MemoryScope, MemoryType};

/// Status of the palace memory stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PalaceStatus {
    pub total_drawers: usize,
    pub total_wings: usize,
    pub total_rooms: usize,
    pub identity_loaded: bool,
}

/// A search hit from any of the three search domains (memory, entity, drawers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedHit {
    pub id: String,
    pub content: String,
    pub source: HitSource,
    pub score: f32,
}

/// Identifies which search domain produced a hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HitSource {
    Memory {
        scope: MemoryScope,
        memory_type: MemoryType,
    },
    Entity {
        entity_type: String,
    },
    Palace {
        wing: String,
        room: String,
    },
}

/// Separate trait for palace operations — does NOT modify `MemoryStorage`.
///
/// Implemented by `SurrealStorage` when the `palace` feature is enabled.
/// Consumers opt in via `use surreal_memory::PalaceStorage`.
#[async_trait]
pub trait PalaceStorage: Send + Sync {
    /// L0 (identity) + L1 (essential story). Optional wing filter.
    async fn palace_wake_up(&self, wing: Option<&str>) -> Result<String>;

    /// L2 — on-demand recall filtered by taxonomy.
    async fn palace_recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<String>;

    /// L3 — deep semantic search over drawers.
    async fn palace_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> Result<String>;

    /// Ingest content into the palace with embedding + dedup.
    async fn palace_ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> Result<String>;

    /// Delete a drawer by ID.
    async fn palace_delete(&self, id: &str) -> Result<()>;

    /// Palace status summary (drawer count, wing/room cardinality).
    async fn palace_status(&self) -> Result<PalaceStatus>;

    /// Compress text using AAAK Dialect pipeline.
    fn palace_compress(&self, text: &str) -> String;

    /// Cross-domain hybrid search using Reciprocal Rank Fusion across
    /// memory, entity, and drawer tables.
    async fn palace_hybrid_search(
        &self,
        query: &str,
        scope: Option<MemoryScope>,
        wing: Option<&str>,
        n: usize,
    ) -> Result<Vec<UnifiedHit>>;
}
```

- [ ] **Step 4: Wire palace module in lib.rs**

In `crates/surreal-memory/src/lib.rs`, add after the existing module declarations:

```rust
#[cfg(feature = "palace")]
pub mod palace;
```

And add conditional re-exports after the existing re-exports:

```rust
#[cfg(feature = "palace")]
pub use palace::{
    FastEmbedService, HitSource, PalaceAdapter, PalaceContext, PalaceStatus, PalaceStorage,
    UnifiedHit,
};
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check --features palace -p surreal-memory 2>&1 | tail -20`

Expected: May have warnings about unused code (PalaceStorage not yet implemented). The key types should compile. If the `ingest` method can't access `self.stack.backend` (it's `pub(super)`), we'll fix in the next step.

- [ ] **Step 6: Commit**

```bash
git add crates/surreal-memory/src/palace/ crates/surreal-memory/src/lib.rs crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat(palace): add PalaceContext, PalaceStorage trait, and palace module root"
```

---

### Task 7: PalaceStorage Implementation on SurrealStorage

**Files:**
- Modify: `crates/surreal-memory/src/storage/surreal.rs`

- [ ] **Step 1: Add OnceCell field to SurrealStorage struct**

At the top of `surreal.rs`, add to imports (conditionally):

```rust
#[cfg(feature = "palace")]
use crate::palace::{PalaceContext, PalaceStatus, PalaceStorage, UnifiedHit, HitSource};
#[cfg(feature = "palace")]
use crate::memory::MemoryScope;
```

Add the field to the `SurrealStorage` struct:

```rust
pub struct SurrealStorage {
    connection: Arc<std::sync::RwLock<ConnectionState>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
    #[cfg(feature = "palace")]
    palace: tokio::sync::OnceCell<PalaceContext>,
}
```

Update `SurrealStorage::new()` to initialize the field:

```rust
    // After the existing field initialization, add:
    #[cfg(feature = "palace")]
    let palace = tokio::sync::OnceCell::new();
```

And include it in the struct construction:

```rust
    Ok(Self {
        connection,
        connection_info,
        embedding_service,
        #[cfg(feature = "palace")]
        palace,
    })
```

- [ ] **Step 2: Add palace initializer helper**

Add a private method to `impl SurrealStorage`:

```rust
    #[cfg(feature = "palace")]
    async fn palace_context(&self) -> Result<&PalaceContext> {
        self.palace
            .get_or_try_init(|| async { PalaceContext::from_storage(self).await })
            .await
    }
```

- [ ] **Step 3: Implement PalaceStorage for SurrealStorage**

Add the implementation block:

```rust
#[cfg(feature = "palace")]
#[async_trait]
impl PalaceStorage for SurrealStorage {
    async fn palace_wake_up(&self, wing: Option<&str>) -> Result<String> {
        self.palace_context().await?.wake_up(wing).await
    }

    async fn palace_recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        self.palace_context().await?.recall(wing, room, limit).await
    }

    async fn palace_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> Result<String> {
        self.palace_context().await?.search(query, wing, room, n).await
    }

    async fn palace_ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> Result<String> {
        self.palace_context()
            .await?
            .ingest(content, wing, room, hall, importance)
            .await
    }

    async fn palace_delete(&self, id: &str) -> Result<()> {
        self.palace_context().await?.delete(id).await
    }

    async fn palace_status(&self) -> Result<PalaceStatus> {
        let status = self.palace_context().await?.status().await?;
        Ok(PalaceStatus {
            total_drawers: status.total_drawers,
            total_wings: status.total_wings,
            total_rooms: status.total_rooms,
            identity_loaded: status.identity_loaded,
        })
    }

    fn palace_compress(&self, text: &str) -> String {
        // Compression doesn't need async init — use a static Dialect
        use mempalace_core::dialect::{Dialect, DialectConfig};
        use std::sync::OnceLock;
        static DIALECT: OnceLock<Dialect> = OnceLock::new();
        let dialect = DIALECT.get_or_init(|| Dialect::new(DialectConfig::default()));
        dialect.compress(text, None)
    }

    async fn palace_hybrid_search(
        &self,
        query: &str,
        scope: Option<MemoryScope>,
        wing: Option<&str>,
        n: usize,
    ) -> Result<Vec<UnifiedHit>> {
        use mempalace_core::reranker::ReciprocRankFusion;
        use mempalace_core::storage::types::DrawerHit;

        // Run all three searches concurrently
        let (memory_scope, agent_id, user_id, session_id) = match &scope {
            Some(MemoryScope::Agent) => (Some("agent"), None, None, None),
            Some(MemoryScope::User) => (None, None, Some(""), None),
            Some(MemoryScope::Session) => (None, None, None, Some("")),
            _ => (None, None, None, None),
        };

        let memory_fut = self.hybrid_search_memories(
            query, user_id, agent_id, session_id, n, 0.7, 0.3,
        );
        let entity_fut = self.semantic_search(query, n, 0.0);
        let palace_fut = self.palace_search(query, wing, None, n);

        let (memories, entities, palace_text) =
            tokio::join!(memory_fut, entity_fut, palace_fut);

        let mut rrf_lists: Vec<Vec<DrawerHit>> = Vec::new();

        // Convert memory hits to DrawerHits
        if let Ok(mems) = memories {
            let hits: Vec<DrawerHit> = mems
                .iter()
                .map(|m| DrawerHit {
                    drawer: mempalace_core::storage::types::Drawer {
                        id: m.id.clone().unwrap_or_default(),
                        content: m.content.clone(),
                        wing: "memory".to_string(),
                        room: format!("{:?}", m.memory_type),
                        hall: String::new(),
                        source_file: None,
                        date: Some(m.created_at.clone()),
                        importance: m.importance as f32,
                        embedding: None,
                    },
                    similarity: m.importance as f32,
                })
                .collect();
            rrf_lists.push(hits);
        }

        // Convert entity hits to DrawerHits
        if let Ok(ents) = entities {
            let hits: Vec<DrawerHit> = ents
                .iter()
                .map(|e| DrawerHit {
                    drawer: mempalace_core::storage::types::Drawer {
                        id: e.entity.name.clone(),
                        content: e.entity.observations.join("; "),
                        wing: "entity".to_string(),
                        room: e.entity.entity_type.clone(),
                        hall: String::new(),
                        source_file: None,
                        date: None,
                        importance: 1.0,
                        embedding: None,
                    },
                    similarity: e.score,
                })
                .collect();
            rrf_lists.push(hits);
        }

        // Palace search returns a string; parse individual lines as hits
        // For RRF we need structured results, so run search_drawers directly
        if let Ok(ctx) = self.palace_context().await {
            use mempalace_core::storage::StorageBackend;
            use mempalace_core::storage::types::DrawerFilter;
            if let Ok(embedder) = ctx.stack_embedder() {
                if let Ok(qvec) = embedder.embed(query).await {
                    let filter = DrawerFilter {
                        wing: wing.map(String::from),
                        room: None,
                        hall: None,
                    };
                    if let Ok(hits) = self.palace_context().await
                        .map(|c| &c)
                        .ok()
                        .map(|_| async { Ok::<_, anyhow::Error>(Vec::new()) })
                    {
                        // Use the adapter directly
                    }
                }
            }
        }

        // Actually: simpler approach — use adapter directly for structured results
        let palace_hits = {
            let ctx = self.palace_context().await;
            if let Ok(ctx) = ctx {
                ctx.search_drawers_structured(query, wing, n).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        };
        rrf_lists.push(palace_hits);

        // Merge with RRF
        let rrf = ReciprocRankFusion::new(60);
        let merged = rrf.merge(rrf_lists);

        // Convert back to UnifiedHit
        let unified: Vec<UnifiedHit> = merged
            .into_iter()
            .map(|hit| {
                let source = match hit.drawer.wing.as_str() {
                    "memory" => {
                        let scope_str = &hit.drawer.room;
                        HitSource::Memory {
                            scope: MemoryScope::Global,
                            memory_type: MemoryType::Semantic,
                        }
                    }
                    "entity" => HitSource::Entity {
                        entity_type: hit.drawer.room.clone(),
                    },
                    _ => HitSource::Palace {
                        wing: hit.drawer.wing.clone(),
                        room: hit.drawer.room.clone(),
                    },
                };
                UnifiedHit {
                    id: hit.drawer.id,
                    content: hit.drawer.content,
                    source,
                    score: hit.similarity,
                }
            })
            .collect();

        Ok(unified)
    }
}
```

**Note:** The `palace_hybrid_search` implementation above is a draft. During implementation, the adapter-level structured search will need a helper on `PalaceContext`:

```rust
// Add to PalaceContext in context.rs:
pub async fn search_drawers_structured(
    &self,
    query: &str,
    wing: Option<&str>,
    n: usize,
) -> Result<Vec<mempalace_core::storage::types::DrawerHit>> {
    use mempalace_core::storage::StorageBackend;
    use mempalace_core::storage::types::DrawerFilter;

    let qvec = self.stack.embedder().embed(query).await?;
    let filter = DrawerFilter {
        wing: wing.map(String::from),
        room: None,
        hall: None,
    };
    self.adapter.search_drawers(&qvec, filter, n).await
}
```

And a way to access the embedder from the stack — we may need to store the embedder directly on `PalaceContext` as a field.

- [ ] **Step 4: Verify compilation**

Run: `cargo check --features palace,embedded -p surreal-memory 2>&1 | tail -30`

Expected: Should compile. Fix any type mismatches between mempalace-core types and surreal-memory types.

- [ ] **Step 5: Commit**

```bash
git add crates/surreal-memory/src/storage/surreal.rs crates/surreal-memory/src/palace/context.rs
git commit -m "feat(palace): implement PalaceStorage trait on SurrealStorage with OnceCell lazy init"
```

---

### Task 8: MCP Tool Definitions & Handlers

**Files:**
- Modify: `src/mcp/handlers.rs`
- Modify: `src/mcp/mod.rs`

- [ ] **Step 1: Add palace param structs to `handlers.rs`**

At the end of the param struct section in `src/mcp/handlers.rs`:

```rust
// ── Palace (MemPalace integration) ───────────────────────────────────────────

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceWakeUpParams {
    #[schemars(description = "Optional wing to scope the wake-up context")]
    pub wing: Option<String>,
    #[schemars(description = "If true, compress the output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceRecallParams {
    #[schemars(description = "Optional wing to filter by")]
    pub wing: Option<String>,
    #[schemars(description = "Optional room to filter by")]
    pub room: Option<String>,
    #[schemars(description = "Maximum number of drawers to recall (default: 20)")]
    pub limit: Option<u32>,
    #[schemars(description = "If true, compress the output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceSearchParams {
    #[schemars(description = "Natural language search query")]
    pub query: String,
    #[schemars(description = "Optional wing to filter by")]
    pub wing: Option<String>,
    #[schemars(description = "Optional room to filter by")]
    pub room: Option<String>,
    #[schemars(description = "Number of results (default: 10)")]
    pub n: Option<u32>,
    #[schemars(description = "If true, compress the output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceIngestParams {
    #[schemars(description = "Content to store in the palace")]
    pub content: String,
    #[schemars(description = "Wing (top-level category)")]
    pub wing: String,
    #[schemars(description = "Room within the wing")]
    pub room: String,
    #[schemars(description = "Hall within the room (default: 'general')")]
    pub hall: Option<String>,
    #[schemars(description = "Importance weight 0.0-1.0 (default: 1.0)")]
    pub importance: Option<f32>,
}

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceDeleteParams {
    #[schemars(description = "Drawer record ID to delete (e.g. 'drawers:abc123')")]
    pub id: String,
}

#[cfg(feature = "palace")]
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceHybridSearchParams {
    #[schemars(description = "Natural language search query")]
    pub query: String,
    #[schemars(description = "Optional memory scope filter ('global', 'agent', 'user', 'session')")]
    pub scope: Option<String>,
    #[schemars(description = "Optional wing to filter palace drawers")]
    pub wing: Option<String>,
    #[schemars(description = "Number of results per source (default: 10)")]
    pub n: Option<u32>,
    #[schemars(description = "If true, compress content using AAAK Dialect")]
    pub compress: Option<bool>,
}
```

- [ ] **Step 2: Add palace handler methods to MemoryHandler**

Add a new `impl` block at the end of `handlers.rs`:

```rust
#[cfg(feature = "palace")]
impl MemoryHandler {
    pub async fn palace_wake_up(
        &self,
        params: PalaceWakeUpParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let result = storage
            .palace_wake_up(params.wing.as_deref())
            .await
            .map_err(Self::internal_error)?;

        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_recall(
        &self,
        params: PalaceRecallParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let limit = params.limit.unwrap_or(20) as usize;
        let result = storage
            .palace_recall(params.wing.as_deref(), params.room.as_deref(), limit)
            .await
            .map_err(Self::internal_error)?;

        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_search(
        &self,
        params: PalaceSearchParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let n = params.n.unwrap_or(10) as usize;
        let result = storage
            .palace_search(&params.query, params.wing.as_deref(), params.room.as_deref(), n)
            .await
            .map_err(Self::internal_error)?;

        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_ingest(
        &self,
        params: PalaceIngestParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let hall = params.hall.unwrap_or_else(|| "general".to_string());
        let importance = params.importance.unwrap_or(1.0);

        let id = storage
            .palace_ingest(&params.content, &params.wing, &params.room, &hall, importance)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({ "id": id }).to_string(),
        )]))
    }

    pub async fn palace_delete(
        &self,
        params: PalaceDeleteParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        storage
            .palace_delete(&params.id)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({ "deleted": true }).to_string(),
        )]))
    }

    pub async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let status = storage.palace_status().await.map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&status).unwrap_or_default(),
        )]))
    }

    pub async fn palace_hybrid_search(
        &self,
        params: PalaceHybridSearchParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self
            .storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| Self::internal_error(anyhow::anyhow!("Palace requires SurrealStorage")))?;

        let scope = params.scope.as_deref().and_then(|s| match s {
            "global" => Some(surreal_memory::MemoryScope::Global),
            "agent" => Some(surreal_memory::MemoryScope::Agent),
            "user" => Some(surreal_memory::MemoryScope::User),
            "session" => Some(surreal_memory::MemoryScope::Session),
            "task" => Some(surreal_memory::MemoryScope::Task),
            _ => None,
        });

        let n = params.n.unwrap_or(10) as usize;
        let mut results = storage
            .palace_hybrid_search(&params.query, scope, params.wing.as_deref(), n)
            .await
            .map_err(Self::internal_error)?;

        if params.compress.unwrap_or(false) {
            for hit in &mut results {
                hit.content = storage.palace_compress(&hit.content);
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_default(),
        )]))
    }
}
```

- [ ] **Step 3: Add `as_any()` to MemoryStorage trait**

The handlers need to downcast `Arc<dyn MemoryStorage>` to `SurrealStorage`. Add to `crates/surreal-memory/src/storage/mod.rs`:

```rust
use std::any::Any;
```

Add to the `MemoryStorage` trait:

```rust
    /// Downcast helper for feature-gated subsystem access (e.g. PalaceStorage).
    fn as_any(&self) -> &dyn Any;
```

Implement in `SurrealStorage`:

```rust
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
```

- [ ] **Step 4: Add tool definitions to `src/mcp/mod.rs`**

At the bottom of the `#[tool_router] impl MemoryMcpServer` block, add (before the closing `}`):

```rust
    // ── Palace (MemPalace integration) ───────────────────────────────────────

    #[cfg(feature = "palace")]
    #[tool(
        description = "Get palace wake-up context: L0 (identity) + L1 (essential story). Provides high-level overview of stored knowledge."
    )]
    async fn palace_wake_up(
        &self,
        Parameters(params): Parameters<handlers::PalaceWakeUpParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_wake_up(params).await
    }

    #[cfg(feature = "palace")]
    #[tool(
        description = "Recall memories from the palace: L2 on-demand retrieval filtered by wing and room taxonomy."
    )]
    async fn palace_recall(
        &self,
        Parameters(params): Parameters<handlers::PalaceRecallParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_recall(params).await
    }

    #[cfg(feature = "palace")]
    #[tool(
        description = "Semantic search over palace drawers: L3 deep similarity search using 384-dim embeddings."
    )]
    async fn palace_search(
        &self,
        Parameters(params): Parameters<handlers::PalaceSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_search(params).await
    }

    #[cfg(feature = "palace")]
    #[tool(
        description = "Ingest content into a palace drawer with automatic embedding and dedup. Organize by wing/room/hall taxonomy."
    )]
    async fn palace_ingest(
        &self,
        Parameters(params): Parameters<handlers::PalaceIngestParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_ingest(params).await
    }

    #[cfg(feature = "palace")]
    #[tool(description = "Delete a palace drawer by ID.")]
    async fn palace_delete(
        &self,
        Parameters(params): Parameters<handlers::PalaceDeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_delete(params).await
    }

    #[cfg(feature = "palace")]
    #[tool(
        description = "Get palace status: total drawers, wings, rooms, and identity state."
    )]
    async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        self.handler.palace_status().await
    }

    #[cfg(feature = "palace")]
    #[tool(
        description = "Cross-domain hybrid search using Reciprocal Rank Fusion across memory, entity, and palace drawer tables. Merges results from all three search domains."
    )]
    async fn palace_hybrid_search(
        &self,
        Parameters(params): Parameters<handlers::PalaceHybridSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_hybrid_search(params).await
    }
```

Also add the conditional imports at the top of `mod.rs`:

```rust
#[cfg(feature = "palace")]
use handlers::{
    PalaceDeleteParams, PalaceHybridSearchParams, PalaceIngestParams,
    PalaceRecallParams, PalaceSearchParams, PalaceWakeUpParams,
};
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check --features palace,embedded 2>&1 | tail -30`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/mcp/mod.rs src/mcp/handlers.rs crates/surreal-memory/src/storage/mod.rs crates/surreal-memory/src/storage/surreal.rs
git commit -m "feat(palace): add 7 MCP palace tools with handlers and as_any() downcast"
```

---

### Task 9: REST API Endpoints

**Files:**
- Create: `src/api/palace.rs`
- Modify: `src/api/mod.rs`

- [ ] **Step 1: Create `src/api/palace.rs`**

```rust
//! Palace REST API routes.
//! GET  /api/v1/palace/wake
//! GET  /api/v1/palace/recall
//! POST /api/v1/palace/search
//! POST /api/v1/palace/ingest
//! DELETE /api/v1/palace/drawer/:id
//! GET  /api/v1/palace/status
//! POST /api/v1/palace/hybrid-search

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use surreal_memory::{PalaceStorage, SurrealStorage};

use super::{AppState, ApiFailure, api_error, bad_request};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/wake", get(wake_up))
        .route("/recall", get(recall))
        .route("/search", post(search))
        .route("/ingest", post(ingest))
        .route("/drawer/{id}", delete(delete_drawer))
        .route("/status", get(status))
        .route("/hybrid-search", post(hybrid_search))
}

fn get_palace_storage(state: &AppState) -> Result<&SurrealStorage, ApiFailure> {
    state
        .storage
        .as_any()
        .downcast_ref::<SurrealStorage>()
        .ok_or_else(|| bad_request("Palace requires SurrealStorage backend"))
}

#[derive(Deserialize)]
struct WakeUpQuery {
    wing: Option<String>,
    compress: Option<bool>,
}

async fn wake_up(
    State(state): State<AppState>,
    Query(q): Query<WakeUpQuery>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let result = storage
        .palace_wake_up(q.wing.as_deref())
        .await
        .map_err(api_error)?;
    let output = if q.compress.unwrap_or(false) {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(serde_json::json!({ "context": output })))
}

#[derive(Deserialize)]
struct RecallQuery {
    wing: Option<String>,
    room: Option<String>,
    limit: Option<u32>,
    compress: Option<bool>,
}

async fn recall(
    State(state): State<AppState>,
    Query(q): Query<RecallQuery>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let limit = q.limit.unwrap_or(20) as usize;
    let result = storage
        .palace_recall(q.wing.as_deref(), q.room.as_deref(), limit)
        .await
        .map_err(api_error)?;
    let output = if q.compress.unwrap_or(false) {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(serde_json::json!({ "context": output })))
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    wing: Option<String>,
    room: Option<String>,
    n: Option<u32>,
    compress: Option<bool>,
}

async fn search(
    State(state): State<AppState>,
    Json(body): Json<SearchBody>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let n = body.n.unwrap_or(10) as usize;
    let result = storage
        .palace_search(&body.query, body.wing.as_deref(), body.room.as_deref(), n)
        .await
        .map_err(api_error)?;
    let output = if body.compress.unwrap_or(false) {
        storage.palace_compress(&result)
    } else {
        result
    };
    Ok(Json(serde_json::json!({ "context": output })))
}

#[derive(Deserialize)]
struct IngestBody {
    content: String,
    wing: String,
    room: String,
    hall: Option<String>,
    importance: Option<f32>,
}

async fn ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestBody>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let hall = body.hall.unwrap_or_else(|| "general".to_string());
    let importance = body.importance.unwrap_or(1.0);
    let id = storage
        .palace_ingest(&body.content, &body.wing, &body.room, &hall, importance)
        .await
        .map_err(api_error)?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn delete_drawer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    storage.palace_delete(&id).await.map_err(api_error)?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn status(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let status = storage.palace_status().await.map_err(api_error)?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

#[derive(Deserialize)]
struct HybridSearchBody {
    query: String,
    scope: Option<String>,
    wing: Option<String>,
    n: Option<u32>,
    compress: Option<bool>,
}

async fn hybrid_search(
    State(state): State<AppState>,
    Json(body): Json<HybridSearchBody>,
) -> Result<Json<serde_json::Value>, ApiFailure> {
    let storage = get_palace_storage(&state)?;
    let scope = body.scope.as_deref().and_then(|s| match s {
        "global" => Some(surreal_memory::MemoryScope::Global),
        "agent" => Some(surreal_memory::MemoryScope::Agent),
        "user" => Some(surreal_memory::MemoryScope::User),
        "session" => Some(surreal_memory::MemoryScope::Session),
        "task" => Some(surreal_memory::MemoryScope::Task),
        _ => None,
    });
    let n = body.n.unwrap_or(10) as usize;
    let mut results = storage
        .palace_hybrid_search(&body.query, scope, body.wing.as_deref(), n)
        .await
        .map_err(api_error)?;

    if body.compress.unwrap_or(false) {
        for hit in &mut results {
            hit.content = storage.palace_compress(&hit.content);
        }
    }

    Ok(Json(serde_json::to_value(results).unwrap_or_default()))
}
```

- [ ] **Step 2: Register palace router in `src/api/mod.rs`**

Add the module declaration (conditionally):

```rust
#[cfg(feature = "palace")]
pub mod palace;
```

In `build_router()`, add the palace route nest before `.layer(CorsLayer::permissive())`:

```rust
        #[cfg(feature = "palace")]
        let router = router.nest("/api/v1/palace", palace::router());
```

Change the router construction to use a mutable binding:

```rust
    let mut router = Router::new()
        .route("/health", get(health_handler))
        // ... existing routes ...
        .merge(mcp_sub);

    #[cfg(feature = "palace")]
    {
        router = router.nest("/api/v1/palace", palace::router());
    }

    router
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --features palace,embedded 2>&1 | tail -20`

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/api/palace.rs src/api/mod.rs
git commit -m "feat(palace): add 7 REST API endpoints for palace operations"
```

---

### Task 10: Logging Update for Fast Embedding Provider

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add Fast provider logging**

In the `load_config()` function, add a match arm for the Fast variant:

```rust
    #[cfg(feature = "palace")]
    {
        if matches!(&config.embedding_provider, embeddings::EmbeddingProvider::Fast) {
            tracing::info!("   Embedding: Fast (all-MiniLM-L6-v2, 384d)");
        }
    }
```

Also add handling in the binary's `src/embeddings/mod.rs` (or `src/config.rs`) — wherever `EmbeddingProvider` is parsed from env vars — to recognize `EMBEDDING_PROVIDER=fast`:

In `src/config.rs` or wherever `EmbeddingProvider` is constructed from env, add:

```rust
    #[cfg(feature = "palace")]
    "fast" => surreal_memory::EmbeddingProvider::Fast,
```

- [ ] **Step 2: Commit**

```bash
git add src/main.rs src/config.rs
git commit -m "feat(palace): add Fast embedding provider to config and startup logging"
```

---

### Task 11: Build Verification & Fix Compilation Issues

**Files:**
- Various (as needed for compilation fixes)

- [ ] **Step 1: Full build with palace feature**

Run: `cargo build --features palace,embedded 2>&1 | tail -40`

Expected: Either PASS or specific compilation errors to fix.

- [ ] **Step 2: Fix any compilation errors**

Common issues to expect and fix:
- `MemoryStack` fields might be `pub(super)` — may need accessor methods or store embedder separately on `PalaceContext`
- `as_any()` may need `'static` bound on `MemoryStorage` trait
- `#[cfg(feature = "palace")]` on `#[tool()]` methods may not be supported by `rmcp` macro — may need a wrapper approach
- Type mismatches between `mempalace_core` and `surreal_memory` versions of `sha2` or other shared deps

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --features palace,embedded -- -D warnings 2>&1 | tail -20`

Expected: PASS (or warnings to fix)

- [ ] **Step 4: Commit fixes**

```bash
git add -A
git commit -m "fix(palace): resolve compilation issues from palace integration"
```

---

### Task 12: Integration Smoke Test

**Files:**
- No new files — test via running server

- [ ] **Step 1: Start the server with palace feature**

Run: `EMBEDDING_PROVIDER=fast cargo run --features palace,embedded 2>&1`

Expected: Server starts, logs "Embedding: Fast (all-MiniLM-L6-v2, 384d)", migration v16 runs on first startup.

- [ ] **Step 2: Test palace_status via REST**

Run: `curl -s http://localhost:3001/api/v1/palace/status | jq`

Expected:
```json
{
  "total_drawers": 0,
  "total_wings": 0,
  "total_rooms": 0,
  "identity_loaded": false
}
```

- [ ] **Step 3: Test ingest + search flow**

```bash
# Ingest
curl -s -X POST http://localhost:3001/api/v1/palace/ingest \
  -H "Content-Type: application/json" \
  -d '{"content":"Rust ownership prevents data races at compile time","wing":"engineering","room":"rust","importance":0.9}' | jq

# Search
curl -s -X POST http://localhost:3001/api/v1/palace/search \
  -H "Content-Type: application/json" \
  -d '{"query":"memory safety","wing":"engineering","n":5}' | jq
```

Expected: Ingest returns `{"id": "drawers:..."}`. Search returns context string with the ingested content.

- [ ] **Step 4: Commit any final fixes**

```bash
git add -A
git commit -m "fix(palace): integration test fixes from smoke testing"
```

---

## Summary

| Task | What it does | Key files |
|------|-------------|-----------|
| 1 | Cargo deps + feature gates | `Cargo.toml`, `crates/surreal-memory/Cargo.toml` |
| 2 | Migration v16 (drawers table) | `migrations/mod.rs` |
| 3 | `db()` handle on SurrealStorage | `storage/surreal.rs` |
| 4 | FastEmbedService bridge | `palace/embedding.rs`, `embeddings/mod.rs` |
| 5 | PalaceAdapter (StorageBackend) | `palace/adapter.rs` |
| 6 | PalaceContext + module root + trait | `palace/mod.rs`, `palace/context.rs`, `lib.rs` |
| 7 | PalaceStorage impl on SurrealStorage | `storage/surreal.rs` |
| 8 | 7 MCP tools + handlers | `mcp/mod.rs`, `mcp/handlers.rs` |
| 9 | 7 REST endpoints | `api/palace.rs`, `api/mod.rs` |
| 10 | Config + logging for Fast provider | `main.rs`, `config.rs` |
| 11 | Build verification + fixes | Various |
| 12 | Integration smoke test | None (runtime) |
