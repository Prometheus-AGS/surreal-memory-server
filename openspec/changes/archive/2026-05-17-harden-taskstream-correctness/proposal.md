# Harden TaskStream Correctness

## Why

After the CRITICAL scope bugs are fixed, the assessment still records HIGH and
MEDIUM defects that make TaskStream unreliable and untrustworthy:

- **H-2** — `add_to_task_stream` updates `total_tokens` in a separate query from
  the memory insert; concurrent adds race and the summarization trigger reads
  stale state.
- **H-3** — Near-zero test coverage (2 unit tests, no integration tests); the
  CRITICAL bugs reached clients precisely because nothing exercised the feature.
- **M-1** — A global `UNIQUE` index on `task_stream.name` blocks two agents from
  reusing a name; the second `create_task_stream` fails.
- **M-2** — `auto_summarize` concatenates raw content (can grow tokens), never
  subtracts compacted tokens from `total_tokens` (counter overcounts forever),
  and creates the summary memory without a `task_stream_id` (orphaned).

This tier makes the existing surface correct and gives it the test coverage that
should have caught Tier 1.

## What Changes

- Wrap the memory insert + `total_tokens` update in a single SurrealDB
  transaction in `add_to_task_stream`.
- Replace the global unique index with a composite
  `FIELDS agent_id, user_id, name UNIQUE` via a new migration.
- Fix `auto_summarize_task_stream`: subtract compacted tokens from
  `total_tokens`, link the summary memory to the stream via `task_stream_id`,
  and use the embedding/LLM summarization path rather than naive `" | "` concat.
- Build an integration test suite covering the full TaskStream lifecycle.

## Non-goals

- The `task_step` abstraction (Tier 3).
- Re-architecting the summarization model — only correctness fixes here.

## Impact

- Affected specs: `task-stream`
- Affected code: `crates/surreal-memory/src/storage/surreal.rs`,
  `crates/surreal-memory/src/storage/migrations/mod.rs`, new
  `crates/surreal-memory/tests/`
- New migration required — increment the `MIGRATIONS` array, keep schema ↔ struct
  sync per `CLAUDE.md`.
