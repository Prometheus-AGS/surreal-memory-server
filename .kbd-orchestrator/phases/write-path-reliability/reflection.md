# KBD Reflection — Phase: write-path-reliability

- **Project**: surreal-memory-server
- **Phase goal**: Make the surreal-memory MCP write path reliable and
  observable end-to-end, so clients stop abandoning the tool and falling back
  to the filesystem. Covers the create/write path, MemPalace, and Mindmaps.
- **Reflected**: 2026-05-19
- **Change backend**: OpenSpec
- **Evolver cycle**: none

---

## 1. Goal Achievement

**Overall: MET.** All 9 assessment findings (2 CRITICAL, 4 HIGH, 2 MEDIUM,
1 LOW) were addressed across 3 ordered, independently-shipped OpenSpec changes.

| Finding | Severity | Change | Status |
|---|---|---|---|
| C-1 first write blocks open-ended on cold model load | CRITICAL | bound-embedding-model-loading | MET |
| C-2 `create_task_stream` hangs despite no embedding | CRITICAL | eliminate-write-path-stalls | MET |
| H-1 no MCP progress/Tasks — every slow call a silent hang | HIGH | add-mcp-progress-reporting | MET |
| H-2 nested retry amplification → multi-minute stalls | HIGH | eliminate-write-path-stalls | MET |
| H-3 MemPalace first call blocks on FastEmbed download | HIGH | bound-embedding-model-loading | MET |
| H-4 model load/inference on the async runtime, no spawn_blocking | HIGH | bound-embedding-model-loading | MET |
| M-1 mindmap large-graph mitigation was warn-only, not a TIMEOUT | MEDIUM | add-mcp-progress-reporting | MET |
| M-2 no startup health gate / warmup for embeddings | MEDIUM | bound-embedding-model-loading | MET |
| L-1 sequential HF downloads, no shared lock | LOW | bound-embedding-model-loading | MET |

The triggering incident — `create_task_stream` and `create_entity` each hanging
4 minutes, forcing a filesystem fallback — is closed. Writes now: load bounded
(timeout + `spawn_blocking`), fail bounded (single operation deadline, capped
reconnect), and report progress (MCP heartbeats). The diagnostic insight that
drove the plan — `create_task_stream` does *no* embedding yet hung identically,
proving a shared upstream cause — held: C-2's fix (retry de-amplification) was
distinct from C-1's (embedding load), and both were needed.

---

## 2. Delivered Changes

| # | Change | Archive | Closes | Tests |
|---|--------|---------|--------|-------|
| 1 | bound-embedding-model-loading | `2026-05-18-bound-embedding-model-loading` | C-1, H-3, H-4, L-1, M-2 | 37 lib (local-embeddings) incl. 4 new candle; 16 bin lib |
| 2 | eliminate-write-path-stalls | `2026-05-18-eliminate-write-path-stalls` | C-2, H-2 | 56 tests incl. 14 retry_tests (3 new) |
| 3 | add-mcp-progress-reporting | `2026-05-19-add-mcp-progress-reporting` | H-1, M-1 | 40 bin lib + 17 + 36 library; new mindmap-timeout + heartbeat tests |

New artifacts: `crates/.../embeddings/candle.rs` reworked (spawn_blocking load +
inference, bounded HF download, parallel fetch, `warmup`/`is_loaded`/`is_ready`);
`EmbeddingService::is_ready()` trait method; `RetryConfig.operation_deadline_ms`
+ `ReconnectGuard`; `src/mcp/progress.rs` (`run_with_progress` heartbeat helper);
`MINDMAP_UPDATE_TIMEOUT` on all three mindmap UPDATE queries. The canonical
`openspec/specs/write-path/spec.md` was created (0 → 7 requirements, valid).
Three dead binary files (`src/embeddings/{candle,cohere,openai}.rs`) were
deleted. No migration was needed — no persisted struct field was added.

---

## 3. Artifact Quality Summary

The dedicated artifact-refiner tool was not available in this environment; the
QA gate was performed by the `rust-reviewer` agent against
`.kbd-orchestrator/constraints.md`. No `.refiner/artifacts/` logs exist — QA
results are recorded in `progress.json → change_results`.

| Metric | Value |
|---|---|
| Changes with QA | 3/3 |
| First-pass pass rate | 1/3 (33%) |
| Changes requiring refinement | 2 (changes 1 and 2) |
| Total refinement iterations | 2 (one per blocked change) |
| Final pass rate after refinement | 3/3 (100%) |

### QA findings caught (and fixed) before archive

- **Change 1** — BLOCK: 1 CRITICAL (3 test `AppState` literals not updated for a
  new struct field — a hard compile failure under the default feature set),
  2 HIGH (a stale `embedding_ready` `AtomicBool` that made `/health` permanently
  misreport; a thread-pool concurrency note). Refined: replaced the stale flag
  with an `EmbeddingService::is_ready()` trait method (a correct fix, not a
  patch); fixed all test sites.
- **Change 2** — BLOCK: 1 HIGH (a timeout firing mid-reconnect could strand
  `ConnectionState::Reconnecting` permanently, locking out the storage
  instance until restart), 3 MEDIUM (`span.enter()` across `.await`; the C-2
  test didn't exercise the deadline path; deadline/SQL-TIMEOUT interaction
  undocumented). Refined: added a `ReconnectGuard` drop-guard, switched to
  `.instrument()`, added a real deadline test using a live in-memory DB.
- **Change 3** — APPROVED first pass; 3 MEDIUM follow-ups noted. One (ticker
  detach-on-panic) was fixed anyway with a `TickerGuard` drop-guard.

### Recurring patterns

- **Struct-field additions break call sites the author didn't grep for** —
  change 1's CRITICAL was 3 missed test `AppState` literals; my initial `grep`
  matched only `build_router` callers, not `.with_state(AppState {…})`. Lesson:
  when adding a struct field, grep the *type name*, not the constructor
  function.
- **Cancellation safety of shared mutable state** — change 2's HIGH (stranded
  `Reconnecting`) and change 3's MEDIUM (detached ticker) are the same class:
  an `async` future cancelled mid-flight leaves a resource in a bad state. Both
  were fixed with the same idiom — a `Drop` guard that restores a safe state.
  Reinforces: any state set *before* an `.await` needs a drop-guard if the
  future can be cancelled.

The QA gate again earned its place: 2 of 3 changes shipped a CRITICAL or HIGH
defect past a green build (the missed `AppState` literals compiled under
non-default features; the stranded-`Reconnecting` bug compiled and passed every
existing test) and were stopped before archive.

---

## 4. Technical Debt Introduced

- **`RetryConfig` gained a `pub` field** (`operation_deadline_ms`) — a breaking
  struct-literal change for any external consumer that constructs it directly.
  UAR is the known consumer (pinned by git rev). Tracked as a follow-up under
  the standing UAR resync.
- **Embedding hot path under high concurrency** — `embed_internal` uses
  `spawn_blocking` + `blocking_lock`; under many concurrent embed calls this can
  accumulate OS threads in tokio's blocking pool. Strictly better than the
  pre-change behavior (which blocked the async runtime), and bounded by the
  pool ceiling — accepted as a known limitation, not a regression.
- **No behavioral test that an MCP progress heartbeat actually fires** — would
  require a mock `Peer` or a full client/server pair. The current coverage is
  the interval-bound unit test plus the mindmap-timeout integration test.
- **`update_task_stream_status` bypasses the new operation deadline** — it reads
  the connection directly instead of routing through `retry_operation`.
  Pre-existing; the pause/resume path is therefore not deadline-bounded.

---

## 5. Deferred Follow-ups (from progress.json)

1. **Pre-existing compile error** — `tests/contract_alignment_test.rs:300/312`
   (`Option<HashMap>` vs `Option<Value>`) fails on clean `main`, unrelated to
   this phase, and breaks the binary's `cargo check --tests`. Needs its own
   change. (Carried over from the prior phase; still open.)
2. **UAR consumer resync** — `MemoryStorage` trait surface and `RetryConfig`
   both changed; UAR's `MemoryService` and `Cargo.toml` git rev must be
   re-pinned per CLAUDE.md rule #7. Highest-priority follow-up.
3. **Route `update_task_stream_status` through `retry_operation`** so the
   pause/resume path is deadline-bounded like every other write.
4. **Bounded-concurrency embed** — add a `Semaphore` if embed concurrency grows.
5. **MCP progress behavioral test** — mock `Peer.notify_progress`.
6. Standing prior-phase items remain: TaskStep REST endpoints, REST JWT/header
   scope extraction, real LLM summarization for `auto_summarize`.

---

## 6. Lessons Captured

- **A green build is not correctness — again.** Both blocked changes compiled
  and (mostly) passed tests; the missed `AppState` literals only failed under
  the default feature set, and the stranded-`Reconnecting` bug passed every
  existing test. Keep the QA gate; it caught both.
- **Cancellation safety is a recurring defect class.** Two separate HIGH/MEDIUM
  findings this phase were "async future cancelled mid-flight leaves shared
  state broken." The fix is always the same: a `Drop` guard. Treat any
  pre-`.await` mutation of shared state as needing one.
- **Grep the type, not the constructor.** The CRITICAL miss in change 1 came
  from grepping `build_router` (the function) instead of `AppState` (the type).
  Field additions ripple to every literal.
- **The diagnostic that anchors the plan must be load-bearing.** The assessment
  singled out "`create_task_stream` does no embedding yet hung identically" as
  proof of a shared cause. That one fact correctly split the work into two
  distinct changes (embedding load vs retry amplification) — had it been
  hand-waved as "same root cause," change 2 would have been missed.
- **Fix the doc to match reality, then make reality match the doc.** M-1 was a
  CLAUDE.md claim of a `TIMEOUT` that never existed. The change implemented the
  `TIMEOUT` *and* corrected the doc — including removing a second
  never-implemented claim (`JSON size monitoring`) found along the way.
- **OpenSpec archive creates a new canonical spec cleanly** when the change's
  delta is the first content for that capability — `write-path` went 0 → 7
  requirements across the three archives with no manual merge needed (unlike
  the prior `task-stream` spec).

---

## 7. Recommended Focus for Next Phase

**Next phase: `uar-consumer-resync`.** Two phases of library changes
(`task-stream-reliability` and `write-path-reliability`) have now altered the
`MemoryStorage` trait surface, added `TaskStep` methods, and changed
`RetryConfig`. None of it is reachable by the primary consumer until UAR's
`MemoryService` wrapper and `Cargo.toml` git rev are updated. This is the
highest-value next step — the library improvements are inert without it.

Fold in, or fix first so CI is green: the pre-existing
`contract_alignment_test.rs` compile error.

Lower priority: TaskStep REST endpoints, deadline-bounding
`update_task_stream_status`, and a real summarization path for `auto_summarize`.

---

## Phase Status: COMPLETE

Assessment → Plan → Execute → Reflect all done. 3/3 changes archived,
QA-passed, canonical `write-path` spec valid (7 requirements). No work
committed to git (KBD does not commit unless asked) — all changes are
staged/uncommitted for the user to review and commit.
