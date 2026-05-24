# Plan — surrealdb-connection-architecture

**Date**: 2026-05-24
**Source assessment**: [`assessment.md`](./assessment.md)
**Change backend**: OpenSpec (detected via `openspec/` directory)
**Single OpenSpec change**: `fix-surrealdb-connection-architecture`

This plan does not wait on the open questions from §6 of the assessment to
start work — the highest-confidence root cause (Finding 1: blocking
`std::sync::RwLock` in async hot path) is structural and fixing it does not
depend on those answers. The questions instead shape **Change 5** (workload
isolation) and **Change 6** (embedded-mode framing). Those two later changes
are sequenced after a checkpoint so the user can answer in flight.

## Reference skills (now installed under `~/.agents/skills/`)

Plan authors and executors should consult these whenever touching SurrealDB:

| Skill | When to use |
|---|---|
| `surrealdb-expert` (`~/.agents/skills/surrealdb-expert/SKILL.md`) | Architecture, connection pooling, permissions, RBAC, performance patterns. Pattern 3 in this skill explicitly recommends a connection pool and warns against per-request connect — the *exact* axis of this phase. |
| `surrealql` (`~/.agents/skills/surrealql/SKILL.md` + `references/`) | Authoring any SurrealQL — schema, indexes, queries, transactions. Confirms that `surrealql` is not ANSI-SQL; assume nothing. |
| `surrealdb-vector` (`~/.agents/skills/surrealdb-vector/SKILL.md`) | HNSW index design (DIMENSION, DIST, EFC, M, M0), KNN queries with `<\|K, EF\|>`, scored results. Touches migrations v5 (1536d memory/entity) and v16 (384d palace). |

There is no `surrealdb-export` skill in the official SurrealDB skill set
(only `surrealql`, `surrealdb-vector`, `surrealdb-python`). The CLAUDE.md /
AGENTS.md updates in **Change 7** reference the three that exist plus the
`surrealdb-expert` third-party skill installed locally.

## Ordered change list

Each change is small, independently verifiable, and reversible.

### Change 1 — Add a deterministic load-repro harness (FIRST)
**Why first**: per §4 of the assessment, "without a repro, fixes are
unfalsifiable." Establishes a measurable baseline before any architectural
change.

**Scope**:
- New `crates/surreal-memory/tests/load_repro.rs` (integration test, gated
  behind `--ignored` so it does not run in CI by default).
- Spawns N concurrent tasks each issuing `hybrid_search_memories` /
  `add_memory` / `add_to_task_stream` against a real server-mode SurrealDB
  (via `testcontainers` or an env-var endpoint) **and** a separate run
  against embedded mode.
- Captures p50/p95/p99 latency, error counts, error classes.
- Optional: `tokio-console` instrumentation behind a feature flag.

**Done when**: harness reproduces the user-reported symptom signature on at
least one of the two modes, with numbers checked into a baseline file
(`crates/surreal-memory/tests/load_repro_baseline.md`). If it does NOT
reproduce, that itself is a finding — escalate before Change 2.

**Agent**: `gan-evaluator` for the harness scaffold; `rust-reviewer` for the
test code.

**Spec impact**: none (test-only).

---

### Change 2 — Atomic-swap connection handle (`ArcSwap<Arc<Surreal<Any>>>`)
**Why second**: directly addresses Finding 1 (CRITICAL) and Finding 2 (HIGH).
Removes the blocking lock from the hot path and from `palace_adapter`.

**Scope**:
- Replace `connection: Arc<std::sync::RwLock<ConnectionState>>` at
  [`surreal.rs:44`](crates/surreal-memory/src/storage/surreal.rs:44) with
  `connection: Arc<arc_swap::ArcSwap<ConnectionCell>>` (or
  `tokio::sync::watch<Arc<ConnectionCell>>`). The cell is a small enum
  matching today's `ConnectionState` but stored behind an atomic swap.
- Hot path: `let cell = self.connection.load_full();` — a single atomic load,
  no contention.
- Cold path (`reconnect_with_attempts`): build the new cell, then
  `self.connection.store(new)`. The `ReconnectGuard` cancellation-safety
  pattern is preserved.
- Remove all 47 `connection.read()` / `connection.write()` call sites in
  favor of a single helper `fn live_db(&self) -> Result<Surreal<Any>>`.
- Remove `pub(crate) fn connection_arc()` in favor of a typed
  `ConnectionView` handed to `PalaceAdapter` (still cheap, still resilient
  to swap).

**Add**: `arc-swap = "1"` to `crates/surreal-memory/Cargo.toml`.

**Constraint check (from `constraints.md`)**: no schema change, no migration
needed, no field added to any persisted struct. The `MemoryStorage` trait
surface does not change. UAR ripple is zero.

**Done when**:
1. `cargo check --workspace --features embedded,metal` passes.
2. `cargo clippy --all-targets --features embedded,metal -- -D warnings` is clean.
3. Existing `crates/surreal-memory/tests/` tests pass unchanged.
4. The Change-1 load harness re-runs and shows ≥3× p99 improvement on at
   least one workload (server mode under concurrent hybrid_search).
5. `await_holding_lock` clippy lint is enabled at deny level for the crate
   so future regressions are caught at build time.

**Agent**: `architect` for the design sketch; `rust-reviewer` for the diff;
`rust-build-resolver` if compile errors stack up.

**Spec impact**: none (internal refactor, no behavior change).

---

### Change 3 — Pass `Config::query_timeout` to the SDK at connect
**Why third**: Finding 6. Pushes the deadline into the protocol layer so a
hung server-side query cannot stall the multiplexed socket for the full
application deadline.

**Scope**:
- In both `connect_with_attempts` and `connect_surreal_for_repair`, build a
  `surrealdb::opt::Config::default().query_timeout(...)` and pass
  `(endpoint, config)` to `surrealdb::engine::any::connect(...)`.
- New `RetryConfig` field: `query_timeout_ms: u64` (default 10_000). Add
  env var `SURREAL_QUERY_TIMEOUT_MS`. Document the relationship: the
  application `operation_deadline_ms` must be ≥ `query_timeout_ms` for
  retries to have a chance to fire (already noted in the comment at
  `surreal.rs:69-80` — extend that comment).
- Update tests in `src/main.rs:428-510` to cover the new env var.

**Done when**: `cargo test` passes; load harness shows that a synthetic slow
query is bounded by the SDK timeout, not the application deadline.

**Agent**: `rust-reviewer`.

**Spec impact**: documentation only (env var added to README and CLAUDE.md).

---

### Change 4 — Typed-error retry classification
**Why fourth**: Finding 5. Replaces the brittle substring matcher at
[`surreal.rs:814-850`](crates/surreal-memory/src/storage/surreal.rs:814).

**Scope**:
- New enum `RetryAction { Retry, Reconnect, FailFast }` in
  `crates/surreal-memory/src/storage/retry.rs`.
- New `fn classify(err: &anyhow::Error) -> RetryAction` that downcasts to
  `surrealdb::Error` and discriminates on variants (`Db`, `Api`, etc.).
  Falls back to a *narrower* substring path only for non-typed sources
  (e.g. anyhow wraps that lose the cause chain).
- `retry_operation` calls `classify(...)` instead of `is_retriable_error`.
- Reconnect is **only** triggered for `Reconnect` actions, not for any
  retriable error. (Today a "lock timeout" error triggers a reconnect — wrong.)
- Add unit tests for each `surrealdb::Error` variant the classifier handles.

**Done when**: tests pass; the load harness shows that transient
server-busy errors back off without churning a reconnect.

**Agent**: `rust-reviewer`.

**Spec impact**: none.

---

### Change 5 — Workload-isolated sessions for server mode (CONDITIONAL)
**Why fifth**: Finding 3. Mitigates head-of-line blocking on the single
multiplexed WebSocket.

**Decision gate**: depends on user answer to assessment §6.1 (vertical vs.
horizontal scale target).

- **If vertical-scale single binary**: implement N cloned `Surreal<C>`
  sessions (per SurrealDB multi-tenancy doc — clones share the physical
  connection while giving independent sessions). Route writes (heavy:
  `compress_memories`, `auto_summarize_task_stream`, mindmap updates) to
  one bucket; reads (hybrid_search) to another.
- **If horizontal-scale behind LB**: defer this change; document the
  recommended deployment shape in `docs/DEPLOYMENT.md` instead and rely
  on multiple processes for isolation. Single shared client per process
  is then correct.

**Scope (if implemented)**:
- New `crates/surreal-memory/src/storage/sessions.rs` with a small
  `SessionPool { read: Vec<Surreal<Any>>, write: Vec<Surreal<Any>> }`.
  Round-robin via `AtomicUsize`. Each entry is a `Surreal::clone()` of the
  primary handle.
- Hot-path helper updated: `live_read_db()` and `live_write_db()`.
- Configurable `SURREAL_READ_SESSIONS` (default 4),
  `SURREAL_WRITE_SESSIONS` (default 2).

**Done when**: load harness shows write-heavy workload no longer increases
read p99.

**Agent**: `architect` then `rust-reviewer`.

**Spec impact**: env vars documented; no schema change.

---

### Change 6 — Embedded-mode concurrency ceiling + clear framing
**Why sixth**: Finding 4. Makes embedded mode honest about its concurrency
ceiling instead of pretending it scales like server mode.

**Scope**:
- New `tokio::sync::Semaphore` (size from `SURREAL_EMBEDDED_MAX_INFLIGHT`,
  default 16 — matches RocksDB stripe count) created **only** when
  `SurrealMode::Embedded`. Hot-path helper acquires a permit before issuing
  the operation; releases on completion.
- Update `CLAUDE.md`, `AGENTS.md`, and `README.md` to label embedded mode
  as *"single-process, single-tenant; use server mode for concurrent
  multi-agent workloads"*. (Done as part of Change 7's doc edits.)
- Remove the implicit promise of horizontal-scale parity for embedded mode
  from any doc that suggests it.

**Done when**: load harness against embedded mode no longer surfaces
"serialization failure" / "lock timeout" errors under bounded concurrency;
errors instead manifest as honest backpressure (slow but not failing).

**Agent**: `rust-reviewer`.

**Spec impact**: none.

---

### Change 7 — Documentation: SurrealDB skill references in CLAUDE.md & AGENTS.md
**Why last**: locks in the operational learnings so future sessions reach
for the right skill at the right time. Touches no code.

**Scope** (edits):
- **CLAUDE.md** — new top-level section "SurrealDB Skill References" listing:
  - `surrealdb-expert` — connection management, permissions, RBAC, performance
  - `surrealql` — DDL/DML authoring, transactions, control flow (note: NOT ANSI-SQL)
  - `surrealdb-vector` — HNSW index tuning (touches migrations v5 and v16)
  - Plus a one-line note that no `surrealdb-export` skill exists in the
    official SurrealDB skill set today; use `surreal export` CLI for backups.
- **CLAUDE.md** — append to "SurrealDB Gotchas" section:
  - "**Connection handle is already `Arc`-wrapped and clone-safe.** Do NOT
    wrap `Surreal<Any>` in a `std::sync::RwLock` or `Mutex` — clone it per
    task. See `crates/surreal-memory/src/storage/surreal.rs` for the
    atomic-swap pattern used here." (post-Change 2)
  - "**Embedded mode has a much lower concurrency ceiling than server mode.**
    The embedded path is bounded by an internal semaphore; for multi-agent
    workloads, use server mode."
- **AGENTS.md** — mirror the same two additions for Codex consumers.
- **AGENTS.md** — fix existing inconsistencies surfaced during the assessment:
  - It still says "v1–v8" migrations while CLAUDE.md says "v1–v16". Sync to v16+.
  - It does not mention the `palace` feature or `Memory Palace` storage path.
    Mirror what CLAUDE.md says, since Codex hits the same code.

**Done when**: both files updated; a quick grep confirms no stale "v8" /
"v16" mismatch; the skill-reference section appears in both.

**Agent**: `doc-updater`.

**Spec impact**: documentation only.

---

## Sequencing and parallelism

```
Change 1 (harness)
   │
   ▼
Change 2 (ArcSwap)  ← critical fix; must precede any "did it work?" measurement
   │
   ├─► Change 3 (query_timeout)        ─┐
   ├─► Change 4 (typed errors)          ├─ can run in parallel after Ch 2
   └─► Change 7 (docs)                  ─┘
   │
   ▼
[USER CHECKPOINT — answer §6 open questions]
   │
   ├─► Change 5 (sessions)              ─┐ conditional
   └─► Change 6 (embedded semaphore)    ─┘ both verified via harness
```

Changes 3, 4, and 7 do not interfere with each other and can be dispatched in
parallel after Change 2 lands.

## Risk register

| Risk | Mitigation |
|---|---|
| `ArcSwap` refactor touches 47 sites; mass-edit could break subtle invariants | Land via single PR with the load harness as gate; require `rust-reviewer` sign-off; do not bundle other changes |
| Removing `connection.read().expect("poisoned")` may hide existing latent panic recovery | Add a `tracing::error!` in the helper for any non-`Connected` state instead of silently swallowing |
| `arc-swap` adds a new dependency to a library crate (UAR ripple) | `arc-swap` is widely used, no transitive surprises; pin the version; document in CLAUDE.md |
| Query timeout too aggressive => false failures on legitimately slow queries | Default 10s is conservative for an MCP server; expose env var; document the relationship to `operation_deadline_ms` |
| Change 5 introduces split-brain (write to session A, read from session B before commit visible to B) | Cloned sessions share the same physical connection per SurrealDB docs — visibility is the same. Confirm with an integration test before merging |
| Embedded semaphore size of 16 is a guess on user hardware | Make it configurable; default matches RocksDB stripe count; instrument with `tracing::debug!` on permit-wait |

## Verification commands per change

Each change must pass before the next starts:

```bash
# After every change (per CLAUDE.md operational rule §3 — run once):
cargo check --workspace --features embedded,metal
./scripts/quality-check.sh

# After Change 1: baseline load report exists
cat crates/surreal-memory/tests/load_repro_baseline.md

# After Change 2: clippy gate
cargo clippy --all-targets --features embedded,metal -- -D warnings -D clippy::await_holding_lock

# After Change 5/6: re-run load harness, diff against baseline
cargo test --test load_repro --release -- --ignored
```

## Open questions still in flight (from assessment §6)

These do not block Changes 1–4 and 7. They DO gate Changes 5 and 6 shape:

1. Vertical vs. horizontal scale target?
2. Is embedded mode a production target or dev-only?
3. Tolerance for `MemoryStorage` trait-surface change (typed errors)?
4. Captured logs of actual timeout/sync errors?

The plan executor should pause at the checkpoint, surface these to the user,
then proceed.

## Next step

Run `/kbd-execute fix-surrealdb-connection-architecture` to dispatch
Change 1 (load harness). The OpenSpec change file is at
[`openspec/changes/fix-surrealdb-connection-architecture/proposal.md`](../../../openspec/changes/fix-surrealdb-connection-architecture/proposal.md).
