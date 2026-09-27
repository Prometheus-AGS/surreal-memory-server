# Independent review: c8 (cb8e7e1) and c3 (c3a1e72)

Reviewer: memory-verifier, fresh context. HEAD c3a1e72, branch feat/surrealdb-3x-connection-model.
Inputs: review packet (intent, user decisions, diffs, receipts, limits), current source, SDK sources
surrealdb-3.3.0 / surrealdb-core-3.3.0 / surrealdb-types-3.3.0 / surrealdb-kvs-{rocksdb,surrealkv}-3.3.0 /
surrealkv-0.21.4, UAR checkout (read-only).
Method: static source review only. No cargo, no services, no database access. Nothing below was executed
by the reviewer; every "falsifier" is a proposed reproduction, not a run result.
Judge: different-family `gpt-5.5` via liter-llm gateway (openai-proxy), given code excerpts plus
source-verified SDK facts, not this reviewer's findings. Judge claims accepted, re-graded or rejected
are listed at the end.

## SDK facts relied on (verified from source)

- F1 Bare statements (no BEGIN) each run in their own transaction and commit independently
  (`surrealdb-core-3.3.0/src/dbs/executor.rs:1455-1492`). Commit conflict -> kind `TransactionConflict`;
  indeterminate commit -> `CommitOutcomeUnknown` -> `TypesError::internal` (no query kind,
  `err/to_types.rs`); other commit failure -> `NotExecuted`.
- F2 BEGIN..COMMIT commit failure (`executor.rs:1917-1965`): statement rows -> `NotExecuted` (or kind
  `None` when indeterminate); separate COMMIT row carries `TransactionConflict` / `None` / `NotExecuted`.
- F3 BEGIN..COMMIT statement-execution error (`executor.rs:2020-2075`): earlier rows -> `NotExecuted`,
  failing row -> its typed error, later rows -> `QueryError::Cancelled`, COMMIT row -> `NotExecuted`,
  transaction cancelled.
- F4 RocksDB backend is `OptimisticTransactionDB` (`surrealdb-kvs-rocksdb-3.3.0/src/owned_tx.rs:15`,
  `set` only buffers, `lib.rs:2050-2075`); SurrealKV validates at commit (`surrealkv-0.21.4/src/oracle.rs:93-105`).
  Storage conflicts therefore surface at commit. The only statement-level source of the
  `TransactionConflict` kind found is `DatastoreError::SequenceReset`
  (`surrealdb-datastore-3.3.0/src/error.rs`).
- F5 `Surreal::clone()` allocates a new session id and calls `clone_session`
  (`surrealdb-3.3.0/src/lib.rs:337-350`); `Drop` sends `SessionId::Drop`.
- F6 `RecordId::parse_simple` always yields a `RecordIdKey::String` and decodes backtick-escaped
  fragments (`surrealdb-types-3.3.0/src/value/record_id/mod.rs:50-56`).

## Findings

### W1 WARNING: update_memory commits an orphan 'updated' history row when the memory is deleted concurrently
- Location: `crates/surreal-memory/src/storage/surreal.rs:1939-1977` (transaction at 1953-1974).
- Claim: the existence check (`get_memory` at 1941-1944) runs outside the transaction. If a concurrent
  `delete_memory` commits between that read and the transaction, `UPDATE type::record($table,$key)`
  is a no-op in 3.x, the `INSERT INTO memory_history` still runs and COMMIT succeeds, and only the
  re-read at 1976 reports "Failed to update memory". The caller gets an error while an audit row
  claiming an update of a deleted memory is durable.
- Evidence: `memory_history.memory_id` is `TYPE record<memory>` with no REFERENCE
  (`migrations/mod.rs:185`), so no existence check. The previous code ran `UPDATE ... RETURN AFTER`,
  failed on an empty result (`updated.context("Failed to update memory")?`) and inserted history only
  after a returned row (diff lines 460-510 of the packet). This is a regression introduced by the
  restructure.
- Fix: make the history insert conditional inside the transaction, for example
  `LET $before = (SELECT content, version FROM ONLY type::record($table,$key)); IF $before = NONE { THROW "memory not found" }; UPDATE ...; INSERT ...`.
  A THROW yields a non-conflict error, so it is surfaced and not retried. This also enables the W2 fix.
- Falsifier: integration test in an isolated `new_mem` store: add a memory, then run
  `update_memory` and `delete_memory` concurrently in a loop; afterwards
  `SELECT count() FROM memory_history WHERE change_type = 'updated' AND memory_id NOT IN (SELECT VALUE id FROM memory) GROUP ALL`
  must be 0. A deterministic variant: submit the exact transaction SQL for a non-existent key and
  assert the commit is rejected.

### W2 WARNING: conflict retries resubmit a stale version and old_content, so concurrent updates silently produce duplicate version numbers
- Location: `surreal.rs:1945-1947` (version/old read before the embedding call) and 1953-1974 (resubmitted
  verbatim); test `crates/surreal-memory/tests/conflict_retry.rs:186-222`.
- Claim: a `TransactionConflict` on update_memory means another transaction wrote the same keys after
  this snapshot, typically another update of the same memory. The helper resubmits the same `$v` and
  `$old`, so the loser commits `version = old.version + 1` again, and its history row repeats the
  winner's version and records an `old_content` that was never the prior content. Before c3 the
  loser got the conflict as an error (packet RED receipt: "FAIL at update_memory with
  TransactionConflict"). Now it succeeds with incorrect audit metadata. The read-outside-transaction
  race itself existed before c3 for non-overlapping interleavings. c3 extends it to every
  same-key write-write conflict and hides it.
- Evidence: in the embedded test, 16 tasks call `update_memory` on one target at the same time. All
  read version 1 before any commit, so every history row is expected to carry version 2 and the
  final memory version to be 2 after 16 updates. The test asserts only the count of 'updated' rows
  (16), which passes whether or not versions are correct. This is a static inference; the version
  values were not observed.
- Fix: compute `version` and `old_content` inside the transaction from `$before` (see W1), for example
  `version = $before.version + 1` and `old_content: $before.content`, so a resubmitted attempt reads
  post-winner state. Then extend the test to assert the target's history versions are 2..=17 distinct
  and the final memory version is 17.
- Falsifier: add to the existing embedded test
  `SELECT count() FROM memory_history WHERE change_type='updated' GROUP BY version` and the target's
  final `version`. If versions are distinct and the final version is 17, this finding is wrong.

### S1 SUGGESTION: delete_entity is two independent bare transactions, and select_statement_error's contract comment overstates "nothing applied"
- Location: `surreal.rs:1721-1734`; doc comment `surreal.rs:114-123`.
- Claim: without BEGIN, statement 0 (entity delete) can commit and statement 1 (relation delete) can
  conflict (F1). `take_errors` then contains only the conflict, and it is classified as a "definite
  abort" although statement 0 applied. Resubmission is safe because both DELETEs are idempotent, and
  the helper's comment at 1170-1172 requires that. So this is not a correctness bug. However, on
  retry exhaustion the entity is gone and its relations remain, and the `select_statement_error`
  comment ("nothing was applied, safe to resubmit") is false for bare multi-statement requests. This
  is pre-existing non-atomicity: before c3, all errors here were dropped.
- Fix: wrap the two DELETEs in `BEGIN ... COMMIT` so F2 applies and the pair is atomic, and restrict
  the "definite abort" wording to single statements and BEGIN blocks.

### S2 SUGGESTION: a statement-level conflict inside BEGIN is reported as "cancelled transaction" and not retried
- Location: `surreal.rs:124-143`.
- Claim: under F3, a statement-level `TransactionConflict` (reachable via `SequenceReset`) is followed by
  `Cancelled` rows. `Cancelled` is neither conflict nor `NotExecuted`, so `definite_abort` is false
  and the first "real" error selected is a `Cancelled` placeholder at a higher index. The outcome is
  safe (no retry of an ambiguous request) but misreports the cause. With F4 this is narrow. No
  migration here defines a SEQUENCE.
- Fix: treat `Cancelled` as a placeholder alongside `NotExecuted`, both for the definite-abort test and
  when choosing the first real error. That is sound because F3 cancels the whole transaction.

### S3 SUGGESTION: add_memory now rejects a caller-supplied Memory.id
- Location: `surreal.rs:1894-1911`.
- Claim: `DbMemory::from` copies `memory.id` into CONTENT (`surreal.rs:609`). The old `db.create("memory").content(..)`
  honoured a supplied id. `CREATE type::record('memory', $key) CONTENT {id: <other>}` targets a
  specific record, so a non-NONE `id` that differs from it is expected to be rejected. Not executed.
  All in-repo callers (`src/mcp/handlers.rs:666`, `src/api/memory.rs:42`, `surreal.rs:2666/2696/3196`)
  and the UAR callers inspected (`src/uar/tools/memory.rs:169`, `src/uar/memory/mcp_server.rs:384`,
  `src/uar/api/memory.rs:44`) use `Memory::new`, so `id` is `None`. Only external library consumers
  are exposed.
- Fix: set `payload.id = None`, or reject a supplied id with an explicit error, and document which one.

### S4 SUGGESTION: update_memory's return value may not be this call's post-image
- Location: `surreal.rs:1976-1978`.
- Claim: the previous `RETURN AFTER` result is replaced by a separate `get_memory` after commit. A
  concurrent update between commit and re-read makes the function return another writer's content.
  MCP `update_memory` and the add_memory dedup path return this value to callers.
- Fix: covered by the W1 rewrite, for example by selecting the row inside the transaction after the
  UPDATE, or accepting the re-read and documenting it.

### S5 SUGGESTION: conflict_retry.rs gate and coverage gaps
- Location: `crates/surreal-memory/tests/conflict_retry.rs:34-64` (global `RETRIES`), 278-302.
- Claims: (a) `RETRIES` is process-global and tests in one binary run in parallel. With
  `--include-ignored`, conflicts from the embedded test can satisfy the server test's `retries > 0`
  inconclusiveness gate. The documented `--ignored` invocation avoids this. The gate cannot be
  satisfied by non-conflict errors: only `is_retryable_conflict` reaches the debug event
  (`surreal.rs:1186-1203`). (b) The server test does not assert the entity table is empty after
  delete_entity or the history row counts (1500 'created' / 1500 'deleted'), so a lost delete or
  duplicated history row there would pass. (c) Loss/duplication proof for add_memory is sound: fixed
  client key plus exact counts (800 memories / 800 'created'). A duplicate resubmission would surface
  as a CREATE conflict error, not a silent duplicate. The update proof is weak (see W2).
- Fix: tag events with a per-test field (for example a namespace) or mark the tests `serial`. Add the
  count assertions.

### S6 SUGGESTION: the readiness probe still clones a session per request (outside c8's diff)
- Location: `src/api/mod.rs:147-155` (`storage.db().is_ok()`).
- Claim: `SurrealStorage::db()` clones `Surreal<Any>` (`surreal.rs:992-994`), which opens and drops a
  server-side session on every `/ready` probe (F5). This is the per-call-clone pattern c8 removed from
  the ledger. It is not a c8 regression and not in operations.rs.
- Fix: probe `live_db()` state through a non-cloning accessor.

## Question-by-question outcome

1. c3 rule soundness: sound for single statements and BEGIN..COMMIT commit conflicts (F1, F2). An
   indeterminate commit carries no `TransactionConflict` kind in either path (F1, F2), so
   `is_retryable_conflict` is false and it is never retried. The unit test at the packet diff lines
   755-760 covers `Query(None)`. For bare multi-statement requests the rule can fire after partial
   application. The only such wrapped site is delete_entity, whose statements are idempotent (S1).
   `NotExecuted` from a non-conflict bare commit is treated as a placeholder. That is benign because a
   resubmission is idempotent there and a persistent failure still surfaces.
2. Call sites: each closure issues exactly one request (`db.query`, `db.create` or `db.delete`) and
   calls no storage method. store_indexed_memory keeps its re-read fallback and now also applies it to
   transport errors (previously `?`), which is benign. add_memory writes the same fields and history
   content. `changed_at` now equals `created_at` instead of a second `Datetime::default()`. The
   re-read returns the same `DbMemory -> Memory` shape. The dedup path is unchanged. update_memory:
   see W1, W2, S4. delete_memory: same statements. Checking the fallback DELETE's result is new, and
   intended. `memory:<uuid>` ids: `record_id_to_string` emits `memory:<uuid>` and `parse_simple`
   round-trips it (F6). Hyphenated memory keys already existed before c3 through
   add_to_task_stream (`surreal.rs:2290`). No `split(':')`/`format!("memory:{..}")` SQL interpolation
   was found in `src/` or the crate other than `surreal.rs:3177`, which is safe for uuid keys.
3. add_to_task_stream matches the old loop: same abort predicate minus the string fallback, the same
   16-attempt cap and backoff, plus the deadline cap. Non-abort errors now return the lowest-index real
   error instead of an arbitrary `HashMap` value. Error context strings changed. Neither repository
   greps for them.
4. Deadline/backoff: sleeps are 2..64 ms and total at most about 0.7 s over 15 retries. No spin. The
   helper holds only an `Arc<Surreal>` while sleeping, with no lock or permit (`live_db`). It is never
   nested in `retry_operation`: the only `retry_operation` caller is `create_record` (`surreal.rs:1444`),
   which does not call the helper. The deadline is checked between attempts, so overshoot is bounded
   by one attempt (SDK `query_timeout_ms`, default 10 s). The deadline starts at helper entry, not
   operation entry. Wrapping attempts in a timeout (judge suggestion) would cancel in-flight commits
   and create indeterminate outcomes, so it is rejected. `classify_surreal_error` now maps conflicts to
   `Retry`. That affects only `create_record` (a single-statement fixed-key CREATE), where
   resubmission is safe.
5. c8: `&connection.db` is shared. operations.rs sends each transaction as one request (BEGIN..COMMIT
   inline at 593-596, 742-745, 846-848). No `set`/`use_ns`/`signin`/`LET` session mutation was found.
   The `Arc<LedgerConnection>` is held across each await, so a concurrent `replace_ledger_connection`
   cannot drop a session in use. The remaining `.clone()` of `Surreal` in operations.rs is test-only
   (1823-2030, 2508-2653). Head-of-line behaviour of one shared server session under load is a
   performance question this review cannot answer statically. The receipts cover the deadline tests
   only.
6. Tests: see S5 and W2. The unit tests' constructors match the SDK signatures
   (`Error::query(String, impl Into<Option<QueryError>>)`, `already_exists(String, impl Into<Option<_>>)`,
   `Display` = message), so the `to_string()` assertions are consistent. They have not been executed.
7. UAR: the vendored copy pins surrealdb `=3.2.4` and is untouched (`vendor/git/surreal-memory-server/crates/surreal-memory/Cargo.toml:48-49`).
   3.2.4 core also emits the typed `TransactionConflict` on COMMIT rows
   (`surrealdb-core-3.2.4/src/dbs/executor.rs:1497-1502`), so a later re-vendor does not depend on the
   dropped string fallback. UAR does not string-match the changed error messages. UAR stringifies ids
   as `serde_json::to_value(r).as_str()` (`src/uar/tools/memory.rs:178-186`, `src/uar/api/memory.rs:55-62`).
   `RecordId` serializes as an object, so that yields `""` regardless of key format. This is a
   pre-existing UAR defect unrelated to c3 and belongs in a peer handoff, not this change.

## Judge reconciliation (gpt-5.5)

- Accepted: update_memory orphan history (W1), with severity lowered from CRITICAL to WARNING. It is a
  race-window audit anomaly with the error still surfaced, not data loss.
- Accepted: bare multi-statement "definite abort" misdescription (S1). Rejected: CRITICAL severity for
  delete_entity. Both statements are idempotent deletes, resubmission cannot corrupt data, and the
  pre-c3 code silently dropped every error on this path.
- Accepted: Cancelled masking (S2) as SUGGESTION. Judge graded it WARNING, but F4 limits the trigger
  to `SequenceReset`, which this schema does not use.
- Accepted as SUGGESTION: add_memory id divergence (S3). No caller sets `id`.
- Rejected: "deadline overshoot" as a WARNING with a timeout wrapper. The overshoot is one attempt,
  documented, bounded by the SDK query timeout. Cancelling an in-flight write would convert conflicts
  into indeterminate commits.
- Rejected: backoff "off by one" (first sleep is 2 ms). This is identical to the pre-c3
  add_to_task_stream loop and has no correctness effect.
- Not raised by the judge, added by the reviewer: W2, S4, S5, S6, and the UAR/c8 analysis.

## Remaining limits

- No cargo or DB execution by the reviewer. Both warnings are static inferences with proposed
  falsifiers. The new unit tests are still unexecuted (per the packet).
- Palace `drawers` writes (HNSW 384d) remain unwrapped (declared out of scope, c4).
- Embedded semaphore permit is not taken by the helper. That matches the pre-c3 behaviour of these methods.
- Code-graph (compass) queries were not run. Call-site coverage used `grep` over `src/`, the crate and
  UAR `src/`, so dyn-trait or macro-generated callers could be missed.

## Tally

CRITICAL 0 / WARNING 2 / SUGGESTION 6. Verdict: PASS-WITH-WARNINGS.

## Sycophancy screen

`detect_sycophancy` (prometheus-skill-pack, strict, target=completion) on the findings summary:
score 0.0, no patterns. The score is not quality evidence. Severity downgrades of judge claims
are justified individually above.
