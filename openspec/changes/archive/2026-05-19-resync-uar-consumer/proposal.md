# Resync the UAR Consumer

## Why

Assessment findings **F-2 (HIGH)**, **F-4** and **F-5** (UAR): the
`universal-agent-runtime` repo at
`/Users/gqadonis/Projects/prometheus/universal-agent-runtime` consumes
`surreal-memory` and will not compile against the updated library.

`src/uar/memory/mcp_server.rs` calls the TaskStream API with the *pre-scoping*
arity — the `task-stream-reliability` phase added `user_id`/`agent_id` scope
arguments to those methods. UAR's `Cargo.lock` also pins an older library
commit (`01e917ac`).

This change MUST land after `commit-and-push-library` — `cargo update` cannot
resolve the new library surface until it is pushed.

## What Changes

- `cargo update -p surreal-memory` in the UAR repo to pull the pushed commit.
- Fix the 4 stale TaskStream call sites in `src/uar/memory/mcp_server.rs`:
  - `get_task_stream(&p.name)` → `(name, user_id, agent_id)`
  - `add_to_task_stream(&p.stream_name, mem)` → `(stream_name, user_id, agent_id, memory)`
  - `get_context_for_task(&p.stream_name, &p.model_name, p.max_tokens)`
    → `(stream_name, user_id, agent_id, model_name, max_tokens)`
  - `archive_task_stream(&p.name)` → `(name, user_id, agent_id)`
- Add `user_id` / `agent_id` fields to the relevant `*Params` structs so the
  scope values can be threaded from the MCP request, matching UAR's existing
  scoping convention.
- Run a full `cargo build` + `cargo test` in the UAR repo — the 4 sites are
  the *known* drift; the build may surface more, and the change must not assume
  they are exhaustive.
- Verify migrations v17→v19 apply cleanly against a **populated** UAR embedded
  database on first start, not only a fresh one.
- Commit the call-site fixes and the refreshed `Cargo.lock` in the UAR repo.

## Non-goals

- *Adopting* the new `TaskStep` API in UAR's MCP surface — that is a feature,
  not a resync. Resync = make it compile and run against the current library.
- Any change to the `surreal-memory` library itself.
- Re-pinning to a `rev` (deferred to `reconcile-library-pin-policy`).

## Impact

- Mutates the UAR repo: `src/uar/memory/mcp_server.rs`, the affected `*Params`
  structs, and `Cargo.lock`.
- No change to surreal-memory-server.
- Depends on `commit-and-push-library`.
