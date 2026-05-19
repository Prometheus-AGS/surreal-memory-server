# KBD Plan — Phase: task-stream-reliability

- **Project**: surreal-memory-server
- **Phase goal**: Close reliability gaps in TaskStream so multi-step process
  tracking works end-to-end without clients falling back to other methods.
- **Planned**: 2026-05-17
- **Change backend**: OpenSpec (`openspec/`, schema `spec-driven`)
- **Evolver bridge**: none (`.evolver/` absent — not an evolution cycle)
- **Source**: `.kbd-orchestrator/phases/task-stream-reliability/assessment.md`

## Ordered Change List

Changes are strictly ordered. Each is a self-contained OpenSpec change so Tier 1
can ship independently and fast. Do **not** start a change before the prior one
is complete and verified.

| # | Change ID | Tier | Addresses | Recommended agent |
|---|-----------|------|-----------|-------------------|
| 1 | `fix-taskstream-scope-bugs` | Stop the bleeding | C-1, C-2 | rust-reviewer + tdd-guide |
| 2 | `harden-taskstream-correctness` | Correctness & confidence | H-2, H-3, M-1, M-2 | tdd-guide + rust-reviewer |
| 3 | `add-taskstream-steps` | Close strategic gap | H-1 | architect → tdd-guide → rust-reviewer |

### 1. fix-taskstream-scope-bugs — CRITICAL, do first
Stops active data loss (C-1: auto-summary deletes other streams' memories) and a
cross-tenant access hole (C-2: stream resolution has no scope filter). Smallest
change set; ships alone. Breaking trait-signature change → UAR re-pin required.

### 2. harden-taskstream-correctness — after #1
Atomic token accounting (H-2), composite unique index (M-1), summarization
accounting + linkage fixes (M-2), and the integration test suite (H-3) that
should have caught the Tier 1 bugs. Adds one migration.

### 3. add-taskstream-steps — after #2
Introduces the missing domain object: a first-class `task_step` with ordering,
status, idempotency key, and resumability. This is the change that lets clients
stop falling back to other methods. New table, struct, trait methods, MCP tools.

## Cross-Cutting Constraints (from `.kbd-orchestrator/constraints.md`)

- Every new persisted field needs a `DEFINE FIELD` migration (Tiers 2 & 3 add
  migrations — increment the `MIGRATIONS` array, keep schema ↔ struct sync).
- After each change: one quality-gate run (`./scripts/quality-check.sh`), then
  proceed. No verification loops.
- After Tiers 1 and 3 (trait signature changes), update UAR's `MemoryService`
  and re-pin the `surreal-memory` git rev in UAR's `Cargo.toml`.

## Next KBD Step

Plan phase complete. Proceed to **`/kbd-execute fix-taskstream-scope-bugs`** to
select an execution backend and dispatch change #1. Execute changes strictly in
order 1 → 2 → 3.
