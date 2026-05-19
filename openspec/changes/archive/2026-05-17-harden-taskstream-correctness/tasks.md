# Tasks — Harden TaskStream Correctness

## 1. H-2: Atomic token accounting
- [x] 1.1 Write a failing concurrency test: N parallel `add_to_task_stream`
      calls, assert final `total_tokens` equals the sum of per-memory counts
- [x] 1.2 Wrap memory-insert + `total_tokens` update in a `BEGIN/COMMIT`
      transaction in `add_to_task_stream`
- [x] 1.3 Confirm the concurrency test passes

## 2. M-1: Composite unique index
- [x] 2.1 Add a new migration: drop the `task_stream_name` index, define
      `task_stream_scope_name` on `FIELDS agent_id, user_id, name UNIQUE`
- [x] 2.2 Increment the `MIGRATIONS` array; verify schema ↔ struct sync
- [x] 2.3 Test: two agents can each create a stream named `build`

## 3. M-2: Summarization correctness
- [x] 3.1 Subtract compacted memories' token totals from the stream's
      `total_tokens` after deletion
- [x] 3.2 Set `task_stream_id` on the generated summary memory so it stays
      attached to the stream
- [x] 3.3 Replace naive `" | "` concat with the existing summarization path
- [x] 3.4 Test: after summarization `total_tokens` decreases, the summary is
      returned by `get_context_for_task`, and the trigger does not re-fire

## 4. H-3: Integration test suite
- [x] 4.1 Create `crates/surreal-memory/tests/task_stream.rs`
- [x] 4.2 Cover create → add → get_context → summarize → archive end-to-end
- [x] 4.3 Run against both embedded and server modes per `CLAUDE.md` rule #5
- [x] 4.4 Run quality gate `./scripts/quality-check.sh`; confirm coverage target
