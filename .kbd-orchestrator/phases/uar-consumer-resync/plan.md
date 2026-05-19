# KBD Plan — Phase: uar-consumer-resync

- **Project**: surreal-memory-server (library) + 2 downstream consumers
- **Planned**: 2026-05-19
- **Change backend**: OpenSpec
- **Source assessment**: `.kbd-orchestrator/phases/uar-consumer-resync/assessment.md`
- **Assessment verdict**: MEDIUM gap, BLOCKED on F-1. 1 BLOCKER, 1 HIGH, 3 MEDIUM, 1 LOW.

---

## Planning Notes

This phase is unusual: it spans **three repositories**.

- `surreal-memory-server` — the library; OpenSpec changes are tracked here.
- `universal-agent-runtime` (UAR) — `/Users/gqadonis/Projects/prometheus/universal-agent-runtime`
- `librefang` — `/Users/gqadonis/Projects/references/librefang`

OpenSpec changes live in `surreal-memory-server/openspec/changes/` (the KBD
project root), but the *code edits* for changes 2-3 land in the consumer repos.
Each consumer repo is its own git repo — commits happen there, separately.

**F-1 gates the entire phase.** Changes 2-5 cannot start until change 1 has
committed and pushed the library. The change order is therefore strict, not a
convenience.

**`cargo update` against a `branch = "main"` pin** resolves to the latest
*pushed* commit. There is no `rev` to bump (the assessment found the pins drifted
from `rev` to `branch`); a plain `cargo update -p surreal-memory` in each
consumer is what refreshes the lock — provided change 1 has pushed first.

QA gate per change: `rust-reviewer` against `.kbd-orchestrator/constraints.md`
(artifact-refiner unavailable in this environment, consistent with prior phases).
For consumer repos, "build health" = that repo's own `cargo build` + `cargo test`.

---

## Ordered Change List

| # | Change ID | Closes | Repo touched | Depends on |
|---|-----------|--------|--------------|------------|
| 1 | `commit-and-push-library` | F-1 | surreal-memory-server | — |
| 2 | `resync-uar-consumer` | F-2, F-4 (UAR), F-5 (UAR) | universal-agent-runtime | change 1 |
| 3 | `resync-librefang-consumer` | F-3, F-4 (librefang), F-5 (librefang) | librefang | change 1 |
| 4 | `reconcile-library-pin-policy` | F-6 | UAR + librefang + surreal-memory-server CLAUDE.md | changes 2, 3 |

Changes 2 and 3 are independent of each other and may run in parallel once
change 1 is done. Change 4 lands last so it pins the *verified* commit.

---

## Change 1 — `commit-and-push-library`

**Goal**: Make the two completed library phases resolvable by consumers.

**Why first**: F-1 BLOCKER. `cargo update` on a branch pin cannot see a working
tree; 31 uncommitted files (the entire `task-stream-reliability` +
`write-path-reliability` output) must be on `main`.

**Scope**:
- The user must authorize the commit — KBD does not commit unsolicited. The
  change's first task is an explicit go/no-go gate with the user.
- Before committing, fix (or explicitly defer with the user's agreement) the
  pre-existing `tests/contract_alignment_test.rs:300/312` compile error
  (`Option<HashMap>` vs `Option<Value>`) so the library's own
  `cargo check --tests` is green — a consumer should not resync to a library
  whose test build is red.
- Commit the two phases (sensible commit boundaries — ideally one commit per
  KBD phase, or per archived OpenSpec change) and push to `main`.
- Record the resulting commit SHA — changes 2-4 pin against it.

**Recommended agent**: rust-build-resolver (for the `contract_alignment_test.rs`
fix), then a plain git commit/push. **QA**: library `cargo check --tests` green.

**Risk**: this is the only change that mutates the library repo's git history.
No code logic changes — the two phases' code is already written, reviewed, and
QA-passed; this change only commits it.

---

## Change 2 — `resync-uar-consumer`

**Goal**: UAR compiles, tests, and runs against the pushed library.

**Repo**: `/Users/gqadonis/Projects/prometheus/universal-agent-runtime`

**Scope** (closes F-2, F-4-UAR, F-5-UAR):
- `cargo update -p surreal-memory` to pull the commit from change 1.
- Fix the 4 stale TaskStream call sites in `src/uar/memory/mcp_server.rs`:
  - `:718` `get_task_stream(&p.name)` → add `user_id`, `agent_id`
  - `:732` `add_to_task_stream(&p.stream_name, mem)` → add `user_id`, `agent_id`
  - `:747` `get_context_for_task(&p.stream_name, &p.model_name, p.max_tokens)`
    → add `user_id`, `agent_id`
  - `:773` `archive_task_stream(&p.name)` → add `user_id`, `agent_id`
- Add `user_id` / `agent_id` fields to the relevant `*Params` structs
  (`TaskStreamNameParams`, `AddToTaskStreamParams`, `GetContextParams`, and the
  archive params) so the scope values can be threaded from the MCP request.
  Match the scoping convention already used elsewhere in UAR.
- `cargo build` + `cargo test` in the UAR repo until green.
- Verify migrations v17→v19 apply cleanly against a **populated** UAR embedded
  DB on first start, not only a fresh one (F-5).
- Commit the call-site fixes + refreshed `Cargo.lock` in the UAR repo.

**Recommended agent**: rust-reviewer for QA; rust-build-resolver if the build
surfaces further drift beyond the 4 known sites.

**Risk**: the assessment identified 4 call sites by signature diff; the build
may surface more (e.g. other modules calling the scoped TaskStream API). The
change must run a full `cargo build`, not assume the 4 sites are exhaustive.

---

## Change 3 — `resync-librefang-consumer`

**Goal**: librefang compiles, tests, and runs against the pushed library.

**Repo**: `/Users/gqadonis/Projects/references/librefang`

**Scope** (closes F-3, F-4-librefang, F-5-librefang):
- `cargo update -p surreal-memory` to pull the commit from change 1.
- `cargo build` + `cargo test` in the librefang workspace.
- Expected outcome: **no code change** — librefang wraps `SurrealStorage`, never
  implements `MemoryStorage`, never calls the TaskStream/TaskStep surface, and
  uses `RetryConfig::default()`. Confirm this expectation; if the build does
  surface drift, fix it (and the plan's "no-code-change" assumption was wrong —
  record that).
- Verify migrations v17→v19 apply cleanly against a populated librefang
  embedded DB (F-5).
- Commit the refreshed `Cargo.lock` in the librefang repo.

**Recommended agent**: rust-reviewer for QA (likely a lock-only change).

**Risk**: low. The main risk is the migration first-run effect on an existing
librefang DB, same as F-5 for UAR.

---

## Change 4 — `reconcile-library-pin-policy`

**Goal**: Resolve the `branch = "main"` vs `rev` pin drift and reconcile
CLAUDE.md rule #7 with reality.

**Repos**: UAR `Cargo.toml`, librefang `Cargo.toml`, surreal-memory-server
`CLAUDE.md`.

**Scope** (closes F-6):
- Decide: keep the `branch = "main"` pin (convenient, but every `cargo update`
  silently absorbs unreviewed `main`) **or** move both consumers to a `rev` pin
  (reviewable, matches the documented rule). Record the decision and rationale
  in the change's proposal.
- If moving to `rev`: pin both consumers to the **verified** commit from
  change 1 (verified = changes 2 and 3 built and tested green against it).
- Reconcile `surreal-memory-server/CLAUDE.md` rule #7 with whatever pin policy
  is chosen — the doc currently describes a `rev` pin that the code does not use.

**Recommended agent**: rust-reviewer (small, doc + Cargo.toml).

**Why last**: it should pin the commit that changes 2-3 have *proven* good, not
a commit that merely exists.

---

## Phase Exit Criteria

- surreal-memory-server: two phases committed + pushed to `main`; library
  `cargo check --tests` green.
- UAR: `cargo build` + `cargo test` green against the new library; lock + call-site
  fixes committed.
- librefang: `cargo build` + `cargo test` green against the new library; lock
  committed.
- Migrations v17→v19 verified safe against a populated DB for each consumer.
- Pin policy decided and recorded; CLAUDE.md rule #7 reconciled.
- All 6 findings (F-1…F-6) closed.

---

## Plan Status: COMPLETE

4 ordered OpenSpec changes emitted. Strict order: change 1 (F-1 blocker) gates
2-4. Next step: `/kbd-execute commit-and-push-library` — which begins with an
explicit user authorization gate for the library commit.
