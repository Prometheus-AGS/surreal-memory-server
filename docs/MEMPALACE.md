# MEMPALACE Integration Plan

**Status:** Plan — ready for implementation  
**Author:** Prometheus AGS  
**Date:** 2026-04-10  

---

## 1. Executive Summary

This document describes how to integrate `mempalace-rs` into the
`surreal-memory-server` workspace. The integration adds four capabilities
that the current codebase lacks:

| Gap (current) | Capability added (post-integration) |
|---|---|
| Local embeddings require `candle-core` (heavy, optional feature) | `FastEmbedder` (fastembed / all-MiniLM-L6-v2) as a zero-dependency local embedding path |
| Cold-start fails silently when `OPENAI_API_KEY` is absent | Sovereign local-first embedding fallback that requires no API key |
| Context injection is a single-pass BM25+vector query | 4-layer `MemoryStack` (L0 identity + L1 essential story + L2 on-demand recall + L3 deep search) |
| No hybrid reranking across result lists | Reciprocal Rank Fusion (RRF) for merging BM25, vector, and palace results |
| No memory compression for token-sensitive contexts | AAAK Dialect pipeline — entities/topics/emotions/flags in a compact pipe-separated format |

---

## 2. Codebase Audit

### 2.1 surreal-memory-server (target)

```
surreal-memory-server/
  Cargo.toml              # workspace root: binary + crates/surreal-memory
  src/
    config.rs             # Config: EmbeddingProvider (OpenAI|Cohere|Local)
    embeddings/mod.rs     # EmbeddingService trait + create_embedding_service()
    storage/mod.rs        # re-exports SurrealStorage from lib crate
    mcp/                  # MCP handler layer
    api/                  # REST handler layer (memory, entities, mindmaps, etc.)
  crates/surreal-memory/
    src/
      lib.rs              # SurrealStorage, MemoryStorage trait, Knowledge graph
      memory.rs           # MemoryScope (Global/Agent/User/Session/Task), MemoryType
      embeddings/mod.rs   # EmbeddingService trait (separate from binary's)
      storage/surreal.rs  # SurrealStorage: Surreal<Any> + EmbeddingService + RetryConfig
      entity.rs           # KnowledgeGraph, SemanticSearchResult
      task_stream.rs      # TaskStream, ContextWindow
      mindmap.rs          # MindMap
```

**Key constraints:**
- `surrealdb = "3.0.5"` (workspace pinned)
- `EmbeddingProvider::Local` path exists but requires `local-embeddings` feature
  (pulls in `candle-core`, `candle-nn`, `candle-transformers`, `tokenizers`, `hf-hub`)
- Cold-start default is `EMBEDDING_PROVIDER=local` → resolves to `BAAI/bge-small-en-v1.5`
  via Candle — but if `local-embeddings` feature is not compiled in, server panics at runtime
- `docs/` directory already exists; `docs/plans/` holds reconnection design docs

### 2.2 mempalace-rs (source)

```
mempalace-rs/
  crates/
    mempalace-core/       # zero-DB Rust library — traits + algorithms
      src/
        embedder.rs       # Embedder trait; FastEmbedder (fastembed, all-MiniLM-L6-v2, 384d)
        storage/          # StorageBackend trait + Drawer/DrawerHit/DrawerFilter types
        layers/
          stack.rs        # MemoryStack: wake_up() recall() search() status()
          layer0.rs       # L0 — identity file (~100 tokens)
          layer1.rs       # L1 — essential story: top-weighted drawers by room (~500-800 tok)
          layer2.rs       # L2 — on-demand recall: filtered by wing/room
          layer3.rs       # L3 — deep search: full semantic cosine similarity
        reranker/rrf.rs   # ReciprocRankFusion: merge N ranked lists → single ranked list
        dialect/          # AAAK Dialect: compress text → entities|topics|"quote"|emotions|FLAGS
        ingest/           # ConvoMiner (conversation), Miner (directory walk + dedup)
        normalize/        # Chunker (paragraph), Formats
        knowledge_graph/  # KG types (separate from surreal-memory's KG)
    mempalace-surreal/    # SurrealDB StorageBackend impl (separate DB connection)
    mempalace-pg/         # PostgreSQL+pgvector StorageBackend impl
```

**Key observations:**
- `mempalace-core` has **no database dependencies** — only `fastembed`, `rusqlite`
  (bundled, used for tests), `walkdir`, `sha2`
- `FastEmbedder` wraps `fastembed = "4"` which downloads `all-MiniLM-L6-v2` (~25MB)
  on first use and caches it — pure CPU, no GPU required
- `mempalace-surreal` creates its **own** `Surreal<Any>` connection and its own
  `drawers` table — it does NOT share an existing connection
- The `Drawer` schema (`wing`, `room`, `hall`, `importance`, `embedding`, `content_hash`)
  is orthogonal to surreal-memory's `Memory` schema — they coexist cleanly
- `surrealdb = { version = "3", features = ["kv-mem", "protocol-ws"] }` in
  `mempalace-surreal` — compatible with `3.0.5` in surreal-memory-server workspace

---

## 3. Integration Architecture

### 3.1 What NOT to do

Do **not** bring `mempalace-surreal` into the workspace as-is. It creates a separate
`Surreal<Any>` connection (its own credentials, namespace, database). Doing so would:
- Create two independent SurrealDB connections to the same database
- Produce a second `drawers` table that cannot be reached by the existing MCP/REST handlers
- Duplicate schema management

### 3.2 Integration Model

Add only `mempalace-core` to the workspace as a new crate:

```
surreal-memory-server/ (workspace)
  Cargo.toml
  crates/
    surreal-memory/            # existing lib — unchanged public API
    mempalace-core/            # NEW: vendored from mempalace-rs/crates/mempalace-core
  src/
    palace/                    # NEW: adapter + MemoryStack wiring
      mod.rs
      adapter.rs               # PalaceAdapter: implements StorageBackend using Surreal<Any>
      context.rs               # PalaceContext: wraps MemoryStack, exposes to MemoryStorage
      embedding.rs             # FastEmbedService: wraps FastEmbedder, implements EmbeddingService
    api/
      palace.rs                # NEW: REST handlers for palace endpoints
    mcp/
      handlers.rs              # MODIFIED: add palace_* MCP tools
```

This preserves:
- Single SurrealDB connection (shared `Surreal<Any>`)
- Single schema migration runner
- Existing `MemoryStorage` trait surface — no breaking changes

---

## 4. Dependency Changes

### 4.1 Workspace `Cargo.toml`

```toml
# Add to [workspace]
members = [
    ".",
    "crates/surreal-memory",
    "crates/mempalace-core",   # NEW
]

# Add to [workspace.dependencies]
fastembed  = "4"
walkdir    = "2"
sha2       = "0.10"   # already present implicitly; make explicit
hex        = "0.4"
```

### 4.2 `crates/surreal-memory/Cargo.toml`

```toml
# Add under [dependencies]
mempalace-core = { path = "../mempalace-core" }

# Add under [features]
palace = []   # enables palace endpoints; off by default
```

### 4.3 `crates/mempalace-core/Cargo.toml`

The crate is vendored verbatim from `mempalace-rs/crates/mempalace-core` with one
change: promote `edition` from workspace (2024) to an explicit declaration:

```toml
[package]
name    = "mempalace-core"
version = "0.1.0"
edition = "2024"
```

Remove the `[dev-dependencies]` reference to `mempalace-surreal` (no longer in scope).

---

## 5. Schema Migration

A new migration (`v7_palace.surql`) adds the `drawers` table to the existing
SurrealDB namespace. This runs via the existing `MigrationRunner` in
`crates/surreal-memory/src/storage/migrations/`.

```sql
-- crates/surreal-memory/src/storage/migrations/v7_palace.surql

-- Drawers table: MemPalace wing/room/hall taxonomy
DEFINE TABLE IF NOT EXISTS drawers SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS content      ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS wing         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS room         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS hall         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS source_file  ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date         ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS importance   ON drawers TYPE float DEFAULT 1.0;
DEFINE FIELD IF NOT EXISTS embedding    ON drawers TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS content_hash ON drawers TYPE option<string>;

-- HNSW vector index for L3 semantic search (384d — all-MiniLM-L6-v2)
DEFINE INDEX IF NOT EXISTS drawer_embedding_idx
  ON drawers FIELDS embedding
  MTREE DIMENSION 384 DIST COSINE;

-- Full-text index for keyword recall
DEFINE INDEX IF NOT EXISTS drawer_content_idx
  ON drawers FIELDS content SEARCH ANALYZER unicode BM25;

-- Content-hash uniqueness check index
DEFINE INDEX IF NOT EXISTS drawer_hash_idx
  ON drawers FIELDS content_hash;
```

**Note on embedding dimensions:** The existing `memories` table uses 384d (bge-small),
768d (bge-base), or 1536d (OpenAI) depending on the configured provider. The `drawers`
table is always 384d (all-MiniLM-L6-v2 via FastEmbedder). These are independent HNSW
indexes — no cross-table vector search is attempted.

---

## 6. Implementation Phases

### Phase 1 — FastEmbedService (cold-start fix)

**Goal:** Replace the Candle dependency for local embeddings with `fastembed`.
After this phase, `EMBEDDING_PROVIDER=local` works without compiling `candle-core`.

**New file:** `src/palace/embedding.rs`

```rust
use mempalace_core::embedder::{Embedder, FastEmbedder, NoOpEmbedder};
use std::sync::Arc;

/// Wraps mempalace-core's FastEmbedder to implement surreal-memory's
/// EmbeddingService trait. Dimensions: 384 (all-MiniLM-L6-v2).
pub struct FastEmbedService {
    inner: Arc<dyn Embedder>,
}

impl FastEmbedService {
    pub async fn new() -> anyhow::Result<Self> {
        let embedder = FastEmbedder::new_default().await?;
        Ok(Self { inner: Arc::new(embedder) })
    }

    pub fn noop() -> Self {
        Self { inner: Arc::new(NoOpEmbedder) }
    }
}

#[async_trait::async_trait]
impl crate::embeddings::EmbeddingService for FastEmbedService {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        self.inner.embed_batch(&refs).await
    }

    fn dimensions(&self) -> usize {
        384
    }
}
```

**Modify** `src/config.rs`: when `EMBEDDING_PROVIDER=local` and the
`local-embeddings` feature is absent, fall back to `FastEmbedService::new()` rather
than panicking. The feature flag `local-embeddings` becomes an opt-in upgrade
(Candle with optional GPU) rather than a required path.

**Modify** `src/embeddings/mod.rs`:

```rust
pub enum EmbeddingProvider {
    OpenAI  { api_key: String, model: String },
    Cohere  { api_key: String, model: String },
    Local   { model_id: String, model_path: Option<String> },
    Fast,   // NEW — fastembed / all-MiniLM-L6-v2, no config required
}
```

When `EMBEDDING_PROVIDER` is unset or set to `"fast"`, resolve to
`FastEmbedService`. `"local"` continues to resolve to Candle when that feature
is compiled in, or silently upgrades to `FastEmbedService` when it is not.

**Tests:** Add `#[tokio::test] async fn fast_embed_roundtrip()` to
`crates/surreal-memory/tests/`.

---

### Phase 2 — PalaceAdapter (StorageBackend over SurrealDB)

**Goal:** Implement mempalace-core's `StorageBackend` trait using the existing
`Surreal<Any>` connection from `SurrealStorage`. The `drawers` table is created
by the migration in Phase 0 above.

**New file:** `src/palace/adapter.rs`

The adapter holds `Arc<surrealdb::Surreal<surrealdb::engine::any::Any>>` and
executes the same SurrealQL that `mempalace-surreal/src/backend.rs` executes,
but through the shared handle rather than a separate connection.

Key method signatures:

```rust
pub struct PalaceAdapter {
    db: Arc<surrealdb::Surreal<surrealdb::engine::any::Any>>,
}

impl PalaceAdapter {
    /// Construct from the existing SurrealStorage's DB handle.
    /// SurrealStorage should expose `pub fn db_handle(&self) -> Arc<...>`.
    pub fn from_storage(storage: &SurrealStorage) -> Self { ... }
}

#[async_trait]
impl StorageBackend for PalaceAdapter {
    async fn add_drawer(&self, drawer: Drawer) -> anyhow::Result<String> { ... }
    async fn search_drawers(&self, query_embedding: &[f32], filter: DrawerFilter, n: usize)
        -> anyhow::Result<Vec<DrawerHit>> { ... }
    // ... remaining 7 methods
}
```

**Modify** `crates/surreal-memory/src/storage/surreal.rs`:

Add a method `pub fn db_handle(&self) -> Arc<Surreal<Any>>` to `SurrealStorage`
so the adapter can share the live connection without cloning credentials.

---

### Phase 3 — MemoryStack Integration

**Goal:** Expose `MemoryStack` operations through `MemoryStorage` so MCP handlers
and REST handlers can call palace operations without importing mempalace-core directly.

**New file:** `src/palace/context.rs`

```rust
pub struct PalaceContext {
    stack: MemoryStack,
}

impl PalaceContext {
    pub fn new(adapter: PalaceAdapter, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            stack: MemoryStack::new(Arc::new(adapter), embedder, None),
        }
    }

    /// L0 + L1 context string — call at agent session start.
    pub async fn wake_up(&self, wing: Option<&str>) -> anyhow::Result<String> {
        self.stack.wake_up(wing).await
    }

    /// L2 filtered recall — call on-demand mid-session.
    pub async fn recall(&self, wing: Option<&str>, room: Option<&str>, limit: usize)
        -> anyhow::Result<String> {
        self.stack.recall(wing, room, limit).await
    }

    /// L3 deep search — semantic similarity over all drawers.
    pub async fn search(&self, query: &str, wing: Option<&str>, room: Option<&str>, n: usize)
        -> anyhow::Result<String> {
        self.stack.search(query, wing, room, n).await
    }
}
```

**Modify** `crates/surreal-memory/src/storage/mod.rs` — add palace methods to
`MemoryStorage` trait behind a `#[cfg(feature = "palace")]` guard:

```rust
// Optional palace extensions — enabled via Cargo feature
#[cfg(feature = "palace")]
async fn palace_wake_up(&self, wing: Option<&str>) -> anyhow::Result<String>;

#[cfg(feature = "palace")]
async fn palace_recall(&self, wing: Option<&str>, room: Option<&str>, limit: usize)
    -> anyhow::Result<String>;

#[cfg(feature = "palace")]
async fn palace_search(&self, query: &str, wing: Option<&str>, room: Option<&str>, n: usize)
    -> anyhow::Result<String>;

#[cfg(feature = "palace")]
async fn palace_ingest(&self, content: &str, wing: &str, room: &str, importance: f32)
    -> anyhow::Result<String>;
```

`SurrealStorage` implements all four by delegating to `PalaceContext`.

---

### Phase 4 — REST API + MCP Tools

**Goal:** Expose palace operations to UAR agents and The Boss via the existing
transport layers.

#### 4a REST Endpoints (`src/api/palace.rs`)

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/palace/wake` | L0 + L1 context string. Query: `?wing=` |
| `GET` | `/api/v1/palace/recall` | L2 filtered recall. Query: `?wing=&room=&limit=` |
| `POST` | `/api/v1/palace/search` | L3 deep search. Body: `{ "query": "", "wing"?: "", "room"?: "", "n"?: 10 }` |
| `POST` | `/api/v1/palace/ingest` | Add a drawer. Body: `{ "content": "", "wing": "", "room": "", "importance"?: 1.0 }` |
| `DELETE` | `/api/v1/palace/drawer/:id` | Remove a drawer by ID |
| `GET` | `/api/v1/palace/status` | `StackStatus`: drawer count, wings, rooms, identity loaded |

Register the router in `src/main.rs` alongside existing `/api/v1/memory`,
`/api/v1/entities`, etc.

#### 4b MCP Tools (`src/mcp/handlers.rs`)

Six new tool registrations under the existing MCP server:

| Tool name | Arguments | Returns |
|---|---|---|
| `palace_wake_up` | `wing?: string` | context string (L0+L1) |
| `palace_recall` | `wing?: string`, `room?: string`, `limit?: u32` | context string (L2) |
| `palace_search` | `query: string`, `wing?: string`, `room?: string`, `n?: u32` | context string (L3) |
| `palace_ingest` | `content: string`, `wing: string`, `room: string`, `importance?: f32` | `{ "id": "..." }` |
| `palace_delete` | `id: string` | `{ "deleted": true }` |
| `palace_status` | _(none)_ | `StackStatus` JSON |

These integrate with the existing `rmcp` tool registration pattern used for
`memory_add`, `memory_search`, etc. No new MCP transport changes required.

---

### Phase 5 — Dialect Compression Pipeline

**Goal:** Make the AAAK Dialect available as an optional compression pass for
outgoing context strings. Apply before context is injected into the LLM prompt
when token budgets are tight.

**New method** on `PalaceContext`:

```rust
pub fn compress_context(&self, text: &str, config: Option<DialectConfig>) -> String {
    let dialect = Dialect::new(config.unwrap_or_default());
    dialect.compress(text, None)
}
```

**New config env var:** `PALACE_COMPRESS_CONTEXT=true` (default: false).
When enabled, context strings returned by `wake_up()`, `recall()`, and `search()`
are passed through the compression pipeline before being returned. This is
particularly relevant when the UAR's `context_builder` applies a 10% token budget
ceiling — the compression ratio (~30x on verbose memory content) can allow
significantly more facts to fit within the budget.

**Expose in REST:** Add `?compress=true` query param to all palace GET endpoints.

---

### Phase 6 — RRF Hybrid Reranking

**Goal:** Merge results from the existing BM25 full-text search, the existing
HNSW vector search, and the palace L3 semantic search using Reciprocal Rank Fusion
into a single unified ranked list.

This is the highest-value, highest-complexity phase. It requires adapting
`DrawerHit` ↔ `SemanticSearchResult` (surreal-memory's result type) so the
reranker can merge across both schemas.

**New adapter type:**

```rust
pub struct UnifiedHit {
    pub id: String,
    pub content: String,
    pub source: HitSource, // enum: Scoped | Palace
    pub score: f32,
}

pub enum HitSource {
    Scoped { scope: MemoryScope, memory_type: MemoryType },
    Palace { wing: String, room: String },
}
```

**New method** on `MemoryStorage` trait (behind `#[cfg(feature = "palace")]`):

```rust
async fn hybrid_search(
    &self,
    query: &str,
    scope: Option<MemoryScope>,
    wing: Option<&str>,
    n: usize,
) -> anyhow::Result<Vec<UnifiedHit>>;
```

**Implementation:** Concurrently execute:
1. `memory_search_by_embedding(query_embedding, scope, n)` — existing vector search on `memories`
2. `entity_search(query, n)` — existing knowledge graph semantic search
3. `palace_search(query, wing, None, n)` — new L3 drawer search

Convert each result list to `Vec<DrawerHit>` (with dummy Drawer wrapping the content),
run `ReciprocRankFusion::new(60).merge(vec![list1, list2, list3])`, then convert
back to `Vec<UnifiedHit>`.

**New MCP tool:** `memory_hybrid_search` with args `query`, `scope?`, `wing?`, `n?`.

---

## 7. New Environment Variables

| Variable | Default | Description |
|---|---|---|
| `PALACE_ENABLED` | `true` | Master switch for palace feature |
| `PALACE_IDENTITY_PATH` | `~/.mempalace/identity.txt` | L0 identity file path |
| `PALACE_COMPRESS_CONTEXT` | `false` | Apply AAAK Dialect compression to context strings |
| `EMBEDDING_PROVIDER` | `fast` | Change default from `local` (Candle) to `fast` (fastembed) |

The existing `LOCAL_EMBEDDING_MODEL` and `MODEL_CACHE_DIR` vars are preserved
for backward compatibility when `EMBEDDING_PROVIDER=local` is explicitly set.

---

## 8. Dependency Version Compatibility

| Crate | surreal-memory-server | mempalace-core | Resolution |
|---|---|---|---|
| `surrealdb` | `3.0.5` | `3.x` (via mempalace-surreal) | Compatible; mempalace-core has no direct SurrealDB dep |
| `fastembed` | absent | `4.x` | Add to workspace; no conflicts |
| `rusqlite` | absent | `0.31` (bundled, dev-dep only) | No conflict; only in test code |
| `walkdir` | absent | `2.x` | Add to workspace |
| `sha2` | `0.11.0` | `0.10.x` | Minor version conflict — mempalace-core uses `0.10`; pin workspace to `0.11` and update mempalace-core to match, or add as non-workspace dep in mempalace-core |
| `regex` | absent | `1.x` | Add to workspace |
| `tokio` | `1.50.0` | `1.x` | Compatible |
| `axum` | `0.8` | `0.7` (binary only) | No conflict — mempalace-core has no axum dep |

The `sha2` minor version mismatch is the only friction point. Resolution: pin
`mempalace-core/Cargo.toml` to `sha2 = "0.11"` to match workspace.

---

## 9. Testing Plan

### Unit tests (add to `crates/surreal-memory/tests/`)

- `palace_fast_embed_roundtrip` — FastEmbedService produces 384-dim vector
- `palace_adapter_add_and_search` — PalaceAdapter add/search against in-memory SurrealDB
- `palace_dialect_compress` — DialectCompress reduces token count
- `palace_rrf_merge` — RRF correctly merges three ranked lists

### Integration tests (add to `tests/`)

- `palace_server_mode.rs` — exercises all six REST endpoints end-to-end
- `palace_mcp.rs` — exercises all six MCP tools via the MCP test harness pattern
  used in `tests/mindmap_server_mode.rs`

### Regression guarantee

The existing 42 MCP tools, all REST endpoints, and both integration test files
(`mindmap_server_mode.rs`, `taskstream_server_mode.rs`) must pass unchanged after
the integration. The `palace` feature is additive — no existing public API surface
is modified.

---

## 10. File Change Summary

| File | Action | Notes |
|---|---|---|
| `Cargo.toml` | Modify | Add `crates/mempalace-core` to `members`; add `fastembed`, `walkdir`, `regex`, `hex` to `[workspace.dependencies]` |
| `crates/mempalace-core/` | Add | Vendored from `mempalace-rs/crates/mempalace-core`; `sha2` version updated to `0.11` |
| `crates/surreal-memory/Cargo.toml` | Modify | Add `mempalace-core` dep; add `palace` feature |
| `crates/surreal-memory/src/storage/mod.rs` | Modify | Add palace methods to `MemoryStorage` trait (feature-gated) |
| `crates/surreal-memory/src/storage/surreal.rs` | Modify | Add `db_handle()` method; impl palace methods on `SurrealStorage` |
| `crates/surreal-memory/src/storage/migrations/mod.rs` | Modify | Register `v7_palace.surql` |
| `crates/surreal-memory/src/storage/migrations/v7_palace.surql` | Add | `drawers` table + HNSW + BM25 + hash index |
| `src/palace/mod.rs` | Add | Module root |
| `src/palace/adapter.rs` | Add | `PalaceAdapter` implementing `StorageBackend` |
| `src/palace/context.rs` | Add | `PalaceContext` wrapping `MemoryStack` |
| `src/palace/embedding.rs` | Add | `FastEmbedService` implementing `EmbeddingService` |
| `src/embeddings/mod.rs` | Modify | Add `EmbeddingProvider::Fast` variant; wire `FastEmbedService` |
| `src/config.rs` | Modify | Add `PALACE_*` env vars; change local-embedding default path |
| `src/api/mod.rs` | Modify | Register palace router |
| `src/api/palace.rs` | Add | 6 REST handlers |
| `src/mcp/handlers.rs` | Modify | Register 6 palace MCP tools |
| `src/main.rs` | Modify | Init `PalaceContext`; add to app state |

---

## 11. Phase Sequencing and Priorities

```
Phase 1  FastEmbedService          ← highest priority; fixes cold-start regression
Phase 2  PalaceAdapter             ← required for all downstream phases
Phase 3  MemoryStack integration   ← required for REST + MCP
Phase 4  REST + MCP tools          ← UAR and The Boss can consume palace from here
Phase 5  Dialect compression       ← quality improvement, not blocking
Phase 6  RRF hybrid reranking      ← highest value, most complex; do last
```

Phases 1–4 can ship as a single PR. Phases 5 and 6 are independent follow-on PRs.

The palace feature flag (`cargo build --features palace`) means the existing
release binary is unaffected until explicitly opted in.
