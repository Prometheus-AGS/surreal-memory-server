# surreal-memory-server

Durable agent memory over SurrealDB, served through MCP and REST. It stores memory that is **typed, scoped and durable** instead of a bag of embeddings: a knowledge graph, scoped memories, hybrid BM25 + vector search, TaskStreams, Mindmaps, and a receipt-driven operation ledger for writes that must survive a crash.

**📖 [Documentation](https://prometheus-ags.github.io/surreal-memory-server/)** — quickstart, guides, REST and MCP reference, configuration, operations, and the reasoning behind the design.

Built and tested against **SurrealDB 4.0.0**. The workspace ships two artifacts: the `surreal-memory-server` binary (MCP over stdio/HTTP, REST, A2A) and the `crates/surreal-memory` library, which is a direct dependency of the Universal Agent Runtime.

## Features

- **Scoped memory** (global, agent, user, session, task) with a mem0-compatible surface and semantic de-duplication.
- **Hybrid search**: BM25 and HNSW merged by reciprocal rank fusion, with a bounded query-embedding cache.
- **Knowledge graph** with Graph-RAG traversal, **TaskStreams** with token-budgeted context, **Mindmaps**, and an optional **Memory Palace**.
- **Verified caller scope** *(new)*: optional JWT authentication with one database per tenant. See [Authentication](#authentication-and-tenancy).
- **Memory expiry** *(new)*: expired memory is invisible to every read at once, and cleanup reports what it really removed.
- **Evidence-backed compression** *(new)*: summaries whose claims quote their sources; sources are archived, not deleted. The default summarizer never leaves the process.
- **Durable operations**: retry-safe writes with receipts, ordered events, and operator retry/reject.
- **Claude Code hooks** *(new)*: `hook session-start|pre-compact|session-end` and `statusline`.
- **59 MCP tools**, mirrored by the REST API.

## Quickstart

```bash
export SURREAL_MODE=embedded
export SURREAL_PATH="$HOME/.surreal-memory/db"
export EMBEDDING_PROVIDER=local            # BAAI/bge-small-en-v1.5, 384 dimensions

cargo build --release --features embedded,local-embeddings
./target/release/surreal-memory-server     # listens on 127.0.0.1:3001
```

```bash
# Store a memory
curl -s -X POST http://127.0.0.1:3001/api/v1/memory -H 'content-type: application/json' \
  -d '{"content":"Alice prefers dark mode","user_id":"alice","agent_id":"claude-code"}'

# Find it again (hybrid search)
curl -s -X POST http://127.0.0.1:3001/api/v1/search -H 'content-type: application/json' \
  -d '{"query":"editor theme","user_id":"alice","limit":3}'
```

Point an MCP client at `http://127.0.0.1:3001/mcp/http` (streamable HTTP), or run the binary as a stdio MCP server. Full walkthrough: [Quickstart](https://prometheus-ags.github.io/surreal-memory-server/docs/guides/quickstart).

## Authentication and tenancy

The API binds to `127.0.0.1` and runs in `AUTH_MODE=local` by default: one trusted local client, no credentials. **That mode is refused at startup on any other bind address.** For a shared or networked deployment, use `AUTH_MODE=jwt`:

```bash
export API_HOST=0.0.0.0
export AUTH_MODE=jwt
export AUTH_JWT_ISSUER=gate
export AUTH_JWT_AUDIENCE=surreal-memory
export AUTH_JWT_ALGORITHM=HS256            # HS256 | RS256 | ES256 | EdDSA
export AUTH_JWT_KEY='at-least-32-bytes-of-secret-or-a-public-key-PEM'
```

Every route except `/health` and `/ready` then needs `Authorization: Bearer <jwt>`. The token's `tenant` claim selects that tenant's own database (a namespace in server mode, a directory in embedded mode); it is never taken from request input. An `ops: ["operator"]` claim allows retry, reject and re-key, and a `user_id` claim restricts a credential to one user's memories. The server only verifies tokens; UAR/Gate issues them. Details, claims and examples: [Authentication and tenancy](https://prometheus-ags.github.io/surreal-memory-server/docs/guides/authentication).

## Durable operation API

New write integrations use:

- `POST /api/v2/operations` — accept a caller ID, canonical payload hash, dependencies, kind, and payload;
- `GET /api/v2/operations/{operation_id}` — reconcile the authoritative receipt;
- `GET /api/v2/operations/{operation_id}/events?after=N` — replay and follow ordered SSE state events;
- `POST /api/v2/operations/{operation_id}/retry` — re-drive a non-terminal operation now instead of waiting for the 60 s reconciliation sweep (operator);
- `POST /api/v2/operations/{operation_id}/reject` — dead-letter a non-terminal operation with `{"reason": "..."}` (operator; refused for `indexed` operations and for the one being processed);
- `GET /api/v2/operations/stats` — per-state counts, paused total, the oldest unfinished operation with its age (alert on `oldest_nonterminal.age_seconds`), and query-cache counters;
- `GET /health` — process liveness;
- `GET /ready` — ledger, storage, coordinator, tokenizer, executor, and search readiness.

`202` means a new operation is durably accepted. `200` on a repeated POST means exact same-ID/same-hash replay. `409` protects an existing ID from a different payload. Only `committed` or `rejected` receipts are terminal. The `payload_hash` is the SHA-256 of the payload as compact JSON with sorted keys; the [guide](https://prometheus-ags.github.io/surreal-memory-server/docs/guides/durable-operations) has a copy-paste recipe.

Paused operations are retried automatically every 60 s. Failures that retrying cannot fix (invalid payload, empty plan, changed deterministic plan) reject the operation instead of pausing it.

Long memories are planned with the active tokenizer. Parts and executor progress are persisted, so restart reuses the plan and resumes only unfinished parts before committing one logical memory.

The machine-readable contract is [OpenAPI 3.1](openapi/surreal-memory-v2.openapi.json). Canonical design and usage documentation is also published in the Prometheus Skill System under [Memory](https://prometheus-ags.github.io/prometheus-skill-system/docs/memory/overview) and [Operation API](https://prometheus-ags.github.io/prometheus-skill-system/docs/memory/operation-api).

## Architecture

```text
HTTP/MCP callers
      │
Auth layer (local, or verified JWT → tenant database)
      │
Axum API + durable operation ledger
      │
persisted tokenizer plan + supervised model executor
      │
surreal-memory library
      │
SurrealDB 4.0 storage, graph, vectors, events, task state
```

The `crates/surreal-memory` library owns storage and embedding abstractions. The root binary supplies Axum REST/MCP routing, authentication, durable operation coordination, supervised execution, and background workers. Optional features include Memory Palace, local embeddings, and CUDA/Metal acceleration. Embedded mode runs on SurrealKV; SurrealDB 4.0 removed RocksDB.

## Build and test

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

Run integration tests against a **dedicated** scratch SurrealDB, not one shared with live services, and point the server-mode tests at it with `TEST_SURREAL_ENDPOINT` (for example `ws://127.0.0.1:28100`). The native service is the canonical local deployment; containers remain optional development packaging, and the Compose files require the `AUTH_JWT_*` variables. See [Deployment](https://prometheus-ags.github.io/surreal-memory-server/docs/operations/deployment).

## Upgrading

Migrations run automatically on the first start of a new binary (currently up to v24). Export your databases first, and read [What's new](https://prometheus-ags.github.io/surreal-memory-server/docs/whats-new) for behavior changes — notably the loopback default for `API_HOST`.

## Recovery contract

Do not infer success from HTTP timeouts, process lifetime, elapsed time, or attempt counts. Preserve the original operation ID and payload hash, retrieve the receipt, resume events after the last processed sequence, and allow persisted plans to continue after restart.

### Scoped database authentication

Set `SURREAL_AUTH_LEVEL=namespace` to authenticate the server with a namespace user,
or `database` for a database user. `SURREAL_NAMESPACE` and `SURREAL_DATABASE` select
the scope. The default remains `root` for existing deployments. The library exposes
`SurrealConfig.auth_level` and `SurrealAuthLevel`; normal startup, reconnects and
repair commands share the same authentication path. The Boss uses a dedicated
namespace user for memory, separate from Compass. In `AUTH_MODE=jwt` with per-tenant
namespaces the server creates a namespace for each new tenant, so its database user
must be allowed to do that (a root-level user).
