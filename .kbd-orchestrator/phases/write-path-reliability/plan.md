# KBD Plan — Phase: write-path-reliability

- **Project**: surreal-memory-server
- **Planned**: 2026-05-18
- **Change backend**: OpenSpec (`openspec/` present, CLI v1.3.1, `change_backend: openspec`)
- **Evolver bridge**: none
- **Assessment**: `.kbd-orchestrator/phases/write-path-reliability/assessment.md`
  (verdict: LARGE GAP — 2 CRITICAL, 4 HIGH, 2 MEDIUM, 1 LOW)

---

## Strategy

The assessment grouped 9 findings into 3 root-cause clusters. Each cluster
becomes one OpenSpec change. Ordering is driven by **dependency** and
**diagnostic value**, not severity alone:

- Change 1 and Change 2 are **independent** — they touch different code
  (embedding loader vs. connection/retry path) and either could ship first.
  They are ordered 1→2 because Change 1's bounded-error contract is the model
  that Change 2 mirrors for the connection path, keeping error types consistent.
- Change 3 **depends on both** — MCP progress/heartbeat reporting is only
  meaningful once the underlying operations return bounded results or typed
  errors. Emitting "progress" around an unbounded hang would be theatre.

All three target the canonical spec `openspec/specs/write-path/spec.md` (created
fresh by this phase — currently only `task-stream` spec exists).

---

## Ordered Change List

### Change 1 — `bound-embedding-model-loading`

**Closes**: C-1, H-3, H-4, L-1, M-2 (root-cause cluster: unbounded, unobserved,
runtime-blocking model loads).

**Scope**:
- Move synchronous, CPU-heavy model load + inference off the tokio runtime via
  `spawn_blocking` — both Candle (`embeddings/candle.rs`) and the binary's copy
  (`src/embeddings/candle.rs`).
- Add a bounded timeout + typed error to the HuggingFace download path
  (`download_model`) so a cold-cache or network failure surfaces as a clear
  error, not an open-ended hang.
- Make Palace `FastEmbedService::new()` cold-load (`palace/embedding.rs`,
  `palace/context.rs`) observe the same bound.
- Add an explicit, optional startup warmup embed (config-gated) so first-write
  latency is paid at boot, not on the user's first call.
- Make `/health` (and the embedding readiness log) reflect *true* write-path
  readiness — distinguish "process up" from "embedding model loaded".
- L-1: parallelize the three HF file downloads.

**Recommended agent**: rust-reviewer for QA gate (artifact-refiner unavailable).
Implementation by primary coding agent.

**Risk**: `spawn_blocking` changes the concurrency shape of `embed`; ensure the
`OnceCell`/`Mutex` invariants in `CandleEmbeddings` still hold. No schema/struct
change expected → no migration. If a config field is added for warmup, it lives
in config structs only (not persisted) — confirm no `DEFINE FIELD` needed.

### Change 2 — `eliminate-write-path-stalls`

**Closes**: C-2, H-2 (root-cause cluster: shared write-path stalling with no
bounded failure).

**Scope**:
- Cap total retry + reconnect wall-clock under a single bounded budget.
  Today `retry_operation` (3×) nests `reconnect` → `connect_with_retry` (10×
  @ ≤5s), compounding to ~150s+. Introduce an overall deadline so any write
  tool returns a result or a typed error within a known bound (target ≤ 30s
  default, configurable).
- Ensure `reconnect` is not invoked unboundedly from inside the per-operation
  retry loop — either share one reconnect attempt across the budget or cap
  connect retries when called from the operation path.
- Instrument the connection-acquisition path with tracing spans so a future
  stall is diagnosable.
- Verify `create_task_stream` and `create_entity` both return bounded under a
  forced-failure SurrealDB (the C-2 reproduction).

**Recommended agent**: rust-reviewer for QA gate.

**Risk**: Changing retry/reconnect semantics affects every storage operation,
not just writes. Must not regress legitimate transient-failure recovery.
No schema change.

### Change 3 — `add-mcp-progress-reporting`

**Closes**: H-1, M-1 (root-cause cluster: client cannot see or survive slow
calls).

**Depends on**: Change 1 and Change 2 (operations must be bounded/typed first).

**Scope**:
- Emit MCP `notifications/progress` heartbeats from slow tool handlers
  (`src/mcp/handlers.rs`, `src/mcp/http.rs`) for: model load/warmup, Palace
  ingest, mindmap node/edge updates, `auto_summarize_task_stream`,
  `compress_memories`.
- M-1: enforce the documented mindmap `TIMEOUT` in the actual `UPDATE` path
  (`update_mindmap_graph` / `append_mindmap_node` in `surreal.rs`) — today it is
  only a `tracing::warn!`. Resolve the CLAUDE.md doc/code drift (either implement
  the 30s TIMEOUT as documented, or correct the doc — implementation preferred).
- Evaluate (spec-level, not necessarily implement) the experimental MCP Tasks
  API for genuinely long operations; record the decision in the change proposal.

**Recommended agent**: rust-reviewer for QA gate.

**Risk**: Progress notifications require a `progressToken` from the client;
handlers must degrade gracefully when absent. SSE transport
(`src/mcp/http.rs`) must actually flush heartbeats.

---

## Out of Scope (carried forward)

- UAR consumer resync (standing `task-stream-reliability` follow-up).
- Pre-existing `tests/contract_alignment_test.rs:300` compile error.
- TaskStep REST endpoints; REST JWT/header scope extraction.
- Re-ingestion tooling for the `authentic-digital-twin-content` substrate docs.

---

## Per-Change Constraints (from `.kbd-orchestrator/constraints.md`)

- Schema ↔ Struct sync: none of the 3 changes is expected to add a *persisted*
  struct field. If any does, a migration (v20+) is mandatory in the same change.
- No parallel storage layers: reuse existing `MemoryStorage` / `SurrealStorage`.
- One build verification per change set.
- Quality gate before archive: `./scripts/quality-check.sh` (fmt, clippy, test).
- KBD does not commit to git unless explicitly asked — changes stay staged.

---

## Phase Status: PLAN COMPLETE

3 OpenSpec changes scaffolded under `openspec/changes/`. Next step:
`/kbd-execute bound-embedding-model-loading` (then change 2, then change 3).
