# Tasks — Fix TaskStream Scope Bugs

## 1. C-1: Scope auto-summarization to its own stream
- [x] 1.1 In `auto_summarize_task_stream` (`surreal.rs`), bind the loaded
      `stream.id` and change the WHERE clause to `task_stream_id = $sid`
- [x] 1.2 Write a failing test: two active streams in the same scope, summarize
      one, assert the other's memories are untouched
- [x] 1.3 Confirm the test passes after the fix

## 2. C-2: Enforce scope on stream resolution
- [x] 2.1 Add `agent_id: Option<&str>` / `user_id: Option<&str>` to
      `get_task_stream` in the `MemoryStorage` trait
- [x] 2.2 Update `SurrealStorage::get_task_stream` to add the scope filter to
      the WHERE clause
- [x] 2.3 Propagate scope through `add_to_task_stream`, `get_context_for_task`,
      `archive_task_stream`, `pause_task_stream`, `delete_task_stream`
- [x] 2.4 Write failing tests: a caller in scope A cannot read/mutate/delete a
      stream owned by scope B
- [x] 2.5 Confirm tests pass after the fix

## 3. MCP handler surface
- [x] 3.1 Add `agent_id`/`user_id` to `TaskStreamNameParams`, `GetContextParams`
      and pass them through in `src/mcp/handlers.rs`
- [x] 3.2 Verify MCP tool schemas regenerate cleanly

## 4. Consumer + verification
- [x] 4.1 Run quality gate: `./scripts/quality-check.sh`
- [x] 4.2 Document the UAR `MemoryService` + `Cargo.toml` rev re-pin needed
      (do not perform the UAR change here — note it for handoff)
