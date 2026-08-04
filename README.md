# surreal-memory-server

Rust memory service with a knowledge graph, scoped memories, hybrid search, TaskStreams, Mindmaps, MCP transports, and a durable receipt-driven operation ledger.

## Durable operation API

New write integrations use:

- `POST /api/v2/operations` — accept a caller ID, canonical payload hash, dependencies, kind, and payload;
- `GET /api/v2/operations/{operation_id}` — reconcile the authoritative receipt;
- `GET /api/v2/operations/{operation_id}/events?after=N` — replay and follow ordered SSE state events;
- `GET /health` — process liveness;
- `GET /ready` — ledger, storage, coordinator, tokenizer, executor, and search readiness.

`202` means a new operation is durably accepted. `200` on a repeated POST means exact same-ID/same-hash replay. `409` protects an existing ID from a different payload. Only `committed` or `rejected` receipts are terminal.

Long memories are planned with the active tokenizer. Parts and executor progress are persisted, so restart reuses the plan and resumes only unfinished parts before committing one logical memory.

The machine-readable contract is [OpenAPI 3.1](openapi/surreal-memory-v2.openapi.json). Canonical design and usage documentation is published in the Prometheus Skill System under [Memory](https://prometheus-ags.github.io/prometheus-skill-system/docs/memory/overview) and [Operation API](https://prometheus-ags.github.io/prometheus-skill-system/docs/memory/operation-api).

## Architecture

```text
HTTP/MCP callers
      │
Axum API + durable operation ledger
      │
persisted tokenizer plan + supervised model executor
      │
surreal-memory library
      │
SurrealDB storage, graph, vectors, events, task state
```

The `crates/surreal-memory` library owns storage and embedding abstractions. The root binary supplies Axum REST/MCP routing, durable operation coordination, supervised execution, and background workers. Optional features include Memory Palace and CUDA/Metal acceleration.

## Build and test

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

The native service is the canonical local deployment. Configure the SurrealDB endpoint and embedding provider through environment variables; containers remain optional development packaging.

## Recovery contract

Do not infer success from HTTP timeouts, process lifetime, elapsed time, or attempt counts. Preserve the original operation ID and payload hash, retrieve the receipt, resume events after the last processed sequence, and allow persisted plans to continue after restart.
