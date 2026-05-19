# Project Constraints — surreal-memory-server

Derived from `CLAUDE.md` "AI Agent Operational Rules" and "CRITICAL: Schema ↔ Struct Sync Rule".

## Blocking Constraints (must never be violated)

- **Schema ↔ Struct sync**: Every field on a Rust struct persisted to SurrealDB MUST have a corresponding `DEFINE FIELD` in `crates/surreal-memory/src/storage/migrations/mod.rs`. SCHEMAFULL tables reject unknown fields at runtime.
- **Migrations are non-negotiable**: Adding any field to `Memory`, `Entity`, `TaskStream`, `MindMap` or other persisted struct requires an immediate new migration with an incremented version in the `MIGRATIONS` array.
- **No parallel storage layers**: Read `lib.rs`, `storage/mod.rs`, and `migrations/mod.rs` before writing storage code. Do not duplicate existing storage abstractions.
- **No secrets in source**: API keys, tokens, passwords must come from environment variables.

## Warning Constraints (should be respected)

- Files under 800 lines; functions under 50 lines.
- One build verification per change set, then proceed.
- For CLI/agent consumers use `user_id = "anonymous"` so queries filter by `agent_id`.
- `option<object>` is not flexible — use `FLEXIBLE TYPE option<object>` for nested JSON.
- Quality gate before commit: `./scripts/quality-check.sh` (fmt, clippy, test).
