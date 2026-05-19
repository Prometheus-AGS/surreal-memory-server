# Tasks — Add TaskStream Steps

## 1. Data model
- [x] 1.1 Create `crates/surreal-memory/src/task_step.rs` with `TaskStep` struct
      and `TaskStepStatus` enum (`pending`/`running`/`completed`/`failed`/`skipped`)
- [x] 1.2 Re-export `TaskStep`, `TaskStepStatus` from `lib.rs`
- [x] 1.3 Add migration: `task_step` table SCHEMAFULL with all struct fields,
      index on `(task_stream_id, ordinal)`, unique index on `idempotency_key`
- [x] 1.4 Increment the `MIGRATIONS` array; verify schema ↔ struct sync

## 2. Storage trait + impl
- [x] 2.1 Add trait methods to `MemoryStorage`: `add_task_step`,
      `update_task_step_status`, `get_task_steps`, `get_current_step`,
      `complete_step`
- [x] 2.2 Implement in `SurrealStorage`; `complete_step` and `add_task_step`
      MUST be idempotent on `idempotency_key` (check-then-act / upsert)
- [x] 2.3 Extend `get_context_for_task` to include step status summary

## 3. Tests
- [x] 3.1 Test: ordered step creation, status transitions, current-step query
- [x] 3.2 Test: idempotent replay — calling `complete_step` twice with the same
      `idempotency_key` produces one completed step, no duplicate side effects
- [x] 3.3 Test: resume — given a stream with steps 1-2 completed and 3 pending,
      `get_current_step` returns step 3
- [x] 3.4 Run against embedded and server modes

## 4. MCP surface
- [x] 4.1 Add MCP tools in `src/mcp/mod.rs` + handlers in `src/mcp/handlers.rs`
      for each new trait method, with typed param structs
- [x] 4.2 Update the `get_info()` instructions string to list the new tools

## 5. Verification + handoff
- [x] 5.1 Run quality gate `./scripts/quality-check.sh`
- [x] 5.2 Document UAR `MemoryService` pass-through additions + `Cargo.toml`
      rev re-pin for handoff
