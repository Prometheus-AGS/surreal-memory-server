# Assessment — surrealdb-3x-connection-model

**Parent phase**: `surrealdb-connection-architecture`
**Date**: 2026-09-26
**Stack under test**: `surrealdb` client 3.3.0 (`ad97c9c`), SurrealDB server 3.3.0
(launch agent `ai.prometheus.surrealdb-native`, and scratch servers from the
same binary on `127.0.0.1:28117`).

## Why a child phase

The parent phase was planned around one defect: a blocking
`std::sync::RwLock` around the connection handle. Measurement on 3.3.0 shows
the connection model has different, larger problems, and the parent's
"clone `Surreal<Any>` per task, it is cheap" premise is false for the 3.x SDK.
This phase re-assesses from measured evidence and gives every change a
load-harness gate.

## Evidence (all from `crates/surreal-memory/tests/load_repro.rs`)

### E1 — Every call opened a new server-side session (fixed by c1)

Server debug log, one mixed-load run before c1: on a single WebSocket the
server processed ~2,700 each of `attach`, `signin`, `use`, `query`, `detach`.
Cause: `live_db()` returned `db.clone()`, and in SDK 3.x
`impl Clone for Surreal<C>` calls `clone_session` (new session: attach, replay
signin and `use`) while `Drop` sends a detach
(`surrealdb-3.3.0/src/lib.rs:337-356`). So every storage call cost five round
trips and a root password verification.

After c1 (`de3582e`, `live_db()` returns a shared `Arc<Surreal<Any>>`):
`signin` fell to ~24 per run and server-mode `hybrid_search × 64` p50 fell from
2.3–3.0 s to 14 ms.

### E2 — Mixed load still resets the WebSocket (open, c2)

| run | mixed 50/50 × 128 errors (server mode) | sample |
|---|---|---|
| before c1, run 1 | 2172 / 3200 | `Connection reset` |
| before c1, run 2 | 2165 / 3200 | `Connection reset` |
| after c1, run 1 | 2324 / 3200 | `Connection reset` |
| after c1, run 2 | 2182 / 3200 | `Connection reset` |

c1 did not change the reset rate, so session churn was not the cause. After c1
the server saw **24 WebSocket connections per run**: one long-lived and 23
short-lived, each short one doing `attach`/`signin`/`use`. So the socket is
torn down ~23 times per run and re-established. The server logs each `/rpc`
request as finished normally with no error at debug level, so the teardown is
observed client-side. Read-only and write-only workloads at ×64 never reset.
Cause unknown; candidates to test, not assume: SDK WebSocket
limits (frame/message size, pending-request or channel capacity), client-side
ping/timeout handling, or something in the mixed interleaving itself.

### E3 — Only one call site uses the retry path (open, c3)

`crates/surreal-memory/src/storage/surreal.rs` has 1 `self.retry_operation(`
call (`create_record`) and 44 direct `self.live_db()` calls. The typed retry,
operation deadline and embedded in-flight semaphore from parent Changes 2–4 and
6 therefore do not apply to `add_memory`, `hybrid_search_memories` or most
other operations. Embedded-mode mixed-load timeouts
(`The query was not executed because it exceeded the timeout: 10s`: 610, 170,
390 of 3200 across runs) occur on paths the semaphore never guards.

### E4 — Palace adapter still clones a session per call (open, c4)

`PalaceAdapter::new` takes `Fn() -> Result<Surreal<Any>>` and
`palace/context.rs` now returns `(**db).clone()`, which opens a session per
palace operation. Fixing it changes a public signature
(`PalaceAdapter::new`), so it needs sign-off (UAR vendors this crate).

### E5 — Repo guidance is wrong for 3.x (open, c6)

`CLAUDE.md` Gotcha #6, `AGENTS.md` and `docs/lessons.md` (2026-05-24 entry)
say to clone `Surreal<Any>` per task. In 3.x that opens a session per clone.

## Measurement caveats

Host load during runs ranged from ~17 to ~250 (10 cores). Error counts and
error classes are reliable; latency columns are only comparable within a run.
c7 exists to produce clean closing numbers on a quiet host.

## Open decision (carried from the parent's assessment §6.1)

Deployment shape — one process scaled vertically vs several processes behind a
load balancer — decides whether c5 builds a read/write session pool or only
documents the deployment. Decide after c2 and c3 have landed and been measured.
