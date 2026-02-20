# Evolution Plan — Iteration 2: Goal-Anchored Roadmap
**Evolution Name:** `surreal-memory-server-baseline`
**Based On:** [Baseline Assessment — 2026-02-20T07:40](assessment-2026-02-20T0740.md)
**Date:** 2026-02-20T07:58:01-06:00
**Iteration:** 2 (Goal-Anchored Planning)
**PMPO Phase:** Analyze + Plan (user-defined goals)

---

## The 4 Goals

| # | Goal | Priority | Key Grounding |
|---|------|----------|---------------|
| G1 | **Crate library for embedding memory** | P0 — blocks G2-G4 | UAR has `Memory` domain struct + `PersistenceLayer::save_memory/search_memory` stub today |
| G2 | **A2A interface + REST API + HTTP/SSE MCP transport** | P1 | A2A v0.3: HTTP/SSE/JSON-RPC, Agent Cards, Task lifecycle |
| G3 | **50% competitive parity (by reputation/use)** | P1 | Graphiti, mem0, Basic Memory, Cognee are the benchmark set |
| G4 | **Long-running named task memory + context management** | P0 | UAR has tiktoken-rs; agents have varying context budgets |

---

## Goal 1 Deep-Dive: Crate Library for Embeddable Memory

### What UAR Needs Today

The UAR's `persistence/mod.rs` already defines the interface stubs:
```rust
async fn save_memory(&self, memory: &Memory) -> Result<()>;
async fn search_memory(agent_id: Option<&str>, query_vec: &[f32], limit: usize, min_score: f32) -> Result<Vec<MemoryMatch>>;
```

The `Memory` struct has `agent_id: Option<String>` (None = Global), but is missing:
- `user_id` — required for user-scoped memory
- `session_id` — required for session-scoped memory  
- `scope` — `User | Agent | Session | Global` discriminant
- `stream_name` / `task_name` — required for Goal 4 (named task memory)
- Temporal fields (`valid_until`, TTL)
- Token count metadata (required for Goal 4 context management)

### Required Cargo Workspace Structure

The project should become a **Cargo workspace** with two publishable crates:

```
surreal-memory-server/
├── Cargo.toml              ← workspace root
├── crates/
│   ├── surreal-memory/     ← NEW: library crate (the embeddable core)
│   │   ├── Cargo.toml      ← publishes to crates.io
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── memory.rs       ← Memory, MemoryScope, MemoryStore trait
│   │       ├── entity.rs       ← Entity, Relation (knowledge graph)
│   │       ├── storage/        ← SurrealStorage impl
│   │       ├── embeddings/     ← EmbeddingService trait + providers
│   │       └── context/        ← NEW: ContextBudget, TokenCounter, chunking
│   └── surreal-memory-server/  ← binary crate (MCP server + A2A + REST)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── mcp/            ← stdio + HTTP/SSE MCP transports
│           ├── a2a/            ← NEW: A2A agent card + task handlers
│           └── api/            ← NEW: REST API routes
```

### Memory Scope Model

```rust
pub enum MemoryScope {
    Global,                          // shared across all agents/users
    Agent { agent_id: String },      // per-agent persistent memory
    User  { user_id: String },       // per-user across agents
    Session {                        // ephemeral, tied to conversation
        session_id: String,
        user_id: Option<String>,
        agent_id: Option<String>,
    },
    Task {                           // long-running named task (Goal 4)
        task_name: String,
        agent_id: Option<String>,
        user_id: Option<String>,
    },
}
```

### How UAR Integrates

```toml
# In UAR Cargo.toml — replace the stub PersistenceLayer memory methods
surreal-memory = { path = "../surreal-memory-server/crates/surreal-memory", features = ["embedded"] }
```

The UAR's existing `PersistenceLayer` trait gets a blanket implementation via `surreal-memory`'s `MemoryStore` trait. The `Memory` domain struct in UAR gets aligned with the library's expanded struct.

---

## Goal 2 Deep-Dive: A2A + REST API + HTTP/SSE Transports

### A2A Protocol Requirements (v0.3, Google)

A2A adds agent-to-agent collaboration on top of MCP's tool-calling model. Key components needed:

| Component | What It Is | Implementation |
|---|---|---|
| **Agent Card** | `/agent.json` — discovery endpoint advertising capabilities, skills, auth | Static JSON served via Axum GET route |
| **Task endpoint** | `POST /a2a/tasks/send` — create a memory task request | Axum handler |
| **Task streaming** | `GET /a2a/tasks/{id}/events` — SSE stream of task progress | Axum SSE |
| **Task status** | `GET /a2a/tasks/{id}` — check status | Axum handler |
| **Authentication** | API key or OAuth2 Bearer declared in Agent Card | Tower middleware |

### Transport Strategy

```
Clients
  │
  ├── stdio ──────── [MCP Server — existing]
  ├── HTTP/SSE ───── [MCP Server — new, rmcp streamable-http feature]  
  ├── REST ────────── [Axum REST API — CRUD on memories/entities/relations]
  └── A2A ─────────── [Axum A2A endpoints — Agent Card + Task lifecycle]
```

The existing `rmcp` crate in UAR already enables `transport-streamable-http-client-reqwest`. Adding the server-side `transport-streamable-http` feature to `rmcp` in `surreal-memory-server` unlocks HTTP/SSE MCP with minimal code.

### REST API Surface (Proposed)

```
POST   /api/v1/memory                    ← create memory
GET    /api/v1/memory?scope=agent:xxx    ← list/search memories
DELETE /api/v1/memory/{id}               ← delete

POST   /api/v1/entities                  ← create entity (or batch)
GET    /api/v1/entities/{name}           ← get entity
PATCH  /api/v1/entities/{name}/observations ← add observations
DELETE /api/v1/entities/{name}

POST   /api/v1/relations
DELETE /api/v1/relations

GET    /api/v1/graph                     ← full graph dump
POST   /api/v1/search                    ← semantic + hybrid search

GET    /agent.json                       ← A2A Agent Card
POST   /a2a/tasks/send                   ← A2A task submission
GET    /a2a/tasks/{id}                   ← A2A task status
GET    /a2a/tasks/{id}/events            ← A2A SSE stream
```

---

## Goal 3 Deep-Dive: 50% Competitive Parity

### Ranked Competitors (by reputation + use)

| Rank | Competitor | Stars/Visitors | Key Feature to Match |
|------|------------|----------------|----------------------|
| 1 | **Graphiti** (Zep) | 19k ⭐ | Bi-temporal KG, hybrid search, incremental updates |
| 2 | **mem0** | 107k visitors | Scoped memory (user/agent/session), REST API |
| 3 | **Anthropic Reference** | ~50k users | Batch create_entities/relations, exact API surface |
| 4 | **Basic Memory** | Official | Local-first markdown notes, persistent fuzzy search |
| 5 | **Cognee** | Fast-growing | Graph-RAG, document → knowledge graph ingestion |
| 6 | **RAG Memory** | Niche | pgvector hybrid, 17 MCP tools, full CRUD |

### 50% Parity Target (3 of 6 leaders fully matched)

**Tier 1 — Achieve by Phase 1 (match Anthropic Reference + mem0 core):**
- Batch `create_entities`, `create_relations`, `delete_entities` (match Anthropic Reference)
- Scoped memory: `user_id`, `agent_id`, `session_id`, `task_name` (match mem0)
- REST API (match mem0's API surface)
- Native HNSW vector index (match RAG Memory's pgvector performance)

**Tier 2 — Achieve by Phase 2 (match Graphiti partially):**
- Hybrid search: full-text + semantic (match Graphiti)
- Named task memory + context windowing (unique, exceeds most competitors)
- HTTP/SSE transport (match all cloud-capable competitors)
- Temporal queries: entity history by time range

---

## Goal 4 Deep-Dive: Long-Running Named Task Memory + Context Management

This is the highest-value differentiator — **no current competitor does this well**.

### The Problem

Deep research, functional specs, analytics reports take hours or days. They span:
- Multiple agent calls (different context windows, different models)
- Different LLMs (GPT-4: 128k tokens, Claude: 200k, Gemini 2.0: 1M, etc.)
- Need to retrieve *relevant* prior context without overflowing the window

### The Solution: Task Memory Streams

```rust
pub struct TaskStream {
    pub name: String,          // "project-phoenix-research"
    pub description: String,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    pub created_at: Datetime,
    pub last_active: Datetime,
    pub status: TaskStreamStatus, // Active | Paused | Archived
    pub memory_count: u64,
    pub total_tokens: u64,     // tracked as memories are added
}

pub enum TaskStreamStatus { Active, Paused, Archived }
```

### Context Management Strategy

When a consumer (UAR or any agent) calls `get_context_for_task(task_name, model, max_tokens)`:

```
1. Load all TaskStream memories, ordered by relevance + recency
2. Compute token budget: max_tokens × context_compression_ratio (configurable, default 0.8)
3. Score each memory: relevance_score × recency_weight × importance_weight
4. Fill context window greedily until budget exhausted
5. Return: ContextWindow { memories: Vec<Memory>, tokens_used: u64, memories_omitted: u64 }
```

**Model context profiles** (stored in config or a SurrealDB table):
```yaml
models:
  - name: gpt-4o
    max_tokens: 128000
    reserved_for_response: 8000
  - name: claude-3-5-sonnet
    max_tokens: 200000
    reserved_for_response: 16000
  - name: gemini-2.0-flash
    max_tokens: 1000000
    reserved_for_response: 32000
```

### New SurrealDB Schema for Task Streams

```sql
DEFINE TABLE task_stream SCHEMAFULL;
DEFINE FIELD name ON task_stream TYPE string;
DEFINE FIELD description ON task_stream TYPE string;
DEFINE FIELD agent_id ON task_stream TYPE option<string>;
DEFINE FIELD user_id ON task_stream TYPE option<string>;
DEFINE FIELD status ON task_stream TYPE string;
DEFINE FIELD created_at ON task_stream TYPE datetime;
DEFINE FIELD last_active ON task_stream TYPE datetime;
DEFINE FIELD total_tokens ON task_stream TYPE int;
DEFINE INDEX task_stream_name ON task_stream FIELDS name UNIQUE;

-- Memory gets a task_stream link
DEFINE FIELD task_stream_id ON memory TYPE option<record<task_stream>>;
DEFINE FIELD token_count ON memory TYPE int;         -- tokens in this memory
DEFINE FIELD importance ON memory TYPE float;        -- 0.0-1.0, AI-assigned
DEFINE FIELD scope ON memory TYPE string;            -- global|agent|user|session|task
DEFINE FIELD user_id ON memory TYPE option<string>;
DEFINE FIELD session_id ON memory TYPE option<string>;
```

### New MCP Tools for Task Streams

```
create_task_stream(name, description, agent_id?, user_id?) → TaskStream
get_task_stream(name) → TaskStream
list_task_streams(agent_id?, user_id?) → Vec<TaskStream>
add_to_task_stream(stream_name, content, importance?, tags?) → Memory
get_context_for_task(stream_name, model_name, max_tokens?) → ContextWindow
summarize_task_stream(stream_name) → Summary  (AI-generated rolling summary)
archive_task_stream(name) → ()
```

---

## Prioritized Execution Plan (All 4 Goals)

### Phase 0 — Unblock (SurrealDB 3.0) — ~4 hours
> Must complete before everything else

| Action | Details |
|---|---|
| A0.1 | Add `#[derive(SurrealValue)]` to `Entity` and `Relation` |
| A0.2 | Replace `sql::Thing` → `RecordId`, `sql::Datetime` → `Datetime` |
| A0.3 | Fix `create()`/`take()`/`select()` return type changes |
| A0.4 | `cargo build` passes with zero errors |

---

### Phase 1 — Library + Scoped Memory — ~3 days
> Delivers Goal 1 (library crate) + half of Goal 3 (parity with Anthropic ref + mem0 core)

| Action | Details | Goal |
|---|---|---|
| A1.1 | Extract `surreal-memory` as a Cargo workspace library crate | G1 |
| A1.2 | Expand `Memory` struct: add `scope`, `user_id`, `session_id`, `task_stream_id`, `token_count`, `importance` | G1, G4 |
| A1.3 | Add `TaskStream` entity + SurrealDB schema | G4 |
| A1.4 | Implement `get_context_for_task()` with model-profile-aware token budget | G4 |
| A1.5 | Add batch `create_entities` + `create_relations` MCP tools | G3 |
| A1.6 | Native HNSW vector index via SurrealQL | G3 |
| A1.7 | Write integration test suite (≥15 tests) | G1, G3 |
| A1.8 | UAR integration: update `surreal-memory` dependency path, align `Memory` domain | G1 |

---

### Phase 2 — Transport Layer + A2A + Hybrid Search — ~1 week
> Delivers Goal 2 (A2A + REST + HTTP/SSE) + remaining Goal 3 parity

| Action | Details | Goal |
|---|---|---|
| A2.1 | Add Axum REST API (`/api/v1/memory`, `/api/v1/entities`, `/api/v1/search`) | G2, G3 |
| A2.2 | HTTP/SSE MCP transport (enable `rmcp` `transport-streamable-http` server feature) | G2 |
| A2.3 | A2A Agent Card (`GET /agent.json`) + Task endpoints | G2 |
| A2.4 | Hybrid search: SurrealQL `SEARCH ANALYZER` (BM25) + HNSW vector, scored and merged | G3 |
| A2.5 | Full-text index on entity name + observations + content fields | G3 |
| A2.6 | Temporal entity history: `get_entity_history(name, from, to)` | G3 |
| A2.7 | CI/CD GitHub Actions pipeline | G3 |
| A2.8 | Publish `surreal-memory` to crates.io (alpha) | G1 |

---

### Phase 3 — Advanced Context + Graph-RAG — ~2 weeks
> Reaches 90%+ goal alignment; completes competitive positioning

| Action | Details | Goal |
|---|---|---|
| A3.1 | Rolling summary of task streams (auto-summarize oldest memories when stream exceeds budget) | G4 |
| A3.2 | Model profile registry: store and query context size profiles by model name | G4 |
| A3.3 | Graph traversal tools: `find_path`, `expand_neighbors`, `get_related` | G3 |
| A3.4 | Migrate embedded from RocksDB → SurrealKV (remove C++ dependency) | G1, G3 |
| A3.5 | Multi-tenant namespacing per user/org via SurrealDB namespaces | G2 |
| A3.6 | Benchmark suite + public performance claims | G3 |

---

## Updated Goal Alignment Projections

| Phase Completed | G1 | G2 | G3 | G4 | Overall |
|---|---|---|---|---|---|
| Baseline (now) | 0% | 0% | 25% | 0% | **25%** |
| After Phase 0 | 10% | 0% | 25% | 0% | **35%** |
| After Phase 1 | 75% | 0% | 55% | 70% | **62%** |
| After Phase 2 | 85% | 80% | 80% | 75% | **80%** |
| After Phase 3 | 95% | 90% | 90% | 95% | **93%** ← terminate |

---

## Risk Register

| Risk | Severity | Mitigation |
|---|---|---|
| SurrealDB 3.0 beta API churn between beta.3 and stable | High | Pin to `3.0.0`, test on stable when released |
| Cargo workspace refactor breaks UAR integration | High | Test UAR `cargo check` after every library change |
| A2A v1.0 spec not yet finalized (v0.3 current) | Medium | Implement against v0.3, mark as versioned API |
| Token counting accuracy across models | Medium | Use `tiktoken-rs` (in UAR) as the canonical counter |
| Context window management is complex UX | Medium | Start with simple greedy fill; add smart ranking in Phase 3 |

---

*Plan generated by PMPO Iterative Evolver — Iteration 2. State updated in `.evolver/evolutions/surreal-memory-server-baseline/state.json`.*
