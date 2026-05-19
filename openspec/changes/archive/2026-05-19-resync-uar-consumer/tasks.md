# Tasks — resync-uar-consumer

Repo: `/Users/gqadonis/Projects/prometheus/universal-agent-runtime`

## 1. Pull the updated library
- [x] 1.1 `cargo update -p surreal-memory` — `01e917ac` → `9051c3d3`
- [x] 1.2 `Cargo.lock` now pins `surreal-memory ...#9051c3d3` (the resync target)

## 2. Fix stale TaskStream call sites (F-2)
- [x] 2.1 `mcp_server.rs` `task_stream_get` — `get_task_stream` now passes `user_id`, `agent_id`
- [x] 2.2 `mcp_server.rs` `task_stream_add` — `add_to_task_stream` now passes `user_id`, `agent_id`
- [x] 2.3 `mcp_server.rs` `task_stream_context` — `get_context_for_task` now passes `user_id`, `agent_id`
- [x] 2.4 `mcp_server.rs` `task_stream_archive` — `archive_task_stream` now passes `user_id`, `agent_id`
- [x] 2.5 Added `user_id`/`agent_id` `#[serde(default)] Option<String>` fields to `TaskStreamNameParams`, `AddToTaskStreamParams`, `GetContextParams` (matching the existing `AutoSummarizeParams` convention)
- [x] 2.6 DEVIATION (build surfaced 4 MORE sites — plan anticipated this): `MemoryService` facade in `service.rs` also had stale arity — `get_task_stream`, `add_to_task_stream`, `task_stream_context`, `archive_task_stream` updated to thread `user_id`/`agent_id` through to the scoped storage API

## 3. Build, test, verify
- [x] 3.1 Full `cargo build` in the UAR repo — clean (8 errors surfaced, all 8 fixed)
- [x] 3.2 `cargo test` — 207 tests pass. 4 `config_integration` tests FAIL — confirmed PRE-EXISTING (fail identically on clean UAR `main` with this change stashed); unrelated to the resync (config-loading defect, not TaskStream). Recorded as a deferred follow-up.
- [x] 3.3 Migration v17→v19 safety: verified at the library level — the library's own migration tests apply v14→v19 against seeded legacy `task_stream` rows incl. the v18 NONE→'' backfill (110-test library run, all pass). Engine-agnostic DDL (`Surreal<Any>`), idempotent + version-gated. NOT independently re-verified by booting UAR against a production DB because UAR's app-boot is blocked by the same pre-existing config defect as 3.2.

## 4. Commit + QA
- [x] 4.1 Committed `b52b804` in the UAR repo — 3 files (mcp_server.rs, service.rs, Cargo.lock), 70+/29-
- [x] 4.2 QA gate: rust-reviewer — APPROVED first pass. Verified all 4 questions (param pattern, pub-signature change, None-scope semantics, no missed call sites). One MEDIUM finding (`MemoryService::new` embedded path uses `SurrealMode::Server` + `surrealkv://`) concerns PRE-EXISTING UAR code NOT touched by this change — recorded as a deferred follow-up, not in scope.
