# Independent review: c1 `de3582e` + c2 `d0caa4c` (surrealdb-3x-connection-model)

- Reviewer: memory-verifier (fresh context; packet + current source at `df77984`)
- Method: static source reading only. No cargo, no services, no SurrealDB access
  (per task constraints). Nothing below is executed evidence unless it says so.
- Sources read: `crates/surreal-memory/src/storage/surreal.rs`,
  `crates/surreal-memory/src/palace/context.rs`, `crates/surreal-memory/src/storage/migrations/mod.rs`,
  `src/api/mod.rs`, `src/api/search.rs`, `src/mcp/handlers.rs`, `src/operations.rs`, `src/main.rs`,
  `crates/surreal-memory/tests/{integration_test.rs,load_repro.rs}`,
  `.kbd-orchestrator/phases/surrealdb-3x-connection-model/assessment.md`, and the SDK/engine
  sources `surrealdb-3.3.0`, `surrealdb-core-3.3.0`, `surrealdb-idx-3.3.0`, `surrealdb-cnf-3.3.0`,
  `surrealdb-rpc-3.3.0` in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.
- Judge: `gpt-5.5` through the liter-llm gateway (different model family). Its points are
  folded in below, and each one is marked as accepted or rejected with the evidence.
- Sycophancy detector: `detect_sycophancy` (standard) run on the findings summary. Score
  0.0, no patterns. A zero score says nothing about whether the findings are correct.

## Answers to review questions

**Q1: is one shared session safe for c1?** Yes, as far as source reading shows. No finding.
- The only `use_ns`/`use_db`/`signin` calls are in `connect_with_config` (`surreal.rs:758-762`).
  The others are the repair CLI (`src/main.rs:388-391`) and tests. There are no `LET`, `.set()` or
  `.unset()` session variables, no SDK `.begin()`/`.transaction()`, and no live queries anywhere
  in `src/` or `crates/`.
- Every `BEGIN ... COMMIT` sits inside a single `query()` string: `surreal.rs:979-982`,
  `1865-1868` and `2157-2171`, plus `src/operations.rs:590`, `739` and `843`.
- `Query::bind` sends per-request `variables` (`surrealdb-3.3.0/src/method/query.rs:33,169-198`).
  It does not change session state.

**Q2: reconnect and palace after c1.** No leaked or double sessions found.
- `reconnect_with_attempts` stores `Connected(Arc::new(db))` (`surreal.rs:1087`).
- The old `Arc` is dropped by its last holder, and `Drop` then sends `SessionId::Drop`
  (`surrealdb-3.3.0/src/lib.rs:352-358`).
- The palace closures load the cell on every operation (`palace/context.rs:45-56`, `84-95`).
  They still clone one session per operation. This is known limit c4.
- In embedded mode, `operation_ledger_connection` clones one session and the ledger holds it.
  In server mode it opens its own connection.
- A pre-existing recovery defect is covered in W4.

**Q3/Q4: c2 SurrealQL and semantics.** The syntax is valid for 3.3.0.
- Parameters are pre-folded to literals before index analysis
  (`surrealdb-core-3.3.0/src/exec/index/analysis.rs:1701-1705`).
- When a KNN candidate exists, KnnScan is forced as the access path
  (`exec/index/access_path.rs:376-391`).
- Scope conjuncts covered by the b-tree indexes `memory_user`, `memory_agent` and
  `memory_session` (`migrations/mod.rs:155-157`) become a prefilter allow-list with three tiers
  (`exec/operators/scan/knn.rs:40-100`, `surrealdb-cnf-3.3.0/src/lib.rs:345-375`):
  - up to 2,000 in-scope rows: exact scoring;
  - up to 100,000 rows: graph search with `ef` × 4;
  - above that: graph search without the boost.
- Scope semantics match the old path:
  - Rows with no embedding were dropped by `filter_map` before and are not indexed now
    (`surrealdb-idx-3.3.0/src/trees/hnsw/index.rs:384`).
  - All-`None` filters now mean "no scope condition", as `true` did before.
  - `session_id` is still an equality filter only when it is `Some`.
- The HNSW index is guaranteed by `ensure_embedding_indexes` in both constructors
  (`surreal.rs:670`, `3402`), with the dimension checked against existing rows
  (`surreal.rs:768-809`).
- The exceptions are W1, W2 and W3.
- Dedup (`add_memory` at `surreal.rs:1737-1757`, `add_to_task_stream` at `2101-2119`) and the
  hybrid ranking are an accepted approximation, not a correctness regression, with two
  conditions: the in-scope set is at most 2,000 rows (exact tier) or the near-duplicate
  (cosine ≥ 0.92) is among the nearest, and the 3.3.0 new planner is in use. Neither condition
  is demonstrated by execution (W2).

**Q5: receipts.** See S2.

**Q6: other CRITICAL items.** None found in c1/c2. Two pre-existing availability issues
(W3, W4) should be decided on before a live deploy.

## Findings

### W1: WARNING (c2). Approximate KNN returns zero rows, with no error, when the HNSW index is unusable
- File: `crates/surreal-memory/src/storage/surreal.rs:1979-1984`
- Claim: `embedding <|K,EF|> $query_emb` depends on `memory_embedding_hnsw` being present and
  online. Without it, SurrealDB 3.3.0 evaluates the conjunct as `false` on every row. The result
  is a full scan that returns `[]` and raises no error. The old exact path did not depend on the
  index.
- Evidence:
  - `surrealdb-core-3.3.0/src/exec/planner/select/mod.rs:1374-1392`: when no KNN source fired,
    the planner restores the full condition, which becomes "membership in the empty KNN result
    set, i.e. `false`".
  - `exec/physical_expr/ops.rs:156-164`: "an Approximate form with no index ... evaluates them
    to `false` per row".
- Impact:
  - `search_memories` and `hybrid_search_memories`' vector branch go empty without any signal.
  - Dedup in `add_memory` and `add_to_task_stream` stops working, so duplicate rows get written.
- Reachable when:
  - an index is in `prepare_remove` or not yet online;
  - another process starts with a different provider dimension and runs
    `rebuild_embedding_indexes` (`surreal.rs:860-874`). That is `REMOVE` then `DEFINE`, and each
    process does it opportunistically with no guard.
- Normal restarts with matching `schema_metadata` do not remove the index.
- Suggested fix: use `embedding <|K,COSINE|> $query_emb`.
  - It uses the HNSW index when the distance matches the index `DIST COSINE`
    (`idx/planner/tree.rs:709-715`, `exec/index/analysis.rs:1966-1968`).
  - Without an index it falls back to brute-force `KnnTopK`, which still returns only K rows.
  - Alternatively, fail loudly: assert the index at startup and before serving.
- Falsifier: on a scratch 3.3.0 DB, seed memories, run
  `REMOVE INDEX memory_embedding_hnsw ON memory`, then call `search_memories`. The claim is
  confirmed if the call returns `Ok([])`.

### W2: WARNING (c2 evidence gap). No executed evidence of retrieval or scope correctness on 3.3.0
- Files: `crates/surreal-memory/tests/integration_test.rs:310-318`,
  `crates/surreal-memory/tests/load_repro.rs:38-55,112-143`
- Claim: nothing executed checks that the new query returns the right rows.
  - `test_memory_search` discards its results.
  - The load harness embeds everything as 1536-d zero vectors. `cosine_similarity` returns 0.0
    for zero norms (`surreal.rs:1366-1374`), so the dedup→`update_memory` branch never runs.
    KNN ordering over zero vectors under COSINE is meaningless, and the latency figures come
    from a degenerate graph.
  - The harness uses one scope (`anonymous`/`load-repro-agent`), so selective scopes, the
    `session_id` filter and the unscoped (all-`None`) path are never exercised.
- Every Q3/Q4 conclusion above rests on reading the 3.3.0 source: the default
  `NewPlannerStrategy::BestEffortReadOnlyStatements` (`surrealdb-rpc-3.3.0/src/capabilities/mod.rs:623-633`)
  and the prefilter tiers.
- If the planner falls back to the legacy executor, `search_with_filter` stops at the worst
  *matching* distance (`surrealdb-idx-3.3.0/src/trees/hnsw/layer.rs:290-345`). That can return
  fewer than K in-scope rows under selective filters, which the old path never did.
- The judge's point is accepted: approximate top-K dedup can miss a near-duplicate that the old
  exact scan would have found. It is bounded by the exact tier at 2,000 or fewer in-scope rows.
- Suggested fix:
  - Add an integration test (isolated namespace, not port 28000 data) with deterministic
    non-zero vectors across at least 3 users/agents/sessions and an unscoped case. It should
    assert that `search_memories` top-K equals the exact client-side ranking from
    `get_all_memories`, and that a near-duplicate insert updates the existing row.
  - Run `EXPLAIN` for the c2 query on an exported copy of the live DB to confirm KnnScan and the
    prefilter tier.
- Falsifier: the test above passes on server and embedded 3.3.0.

### W3: WARNING (pre-existing; limits the c2 claim). Caller-controlled limit and whole-scope fetches can still exceed 64 MiB and reset the shared socket
- Files: `src/mcp/handlers.rs:752-770` (`limit: Option<usize>`), `src/mcp/handlers.rs:1536-1548`,
  `src/api/search.rs:20-48`, and `crates/surreal-memory/src/storage/surreal.rs`:
  - `1979-1982`: K and EF come from `limit`;
  - `2437`, `2457`: `limit * 2`;
  - `1889-1895`: `delete_all_memories` goes through `get_all_memories`;
  - `1905-1947`: `get_all_memories` runs `SELECT *` with embeddings;
  - `2513-2540`: `compress_memories`.
  - `src/api/mindmaps.rs:216` also calls `get_all_memories`.
- Claim: nothing bounds `limit`, and the query is `SELECT *` including embeddings.
  - One request with a large `limit` returns the whole scope again. That is the same over-64-MiB
    response (`surrealdb-3.3.0/src/engine/remote/ws/native.rs:138-139`), so the SDK drops the
    shared socket and every in-flight request fails with `Connection reset`.
  - `limit * 2` overflows (a panic in debug builds) for very large values.
  - The remaining whole-scope readers are one table-growth step away from the same failure. The
    assessment estimates about 13 MB per call on the live 3,281-row, 384-d DB.
- Escalation note: the REST + HTTP MCP listener binds `0.0.0.0` with no auth layer visible in
  `src/api/mod.rs` (`src/main.rs:603-611`). If that port is reachable by untrusted callers, the
  judge's view that this blocks deployment is reasonable. This belongs to transport-security
  ownership (memory-transports) and was not introduced by c1 or c2.
- Suggested fix:
  - Clamp `limit` at storage entry (and at the transports).
  - Use checked or saturating `limit * 2`.
  - Project out `embedding` where callers do not need it.
  - Paginate `get_all_memories` and `compress_memories`.
- Falsifier: on a scratch server with at least 5k 1536-d memories in one scope, call MCP
  `search_memories` with `limit=100000` while other requests are in flight. The claim is
  confirmed if those in-flight requests fail with `Connection reset`.

### W4: WARNING (pre-existing; Q2). A `Failed` connection cell is never recovered
- File: `crates/surreal-memory/src/storage/surreal.rs:262-285`, `902-911`, `1073-1096`, `1161-1163`
- Claim: nothing moves the cell out of `Failed`.
  - `live_db()` bails on `Reconnecting` or `Failed`.
  - `retry_operation_inner` calls `self.live_db()?` before any reconnect attempt, so after a
    failed reconnect (`1088-1095`) or a cancelled `ReconnectGuard` (`278-283`) it never
    reconnects.
  - None of the other `live_db()` call sites reconnect.
  - The store stays dead until the process restarts. The comment at `1066-1069` says
    "subsequent calls retry rather than bail forever", which is not what the code does.
  - Concurrent `create_record` reconnects are not single-flight: each opens its own WebSocket,
    and the last store wins.
- c1 changed only the types here, not the behaviour. It matters for a live deploy because a
  SurrealDB restart during a `create_task_stream` or `create_mindmap` call can trigger it.
- Suggested fix: attempt a single-flight reconnect when the retry path sees `Failed` or a stale
  `Reconnecting`, or remove the app-level reconnect and rely on the SDK's WebSocket
  auto-reconnect with replay (`ws/native.rs:209-217`).
- Falsifier: point a storage instance at a scratch server, stop the server, call
  `create_task_stream` until the reconnect fails, restart the server, then call any storage
  method. The claim is confirmed if it still errors with `Connection failed`.

### S1: SUGGESTION. The readiness probe opens and closes a server session on every probe
- File: `src/api/mod.rs:148-155`
- Claim: `storage.db().is_ok()` calls `Surreal::clone()`, which costs an attach, a signin
  replay, `use` and a detach on every probe (`surrealdb-3.3.0/src/lib.rs:337-358`). It only
  checks the cell state, not whether the database is live.
- This is pre-existing, but it contradicts the new doc at `surreal.rs:926-933` and adds to the
  session counts used as evidence.
- Suggested fix: use `health_check()` or a non-cloning `is_connected()`.

### S2: SUGGESTION. The receipts support narrower claims than stated
- "0 connection errors": supported for one c2 run (3,200 ops) of the harness workload only.
  That run had one scope, zero vectors, and no `get_all`, compress or palace traffic. c1 had
  two runs; c2 has one.
- "1 WebSocket per run" and "~2,700 → ~24": the raw server-log counts are neither in the packet
  nor in the repo; `assessment.md` only summarises them.
  - The measured quantity was `signin`/`attach` events, not sessions.
  - After c1, "~24" equals the WebSocket connection count (1 + 23 reconnects, each replaying
    signin). It is not 24 sessions.
  - No session or signin count is reported for c2.
- Latency: the read-only hybrid search p50 went from 14 ms (c1) to 32 ms (c2), but the runs
  are not comparable (host load 9–250), and zero vectors make the KNN timing unrepresentative.
- Suggested fix: commit the exact log-count commands and their output for each run, and
  describe the before/after as signins/attaches and WebSocket connections.

## Judge points rejected (with evidence)
- "`knn` is computed but not appended to `parts`": false. `parts.push(&knn)` is at `surreal.rs:1983`.
- "An all-`None` scope gives `SELECT * FROM memory WHERE ` (syntax error)": false. The KNN
  conjunct is always pushed, so `parts` is never empty.
- W4 as HIGH: this protocol has no HIGH level. It stays a WARNING because it is pre-existing and
  independent of c1/c2. It is still flagged for the deploy decision.

## Remaining limits of this review
- Nothing was executed. Every SDK and engine conclusion comes from reading the 3.3.0 sources.
- The live DB's `INFO FOR TABLE memory` and index state were not inspected (access not
  allowed).
- The UAR vendored copy's use of `SurrealStorage::db()` was not checked; it lives in a peer repo.

## Counts
CRITICAL 0, WARNING 4, SUGGESTION 2. Verdict: PASS-WITH-WARNINGS.
