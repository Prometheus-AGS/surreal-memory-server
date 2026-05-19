# Add MCP Progress Reporting

## Why

Assessment findings **H-1** and **M-1** form one root-cause cluster: a client
cannot see or survive a slow tool call.

The MCP layer (`src/mcp/handlers.rs`, `src/mcp/http.rs`) returns tool results
synchronously with no `notifications/progress` emission and no Tasks-API path.
Any operation exceeding the client's timeout appears as a dead server — which is
exactly how the incident's 4-minute hang presented. Per the MCP spec
(2025-06-18, stable), slow tools should emit progress heartbeats; clients with
`resetTimeoutOnProgress: true` reset their timeout on each heartbeat.

Separately, **M-1** is a documentation/code drift: CLAUDE.md claims a "30-second
query timeout" mitigation for mindmaps > 500 nodes, but `add_mindmap_node` /
`add_mindmap_edge` (`storage/surreal.rs`) only emit a `tracing::warn!` — there is
no enforced `TIMEOUT` on the mindmap `UPDATE` path. The documented mitigation is
partially missing in code.

This change MUST land after `bound-embedding-model-loading` and
`eliminate-write-path-stalls`: emitting "progress" around an operation that can
still hang unbounded would be theatre. Progress reporting is only meaningful once
the underlying operations return bounded results or typed errors.

## What Changes

- Emit MCP `notifications/progress` heartbeats from slow tool handlers for:
  model load/warmup, Palace ingest, mindmap node/edge updates,
  `auto_summarize_task_stream`, and `compress_memories`.
- Ensure the SSE transport (`src/mcp/http.rs`) actually flushes heartbeats; the
  handler must degrade gracefully when the client supplies no `progressToken`.
- **M-1**: enforce the documented `TIMEOUT` on the mindmap `UPDATE` path
  (`update_mindmap_graph` / `append_mindmap_node`) so large-mindmap updates fail
  fast as documented, instead of only logging a warning. Resolve the CLAUDE.md
  doc/code drift — implementation preferred over correcting the doc downward.
- Evaluate the experimental MCP Tasks API (spec 2025-11-25) for genuinely
  long-running operations and record the decision (adopt now / defer) in this
  change before archive.

## Tasks API Decision (recorded before archive)

**Decision: DEFER the experimental MCP Tasks API.**

Rationale:
- The MCP Tasks API ("submit now, poll later", spec 2025-11-25) is explicitly
  experimental and still moving — the spec warns implementers to pin to a spec
  version and expect breaking changes.
- After `bound-embedding-model-loading` and `eliminate-write-path-stalls`, every
  surreal-memory write/slow operation is now bounded (≤ ~30s) by a hard
  deadline. The Tasks API targets operations that genuinely exceed ~30s and
  benefit from resumability — surreal-memory has none in that class once the
  prior two changes landed.
- `notifications/progress` heartbeats (this change) fully address the observed
  failure mode: a slow-but-working call no longer looks like a dead server,
  because clients with `resetTimeoutOnProgress: true` reset their timeout on
  each heartbeat.
- Adopting Tasks now would add a stateful `taskId` lifecycle, polling
  endpoints, and TTL/garbage-collection surface for zero current benefit.

Revisit if/when surreal-memory grows an operation that is both genuinely
long-running (minutes) and benefits from resumability — e.g. a bulk re-embed or
a full-corpus palace re-index.

## Non-goals

- Implementing the full MCP Tasks API "submit now, poll later" lifecycle —
  this change scopes the *evaluation and decision*; adoption is deferred (see above).
- Fixing the model-load hang or the retry stall (prior two changes).
- Rearchitecting the mindmap storage model (single-record JSON rewrite) — only
  the missing `TIMEOUT` enforcement is in scope here.

## Impact

- No persisted struct field added → no migration.
- Touches `src/mcp/handlers.rs`, `src/mcp/http.rs`, and the mindmap `UPDATE`
  path in `crates/surreal-memory/src/storage/surreal.rs`.
- Updates CLAUDE.md "Mindmap Performance Limitations" to match the implemented
  behavior.
- Depends on the bounded-operation guarantees from the prior two changes.
