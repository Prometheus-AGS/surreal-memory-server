# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a high-performance Model Context Protocol (MCP) memory server built in Rust, providing semantic search capabilities with multiple embedding providers. It has TWO artifacts:

1. **Binary** (`src/main.rs`) — Standalone MCP server exposing 40+ tools via stdio/HTTP
2. **Library** (`crates/surreal-memory/`) — Embeddable crate consumed by other Rust projects

The library is the more important artifact. It is a direct dependency of:
- **universal-agent-runtime** (UAR) — the full agentic streaming LLM application at `github.com/Prometheus-AGS/universal-agent-runtime`
- **uar-wisc** — a WISC context management CLI binary inside the UAR repo (`src/bin/wisc.rs`)

## CRITICAL: Schema ↔ Struct Sync Rule

**Every field on a Rust struct that gets persisted to SurrealDB MUST have a corresponding DEFINE FIELD in the migrations.**

SurrealDB SCHEMAFULL tables reject any field not defined in the schema. This has caused production bugs:

- `TaskStream.auto_summarize`, `TaskStream.summary_count`, `TaskStream.model_id` were added to the Rust struct but NOT to the v3 migration → `create_task_stream` failed at runtime with "Found field 'auto_summarize', but no such field exists"
- `Memory.metadata` was defined as `option<object>` (strict) → callers passing nested JSON got "Found field 'metadata.agent', but no such field exists"

**Rule: When you add a field to ANY struct in `crates/surreal-memory/src/`, you MUST also add a migration in `crates/surreal-memory/src/storage/migrations/mod.rs`.**

For FLEXIBLE (arbitrary nested JSON) fields, use: `DEFINE FIELD IF NOT EXISTS fieldname ON table FLEXIBLE TYPE option<object>;`

## Ecosystem Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Claude Code / Coding Agents                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Claude Code   │  │ Cursor       │  │ Other IDEs   │  │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  │
│         │                  │                  │          │
│         ▼                  ▼                  ▼          │
│  ┌──────────────────────────────────────────────────┐   │
│  │   surreal-memory-server (MCP stdio/HTTP)          │   │
│  │   40+ tools: add_memory, search, graph-RAG, etc.  │   │
│  └──────────────────────┬───────────────────────────┘   │
│                         │ uses                           │
│  ┌──────────────────────▼───────────────────────────┐   │
│  │         surreal-memory (library crate)             │   │
│  │  MemoryStorage trait │ SurrealStorage impl         │   │
│  │  Knowledge Graph     │ Scoped Memory (mem0)        │   │
│  │  TaskStreams          │ Mindmaps                    │   │
│  │  Hybrid BM25+HNSW    │ Model Profiles              │   │
│  └──────────────────────┬───────────────────────────┘   │
│                         │ also used by                   │
│  ┌──────────────────────▼───────────────────────────┐   │
│  │   universal-agent-runtime (UAR)                    │   │
│  │   MemoryService (scope enforcement, UserContext)   │   │
│  │   context_builder (token-budgeted prompt injection)│   │
│  │   auto_capture (post-stream memory extraction)     │   │
│  │   uar-wisc CLI (WISC commands for Claude Code)     │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## Library Crate (`crates/surreal-memory/`)

### Public API Surface

The library re-exports everything from `src/lib.rs`:

```rust
pub use embeddings::{EmbeddingProvider, EmbeddingService};
pub use entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult};
pub use memory::{Memory, MemoryHistory, MemoryScope, MemoryType};
pub use mindmap::{ExportFormat, MapType, MindMap, MindMapEdge, MindMapNode};
pub use model_profiles::{MODEL_PROFILES, ModelProfile, profile_for};
pub use storage::MemoryStorage;
pub use storage::surreal::SurrealStorage;
pub use task_stream::{ContextWindow, TaskStream, TaskStreamStatus};
```

### MemoryStorage Trait (35+ methods)

This is the core interface. Consumers depend on this trait, not on SurrealStorage directly.

Key method groups:
- **Scoped Memory**: `add_memory`, `search_memories`, `hybrid_search_memories`, `compress_memories`, `add_memories_from_conversation`
- **Knowledge Graph**: `create_entity`, `semantic_search`, `find_path`, `expand_neighbors`, `get_related`
- **TaskStreams**: `create_task_stream`, `add_to_task_stream`, `get_context_for_task`, `auto_summarize_task_stream`
- **Mindmaps**: `create_mindmap`, `add_mindmap_node`, `export_mindmap`, `generate_persona_mindmap`

### Memory Scoping Model

| Scope   | Who writes       | Who reads      | Use case                |
|---------|-----------------|----------------|-------------------------|
| Global  | System/admin    | Everyone       | Shared knowledge        |
| Agent   | agent_id match  | agent_id match | Per-agent context       |
| User    | user_id match   | user_id match  | Per-user personalization|
| Session | session_id match| session_id match| Conversation context   |
| Task    | TaskStream API  | TaskStream API | Long-running task state |

### Memory Types

| Type       | Purpose                                    |
|------------|-------------------------------------------|
| Semantic   | Facts, knowledge, general information      |
| Episodic   | Events, decisions, session handoffs        |
| Procedural | How-to knowledge, error resolutions        |
| Associative| Connections between concepts               |

## How UAR Consumes This Library

The `universal-agent-runtime` uses surreal-memory via a `MemoryService` facade (`src/uar/memory/service.rs`):

1. **`MemoryService::new(MemoryConfig)`** — Creates `SurrealStorage` + embedding provider from config
2. **`context_builder::build_context_with_hits()`** — Hybrid search → composite scoring (importance×0.4, recency×0.35, access×0.25) → token-budget truncation → formatted prompt block
3. **`auto_capture::capture_from_stream_end()`** — Post-LLM-turn memory extraction via `add_memories_from_conversation`
4. **`uar-wisc` CLI** — Thin binary composing MemoryService calls into WISC commands:
   - `decide` → dual-write: `create_entity` + `add_memory(Episodic, importance=0.9)`
   - `resolve` → `add_memory(Procedural, importance=0.8)` with error/resolution content
   - `checkpoint` → `add_to_task_stream` with TaskStream auto-creation
   - `recall` → `hybrid_search_memories` with cross-agent support
   - `prime` → `context_builder::build_context_with_hits` + `get_context_for_task`
   - `handoff` → `add_memory(Episodic, importance=1.0)` + `compress_memories`
   - `compact` → `auto_summarize_task_stream`

When changing the `MemoryStorage` trait or `SurrealStorage` implementation, be aware that UAR's `MemoryService` wraps every method with scope enforcement and `UserContext` checks. Changes here ripple through.

## Development Commands

### Building
```bash
cargo build --release                                    # Standard
cargo build --release --features cuda                    # CUDA GPU
cargo build --release --features metal                   # Metal GPU (macOS)
cargo build --release --features server-only             # No embedded DB
./build.sh                                               # Recommended for Apple Silicon
```

### Quality Gate (required before commits)
```bash
./scripts/quality-check.sh
cargo fmt --all --check
cargo clippy --all-targets --features embedded,metal -- -D warnings
cargo test --all-targets --features embedded,metal
FEATURE_FLAGS=embedded,cuda ./scripts/quality-check.sh   # Custom features
```

### Running
```bash
EMBEDDING_PROVIDER=local ./target/release/surreal-memory-server
EMBEDDING_PROVIDER=openai OPENAI_API_KEY=sk-... ./target/release/surreal-memory-server
./download-model.sh                                      # Pre-download models
```

## Key Files

| File | Purpose |
|------|---------|
| `crates/surreal-memory/src/lib.rs` | Library public API — all re-exports |
| `crates/surreal-memory/src/storage/mod.rs` | `MemoryStorage` trait (35+ methods) |
| `crates/surreal-memory/src/storage/surreal.rs` | `SurrealStorage` implementation |
| `crates/surreal-memory/src/storage/migrations/mod.rs` | Schema migrations (v1–v8) |
| `crates/surreal-memory/src/memory.rs` | `Memory`, `MemoryScope`, `MemoryType` structs |
| `crates/surreal-memory/src/entity.rs` | `Entity`, `Relation`, `KnowledgeGraph` |
| `crates/surreal-memory/src/task_stream.rs` | `TaskStream`, `ContextWindow` |
| `crates/surreal-memory/src/mindmap.rs` | `MindMap`, `MindMapNode`, `MindMapEdge` |
| `crates/surreal-memory/src/model_profiles.rs` | Token budget registry |
| `src/mcp/mod.rs` | MCP server tool definitions (40+ tools) |
| `src/mcp/handlers.rs` | MCP tool handler implementations |

## Configuration

**Database**: `SURREAL_MODE=embedded|server`, `SURREAL_PATH`, `SURREAL_ENDPOINT`
**Embeddings**: `EMBEDDING_PROVIDER=local|openai|cohere`, `LOCAL_EMBEDDING_MODEL`, `MODEL_CACHE_DIR`
**Features**: `embedded` (default), `server-only`, `cuda`, `metal`, `local-embeddings`

## Migration System

Migrations live in `crates/surreal-memory/src/storage/migrations/mod.rs`.

**Adding a new migration:**
1. Add a `const MIGRATION_VN_SQL: &str = "..."` with your DDL
2. Add a `Migration { version: N, name: "...", sql: MIGRATION_VN_SQL }` to the `MIGRATIONS` array
3. The runner auto-applies pending migrations on startup
4. Migrations are recorded in `schema_version` table with checksum

**Current migrations:**
- v1: Entity + Relation tables
- v2: Memory table (scoped, mem0-compatible)
- v3: TaskStream table
- v4: MemoryHistory audit log
- v5: HNSW vector indexes (entity + memory)
- v6: Mindmap table + BM25 full-text indexes
- v7: TaskStream auto-summarization fields (`auto_summarize`, `summary_count`, `model_id`)
- v8: Memory metadata FLEXIBLE (allows arbitrary nested JSON)

## Common Patterns

### Adding a New Field to a Persisted Struct
1. Add the field to the Rust struct (e.g., in `memory.rs`, `task_stream.rs`)
2. Add `#[serde(default)]` or `#[serde(default = "...")]` for backward compatibility
3. Create a new migration in `migrations/mod.rs` with the `DEFINE FIELD` statement
4. Test with BOTH embedded and server modes
5. Update any MCP handler params in `src/mcp/handlers.rs` if the field should be settable via MCP

### Adding a New MCP Tool
1. Add handler params struct + handler method in `src/mcp/handlers.rs`
2. Add `#[tool(...)]` annotated method in `src/mcp/mod.rs`
3. Update the `instructions` string in `get_info()` to list the new tool

### Adding a New Embedding Provider
1. Create module in `src/embeddings/` or `crates/surreal-memory/src/embeddings/`
2. Implement the `EmbeddingService` trait
3. Add variant to `EmbeddingProvider` enum
4. Wire into `create_embedding_service()` factory

## SurrealDB Gotchas

1. **SCHEMAFULL rejects unknown fields** — If a Rust struct has a field not in the schema, INSERT/CREATE fails silently with "Found field X, but no such field exists". Always check migrations match structs.
2. **`option<object>` is NOT flexible** — It accepts `None` or a flat `{}`, but rejects nested JSON like `{"key": {"nested": "value"}}`. Use `FLEXIBLE TYPE option<object>` for arbitrary JSON.
3. **HNSW index dimensions must match embeddings** — The v5 migration hardcodes `DIMENSION 1536`. If you switch to a model with different dimensions, you need a new migration to recreate the index.
4. **`IF NOT EXISTS` is idempotent** — All DDL uses this, so migrations are safe to re-run.
5. **Embedded mode creates RocksDB files** — Multiple processes CANNOT share the same embedded DB path simultaneously. For multi-process access, use server mode (`SURREAL_MODE=server`).

## Performance Considerations

- Local embeddings need GPU; without Metal/CUDA they're CPU-bound and slow
- Hybrid search (BM25+HNSW) is the primary retrieval path — both indexes must exist
- `get_context_for_task` sorts by importance and truncates to token budget — it's the hot path for LLM prompt construction
- `compress_memories` and `auto_summarize_task_stream` are expensive — they re-embed the summary

## AI Agent Operational Rules

These rules exist because of real failures observed during the WISC CLI integration (2026-03-17). Follow them.

### 1. Read Before You Write
**ALWAYS examine the existing codebase FIRST before writing any code.** Do not build anything from scratch until you have read:
- `crates/surreal-memory/src/lib.rs` (public API)
- `crates/surreal-memory/src/storage/mod.rs` (MemoryStorage trait)
- `crates/surreal-memory/src/storage/migrations/mod.rs` (current schema)

If the task involves the UAR consumer, also read:
- `src/uar/memory/service.rs` (MemoryService facade)
- `src/uar/memory/context_builder.rs` (prompt injection logic)
- `src/config.rs` (MemoryConfig struct)

**Failure mode this prevents:** Building a parallel storage layer that duplicates what already exists. This wasted 3+ turns in the WISC integration.

### 2. Do What You're Asked, Nothing Else
When told to "commit and push", do `git add -A && git commit -m "..." && git push`. Do not:
- Run another build verification cycle
- Refactor code you weren't asked to change
- Start testing flows that weren't requested
- Wait for compilation before committing (if it compiled earlier, it still compiles)

**Failure mode this prevents:** Spinning in verification loops instead of executing a simple command. This wasted 4+ turns.

### 3. One Build Verification, Then Move On
Run `cargo check` or `cargo build` exactly ONCE after making changes. If it succeeds, proceed. Do not:
- Run it again "to be sure"
- Wait for process output that already showed success
- Kill and restart builds because you couldn't read the output
- Run clippy separately if the build already passed

**Failure mode this prevents:** Timeout/retry spirals with Desktop Commander process tools.

### 4. Migrations Are Non-Negotiable
When adding ANY field to `Memory`, `Entity`, `TaskStream`, `MindMap`, or any other persisted struct:
1. Add the field to the struct with `#[serde(default)]`
2. Add a migration IMMEDIATELY — not "later", not "in a follow-up"
3. The migration goes in `crates/surreal-memory/src/storage/migrations/mod.rs`
4. Increment the version number, add to the `MIGRATIONS` array

Do NOT assume "the field will just work" — SurrealDB SCHEMAFULL tables reject unknown fields at runtime.

### 5. Test Against the Actual Consumer
When making changes to surreal-memory, verify against the REAL consumer (UAR), not a toy test. The integration bugs found during WISC development were:
- `TaskStream` creation failed because of missing schema fields
- `Memory` metadata insertion failed because of SCHEMAFULL strictness
- `UserContext` scoping with `user_id = agent_id` caused empty search results (should be `user_id = "anonymous"` for CLI agents)

These would not have been caught by unit tests alone.

### 6. Know the Scoping Rules
For CLI/agent consumers (not web UI):
- Use `user_id = "anonymous"` in UserContext so queries filter by `agent_id`, not `user_id`
- Use `agent_id` as the primary identity discriminator
- Pass `session_id = None` for cross-session queries (like `prime` and `recall`)
- Pass `session_id = Some(...)` only for session-specific writes

### 7. The UAR Cargo.toml Pin
UAR pins this library by git rev:
```toml
surreal-memory = { git = "https://github.com/Prometheus-AGS/surreal-memory-server", rev = "COMMIT_SHA", ... }
```
After pushing changes here, you MUST update the rev in UAR's Cargo.toml and run `cargo update -p surreal-memory` to refresh Cargo.lock.
