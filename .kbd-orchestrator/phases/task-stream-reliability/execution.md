# KBD Execution — Phase: task-stream-reliability

- **Project**: surreal-memory-server
- **Backend**: `openspec` — spec-backed traceability; all 3 changes validated
- **Dispatch model**: in-session TDD execution, change-by-change, strict order
- **Started**: 2026-05-17

## Change Order

1. `fix-taskstream-scope-bugs` ← executing now
2. `harden-taskstream-correctness`
3. `add-taskstream-steps`

## Change 1 — fix-taskstream-scope-bugs

### Blast radius (discovered during execute)
- `crates/surreal-memory/src/storage/mod.rs` — `MemoryStorage` trait signatures
- `crates/surreal-memory/src/storage/surreal.rs` — `SurrealStorage` impl + `auto_summarize` query
- `src/mcp/handlers.rs`, `src/mcp/mod.rs` — MCP tool layer
- `src/api/taskstreams.rs` — REST layer (NOT in original proposal — added to scope)
- `crates/surreal-memory/tests/integration_test.rs` — existing tests (must update call sites)

### Scope correction vs. assessment
The assessment said "no tests/ directory" — incorrect. `crates/surreal-memory/tests/integration_test.rs`
exists with TaskStream lifecycle coverage. H-3 (Tier 2) is therefore "expand"
not "create from zero". REST layer `src/api/taskstreams.rs` is also affected and
was missing from the Tier 1 proposal.

### Per-change QA
> 5+ files modified, not docs-only → artifact-refiner QA gate REQUIRED after DONE.

## Dispatch contract
TDD: write failing scope/isolation tests first, then propagate scope params
through trait → impl → MCP → REST → test call sites, then one quality-gate run.
