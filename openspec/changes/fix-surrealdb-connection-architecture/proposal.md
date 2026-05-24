# Proposal — fix-surrealdb-connection-architecture

**Change ID**: `fix-surrealdb-connection-architecture`
**Phase**: `surrealdb-connection-architecture`
**Created**: 2026-05-24
**Status**: draft
**Authors**: kbd-plan (Claude Code, surreal-memory-server)

## Why

`SurrealStorage` wraps `Surreal<Any>` in
`Arc<std::sync::RwLock<ConnectionState>>` and uses that lock at 47 hot-path
sites across async code. Under load this produces the user-reported symptoms:

- Server-mode timeouts (writer starvation + head-of-line blocking on the
  single multiplexed WebSocket + no SDK-level query timeout).
- Embedded-mode "synchronization errors" (application lock layered on top
  of RocksDB's stripe locks; transient errors classified as retriable then
  loop on the contended path).
- Hardware sensitivity (contention-driven, not workload-driven).

SurrealDB's own Rust SDK guidance and `surrealdb-expert` skill both say:
`Surreal<C>` is internally `Arc`-wrapped and clone-safe — do not wrap it in
another lock; clone the handle per task; use a pool of cloned sessions for
workload isolation.

Tuning the existing retry/timeout knobs is **not a fix** — it pushes the
threshold without removing the contention. This phase makes the connection
topology match SurrealDB's recommended shape.

## What changes

Seven sequenced, independently verifiable changes:

1. **Load-repro harness** — establishes a measurable baseline.
2. **`ArcSwap` connection cell** — removes blocking lock from hot path.
3. **`Config::query_timeout`** — SDK-level per-query deadline.
4. **Typed-error retry classification** — `surrealdb::Error` variants, not strings.
5. **Workload-isolated sessions** (conditional on deployment-shape answer).
6. **Embedded-mode semaphore + honest documentation framing**.
7. **CLAUDE.md / AGENTS.md updates** referencing the SurrealDB agent skills
   (`surrealdb-expert`, `surrealql`, `surrealdb-vector`) and the connection-
   handling rule.

Full design rationale lives in
[`.kbd-orchestrator/phases/surrealdb-connection-architecture/plan.md`](../../../.kbd-orchestrator/phases/surrealdb-connection-architecture/plan.md).

## Impact

- **Public API surface**: unchanged. `MemoryStorage` trait does not change.
  UAR ripple = zero (verify at Change 4 if typed errors leak through; if so,
  pause and escalate before merging).
- **Schema / migrations**: **none**. No persisted struct gains a field.
  Per the project's blocking constraint (`constraints.md`), this is checked
  at each change.
- **Dependencies**: adds `arc-swap = "1"` to `crates/surreal-memory`.
- **Configuration**: adds env vars `SURREAL_QUERY_TIMEOUT_MS`,
  `SURREAL_READ_SESSIONS`, `SURREAL_WRITE_SESSIONS`,
  `SURREAL_EMBEDDED_MAX_INFLIGHT`. All have safe defaults.
- **Performance**: expected ≥3× p99 improvement on concurrent
  `hybrid_search_memories` (measured via Change 1 harness, gating Change 2).
- **Risk**: medium — touches 47 call sites in a hot path. Mitigated by
  test-first sequencing (harness before refactor) and isolated PRs per change.

## Out of scope

- Multi-node SurrealDB clustering / horizontal scale of the DB itself.
- Replacing RocksDB with TiKV / FoundationDB.
- Re-architecting `Memory Palace` (`palace` feature) beyond the lock-pattern
  cleanup it inherits from Change 2.
- Changing the `MemoryStorage` trait surface (typed errors stay internal).

## Open questions (do not block start)

See [`assessment.md` §6](../../../.kbd-orchestrator/phases/surrealdb-connection-architecture/assessment.md).
Required before Changes 5 and 6 land; Changes 1–4 and 7 proceed immediately.
