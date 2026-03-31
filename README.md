# surreal-memory-server

A production-grade, **dual-transport AI memory system** built in Rust. Exposes a full knowledge graph and scoped memory store via both **MCP (Model Context Protocol)** and a **REST API**, backed by SurrealDB with HNSW vector search.

[![CI](https://github.com/Prometheus-AGS/surreal-memory-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Prometheus-AGS/surreal-memory-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                   surreal-memory-server                  │
│                                                          │
│  ┌─────────────────┐      ┌────────────────────────────┐ │
│  │   MCP Transport │      │      REST API (Axum)       │ │
│  │  stdio  │  HTTP │      │  /api/v1/memory            │ │
│  │  (JSONRPC 2.0)  │      │  /api/v1/entities          │ │
│  └────────┬────────┘      │  /api/v1/mindmaps          │ │
│           │               │  /api/v1/search            │ │
│           └───────────────│  /a2a/tasks/:id/events     │ │
│                           └──────────────┬─────────────┘ │
│                                          │               │
│           ┌──────────────────────────────┘               │
│           ▼                                              │
│   ┌────────────────┐    ┌──────────────────────────────┐ │
│   │ surreal-memory │    │   TTL Decay Worker (async)   │ │
│   │  (library crate│    └──────────────────────────────┘ │
│   │   see /crates) │                                      │
│   └───────┬────────┘                                      │
│           │                                              │
│   ┌───────▼──────────────────────────────────────────┐  │
│   │            SurrealDB (embedded or server)         │  │
│   │   HNSW vectors · BM25 fulltext · RocksDB store   │  │
│   └──────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

## Feature Matrix

| Capability | Description |
|---|---|
| 🧠 **Knowledge Graph** | Entities, relations, HNSW vector search, Graph-RAG traversal (`find_path`, `expand_neighbors`, `get_related`) |
| 🗂️ **Scoped Memory** | mem0-compatible API — 4 memory types (Episodic, Semantic, Procedural, Associative), 3 scopes (User, Session, Agent) |
| 🔀 **Hybrid Search** | Weighted BM25 + HNSW vector score merging |
| 📋 **TaskStreams** | Named task contexts with model-aware token budgeting and rolling auto-summarization |
| 🗺️ **Mindmaps** | 5 map types, export to JSON/Mermaid/Markdown, automatic persona generation |
| 📐 **Model Profiles** | Built-in token budget registry for GPT-4o, Claude 3.5, Gemini 1.5/2.0 Pro, Llama, Mistral |
| ⏱️ **Temporal History** | `get_entity_history` and `get_graph_at_time` — query the graph as it was at any point in the past |
| 📡 **A2A SSE** | `GET /a2a/tasks/:id/events` — server-sent event stream for agent-to-agent task coordination |
| ⏳ **TTL Decay** | Background worker applies configurable memory decay; expired memories purged automatically |
| 🔌 **Dual Transport** | `stdio` MCP for Claude Desktop / Cursor; `HTTP` MCP + REST for direct API access |
| 🐳 **Docker-ready** | Multi-stage `Dockerfile` with non-root user and health check |
| 🤖 **CI** | GitHub Actions runs `cargo check`, `clippy -D warnings`, and `cargo test` on every push |

---

## Workspace Layout

```
surreal-memory-server/
├── crates/
│   └── surreal-memory/          # Embeddable library crate (publish to crates.io separately)
│       ├── src/
│       │   ├── embeddings/      # EmbeddingService trait + OpenAI / Cohere / Candle impls
│       │   ├── storage/         # MemoryStorage trait + SurrealDB impl + versioned migrations
│       │   ├── entity.rs        # Entity, Relation, KnowledgeGraph
│       │   ├── memory.rs        # Memory, MemoryType, MemoryScope, MemoryHistory
│       │   ├── mindmap.rs       # MindMap, MindMapNode, MapType
│       │   ├── model_profiles.rs# Built-in LLM token budget registry
│       │   └── task_stream.rs   # TaskStream, TaskStreamStatus, TaskStreamContext
│       └── tests/
│           └── integration_test.rs  # 14-test suite (RocksDB embedded, no external deps)
├── src/
│   ├── api/                     # Axum REST handlers
│   │   ├── a2a.rs               # A2A SSE stream endpoint
│   │   ├── entities.rs          # /api/v1/entities
│   │   ├── memory.rs            # /api/v1/memory
│   │   ├── mindmaps.rs          # /api/v1/mindmaps
│   │   └── search.rs            # /api/v1/search
│   ├── mcp/
│   │   ├── handlers.rs          # MCP tool implementations (30+ tools)
│   │   ├── http.rs              # Streamable HTTP MCP transport
│   │   └── mod.rs               # stdio MCP transport
│   ├── workers/
│   │   └── ttl.rs               # Background TTL decay worker
│   └── main.rs
├── .github/workflows/ci.yml     # GitHub Actions CI
├── Dockerfile                   # Multi-stage production image
├── docker-compose.yaml
└── Cargo.toml                   # Workspace manifest
```

---

## Quick Start

### Binary (MCP Server)

```bash
# Clone and build
git clone https://github.com/Prometheus-AGS/surreal-memory-server
cd surreal-memory-server

# Copy environment config
cp .env.example .env

# Build and run (embedded SurrealDB, local Candle embeddings)
cargo build --release
./target/release/surreal-memory-server
```

### Docker

```bash
docker build -t surreal-memory-server .

docker run -p 3000:3000 \
  -e EMBEDDING_PROVIDER=openai \
  -e OPENAI_API_KEY=sk-... \
  -v $(pwd)/data:/app/data \
  surreal-memory-server
```

### Docker Compose

```bash
docker-compose up
```

---

## Configuration

Copy `.env.example` to `.env` and edit:

```bash
# ── SurrealDB ────────────────────────────────────────────
SURREAL_MODE=embedded          # embedded | server
SURREAL_PATH=./data/memory.db  # path for embedded RocksDB store
SURREAL_NAMESPACE=memory
SURREAL_DATABASE=mcp

# For server mode:
# SURREAL_MODE=server
# SURREAL_ENDPOINT=ws://localhost:8000
# SURREAL_USERNAME=root
# SURREAL_PASSWORD=root

# ── Embedding Provider ───────────────────────────────────
EMBEDDING_PROVIDER=local       # local | openai | cohere

# Local (no API key, runs offline)
LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
MODEL_CACHE_DIR=./models

# OpenAI (text-embedding-3-small generates 1536-dim vectors)
# EMBEDDING_PROVIDER=openai
# OPENAI_API_KEY=sk-...
# OPENAI_EMBEDDING_MODEL=text-embedding-3-small

# Cohere
# EMBEDDING_PROVIDER=cohere
# COHERE_API_KEY=...
# COHERE_EMBEDDING_MODEL=embed-english-v3.0

# ── Server ───────────────────────────────────────────────
API_PORT=3000
RUST_LOG=info
```

### Embedding Providers

| Provider | Dimensions | Speed | Privacy | Cost |
|---|---|---|---|---|
| Local `bge-small-en-v1.5` | 384 | ⚡⚡⚡ | ✅ offline | Free |
| Local `bge-base-en-v1.5` | 768 | ⚡⚡ | ✅ offline | Free |
| Local `bge-large-en-v1.5` | 1024 | ⚡ | ✅ offline | Free |
| OpenAI `text-embedding-3-small` | 1536 | ⚡⚡ | ❌ | $ |
| Cohere `embed-english-v3.0` | 1024 | ⚡⚡ | ❌ | $ |

> **GPU acceleration:** `cargo build --release --features cuda` (CUDA) or `--features metal` (macOS)

### Retry Configuration

The memory server includes automatic retry and reconnection logic to handle transient failures. This makes the system resilient to network hiccups, database restarts, and temporary connection issues.

#### How It Works

- **Initial Connection**: Retries connection establishment during startup (configurable, default: 10 attempts)
- **Operation Retries**: Automatically retries failed operations with exponential backoff (default: 3 attempts)
- **Smart Reconnection**: Detects connection loss and attempts automatic reconnection before retrying operations
- **Error Classification**: Only retries retriable errors (network issues, timeouts, transient DB errors)

#### Configuration

Configure retry behavior via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `SURREAL_MAX_CONNECT_RETRIES` | Maximum connection attempts during startup | `10` |
| `SURREAL_MAX_OPERATION_RETRIES` | Maximum retry attempts per operation | `3` |
| `SURREAL_BASE_RETRY_DELAY_MS` | Starting delay for exponential backoff (ms) | `100` |
| `SURREAL_MAX_RETRY_DELAY_MS` | Maximum delay cap for backoff (ms) | `5000` |
| `SURREAL_RETRY_JITTER_FACTOR` | Jitter factor for delay randomization (0.0-1.0) | `0.25` |

#### Example

```bash
# Use aggressive retries for high-reliability deployments
SURREAL_MAX_CONNECT_RETRIES=20 \
SURREAL_MAX_OPERATION_RETRIES=5 \
./surreal-memory-server
```

#### Docker Compose

The retry configuration is already included in `docker-compose.yaml` with sensible defaults. Override them as needed:

```yaml
services:
  surreal-memory-server:
    environment:
      - SURREAL_MAX_CONNECT_RETRIES=20  # Override default
      - SURREAL_MAX_OPERATION_RETRIES=5  # Override default
```

#### Observability

Retry attempts are logged with structured events:

```
WARN operation=add_memory attempt=2 max_attempts=3 error="Connection timeout" next_delay_ms=200 "Retrying operation after transient failure"
```

Monitor these log events to understand retry behavior in production.

---

## MCP Integration (Claude Desktop / Cursor)

`surreal-memory-server` ships with a fully compliant, production-ready MCP implementation, surfacing **42 distinct memory and graph tools** directly to LLM clients.

### HTTP MCP transport (Streamable HTTP)

The primary and recommended transport is via Streamable HTTP (`/mcp/http`), using a single unified endpoint for Server-Sent Events (SSE) and message POSTs over HTTP via a custom Axum wrapper around the `rmcp` Tower service. This requires zero sidecar binaries and allows distributed agentic networks to connect natively. Legacy endpoints (`GET /mcp/sse` and `POST /mcp/messages`) are also maintained for backwards compatibility with older clients.

```
MCP Streamable Endpoint:  http://localhost:23001/mcp/http
MCP Legacy SSE Endpoint:  http://localhost:23001/mcp/sse
```

#### Testing with the Official MCP Inspector

You can instantly verify the 42-tool schema payload and test the streamable HTTP stability against the strict official test client using `npx`:

```bash
# Ensure the surreal-memory-server is running (e.g. via docker compose up -d)
npx -y @modelcontextprotocol/inspector sse http://localhost:23001/mcp/http
```
This will launch the MCP Inspector web UI. Click "Connect" and expand "List Tools" to browse and test all of the endpoints dynamically.

### stdio transport

For local strict coupling (like Claude Desktop without a network listener), you can still invoke the server directly via `stdio`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/surreal-memory-server",
      "env": {
        "EMBEDDING_PROVIDER": "local",
        "LOCAL_EMBEDDING_MODEL": "BAAI/bge-small-en-v1.5",
        "SURREAL_MODE": "embedded",
        "SURREAL_PATH": "/path/to/data/memory.db"
      }
    }
  }
}
```

### Available MCP Tools (42)

The server exposes 42 strictly schema-typed tools, encompassing the entire underlying `surreal-memory` backend array:

| Category | Tools |
|---|---|
| **Scoped Memory** (mem0) | `add_memory`, `search_memories`, `hybrid_search_memories`, `get_memory`, `update_memory`, `delete_memory`, `delete_all_memories`, `get_all_memories`, `get_memory_history`, `compress_memories`, `add_memories_from_conversation` |
| **Knowledge Graph** | `create_entity`, `create_entities`, `get_entity`, `update_entity`, `delete_entity`, `create_relation`, `create_relations`, `get_relations`, `delete_relation`, `add_observations`, `get_graph`, `read_graph` |
| **Graph-RAG** | `find_path`, `expand_neighbors`, `get_related`, `semantic_search` |
| **Temporal History** | `get_entity_history`, `get_graph_at_time` |
| **TaskStreams** | `create_task_stream`, `add_to_task_stream`, `get_context_for_task`, `list_task_streams`, `get_task_stream`, `archive_task_stream`, `auto_summarize_task_stream` |
| **Mindmaps** | `create_mindmap`, `get_mindmap`, `add_mindmap_node`, `delete_mindmap_node`, `add_mindmap_edge`, `list_mindmaps`, `delete_mindmap`, `export_mindmap`, `generate_persona_mindmap`, `generate_ideation_mindmap` |

---

## REST API

All endpoints accept and return JSON. Base path: `http://localhost:3000`

### Health

```
GET /health  →  { "status": "ok" }
```

### Memory  `/api/v1/memory`

```
POST   /api/v1/memory          Create memory
GET    /api/v1/memory          List memories (query: user_id, agent_id, session_id)
GET    /api/v1/memory/:id      Get memory by ID
PATCH  /api/v1/memory/:id      Update memory content
DELETE /api/v1/memory/:id      Delete memory
DELETE /api/v1/memory          Delete all (filterable)
POST   /api/v1/search          Semantic / hybrid search
```

### Entities  `/api/v1/entities`

```
POST   /api/v1/entities              Create entity
GET    /api/v1/entities/:name        Get entity
DELETE /api/v1/entities/:name        Delete entity
POST   /api/v1/entities/:name/obs    Add observations
GET    /api/v1/entities/:name/rels   Get relations
POST   /api/v1/entities/relations    Create relation
DELETE /api/v1/entities/relations    Delete relation
GET    /api/v1/entities/graph        Full knowledge graph
GET    /api/v1/entities/graph/path   Path between entities
GET    /api/v1/entities/graph/expand Expand neighborhood
GET    /api/v1/entities/related      Get related entities
```

### Mindmaps  `/api/v1/mindmaps`

```
POST   /api/v1/mindmaps              Create mindmap
GET    /api/v1/mindmaps              List mindmaps
GET    /api/v1/mindmaps/:name        Get mindmap
DELETE /api/v1/mindmaps/:name        Delete mindmap
POST   /api/v1/mindmaps/:name/nodes  Add node (supports optional metadata JSON)
POST   /api/v1/mindmaps/:name/edges  Add edge
DELETE /api/v1/mindmaps/:name/nodes/:id  Remove node
GET    /api/v1/mindmaps/:name/export Export (json|mermaid|markdown)
```

### TaskStreams  `/api/v1/taskstreams`

```
POST   /api/v1/taskstreams                   Create task stream
GET    /api/v1/taskstreams                   List task streams
GET    /api/v1/taskstreams/:name             Get task stream
DELETE /api/v1/taskstreams/:name             Delete task stream
POST   /api/v1/taskstreams/:name/archive     Archive task stream and return updated record
POST   /api/v1/taskstreams/:name/memories    Add memory to task stream
GET    /api/v1/taskstreams/:name/context     Get model-budgeted context window
POST   /api/v1/taskstreams/:name/summarize   Trigger rolling auto-summarization
```

### A2A SSE

```
GET /a2a/tasks/:id/events   Server-sent event stream for task coordination
```

---

## Library Crate

`surreal-memory` is also published as a standalone embeddable library:

```toml
[dependencies]
surreal-memory = "0.1"
```

See [`crates/surreal-memory/README.md`](crates/surreal-memory/README.md) for the full library API.

---

## Schema Migrations

The database schema is versioned and applied automatically at startup:

| Version | Name | What it adds |
|---|---|---|
| v1 | `baseline` | `entity`, `relation`, `schema_version` tables |
| v2 | `scoped_memory` | `memory` table with scopes, types, TTL, embeddings |
| v3 | `task_streams` | `task_stream` table, token budgeting |
| v4 | `model_profiles` | `model_profile` table |
| v5 | `hnsw_vector_indexes` | HNSW indexes on entity + memory embeddings |
| v6 | `mindmap_table` | `mindmap` table with 5 map types, nodes, edges, tags |

Migrations are safe to run against existing data — all DDL uses `IF NOT EXISTS`.

---

## Testing

```bash
# Run all tests (no external dependencies required)
cargo test --workspace

# Run only integration tests
cargo test -p surreal-memory --test integration_test

# Run with output
cargo test --workspace -- --nocapture
```

Integration tests use an **embedded RocksDB** backend in a temp directory — no SurrealDB server needed. 4 tests that require server-mode for enum round-trip deserialization are marked `#[ignore]` and can be run against a live server.

---

## CI / CD

GitHub Actions runs on every push and pull request to `main`:

1. `cargo check --workspace`
2. `cargo clippy --workspace -- -D warnings`
3. `cargo test --workspace`

See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

---

## Building the Docker Image

```bash
# Build production image (multi-stage, ~50MB runtime layer)
docker build -t surreal-memory-server .

# The image:
# - Build stage: rust:1.83-slim — compiles the workspace
# - Runtime stage: debian:bookworm-slim — minimal runtime
# - Non-root user: smserver
# - Health check: GET /health every 30s
# - Exposed port: 3000 (configurable via API_PORT env var)
```

---

## Development

```bash
# Check
cargo check --workspace

# Lint (warnings as errors)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Test
cargo test --workspace

# Run in dev mode
RUST_LOG=debug cargo run
```

---

## License

MIT — see [LICENSE](LICENSE)
