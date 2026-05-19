# Tasks — resync-uar-consumer

Repo: `/Users/gqadonis/Projects/prometheus/universal-agent-runtime`

## 1. Pull the updated library
- [ ] 1.1 `cargo update -p surreal-memory` to resolve the commit from `commit-and-push-library`
- [ ] 1.2 Confirm `Cargo.lock` now pins the new library commit

## 2. Fix stale TaskStream call sites (F-2)
- [ ] 2.1 `src/uar/memory/mcp_server.rs:718` — `get_task_stream` add `user_id`, `agent_id`
- [ ] 2.2 `src/uar/memory/mcp_server.rs:732` — `add_to_task_stream` add `user_id`, `agent_id`
- [ ] 2.3 `src/uar/memory/mcp_server.rs:747` — `get_context_for_task` add `user_id`, `agent_id`
- [ ] 2.4 `src/uar/memory/mcp_server.rs:773` — `archive_task_stream` add `user_id`, `agent_id`
- [ ] 2.5 Add `user_id`/`agent_id` fields to the relevant `*Params` structs; thread the scope values through, matching UAR's existing scoping convention

## 3. Build, test, verify
- [ ] 3.1 Full `cargo build` in the UAR repo — fix any further drift the build surfaces beyond the 4 known sites
- [ ] 3.2 `cargo test` in the UAR repo green
- [ ] 3.3 Verify migrations v17→v19 apply cleanly against a populated UAR embedded DB on first start (F-5)

## 4. Commit + QA
- [ ] 4.1 Commit the call-site fixes + refreshed `Cargo.lock` in the UAR repo
- [ ] 4.2 QA gate: rust-reviewer against `.kbd-orchestrator/constraints.md`
