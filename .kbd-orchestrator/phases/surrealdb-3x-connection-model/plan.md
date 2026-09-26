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

### c2 — connection-reset-cause — NEXT
Find why the WebSocket is torn down ~23 times per mixed-load run, then fix the
cause.
1. Capture the full client-side error chain and SDK tracing
   (`surrealdb=debug`) for one mixed run, plus a packet-level view of which
   side sends the reset.
2. Test candidates one at a time against that evidence: SDK WebSocket
   frame/message limits, pending-request or channel capacity, ping/timeout
   handling, the mixed interleaving itself.
3. Fix the cause. Do not paper over it with retries (c3 comes after).
**Gate**: mixed 50/50 × 128 in server mode shows 0 `connection` errors and one
WebSocket per run in the server log.

### c3 — retry-coverage
Route all 44 direct `live_db()` operations through `retry_operation`, so every
operation gets typed retry, the operation deadline and the embedded in-flight
permit.
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
