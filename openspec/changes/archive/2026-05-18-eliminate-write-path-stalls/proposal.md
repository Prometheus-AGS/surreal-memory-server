# Eliminate Write-Path Stalls

## Why

Assessment findings **C-2** and **H-2** form one root-cause cluster: the write
path can stall for minutes with no result and no error.

The single most diagnostic fact from the incident: `create_task_stream`
(`storage/surreal.rs`) does **no embedding** — it sets timestamps and calls
`create_record`. Yet it hung the identical 4 minutes as `create_entity`. This
**rules out the embedding model as the sole cause** and proves a shared,
upstream defect on the write path.

The defect is nested retry amplification. `create_record` → `retry_operation`
retries up to `max_operation_retries` (3). On a retriable error it calls
`reconnect` → `connect_with_retry`, which loops up to `max_connect_retries`
(**10**, per `docker-compose.yaml`) with backoff up to 5s each. Worst case:
3 × (10 × ~5s connect + per-attempt op time) ≈ **150s+** of silent blocking
before any error surfaces. The `TIMEOUT 30s` on the SQL bounds the *query* but
not this *outer* amplification.

## What Changes

- Introduce a single bounded wall-clock budget for a write operation that spans
  all retry + reconnect attempts (target ≤ 30s default, configurable). When the
  budget is exhausted the operation returns a typed error — never an open hang.
- Ensure `reconnect` / `connect_with_retry` is not invoked unboundedly from
  inside the per-operation retry loop: either share one reconnect attempt across
  the budget, or cap connect retries when reached via the operation path
  (distinct from the initial-startup connect, which may keep its larger budget).
- Add tracing spans to the connection-acquisition and retry path so a future
  stall is diagnosable from logs.
- Reproduce C-2: confirm `create_task_stream` and `create_entity` both return a
  bounded result or typed error against a forced-failure / unreachable SurrealDB.

## Non-goals

- Removing retry/reconnect resilience — legitimate transient-failure recovery
  must still work; only the *unbounded compounding* is removed.
- Changing the SurrealDB connection driver or pool implementation.
- The embedding model load hang (covered by `bound-embedding-model-loading`).
- MCP-layer progress reporting (covered by `add-mcp-progress-reporting`).

## Impact

- No persisted struct field added → no migration.
- Touches `retry_operation`, `reconnect`, `connect_with_retry` in
  `crates/surreal-memory/src/storage/surreal.rs` — these are shared by **every**
  storage operation, not just writes. Regression risk: must not break valid
  transient-failure recovery. One build verification, full test suite.
- Retry config (`SURREAL_MAX_OPERATION_RETRIES`, `SURREAL_MAX_CONNECT_RETRIES`,
  delays) semantics may be clarified; document any behavior change.
