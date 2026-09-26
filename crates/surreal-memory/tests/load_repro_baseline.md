# Load-repro baseline — surrealdb-connection-architecture

Each test run in `tests/load_repro.rs` appends a section to this file with
the current commit's measurements. New sections at the bottom; never edit
or delete old ones — diffs across commits are how we know the refactor
actually moved the needle.

## How to run

```bash
# 1. Start the docker-compose SurrealDB (server-mode tests)
docker-compose up -d surrealdb

# 2. Server-mode workloads
cargo test --test load_repro --features embedded,metal --release -- --ignored server_mixed_load

# 3. Embedded-mode workloads (uses a temp RocksDB path; auto-cleaned)
cargo test --test load_repro --features embedded,metal --release -- --ignored embedded_mixed_load

# 4. Both:
cargo test --test load_repro --features embedded,metal --release -- --ignored
```

## What's measured

Three workloads × two modes:

| Workload | Concurrency | Ops/task | Description |
|---|---|---|---|
| `hybrid_search × 64` | 64 | 25 | Read-only pressure on `hybrid_search_memories` |
| `add_memory × 64` | 64 | 25 | Write-only pressure on `add_memory` |
| `mixed 50/50 × 128` | 128 | 25 | Interleaved reads + writes; the head-of-line-blocking workload |

Errors are bucketed into: `timeout`, `lock`, `serialization`, `connection`,
`other`. The mix of these classes is the *signature* of the architectural
defects from the assessment (§2). After Change 2 lands, the same workloads
must show a materially different signature — that is the gate.

## Pass criteria for Change 2 (ArcSwap refactor)

- `mixed 50/50 × 128` server-mode p99 drops by ≥3× vs. baseline.
- `mixed 50/50 × 128` embedded-mode `lock` / `serialization` error count
  drops to **zero** under the same bounded concurrency.

If either fails, Change 2 is not done.

---

<!-- Append new ## sections below this line. Do not edit older sections. -->

## Context for the 2026-09-26 runs

- **Code**: `HEAD` at `ad97c9c` (post-Change-2: `ArcSwap` connection cell,
  typed retry, embedded semaphore) with the `surrealdb` client pinned to 3.3.0.
- **Server mode**: a fresh `surreal` 3.3.0 scratch server (same binary as the
  launch agent) on `127.0.0.1:28117` with its own RocksDB dir, restarted per
  run. Never the shared `:28000`.
- **No pre-refactor baseline exists.** The harness landed in the same commit as
  the Change-2 fix (`7072370`), and the pre-fix code uses the 3.0.5 client,
  which this project no longer supports. Change-2's "≥3× p99" gate therefore
  cannot be evaluated and is recorded as **unproven**, not passed.
- **Host was saturated** (1-min load 110–250 on 10 cores), so latency columns
  are not comparable to future runs on an idle host. Error counts and error
  classes are the reliable signal. Run 2 added a first-message sample per
  error class.

## server-mode (surrealdb 3.3.0 client + server, run 1) — 2026-09-26T09:29:33.236375+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 3047 | 4274 | 4882 | 0 | — |
| add_memory × 64 | 1600 | 10538 | 16781 | 18521 | 0 | — |
| mixed 50/50 × 128 | 3200 | 4455 | 69727 | 158016 | 2172 | connection:2172 |

## embedded mode (surrealdb 3.3.0 client + server, run 1) — 2026-09-26T09:36:38.649402+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 3 | 9 | 53 | 0 | — |
| add_memory × 64 | 1600 | 942 | 4525 | 7194 | 0 | — |
| mixed 50/50 × 128 | 3200 | 11466 | 28206 | 40050 | 610 | timeout:610 |

## server-mode (surrealdb 3.3.0 client + server, run 2) — 2026-09-26T09:50:14.514707+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 2286 | 2916 | 3566 | 0 | — |
| add_memory × 64 | 1600 | 7776 | 14036 | 14966 | 0 | — |
| mixed 50/50 × 128 | 3200 | 3881 | 35928 | 40118 | 2165 | connection:2165 |

- mixed 50/50 × 128 `connection` sample: Connection reset

## embedded mode (surrealdb 3.3.0 client + server, run 2) — 2026-09-26T09:54:31.776158+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 4 | 14 | 17 | 0 | — |
| add_memory × 64 | 1600 | 635 | 2155 | 2963 | 0 | — |
| mixed 50/50 × 128 | 3200 | 6520 | 17290 | 21084 | 170 | timeout:170 |

- mixed 50/50 × 128 `timeout` sample: The query was not executed because it exceeded the timeout: 10s

## Findings (2026-09-26)

1. **Server mode, mixed 50/50 × 128: ~68% of operations fail** (2172 and 2165
   of 3200 across two runs), all classed `connection`; the first sampled
   message is `Connection reset`. Read-only and write-only workloads at 64
   concurrency see zero errors, so the failure is specific to concurrent mixed
   read/write load. The scratch server logged nothing at `warn`, so the reset
   is observed client-side. Unconfirmed cascade hypothesis for a follow-up
   change: `retry_operation_inner` calls `live_db()?` before the retry loop
   handles errors, so while one task holds the cell in `Reconnecting` every
   concurrent operation fails immediately, and concurrent reconnects are not
   single-flighted (each replaces the shared handle and drops in-flight queries).
2. **Embedded mode, mixed 50/50 × 128: 19% then 5% of operations time out**
   (610, then 170 of 3200), sample `The query was not executed because it
   exceeded the timeout: 10s` — the SDK `query_timeout`, not the 30 s
   operation deadline. Zero `lock` / `serialization` errors, which meets
   Change-2's embedded criterion.
3. **No hangs.** Every run completed within its 20-minute cap. (A pre-fix
   build did stall for 1h48m, but it ran the 3.0.5 client against the 3.3.0
   server, so that result is confounded and is not used as evidence.)

Change 1 is closed on these measurements. Findings 1 and 2 are follow-up
work; this change does not modify production code.

## Context for the child phase `surrealdb-3x-connection-model`, c1 runs

- **Code**: `de3582e` (`live_db()` shares one `Arc<Surreal<Any>>` instead of
  opening an SDK session per call). Scratch 3.3.0 server at `--log=debug`.
- **Host load** at start: 18.8 (run 1), 90.3 (run 2). Run 2 embedded mode was
  stopped early; it would not change the conclusions.
- **Result**: `signin` per run ~2,700 → ~24; server `hybrid_search × 64` p50
  2.3–3.0 s → 14 ms. Mixed-load `Connection reset` count is unchanged, and the
  server saw 24 WebSocket connections per run (1 long-lived, 23 short). See
  `.kbd-orchestrator/phases/surrealdb-3x-connection-model/assessment.md`.

## server-mode (3.3.0, shared session, run 1) — 2026-09-26T13:55:12.546719+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 14 | 25 | 34 | 0 | — |
| add_memory × 64 | 1600 | 5128 | 13286 | 17100 | 0 | — |
| mixed 50/50 × 128 | 3200 | 1120 | 45749 | 96606 | 2324 | connection:2324 |

- mixed 50/50 × 128 `connection` sample: Connection reset

## embedded mode (3.3.0, shared session, run 1) — 2026-09-26T13:58:46.470320+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 3 | 7 | 13 | 0 | — |
| add_memory × 64 | 1600 | 571 | 2126 | 3139 | 0 | — |
| mixed 50/50 × 128 | 3200 | 5278 | 15896 | 22667 | 390 | timeout:390 |

- mixed 50/50 × 128 `timeout` sample: The query was not executed because it exceeded the timeout: 10s

## server-mode (3.3.0, shared session, run 2) — 2026-09-26T14:07:03.315030+00:00

| workload | n | p50_ms | p95_ms | p99_ms | err_total | err_breakdown |
|---|---|---|---|---|---|---|
| hybrid_search × 64 | 1600 | 14 | 18 | 24 | 0 | — |
| add_memory × 64 | 1600 | 4860 | 13702 | 17357 | 0 | — |
| mixed 50/50 × 128 | 3200 | 5683 | 45762 | 86118 | 2182 | connection:2182 |

- mixed 50/50 × 128 `connection` sample: Connection reset
