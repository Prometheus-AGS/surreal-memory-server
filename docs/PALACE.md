# Memory Palace Architecture

The **Memory Palace** is an opt-in spatial memory organization system integrated into surreal-memory, inspired by the ancient [method of loci](https://en.wikipedia.org/wiki/Method_of_loci) mnemonic technique. It is powered by [mempalace-rs](https://github.com/jxoesneon/mempalace-rs), a high-performance Rust implementation of the MemPalace AI memory system.

**Feature flag:** `palace`

```bash
cargo build --release --features palace          # with default embedded DB
cargo build --release --features server-only,palace  # server mode (Docker default)
```

---

## Motivation

The existing surreal-memory system provides flat scoped memory (user/agent/session), a knowledge graph, and task streams. The Memory Palace adds a complementary organizational layer:

1. **Spatial taxonomy** — Wing/Room/Hall hierarchy for thematic organization of knowledge
2. **4-layer memory stack** — L0-L3 hierarchy balancing instant identity recall with deep semantic search
3. **Lightweight embeddings** — 384-dim FastEmbed (all-MiniLM-L6-v2) that works offline without API keys
4. **Cross-domain hybrid search** — Reciprocal Rank Fusion (RRF) merges results from memory, entity, and palace tables in a single query
5. **Token compression** — AAAK Dialect reduces context by up to 30x for LLM prompt injection

---

## Conceptual Model

The Memory Palace uses a spatial metaphor to organize knowledge:

```
Palace (your entire knowledge store)
├── Wing: "engineering"              (thematic collection)
│   ├── Room: "databases"            (subdivision)
│   │   ├── Hall: "indexing"         (navigation pathway)
│   │   │   └── Drawer: "HNSW uses MTREE in SurrealDB v3"  (individual memory)
│   │   │   └── Drawer: "BM25 full-text search analyzer"
│   │   └── Hall: "migrations"
│   │       └── Drawer: "Always use IF NOT EXISTS in DDL"
│   └── Room: "rust"
│       └── Hall: "async"
│           └── Drawer: "tokio::sync::OnceCell for lazy init"
├── Wing: "product"
│   └── Room: "roadmap"
│       └── Drawer: "Q2 focus: memory palace integration"
└── Wing: "personal"
    └── Room: "preferences"
        └── Drawer: "Prefers terse responses, no trailing summaries"
```

### 4-Layer MemoryStack

| Layer | Name | Token Budget | Purpose |
|-------|------|-------------|---------|
| **L0** | Identity | ~100 tokens | Core persona — who the agent is |
| **L1** | Essential | ~500-800 tokens | Recency-biased recent events |
| **L2** | On-Demand | Variable | Similarity-searched context, retrieved as needed |
| **L3** | Search | Variable | Full semantic search over all drawers |

The layers are accessed through dedicated operations:
- `palace_wake_up` returns L0 + L1 (identity + essential story)
- `palace_recall` returns L2 (on-demand similarity recall)
- `palace_search` returns L3 (deep semantic search)

---

## Architecture

### Integration Design

The palace integration is fully additive — the existing `MemoryStorage` trait is untouched. A separate `PalaceStorage` trait defines the palace API surface:

```
┌─────────────────────────────────────────────────────────────┐
│                     SurrealStorage                           │
│                                                              │
│  ┌─────────────────────┐    ┌─────────────────────────────┐  │
│  │   MemoryStorage     │    │      PalaceStorage          │  │
│  │   (35+ methods)     │    │      (8 methods)            │  │
│  │   Always available  │    │      palace feature only     │  │
│  └─────────┬───────────┘    └──────────┬──────────────────┘  │
│            │                           │                      │
│            │                  ┌────────▼──────────┐          │
│            │                  │   PalaceContext    │          │
│            │                  │   (OnceCell lazy)  │          │
│            │                  │                    │          │
│            │                  │  ┌──────────────┐  │          │
│            │                  │  │ MemoryStack  │  │          │
│            │                  │  │ (L0-L3)      │  │          │
│            │                  │  └──────────────┘  │          │
│            │                  │  ┌──────────────┐  │          │
│            │                  │  │PalaceAdapter │  │          │
│            │                  │  │(StorageBack- │  │          │
│            │                  │  │ end impl)    │  │          │
│            │                  │  └──────────────┘  │          │
│            │                  │  ┌──────────────┐  │          │
│            │                  │  │   Dialect    │  │          │
│            │                  │  │ (AAAK comp.) │  │          │
│            │                  │  └──────────────┘  │          │
│            │                  └────────────────────┘          │
│            │                           │                      │
│  ┌─────────▼───────────────────────────▼──────────────────┐  │
│  │              Shared Surreal<Any> connection              │  │
│  └─────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

Key design decisions:
- **Separate `PalaceStorage` trait** — no breaking change to `MemoryStorage`, clean opt-in for consumers
- **`tokio::sync::OnceCell`** — lazy initialization on first palace call, zero cost when unused
- **Shared DB connection** — `PalaceAdapter` reuses the existing `Surreal<Any>` handle (Arc-wrapped, cheap to clone)
- **384-dim independent vector space** — palace drawers use `all-MiniLM-L6-v2` (384d), separate from the memory table's 1536d HNSW index

### Module Structure

```
crates/surreal-memory/src/palace/
├── mod.rs          # PalaceStorage trait, PalaceStatus, UnifiedHit, HitSource
├── adapter.rs      # PalaceAdapter: mempalace_core::StorageBackend over SurrealDB
├── context.rs      # PalaceContext: MemoryStack + PalaceAdapter + Dialect facade
└── embedding.rs    # FastEmbedService: 384-dim embedder bridging to EmbeddingService
```

### Database Schema (Migration v16)

The `drawers` table is SCHEMAFULL with the following structure:

| Field | Type | Notes |
|-------|------|-------|
| `content` | `string` | The stored text content |
| `wing` | `string` | Thematic collection (e.g., "engineering") |
| `room` | `string` | Subdivision (e.g., "databases") |
| `hall` | `string` | Navigation pathway (default: "general") |
| `source_file` | `option<string>` | Origin file path if mined from codebase |
| `date` | `option<string>` | Temporal marker |
| `importance` | `float` | Importance score (default: 1.0) |
| `embedding` | `option<array<float>>` | 384-dim FastEmbed vector |
| `content_hash` | `option<string>` | SHA-256 for deduplication |

**Indexes:**
- `drawer_embedding_idx` — MTREE HNSW, 384 dimensions, COSINE distance
- `drawer_content_idx` — BM25 full-text search (unicode analyzer)
- `drawer_hash_idx` — Content hash for dedup checks
- `drawer_wing_idx` — Wing taxonomy lookup
- `drawer_room_idx` — Room taxonomy lookup

---

## PalaceStorage Trait

```rust
#[async_trait]
pub trait PalaceStorage: Send + Sync {
    /// Wake-up context (L0 identity + L1 essential story)
    async fn palace_wake_up(&self, wing: Option<&str>) -> Result<String>;

    /// On-demand recall (L2) filtered by wing/room
    async fn palace_recall(&self, wing: Option<&str>, room: Option<&str>, limit: usize) -> Result<String>;

    /// Deep search (L3) — full semantic search over drawers
    async fn palace_search(&self, query: &str, wing: Option<&str>, room: Option<&str>, n: usize) -> Result<String>;

    /// Ingest content into the palace. Returns drawer ID.
    async fn palace_ingest(&self, content: &str, wing: &str, room: &str, hall: &str, importance: f32) -> Result<String>;

    /// Delete a drawer by ID
    async fn palace_delete(&self, id: &str) -> Result<()>;

    /// Palace status summary (drawer/wing/room counts)
    async fn palace_status(&self) -> Result<PalaceStatus>;

    /// Compress text using AAAK Dialect (~30x token reduction)
    fn palace_compress(&self, text: &str) -> String;

    /// Hybrid search across memory + entity + palace via RRF
    async fn palace_hybrid_search(&self, query: &str, scope: Option<MemoryScope>, wing: Option<&str>, n: usize) -> Result<Vec<UnifiedHit>>;
}
```

---

## RRF Hybrid Search

The `palace_hybrid_search` operation is the most powerful search primitive in the system. It concurrently queries three independent search domains and merges results using Reciprocal Rank Fusion:

```
                    ┌─────────────────────────┐
                    │    palace_hybrid_search  │
                    │    query: "vector index" │
                    └────────┬────────────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
     ┌────────────┐  ┌────────────┐  ┌────────────┐
     │  memory    │  │  entity    │  │  drawers   │
     │  BM25+HNSW│  │  HNSW     │  │  HNSW     │
     │  1536-dim  │  │  1536-dim  │  │  384-dim   │
     └─────┬──────┘  └─────┬──────┘  └─────┬──────┘
           │               │               │
           └───────────────┼───────────────┘
                           ▼
                  ┌────────────────┐
                  │  RRF Merge     │
                  │  k=60          │
                  │  rank-based    │
                  └───────┬────────┘
                          ▼
                  Vec<UnifiedHit>
                  (source-tagged)
```

RRF uses **rank-based** scoring (not score-based), so the incomparable cosine similarities from different embedding dimensions (384d vs 1536d) do not affect correctness. Each result is tagged with its `HitSource` so consumers know where each hit originated:

```rust
pub enum HitSource {
    Memory { scope: MemoryScope, memory_type: MemoryType },
    Entity { entity_type: String },
    Palace { wing: String, room: String },
}
```

---

## MCP Tools

| Tool | Arguments | Returns |
|------|-----------|---------|
| `palace_wake_up` | `wing?: string`, `compress?: bool` | Context string (L0+L1) |
| `palace_recall` | `wing?: string`, `room?: string`, `limit?: u32`, `compress?: bool` | Context string (L2) |
| `palace_search` | `query: string`, `wing?: string`, `room?: string`, `n?: u32`, `compress?: bool` | Context string (L3) |
| `palace_ingest` | `content: string`, `wing: string`, `room: string`, `hall?: string`, `importance?: f32` | `{ "id": "..." }` |
| `palace_delete` | `id: string` | `{ "deleted": true }` |
| `palace_status` | _(none)_ | `PalaceStatus` JSON |
| `palace_hybrid_search` | `query: string`, `scope?: string`, `wing?: string`, `n?: u32`, `compress?: bool` | `Vec<UnifiedHit>` JSON |

**Defaults:** `hall="general"`, `importance=1.0`, `limit=20`, `n=10`

When the `palace` feature is disabled, all 7 tools still appear in the MCP schema but return a helpful error message indicating the feature is not enabled.

---

## REST Endpoints

| Method | Path | Maps to |
|--------|------|---------|
| `GET` | `/api/v1/palace/wake` | `palace_wake_up(?wing=&compress=)` |
| `GET` | `/api/v1/palace/recall` | `palace_recall(?wing=&room=&limit=&compress=)` |
| `POST` | `/api/v1/palace/search` | `palace_search({ query, wing?, room?, n?, compress? })` |
| `POST` | `/api/v1/palace/ingest` | `palace_ingest({ content, wing, room, hall?, importance? })` |
| `DELETE` | `/api/v1/palace/drawer/:id` | `palace_delete(id)` |
| `GET` | `/api/v1/palace/status` | `palace_status()` |
| `POST` | `/api/v1/palace/hybrid-search` | `palace_hybrid_search({ query, scope?, wing?, n?, compress? })` |

All endpoints are behind `#[cfg(feature = "palace")]` and only registered when the feature is enabled.

---

## Usage Examples

### Organizing Agent Knowledge

```bash
# Create a knowledge taxonomy for an engineering agent
curl -X POST http://localhost:3001/api/v1/palace/ingest \
  -H 'Content-Type: application/json' \
  -d '{
    "content": "When deploying to Kubernetes, always set resource limits and use rolling updates",
    "wing": "devops",
    "room": "kubernetes",
    "hall": "best-practices",
    "importance": 0.85
  }'

curl -X POST http://localhost:3001/api/v1/palace/ingest \
  -H 'Content-Type: application/json' \
  -d '{
    "content": "PostgreSQL connection pooling with PgBouncer reduces connection overhead by 10x",
    "wing": "engineering",
    "room": "databases",
    "hall": "performance",
    "importance": 0.9
  }'
```

### Agent Context Loading

```bash
# At session start: load identity + recent context
curl 'http://localhost:3001/api/v1/palace/wake?compress=true'

# Mid-conversation: recall relevant context from a specific domain
curl 'http://localhost:3001/api/v1/palace/recall?wing=engineering&room=databases&limit=5'

# Deep search when the agent needs specific knowledge
curl -X POST http://localhost:3001/api/v1/palace/search \
  -H 'Content-Type: application/json' \
  -d '{"query": "connection pooling strategies", "wing": "engineering", "n": 10}'
```

### Cross-Domain Search

```bash
# Search across memory table, knowledge graph, AND palace drawers
curl -X POST http://localhost:3001/api/v1/palace/hybrid-search \
  -H 'Content-Type: application/json' \
  -d '{"query": "database performance optimization", "n": 10}'

# Response includes source-tagged results:
# [
#   { "id": "...", "content": "...", "source": { "type": "palace", "wing": "engineering", "room": "databases" }, "score": 0.032 },
#   { "id": "...", "content": "...", "source": { "type": "memory", "scope": "Agent", "memory_type": "Procedural" }, "score": 0.029 },
#   { "id": "...", "content": "...", "source": { "type": "entity", "entity_type": "Technology" }, "score": 0.025 }
# ]
```

---

## Dependency: mempalace-core

The palace feature depends on [mempalace-core](https://github.com/GQAdonis/mempalace-rs), a fork of the [mempalace-rs](https://github.com/jxoesneon/mempalace-rs) project. It provides:

| Component | What it does |
|-----------|-------------|
| `MemoryStack` | 4-layer memory abstraction (L0-L3) |
| `StorageBackend` | Trait for pluggable storage (9 methods) |
| `FastEmbedder` | 384-dim all-MiniLM-L6-v2 embedder (~25-80 MB model download on first use) |
| `Dialect` | AAAK regex-based text compression |
| `ReciprocRankFusion` | Rank fusion reranker for merging heterogeneous search results |
| `Drawer`, `DrawerHit` | Core data types for storage and search |

The dependency is declared as a git dependency:

```toml
[workspace.dependencies]
mempalace-core = { git = "https://github.com/GQAdonis/mempalace-rs.git", branch = "main" }
```

### First-Run Model Download

On the first call to any palace operation, FastEmbedder downloads the `all-MiniLM-L6-v2` model (~25-80 MB). This is a one-time download cached locally. Subsequent starts reuse the cached model.

---

## Performance Characteristics

- **Embedding dimensions:** 384 (vs 1536 for memory/entity tables)
- **Model:** all-MiniLM-L6-v2 (offline, no API key required)
- **Cold start:** ~300ms for model loading (first call only, OnceCell cached)
- **AAAK compression:** ~1,788 ops/sec
- **Deduplication:** SHA-256 content hashing prevents duplicate drawers
- **RRF merge:** O(n log n) rank fusion across three result lists

---

## Relationship to Existing Memory System

The palace is complementary, not a replacement:

| Aspect | Scoped Memory | Knowledge Graph | Memory Palace |
|--------|--------------|-----------------|---------------|
| **Organization** | Flat (scope + type) | Graph (entity + relation) | Spatial (wing/room/hall) |
| **Search** | BM25 + HNSW (1536d) | HNSW (1536d) | HNSW (384d) + BM25 |
| **Best for** | Conversation context, facts | Entity relationships, Graph-RAG | Thematic knowledge, agent identity |
| **Embedding** | Configurable provider | Same as memory | Always FastEmbed (384d) |
| **Compression** | None | None | AAAK Dialect (opt-in) |

Use `palace_hybrid_search` to search across all three simultaneously with automatic rank fusion.
