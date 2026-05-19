# Add TaskStream Steps

## Why

Assessment finding **H-1** is the strategic root cause of why clients abandon
TaskStreams: a `TaskStream` is a name plus an undifferentiated bag of `Memory`
rows. It has no notion of a *step* — no ordering, no per-step status, no
idempotency key, no checkpoint/resume. Clients tracking a multi-step process
cannot answer "which step am I on?", "did step 3 already run?", or "resume from
the last good checkpoint", so they fall back to other methods.

Web research on durable-execution engines (Temporal, Restate, Conductor,
LangGraph) converges on a consistent model: explicit journaled steps, idempotency
keys per step, and checkpointing for resumability. This change brings that model
to TaskStreams.

This tier MUST land after Tiers 1 and 2 — it builds new schema and API on top of
a stream surface that is first made correct and safe.

## What Changes

- Introduce a first-class `task_step` record: `id`, `task_stream_id`, `ordinal`,
  `name`, `status` (`pending`/`running`/`completed`/`failed`/`skipped`),
  `idempotency_key`, `started_at`, `completed_at`, `result`, `error`.
- New migration defining the `task_step` table (SCHEMAFULL, schema ↔ struct
  synced) with indexes on `task_stream_id` + `ordinal`.
- New `MemoryStorage` trait methods: `add_task_step`, `update_task_step_status`,
  `get_task_steps`, `get_current_step`, `complete_step` (idempotent on
  `idempotency_key`).
- New MCP tools mirroring the above so coding agents can drive step tracking.
- `get_context_for_task` extended to surface step status alongside memories.
- Integration tests for the full step lifecycle including idempotent replay and
  resume-from-checkpoint.

## Non-goals

- A general workflow/DAG engine — steps are linear and ordinal-based.
- Automatic step execution or retries — the client drives transitions; this
  change provides durable *tracking* and idempotency, not orchestration.
- Replacing the existing memory-bag model — steps are additive; memories remain.

## Impact

- Affected specs: `task-stream`
- Affected code: new `crates/surreal-memory/src/task_step.rs`,
  `crates/surreal-memory/src/storage/mod.rs`,
  `crates/surreal-memory/src/storage/surreal.rs`,
  `crates/surreal-memory/src/storage/migrations/mod.rs`,
  `crates/surreal-memory/src/lib.rs` (re-exports), `src/mcp/mod.rs`,
  `src/mcp/handlers.rs`
- New migration; UAR `MemoryService` gains new pass-through methods and the
  `Cargo.toml` rev must be re-pinned.
