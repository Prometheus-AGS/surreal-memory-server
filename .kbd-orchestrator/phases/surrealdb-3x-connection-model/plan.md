# Plan — surrealdb-3x-connection-model

Child of `surrealdb-connection-architecture`. Evidence in `assessment.md`.

## Rules for every change

- 3.3.0 only: `surrealdb` client 3.3.0, SurrealDB server 3.3.0. No older
  client or server is used for comparison.
- Load-harness runs use a fresh scratch server from the launch agent's binary
  (`~/.prometheus/bin/surreal-3.3.0`) on its own port and data dir. Never the
  shared `:28000`.
- A change is complete only when its gate below is met by a harness run on
  the change's own commit. Compiling is not a gate.
- Record every gate run in `crates/surreal-memory/tests/load_repro_baseline.md`
  with the commit and host load.
- `MemoryStorage` trait surface does not change. Any public signature change
  needs explicit sign-off first.

## Changes

### c1 — shared-session — COMPLETE (`de3582e`)
`ConnectionCell::Connected` holds `Arc<Surreal<Any>>`; `live_db()` returns the
`Arc` instead of calling `Surreal::clone()`.
**Gate met**: `signin` per run ~2,700 → ~24; server `hybrid_search × 64` p50
2.3–3.0 s → 14 ms. Did not change the reset rate (E2).

### c2 — connection-reset-cause — COMPLETE (`d0caa4c`)
**Cause**: `search_memories` fetched the whole scoped table with embeddings;
responses passed the SDK's 64 MiB WebSocket limit and the SDK dropped the
socket. **Fix**: one scoped KNN query on `memory_embedding_hnsw`.
**Gate met**: server mixed 0 errors (was ~2,200/3,200), 1 WebSocket per run
(was 20–24); embedded mixed 0 timeouts (was 170–610).

Original scope: find why the WebSocket is torn down ~23 times per mixed-load
run, then fix the cause.
1. Capture the full client-side error chain and SDK tracing
   (`surrealdb=debug`) for one mixed run, plus a packet-level view of which
   side sends the reset.
2. Test candidates one at a time against that evidence: SDK WebSocket
   frame/message limits, pending-request or channel capacity, ping/timeout
   handling, the mixed interleaving itself.
3. Fix the cause. Do not paper over it with retries (c3 comes after).
**Gate**: mixed 50/50 × 128 in server mode shows 0 `connection` errors and one
WebSocket per run in the server log.

### c8 — ledger-cancellation-recovery — COMPLETE (`cb8e7e1`)
**Cause**: every operation-ledger call cloned the SDK handle (11 sites in
`src/operations.rs`); in SDK 3.3.0 each clone replays attach/signin/use one
acknowledged round trip at a time before its query, so post-deadline calls
missed their budget (`operation_query_deadline` 503/500, 3/3 before the fix).
Diagnosis: `.agent-team/memory-core/reviews/c8-ledger-diagnosis.md`.
**Fix A**: ledger calls borrow the connection's handle.
**Gate**: `operation_query_deadline` 16/20 at host load ~35–70, then 80/80 at
load ~12–16 (with response bodies printed on failure); operations unit tests
17/17; `executor_recovery` 4/4. Residual failures under heavy load are
consistent with the 10 ms test budget, not confirmed by a captured body.
**Not in c8 (open decision)**: Fix B, replacing the ledger transport only on
connection-class errors rather than on a plain deadline, which changes the
archived `operation-ledger-connection-recovery` spec.

### c3 — retry-coverage — COMPLETE (`c3a1e72`, `b371ce8`; re-scoped: survive 3.3 transaction conflicts)
**Cause**: SurrealDB 3.3 rejects writes to HNSW-indexed tables with a typed,
retryable `TransactionConflict`; most writes did not retry and several never
inspected statement errors (silent write loss).
**Fix**: typed detection (`is_retryable_conflict`, `check_statements`),
`write_retrying_conflicts` helper (16 attempts, 2–64 ms, capped by
`operation_deadline_ms`), ~14 write sites; `add_memory`/`update_memory` as
single transactions (`update_memory` derives version inside the transaction
after review W1/W2); `delete_entity` in one transaction.
**Gate met**: `conflict_retry` embedded failed 3/3 before, passes after with
8/24/7 retries and exact counts and consecutive versions; server stress
before: `delete_memory` conflicts ~every 30 pairs, after: 0 errors (79 and 8
retries in two runs); full workspace quality run clean at `b371ce8`.
Review: `.agent-team/memory-core/reviews/c3-c8-findings.md` (PASS-WITH-WARNINGS,
W1/W2/S1 fixed; S2–S6 noted).
Design: `.agent-team/memory-core/reviews/c3-conflict-retry-design.md`.
Route all 44 direct `live_db()` operations through `retry_operation`, so every
operation gets typed retry, the operation deadline and the embedded in-flight
permit. Note: after c2 the embedded mixed-load timeouts are already 0, so this
change is now about consistent failure handling rather than a measured
failure; re-check its value against the harness before starting.
**Gate**: embedded mixed 50/50 × 128 shows 0 `timeout` errors, or honest
backpressure (bounded queueing) rather than SDK query timeouts; no regression
in server mode.

### c4 — palace-session (needs sign-off)
Stop `palace/context.rs` from cloning a session per palace operation. Requires
changing `PalaceAdapter::new` to accept a shared handle. Get sign-off before
starting; coordinate with the UAR vendored copy.
**Gate**: palace workload shows `signin` count independent of operation count.

### c5 — session-pool-decision
Carried from parent change-5. With c2 and c3 measured, and the deployment-shape
answer (vertical vs horizontal), either build a read/write session pool or
document the deployment shape in `docs/DEPLOYMENT.md`.
**Gate**: if built, write-heavy load no longer raises read p99; if documented,
the doc states the measured limits.

### c6 — docs-3x-model
Correct `CLAUDE.md` Gotcha #6, `AGENTS.md` and `docs/lessons.md`: in SDK 3.x a
`Surreal` clone is a new session; share the connected handle instead.
**Gate**: no remaining "clone per task" guidance in the repo.

### c7 — closing-evidence
Full harness, both modes, two rounds, on a quiet host (1-min load < 10).
**Gate**: results recorded; phase goals met or gaps listed.

## Order

c2 → c3 → (c4 if signed off) → c5 → c6 → c7. c6 may land any time; it has no
code dependency.
