# MemPalace Integration Design

**Date**: 2026-04-10
**Status**: Draft
**Supersedes**: `docs/MEMPALACE.md` (original plan, now stale on details)

## Overview

Integrate `mempalace-core`'s 4-layer memory model into the `surreal-memory` library crate, sharing the existing SurrealDB connection. The integration is feature-gated behind `palace`, adds a separate `PalaceStorage` trait (leaving `MemoryStorage` untouched), and exposes 7 MCP tools + 7 REST endpoints + RRF hybrid reranking across all three search domains.

## Goals

1. Add spatial memory organization (wing/room/hall taxonomy) to surreal-memory
2. Provide a lightweight 384-dimensional embedding path (FastEmbedder) that works without API keys
3. Enable cross-domain hybrid search via Reciprocal Rank Fusion across memory, entity, and drawer tables
4. Keep the integration fully additive — zero impact when the `palace` feature is disabled

## Non-Goals

- Modifying the `MemoryStorage` trait
- Vendoring mempalace-rs source code
- Global always-on compression
- Upstream changes to mempalace-rs

## Dependency Structure

### Git dependency (not vendored)

```toml
# Workspace root Cargo.toml
[workspace.dependencies]
mempalace-core = { git = "https://github.com/GQAdonis/mempalace-rs.git", branch = "main" }

# crates/surreal-memory/Cargo.toml
[dependencies]
mempalace-core = { workspace = true, optional = true }

[features]
palace = ["dep:mempalace-core"]

# Root binary Cargo.toml
[features]
palace = ["surreal-memory/palace"]

[dependencies]
mempalace-core = { workspace = true, optional = true }
```

Cargo resolves `sha2` (0.10 vs 0.11) and `dirs` (5 vs 6) version mismatches automatically — both are `0.x` semver-incompatible, so they compile as separate versions. If this causes issues, open a PR upstream to bump.

Updates to mempalace-rs `main` are picked up via `cargo update -p mempalace-core`. For deterministic builds, pin to `rev = "..."` after initial integration stabilizes.

## DB Handle Exposure

Add to `SurrealStorage` in `crates/surreal-memory/src/storage/surreal.rs`:

```rust
/// Clone the live Surreal<Any> handle for shared use by subsystems.
/// Surreal<Any> is internally Arc-wrapped — cloning is cheap.
pub fn db(&self) -> Result<Surreal<Any>> {
    // Same pattern as health_check() — match on ConnectionState
}
```

The `PalaceAdapter` calls `db()` per-operation so it inherits reconnection state transparently.

## Migration v16 — Drawers Table

Added to `crates/surreal-memory/src/storage/migrations/mod.rs`:

```sql
-- v16: MemPalace drawers table — wing/room/hall taxonomy
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

-- HNSW vector index for L3 semantic search (384d — all-MiniLM-L6-v2)
DEFINE INDEX IF NOT EXISTS drawer_embedding_idx
  ON drawers FIELDS embedding
  MTREE DIMENSION 384 DIST COSINE;

-- Full-text index for keyword recall
DEFINE INDEX IF NOT EXISTS drawer_content_idx
  ON drawers FIELDS content SEARCH ANALYZER unicode BM25;

-- Content-hash index for dedup checks
DEFINE INDEX IF NOT EXISTS drawer_hash_idx
  ON drawers FIELDS content_hash;

-- Taxonomy lookup indexes
DEFINE INDEX IF NOT EXISTS drawer_wing_idx ON drawers FIELDS wing;
DEFINE INDEX IF NOT EXISTS drawer_room_idx ON drawers FIELDS room;
```

Key decisions:
- **384 dimensions** match `all-MiniLM-L6-v2` from FastEmbedder. Independent from the memory table's 1536d HNSW index.
- **SCHEMAFULL** consistent with all other tables.
- **`embedding.*` element type** required by SurrealDB for SCHEMAFULL array-of-float fields.
- **Wing/room indexes** added for taxonomy aggregation queries (`list_wings`, `list_rooms`).

## Palace Module Structure

All palace code in `crates/surreal-memory/src/palace/`, gated behind `#[cfg(feature = "palace")]`.

```
crates/surreal-memory/src/
  palace/
    mod.rs          — module root, re-exports, PalaceStorage trait, UnifiedHit
    adapter.rs      — PalaceAdapter: StorageBackend over shared Surreal<Any>
    context.rs      — PalaceContext: wraps MemoryStack, exposes high-level API
    embedding.rs    — FastEmbedService: bridges FastEmbedder → EmbeddingService
    tests.rs        — unit + trait tests
```

### PalaceAdapter

Implements `mempalace_core::storage::StorageBackend` by executing SurrealQL against the shared connection.

```rust
pub struct PalaceAdapter {
    db_fn: Arc<dyn Fn() -> Result<Surreal<Any>> + Send + Sync>,
}
```

Each operation calls `db_fn()` fresh so reconnections are respected. The 9 `StorageBackend` methods map to straightforward SurrealQL:

| Method | SurrealQL |
|---|---|
| `add_drawer` | `CREATE drawers CONTENT { ... }` |
| `delete_drawer` | `DELETE drawers:⟨id⟩` |
| `get_drawer` | `SELECT * FROM drawers:⟨id⟩` |
| `list_drawers` | `SELECT * FROM drawers WHERE wing=$wing AND room=$room LIMIT $n` |
| `search_drawers` | `SELECT *, vector::similarity::cosine(...) AS score FROM drawers WHERE embedding <\|$n,384\|> $qvec` |
| `check_duplicate` | `SELECT count() FROM drawers WHERE content_hash=$hash` |
| `list_wings` | `SELECT wing, count() AS count FROM drawers GROUP BY wing` |
| `list_rooms` | `SELECT room, wing, count() AS count FROM drawers GROUP BY room, wing` |
| `drawer_count` | `SELECT count() FROM drawers` |

### FastEmbedService

Bridges `mempalace_core::embedder::FastEmbedder` (384d, all-MiniLM-L6-v2) to `crate::embeddings::EmbeddingService`:

```rust
pub struct FastEmbedService {
    inner: Arc<dyn mempalace_core::embedder::Embedder>,
}

impl EmbeddingService for FastEmbedService {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> { ... }
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> { ... }
    fn dimensions(&self) -> usize { 384 }
}
```

### EmbeddingProvider::Fast variant

Added to `crates/surreal-memory/src/embeddings/mod.rs` behind feature gate:

```rust
#[cfg(feature = "palace")]
Fast,
```

Wired into `create_embedding_service()` to construct `FastEmbedService`.

### PalaceContext

Composes `MemoryStack` + `PalaceAdapter` + embedding:

```rust
pub struct PalaceContext {
    stack: mempalace_core::layers::MemoryStack,
    adapter: Arc<PalaceAdapter>,
    dialect: mempalace_core::dialect::Dialect,
}

impl PalaceContext {
    pub async fn from_storage(storage: &SurrealStorage) -> Result<Self> { ... }
    pub async fn wake_up(&self, wing: Option<&str>) -> Result<String> { ... }
    pub async fn recall(&self, wing: Option<&str>, room: Option<&str>, limit: usize) -> Result<String> { ... }
    pub async fn search(&self, query: &str, wing: Option<&str>, room: Option<&str>, n: usize) -> Result<String> { ... }
    pub async fn status(&self) -> Result<StackStatus> { ... }
    pub async fn ingest(&self, content: &str, wing: &str, room: &str, hall: &str, importance: f32) -> Result<String> { ... }
    pub async fn delete(&self, id: &str) -> Result<()> { ... }
    pub fn compress(&self, text: &str) -> String { ... }
}
```

## PalaceStorage Trait (separate from MemoryStorage)

```rust
#[cfg(feature = "palace")]
#[async_trait]
pub trait PalaceStorage: Send + Sync {
    async fn palace_wake_up(&self, wing: Option<&str>) -> Result<String>;
    async fn palace_recall(&self, wing: Option<&str>, room: Option<&str>, limit: usize) -> Result<String>;
    async fn palace_search(&self, query: &str, wing: Option<&str>, room: Option<&str>, n: usize) -> Result<String>;
    async fn palace_ingest(&self, content: &str, wing: &str, room: &str, hall: &str, importance: f32) -> Result<String>;
    async fn palace_delete(&self, id: &str) -> Result<()>;
    async fn palace_status(&self) -> Result<PalaceStatus>;
    fn palace_compress(&self, text: &str) -> String;
    async fn palace_hybrid_search(&self, query: &str, scope: Option<MemoryScope>, wing: Option<&str>, n: usize) -> Result<Vec<UnifiedHit>>;
}
```

`SurrealStorage` implements this using a `tokio::sync::OnceCell<PalaceContext>` for lazy initialization:

```rust
pub struct SurrealStorage {
    // existing fields...
    #[cfg(feature = "palace")]
    palace: tokio::sync::OnceCell<PalaceContext>,
}
```

### Benefits of separate trait

- **`MemoryStorage` untouched** — no breaking change, no conditional compilation on the core trait
- **UAR opt-in is clean** — `use surreal_memory::PalaceStorage` when needed
- **Independently testable and mockable**
- **`PalaceContext` lifecycle is internal** — `OnceCell` initializes on first use

## RRF Hybrid Reranking

`palace_hybrid_search` concurrently executes three search paths:

1. `hybrid_search_memories(query, scope, n)` — BM25+HNSW on `memory` table
2. `semantic_search(query, n)` — vector search on `entity` table
3. `palace_search(query, wing, None, n)` — L3 on `drawers` table

Results are converted to `Vec<DrawerHit>`, fused via `mempalace_core::reranker::ReciprocRankFusion::new(60).merge(lists)`, then returned as `Vec<UnifiedHit>`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedHit {
    pub id: String,
    pub content: String,
    pub source: HitSource,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HitSource {
    Memory { scope: MemoryScope, memory_type: MemoryType },
    Entity { entity_type: String },
    Palace { wing: String, room: String },
}
```

RRF uses rank fusion (not score fusion), so the incomparable 384d vs 1536d cosine scores do not affect correctness.

## Dialect Compression

Available via `palace_compress()` and `?compress=true` on REST endpoints. AAAK Dialect is a pure in-memory transformer (regex-based, zero dependencies beyond `regex`). Applied opt-in per-request only — no global env var.

## MCP Tools (7 new)

| Tool | Args | Returns |
|---|---|---|
| `palace_wake_up` | `wing?: string, compress?: bool` | Context string (L0+L1) |
| `palace_recall` | `wing?: string, room?: string, limit?: u32, compress?: bool` | Context string (L2) |
| `palace_search` | `query: string, wing?: string, room?: string, n?: u32, compress?: bool` | Context string (L3) |
| `palace_ingest` | `content: string, wing: string, room: string, hall?: string, importance?: f32` | `{ "id": "..." }` |
| `palace_delete` | `id: string` | `{ "deleted": true }` |
| `palace_status` | _(none)_ | `PalaceStatus` JSON |
| `palace_hybrid_search` | `query: string, scope?: string, wing?: string, n?: u32, compress?: bool` | `Vec<UnifiedHit>` JSON |

Defaults: `hall="general"`, `importance=1.0`, `limit=20`, `n=10`.

## REST Endpoints

| Method | Path | Maps to |
|---|---|---|
| `GET` | `/api/v1/palace/wake` | `palace_wake_up(?wing=&compress=)` |
| `GET` | `/api/v1/palace/recall` | `palace_recall(?wing=&room=&limit=&compress=)` |
| `POST` | `/api/v1/palace/search` | `palace_search({ query, wing?, room?, n?, compress? })` |
| `POST` | `/api/v1/palace/ingest` | `palace_ingest({ content, wing, room, hall?, importance? })` |
| `DELETE` | `/api/v1/palace/drawer/:id` | `palace_delete(id)` |
| `GET` | `/api/v1/palace/status` | `palace_status()` |
| `POST` | `/api/v1/palace/hybrid-search` | `palace_hybrid_search({ query, scope?, wing?, n?, compress? })` |

All behind `#[cfg(feature = "palace")]` at the binary level.

## Testing Strategy

### Unit tests (`crates/surreal-memory/src/palace/tests.rs`)

Behind `#[cfg(all(test, feature = "palace"))]`:

- **FastEmbedService**: dimensionality, batch consistency, NoOp variant
- **PalaceAdapter** (require embedded SurrealDB): CRUD round-trip, cosine search, dedup, taxonomy aggregation
- **PalaceContext**: ingest→recall, ingest→search, wake_up, compress
- **PalaceStorage trait on SurrealStorage**: trait dispatch, OnceCell lazy init

### Integration test (`tests/palace_integration.rs`)

Full flow: embedded SurrealDB → migration v16 → write → search → hybrid-search → compress.

### CI

```yaml
- name: Test (palace)
  run: cargo test --features embedded,palace
```

### Not tested

- mempalace-core internals (upstream responsibility)
- MCP tool JSON schema (covered by existing harness)
- REST serialization (covered by axum patterns)

## Files Touched

| Action | Path |
|---|---|
| Modify | `Cargo.toml` (workspace dep) |
| Modify | `crates/surreal-memory/Cargo.toml` (optional dep + feature) |
| Modify | `crates/surreal-memory/src/lib.rs` (conditional module + re-exports) |
| Modify | `crates/surreal-memory/src/storage/surreal.rs` (`db()`, `OnceCell`, `PalaceStorage` impl) |
| Modify | `crates/surreal-memory/src/storage/migrations/mod.rs` (v16) |
| Modify | `crates/surreal-memory/src/embeddings/mod.rs` (`Fast` variant) |
| Create | `crates/surreal-memory/src/palace/mod.rs` |
| Create | `crates/surreal-memory/src/palace/adapter.rs` |
| Create | `crates/surreal-memory/src/palace/context.rs` |
| Create | `crates/surreal-memory/src/palace/embedding.rs` |
| Create | `crates/surreal-memory/src/palace/tests.rs` |
| Modify | `src/main.rs` (palace feature gate, router registration) |
| Modify | `src/mcp/mod.rs` (7 tool definitions) |
| Modify | `src/mcp/handlers.rs` (7 handler implementations) |
| Create | `src/api/palace.rs` (REST endpoints) |
| Create | `tests/palace_integration.rs` |
| Modify | CI config (add palace test job) |

## Risk Items

1. **`mempalace-core` API stability** — `branch = "main"` means upstream breaks propagate. Mitigation: pin to `rev = "..."` after initial integration works.
2. **FastEmbedder model download on first use** — ~80MB `all-MiniLM-L6-v2` downloaded on first `PalaceContext` init. Mitigation: `OnceCell` means once per process; documented in MCP instructions.
3. **384d vs 1536d vector spaces** — drawers and memories use different embedding models, cosine scores not comparable. Mitigation: RRF uses rank fusion, not score fusion.
