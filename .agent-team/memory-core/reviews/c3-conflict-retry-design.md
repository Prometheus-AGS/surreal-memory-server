# c3 design: every storage write survives SurrealDB 3.3 retryable transaction conflicts

Role: memory-storage (design only, nothing implemented). KBD child `surrealdb-3x-connection-model`,
change `c3-retry-coverage`. Its recorded title is "Route every storage operation through the retry,
deadline and embedded-permit path". The lead has re-scoped it to conflict survival. This design
**deliberately does not** put every operation behind the embedded permit (see section 4). The re-scope
should be recorded through KBD tooling. I did not edit the generated projection.

Branch/HEAD read: `feat/surrealdb-3x-connection-model` @ `c7b2b34`. All line numbers below refer to
the working tree at that commit.

Tooling note: the Compass graph (`.compass/verification.json`, source commit `dd7fdcd`) is 50 commits
behind HEAD. `/usr/local/bin/compass callers …` fails with
`snapshot format is unsupported: expected compass.store.graph-index/1, found …/2`. All call-site
evidence below therefore comes from `rg` and direct source reads, not from the graph.

## 0. Evidence corrections and additions

1. **The failing integration test runs in embedded mode, not server mode.**
   `delete_memory_commits_audit_row_and_removal_together` (`tests/integration_test.rs:254-288`) uses
   `make_storage()` (`:37-44`), which calls `SurrealStorage::new_mem` (`surreal.rs:3381-3419`), and
   that opens an embedded RocksDB in a temp directory. So the conflict reproduces in embedded mode as
   well as on the 3.3.0 server. "Resource busy" is RocksDB's `Busy` status text, which is consistent
   with that.
2. **What the core does, in `surrealdb-core-3.3.0/src/dbs/executor.rs`:**
   - Implicit (single-statement) transaction, `:1466-1485`: a commit conflict propagates unwrapped
     and is typed `QueryError::TransactionConflict` (`err/to_types.rs:79-80`).
   - Explicit `BEGIN…COMMIT` whose commit fails, `:1917-1965`:
     - every statement slot is rewritten to `QueryError::NotExecuted`;
     - a dedicated COMMIT row is appended carrying `TransactionConflict`, or `NotExecuted` for other
       commit failures, or **no kind** (`Query(None)`, "commit outcome is unknown") when the outcome
       is indeterminate.
   - An indeterminate outcome must never be retried.
3. **Why `IndexedResults::check()` hides the conflict.** It returns the *lowest-index* error and
   consumes the response (`surrealdb-3.3.0/src/method/query.rs:735-750`). Inside a transaction that
   error is a `NotExecuted` placeholder. `take_errors(&mut self) -> HashMap<usize, Error>` (`:704-718`)
   exposes every statement error. This matches the lead's observation.
4. **The typed details survive every transport we use:**
   - WS query rows: `surrealdb-rpc-3.3.0/src/query.rs:69-106` serialises `kind` and `details` and
     decodes them back into `ErrorDetails`.
   - RPC-level errors (used by SDK `.create()` / `.delete()`) decode via `TypesError::from_value`
     (`surrealdb-rpc-3.3.0/src/response.rs:230`).
   - Embedded converts in-process (`to_types.rs:79`).
   - Client and server are both pinned `=3.3.0`: `Cargo.toml:14-15` and the compose images
     `v3.3.0@sha256:681c…`.
   - I did **not** runtime-verify WS decoding. The stress test in section 5b must prove it.
5. **A precedent already exists.** `add_to_task_stream` (`surreal.rs:2182-2242`) already implements
   a take_errors-based conflict loop:
   - it uses `MAX_TRANSACTION_CONFLICT_RETRIES = 16` (`:92`) and `transaction_conflict_backoff`
     (`:96-99`, 2 ms…64 ms);
   - it has string fallbacks and no deadline;
   - this design generalises that loop and deletes the duplicate.
6. **There is a silent write-loss bug, which is worse than a surfaced conflict.** Several writes
   `.await?` the query and never inspect statement errors. A conflict there returns `Ok(())` and the
   write is lost:
   - `add_memory` history INSERT `:1778-1786`
   - `update_memory` history INSERT `:1839-1848`
   - `delete_entity` `:1613`
   - `delete_relation` `:1660`
   - `delete_memory` fallback DELETE `:1884`
   - `delete_task_stream` mindmap UPDATE `:2388`
   - palace `add_drawer` / `delete_drawer` (`palace/adapter.rs:135-160`, `:167-171`)

## 1. Classification rule (typed)

**Retryable conflict, per error.** An error is retryable when
`err.query_details() == Some(&surrealdb_types::QueryError::TransactionConflict)`. There is no string
matching.

**Retryable conflict, per response (a "definite abort").** A response is retryable only when both
hold:

- at least one statement error is `TransactionConflict`; and
- every other statement error is `TransactionConflict` or `NotExecuted`.

Anything else is surfaced unchanged: a real error such as a schema violation, `AlreadyExists` or
`TimedOut`, and the indeterminate `Query(None)` case. A definite abort guarantees nothing in that
request was applied. That guarantee is why resubmitting the *same request* is safe even for
non-idempotent statements such as counter `+=` or random-id `INSERT`.

Proposed shape, all in `crates/surreal-memory/src/storage/surreal.rs` near `:92-163`:

```rust
use surrealdb_types::QueryError;

fn is_retryable_conflict(err: &surrealdb::Error) -> bool {
    err.query_details() == Some(&QueryError::TransactionConflict)
}

/// Pure, unit-testable: choose the error that represents a failed response.
/// - definite abort (all Conflict/NotExecuted, >=1 Conflict) -> the Conflict error
/// - otherwise the first error that is not a NotExecuted placeholder
/// - if all are NotExecuted (non-conflict commit failure) -> the highest index (the COMMIT row,
///   which carries "Cannot COMMIT: <cause>")
fn select_statement_error(errors: HashMap<usize, surrealdb::Error>) -> surrealdb::Error;

/// Drop-in replacement for `IndexedResults::check()` that inspects every statement.
fn check_statements(mut r: surrealdb::IndexedResults) -> Result<surrealdb::IndexedResults, surrealdb::Error> {
    let errors = r.take_errors();
    if errors.is_empty() { Ok(r) } else { Err(select_statement_error(errors)) }
}
```

**How it fits `RetryAction`** (`:125-134`). Add one typed line at the top of
`classify_surreal_error` (`:138`):
`if is_retryable_conflict(err) { return RetryAction::Retry; }`.

- `Retry` means back off on the same connection without touching the cell, which is exactly the
  right response to a conflict.
- This alone fixes the one existing `retry_operation` caller, `create_record` (`:1329-1366`), whose
  single-statement `.check()` already yields the conflict error.
- `classify_error` (`:1034`) reaches it through `downcast_ref::<surrealdb::Error>()`, which works
  through `.context()` layers.
- Do **not** touch the remaining string matching for transport, timeout and lock. The constraints
  say to preserve connection/retry classification. It is also outside this change.

**Retry loop for direct writes.** One helper, also in `surreal.rs`:

```rust
/// Resubmit ONE request while the database reports a definite conflict abort.
/// `attempt` must send exactly one request that is (a) a single statement, (b) one BEGIN…COMMIT,
/// or (c) several statements that are each idempotent. It must not embed, call other storage
/// methods, or span a read in one request and a write in another.
async fn write_retrying_conflicts<T, F, Fut>(&self, op: &str, mut attempt: F) -> Result<T>
where F: FnMut() -> Fut, Fut: Future<Output = Result<T, surrealdb::Error>>;
```

The loop:

- `Ok` returns.
- On a retryable conflict, `conflicts += 1`. It fails with context
  `"{op}: still conflicting after {n} attempts"` when `conflicts >= MAX_TRANSACTION_CONFLICT_RETRIES`
  or when `now + backoff` would pass `operation_deadline_ms`. Otherwise it emits
  `tracing::debug!(operation, attempt, "retrying write after transaction conflict")` and sleeps
  `transaction_conflict_backoff(conflicts)`.
- Any other error returns immediately as `Err(anyhow::Error::new(e).context(op))`.

The deadline is checked **between** attempts. An in-flight write is never cancelled: cancelling a
write mid-flight makes its outcome unknown. The SDK `query_timeout` (connect-time, `RetryConfig`
`:191-203`) bounds each attempt.

Call-site shape: `self.write_retrying_conflicts("delete_memory", || async { check_statements(db.query(SQL).bind(..).await?) }).await?`.
The closure is non-`move` and borrows the method's locals, with `bind` values cloned inside. The
implementer must confirm the borrow shape compiles.

## 2. Write paths that can hit a conflict

HNSW-indexed tables are `memory` and `entity` (v5 `migrations/mod.rs:197-200`, rebuilt by
`ensure_embedding_indexes` `surreal.rs:770-900`), and palace `drawers` (v16 `migrations/mod.rs:332-334`).
None of these tables has a full-text index.

### A. HNSW tables, and anything in the same request

| # | Method (surreal.rs) | Request(s) that write | Today |
|---|---|---|---|
| A1 | `add_memory` `:1731-1789` | `db.create("memory")` `:1768`, then a separate history `INSERT` `:1778` | no retry; history errors silently dropped |
| A2 | `update_memory` `:1807-1852` | `UPDATE memory … RETURN AFTER` `:1819`, then a separate history `INSERT` `:1839` | no retry; history errors dropped |
| A3 | `delete_memory` `:1854-1889` | `BEGIN; INSERT memory_history; DELETE memory; COMMIT` `:1865-1880`; fallback `DELETE` `:1884` | `.check()` surfaces the NotExecuted placeholder (**the failing test**); fallback errors dropped |
| A4 | `store_indexed_memory` `:960-1001` (called from `src/operations.rs:1303`) | `BEGIN; CREATE memory:$key; CREATE memory_history:$hkey; COMMIT` `:978-992` | `.check()`; on error re-reads, then fails |
| A5 | `add_to_task_stream` `:2076-2269` | `BEGIN; CREATE memory:$mkey; INSERT memory_history; UPDATE task_stream +=; COMMIT` | already loops on conflict (bespoke; see section 3) |
| A6 | `auto_summarize_task_stream` `:2995-3135` | per-memory `db.delete(("memory", key))` `:3087`; then `add_memory` (A1); then `UPDATE task_stream … summary_count += 1` `:3115-3129` | no retry |
| A7 | `create_entity` `:1560-1574` | `db.create("entity")` | no retry |
| A8 | `update_entity` `:1594-1609` | `UPDATE entity … WHERE name RETURN AFTER` | no retry (`take(0)` surfaces the error) |
| A9 | `delete_entity` `:1611-1617` | `DELETE entity …; DELETE relation …` (two implicit transactions, one request) | no retry; **errors dropped** |
| A10 | palace `add_drawer` / `delete_drawer` (`palace/adapter.rs:126-173`) | `CREATE type::thing('drawers', $id)` / `DELETE` | no retry; **errors dropped**. Retrieval-owned and c4-coupled; see section 6 |

Covered transitively, with no change of their own:

- `create_entities` `:1576` goes through A7.
- `add_observations` `:1678` goes through A8.
- `delete_all_memories` `:1891`, `compress_memories` `:2508` (which also discards delete errors with
  `let _ =`, pre-existing), `expire_stale_memories` `:2613` and `delete_task_stream` `:2360` all go
  through A3.
- `add_memories_from_conversation` `:2578` goes through A1.

### B. Other tables

These have no HNSW, so conflicts come only from genuine same-key concurrency. The same rule is
safe here. Including them is the lead's breadth decision (section 7):

- `update_task_stream_status` `:1256`
- `update_mindmap_graph` / `append_mindmap_node` / `append_mindmap_edge` `:1404-1500` (these use
  `.check()`)
- `create_relation` `:1629`
- `delete_relation` `:1660` (errors dropped)
- `delete_task_stream`'s mindmap `UPDATE` `:2388` (errors dropped) and `DELETE task_stream` `:2394`
- `update_task_step_status` `:2681-2724`
- `delete_mindmap` `:2984`
- `try_update_persona_mindmap` `:3137`
- `create_record` users `create_task_stream` `:2036`, `add_task_step` `:2635`, `create_mindmap`
  `:2798`, which are fixed by the classifier line alone
- The startup `upsert` `:801` stays out of scope.

## 3. Idempotency and the smallest safe retry unit

The principle is that **the retry unit is exactly one request**. A definite abort means that request
applied nothing, so resubmitting it is safe whatever the statement semantics. The only unsafe
pattern is retrying a *sequence* of requests in which an earlier request already committed. The
table below therefore identifies the multi-request sequences.

| Path | Retry unit | Safe as-is? | Change |
|---|---|---|---|
| A3 `delete_memory` | the BEGIN…COMMIT request; the fallback DELETE separately | yes: definite abort, random history id, DELETE idempotent | replace `.check()` with `check_statements` inside the helper; wrap the fallback |
| A4 `store_indexed_memory` | the BEGIN…COMMIT request | yes: fixed keys | wrap; keep the existing non-conflict fallback (re-read, return existing) |
| A5 `add_to_task_stream` | the BEGIN…COMMIT request | yes (already relies on this) | replace the bespoke loop `:2182-2242` with the helper. This deletes ~50 lines and the string fallbacks and adds the deadline check. Attempts and backoff are unchanged. |
| A6 deletes | each `db.delete` | yes: idempotent | wrap each |
| A6 `UPDATE task_stream` | that request | yes: definite abort means `+=` did not apply | wrap |
| A7 `create_entity` | the create request | yes: nothing written on abort, and the UNIQUE `entity_name` cannot trip on its own row | wrap |
| A8 `update_entity` | the UPDATE | yes: absolute `SET` | wrap (use `check_statements` so an error on statement 0 is not masked) |
| A9 `delete_entity` | the two-DELETE request | yes: both idempotent (case c) | wrap and add `check_statements`, which also fixes the silent loss |
| A1 `add_memory` | **not safe as two requests** | see below | **restructure** |
| A2 `update_memory` | **not safe as two requests** | see below | **restructure** |

**Why A1 and A2 need restructuring (the only two).** Each unit could be retried separately without
duplication. The problem is that adding `check_statements` to the second request (the history
INSERT) creates a new outcome: **"Err returned but the memory was written"**.

- Today that failure is silently swallowed.
- After the change it would surface.
- A caller retrying `add_memory` could then duplicate the row. The duplicate is caught only by the
  0.92 dedup, and not with the zero-vector test embedder, whose `cosine_similarity` returns 0
  (`:1368-1378`).

The minimal fix is the pattern this file already uses three times (A3, A4, A5): one
`BEGIN; CREATE/UPDATE memory; INSERT memory_history; COMMIT;` request, retried as a unit.

- A1: generate `let key = Uuid::new_v4().to_string()` client-side (as in `add_to_task_stream`
  `:2154`), then use `CREATE type::record('memory', $key) CONTENT $memory` with `DbMemory` (as A4
  does) and re-read with `get_memory`.
  **Interface effect:** new `add_memory` ids become `memory:⟨uuid⟩` instead of server-generated
  `memory:abc…`. `add_to_task_stream` already produces the uuid form, and `parse_record_id_str` /
  `record_id_to_string` handle both. UAR receives ids as strings. Record this in the UAR receipt.
- A2: `BEGIN; UPDATE type::record($table,$key) SET …; INSERT memory_history {…}; COMMIT;`, then
  re-read with `get_memory`. Do not depend on transaction result indices; the reason is noted at
  `:2244-2247`.

If the lead rejects restructuring, the fallback is to wrap each request separately but **keep the
history INSERT unchecked** (today's behaviour). That leaves the silent audit-row loss in place and
documented.

**Pre-existing, not worsened, out of scope:**

- lost-update races on read-modify-write paths (`update_memory` version `old.version + 1`,
  `add_observations`);
- dedup TOCTOU in A1 and A5.

The conflict that triggers these retries comes from HNSW background maintenance, not from a
concurrent writer, so the retry does not clobber anyone. Where it does come from a concurrent
writer, the stale value was computed in an earlier request and was never protected by the
transaction anyway.

## 4. Bounds and embedded admission

**No new knobs.** Direct writes use the existing
`MAX_TRANSACTION_CONFLICT_RETRIES` (16) and `transaction_conflict_backoff` (2→64 ms, capped at
100 ms), which are already the conflict policy for A5. Total wall-clock is additionally capped by the
existing `RetryConfig.operation_deadline_ms` (30 s default), checked between attempts. `create_record`
keeps its `retry_operation` path, which means `max_operation_retries` (3) and `calculate_delay`
(100 ms × 2^n ± 25 %).

The lead asked to "reuse RetryConfig (max_operation_retries, backoff)" for everything. That option is
listed as an open decision in section 7. Its cost is that A5 regresses from 16 attempts to 3 against
real same-stream concurrency, and each conflict adds at least 100 ms.

**Embedded semaphore.** The helper **does not acquire** the embedded permit. Today only
`retry_operation` acquires it (`:1123-1133`), and every direct path bypasses it. Keeping that
unchanged preserves the measured admission behaviour; I have no evidence to justify changing it.

The helper also must never run inside `retry_operation`, and it must never call a storage method
that could itself be wrapped. Nesting permit acquisitions with 16 permits deadlocks at 16 concurrent
outer holders; `add_memory` → `search_memories` is the obvious trap. This is also why whole-method
wrapping, which is the original c3 title, is not proposed.

A conflict backoff holds no permit and no lock. `create_record` holds its permit across its Retry
sleep, as it already does.

**W4** (`c1-c2-findings.md:160-172`) is not coupled to this change. The helper never touches the
connection cell: it reuses the `Arc<Surreal<Any>>` the method already holds, because
conflict = Retry = same connection. W4 stays a separate change.

## 5. Tests

**(a) Pure unit tests** in the `surreal.rs` test module (`:3620+`). They need no DB. Construct errors
with the public `surrealdb::Error::query(msg, QueryError::X)` (`surrealdb-types-3.3.0/src/error.rs:183`).

1. `is_retryable_conflict` accepts `TransactionConflict` and rejects `NotExecuted`, `TimedOut`,
   `Cancelled`, `Query(None)` (indeterminate), and `already_exists` / `internal` errors.
2. `classify_surreal_error(conflict) == Retry`. Also assert this through `classify_error` on
   `anyhow::Error::new(conflict).context("x").context("y")` to prove the downcast works through
   context.
3. `select_statement_error`:
   - `{0: NotExecuted, 1: NotExecuted, 2: Conflict}` returns Conflict (the lead's failing shape);
   - `{0: Conflict}` returns Conflict;
   - `{0: NotExecuted, 1: Conflict, 2: already_exists}` returns the non-retryable error;
   - `{0: NotExecuted, 1: NotExecuted(COMMIT)}` returns index 1;
   - `{0: Query(None)}` returns it and it is not retryable.
4. Wire-shape check: `Error::from_wire("conflict", Some("Query"), Some({kind:"TransactionConflict"}))`
   is retryable. This mirrors `error.rs:1354-1359`; `from_wire` is `#[doc(hidden)] pub`.
5. Keep the three existing classifier tests (`:3686-3746`) unchanged. They guard the preserved
   string classification.

**(b) Integration evidence**, in a new file `crates/surreal-memory/tests/conflict_retry.rs` (writer
assigned by the lead). It is `#[ignore]`-gated like `load_repro.rs`. Use a deterministic
**non-zero** per-text embedder, as `search_correctness.rs:36` does, so HNSW does real graph
maintenance; zero vectors may take a degenerate path.

- **b1, server** (`TEST_SURREAL_ENDPOINT`, scratch 3.3.0 server only, never :28000): run N
  sequential `add_memory(distinct)` + `delete_memory` pairs. Then `create_entity` + `delete_entity`
  pairs to cover the SDK-method and multi-statement paths.
  - **Hard assertion:** zero surfaced errors.
  - **Non-vacuity:** a `tracing_subscriber` layer (already a dev-dependency) counts the helper's
    "retrying write after transaction conflict" events. The run records the count and requires ≥1.
    A run with 0 conflicts is **inconclusive, not a pass**; rerun or increase N.
  - Also run the raw repro SQL once through a plain connection (`integration_test.rs:80-92` style)
    to record the environment's baseline conflict rate beside the result.
  - **Sizing:** to see ≥1 conflict with 99 % probability, N ≥ ln(0.01)/ln(1−p) ≈ 4.6/p. At the
    measured p ≈ 1/300 per pair, N ≈ 1,400. Use **N = 1,500 pairs** (~3,000 HNSW writes). The 1/300
    estimate comes from a tiny sample (its 95 % CI is roughly 0.01 %–1.8 %), which is why the
    retry-count gate, not N, decides conclusiveness.
  - After the fix, residual surfaced failure needs 16 consecutive conflicts, which is negligible if
    conflicts are independent. Correlated bursts are bounded by the deadline and would show up as
    errors.
- **b2, embedded** (temp RocksDB via `new_mem`, deterministic setup): 16 concurrent tasks × 50
  distinct `add_memory`, plus 16 concurrent `update_memory` on one id.
  - Assert all return Ok, the memory count is exactly 800, and there is exactly one `created` history
    row per memory (this proves no duplicate on retry).
  - Record the retry count the same way. Concurrency should force conflicts far above the sequential
    rate, but that is **unmeasured**. The implementer must first show b2 fails on the unfixed code
    (RED) before claiming it as evidence.

**(c) Regression.** Run `delete_memory_commits_audit_row_and_removal_together` repeatedly. Today it
fails about 1 in 5. Twenty consecutive passes gives only weak evidence: 0.8^20 ≈ 1.2 % chance of
passing by luck on unfixed code. b1/b2 are the real proof. Run everything in the lead's serialized
completed-boundary batch, never in parallel with fmt or other cargo runs (`docs/lessons.md`).

## 6. Scope boundary: explicitly out

- **W4** Failed-cell recovery and non-single-flight reconnect. These are not coupled (section 4).
- **c4 palace session / public API.** Palace `add_drawer` / `delete_drawer` have the same
  silent-loss and no-retry defect, but `palace/adapter.rs` is retrieval-owned and its `db_fn`
  signature is the c4 API change.
  - Recommendation: expose `is_retryable_conflict` / `check_statements` as `pub(crate)` so c4 can
    adopt them.
  - Note separately for retrieval: `adapter.rs:135` uses `type::thing`, while the rest of this
    codebase uses `type::record`. Because the result is unchecked, a rejection there would also be
    silent. **Unverified** on 3.3.
- **Ledger Fix B** (`src/operations.rs`, runtime-owned). `store_indexed_memory`'s signature and
  behaviour stay the same for it.
- **Everything else:**
  - the original c3 goal of routing all operations through the permit and deadline;
  - transport-error (`Reconnect`) replay of non-idempotent writes in `create_record`, which is a
    pre-existing unknown-outcome risk;
  - lost-update and dedup races;
  - `compress_memories` / `expire_stale_memories` swallowing delete errors;
  - read paths.

## 7. Open decisions for the lead

1. **Retry budget.** Recommended: the existing conflict constants (16 attempts, 2–64 ms) plus the
   `operation_deadline_ms` cap. Alternative: `RetryConfig` (3 attempts, ≥100 ms), which would regress
   A5.
2. **A1/A2 restructure into single transactions.** Recommended, but it changes the `add_memory` id
   format to uuid keys. Alternative: wrap per request and leave the history INSERT unchecked.
3. **Breadth.**
   - Recommended: group A plus the error-dropping B sites (`delete_relation`, `delete_task_stream`
     UPDATE), about 14 sites.
   - Minimal: group A only.
   - Full: all of B.
4. **No message-string fallback for pre-3.3 servers.** Recommended, because both pins are 3.3.0. The
   version of the shared :28000 server and of UAR deployments is unknown to me.

Proposed `docs/lessons.md` line, for the lead or the owner of that file to add:
"`IndexedResults::check()` returns the lowest-index error; inside BEGIN…COMMIT that is the
NotExecuted placeholder, not the cause. Classify from `take_errors()`."
