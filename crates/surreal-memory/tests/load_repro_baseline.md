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
