# Fix TaskStream Scope Bugs

## Why

The TaskStream assessment (`.kbd-orchestrator/phases/task-stream-reliability/assessment.md`)
found two CRITICAL defects that make TaskStream unsafe for any multi-stream or
multi-tenant use:

- **C-1 — Data loss.** `auto_summarize_task_stream` (`crates/surreal-memory/src/storage/surreal.rs:2447`)
  filters memories by `task_stream_id != NONE` but never binds the loaded
  `stream.id`. With more than one stream in scope, it deletes the oldest half of
  memories *across all of them*. It auto-fires from `add_to_task_stream`.
- **C-2 — Cross-tenant access.** `get_task_stream` and every stream mutator
  resolve streams by global `WHERE name = $name` with no `agent_id`/`user_id`
  filter. Any caller can read, mutate, archive, or delete another agent's stream.

This is the highest-urgency tier — it stops active data loss and an access-control
hole — and is intentionally the smallest change so it can ship independently.

## What Changes

- Scope the `auto_summarize_task_stream` memory-selection query to the specific
  `stream.id` (`WHERE task_stream_id = $sid`), not `!= NONE`.
- Add `agent_id` / `user_id` scope parameters to `get_task_stream` and propagate
  scope filtering through `add_to_task_stream`, `get_context_for_task`,
  `archive_task_stream`, `pause_task_stream`, `delete_task_stream`.
- Update the `MemoryStorage` trait signatures and `SurrealStorage` impl.
- Update MCP handler param structs and handlers in `src/mcp/handlers.rs`.
- Add regression tests proving multi-stream isolation and cross-scope rejection.

## Non-goals

- The composite unique index, token-accounting fix, and summary linkage (Tier 2).
- The `task_step` abstraction (Tier 3).
- Any change to the embedding or memory-scoping subsystems beyond TaskStream.

## Impact

- Affected specs: `task-stream`
- Affected code: `crates/surreal-memory/src/storage/mod.rs`,
  `crates/surreal-memory/src/storage/surreal.rs`, `src/mcp/handlers.rs`
- **Breaking**: `MemoryStorage` trait signatures change — UAR's `MemoryService`
  wrapper must be updated, and the UAR `Cargo.toml` git rev re-pinned per
  `CLAUDE.md` rule #7.
