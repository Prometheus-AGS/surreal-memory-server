# KBD Reflection — Phase: task-stream-reliability

- **Project**: surreal-memory-server
- **Phase goal**: Close reliability gaps in TaskStream so multi-step process
  tracking works end-to-end without clients falling back to other methods.
- **Reflected**: 2026-05-17
- **Change backend**: OpenSpec
- **Evolver cycle**: none

---

## 1. Goal Achievement

**Overall: MET.** All 7 assessment findings (2 CRITICAL, 3 HIGH, 2 MEDIUM) were
addressed across 3 ordered, independently-shipped OpenSpec changes.

| Finding | Severity | Change | Status |
|---|---|---|---|
| C-1 auto-summary cross-stream data loss | CRITICAL | fix-taskstream-scope-bugs | MET |
| C-2 cross-tenant stream access | CRITICAL | fix-taskstream-scope-bugs | MET |
| H-1 no step/checkpoint/idempotency model | HIGH | add-taskstream-steps | MET |
| H-2 non-atomic token accounting race | HIGH | harden-taskstream-correctness | MET |
| H-3 near-zero test coverage | HIGH | harden-taskstream-correctness | MET |
| M-1 global unique index blocks name reuse | MEDIUM | harden-taskstream-correctness | MET |
| M-2 summary accounting + orphaned summary | MEDIUM | harden-taskstream-correctness | MET |

The root cause of client abandonment (H-1 — TaskStream had no concept of an
ordered, status-tracked step) is now closed: `TaskStep` provides ordering,
per-step status, idempotency keys, and resume-from-checkpoint.

---

## 2. Delivered Changes

| # | Change | Archive | Tests |
|---|--------|---------|-------|
| 1 | fix-taskstream-scope-bugs | `2026-05-17-fix-taskstream-scope-bugs` | integration_test 24 pass |
| 2 | harden-taskstream-correctness | `2026-05-17-harden-taskstream-correctness` | integration_test 27, migrations lib 9 |
| 3 | add-taskstream-steps | `2026-05-17-add-taskstream-steps` | integration_test 35, surreal-memory lib 33 |

New artifacts: `task_step.rs` module, migrations v18 (composite unique index) and
v19 (`task_step` table), 5 new `MemoryStorage` methods, 5 new MCP tools.
Canonical spec `openspec/specs/task-stream/spec.md` grew from 0 → 10 requirements.

---

## 3. Artifact Quality Summary

The dedicated artifact-refiner tool was not available in this environment; the
QA gate was performed by the `rust-reviewer` agent against
`.kbd-orchestrator/constraints.md`. No `.refiner/artifacts/` logs exist — QA
results are recorded in `progress.json → change_results`.

| Metric | Value |
|---|---|
| Changes with QA | 3/3 |
| First-pass pass rate | 1/3 (33%) |
| Changes requiring refinement | 2 (changes 2 and 3) |
| Total refinement iterations | 2 (one per blocked change) |
| Final pass rate after refinement | 3/3 (100%) |

### QA findings caught (and fixed) before archive

- **Change 2** — BLOCK: 1 CRITICAL (v18 migration missing `NONE`→`''` backfill,
  leaving a mixed-state unique index), 1 HIGH (transaction result read by
  hardcoded statement index), 1 MEDIUM (token drift).
- **Change 3** — BLOCK: 2 HIGH (`started_at` unset on direct `complete_step`;
  imprecise failed-step doc), 2 MEDIUM (TOCTOU in `add_task_step`; `cargo fmt`).

### Recurring patterns

- **Migration / schema edge cases** (change 2): SurrealDB composite-index NULL
  semantics required a backfill not obvious from the struct change alone.
- **Driver-dependent assumptions** (change 2): result-set indexing for
  multi-statement transactions. Both were caught only by review, not by the
  build — reinforcing that the build passing is not the same as correctness.

The QA gate demonstrably earned its place: 2 of 3 changes shipped a CRITICAL or
HIGH defect past a green build and were stopped before archive.

---

## 4. Technical Debt Introduced

- **`auto_summarize` still uses naive `" | "` concatenation** — not true
  summarization. M-2 fixed the accounting and linkage but left the compaction
  quality weak (no LLM/embedding summary path existed to call). Tracked as a
  future improvement, not a regression.
- **No REST endpoints for `TaskStep`** — the 5 new methods are exposed via MCP
  only. REST `src/api/taskstreams.rs` was out of scope for change 3.
- **`get_current_step`** treats a `Failed` step as blocking (correct
  durable-execution semantics) but there is no first-class `retry_step` /
  `skip_step` convenience — callers use `update_task_step_status` directly.

---

## 5. Deferred Follow-ups (from progress.json)

1. **REST scope gap** — `src/api/taskstreams.rs` passes `None` for
   `user_id`/`agent_id`; needs auth-header/JWT scope extraction. Pre-existing,
   not a regression.
2. **Pre-existing compile error** — `tests/contract_alignment_test.rs:300`
   (`Option<HashMap>` vs `Option<Value>`) fails on clean `main`, unrelated to
   TaskStream, breaks `cargo check --tests`. Needs its own change.
3. **UAR consumer re-pin** — `MemoryStorage` trait signatures changed (breaking)
   and 5 `TaskStep` methods were added. UAR's `MemoryService` wrapper and the
   `surreal-memory` git rev in UAR's `Cargo.toml` must be updated per CLAUDE.md
   rule #7. **This is the highest-priority follow-up** — the library change is
   not consumable until UAR is updated.
4. **TaskStep REST surface** — add REST endpoints to match the MCP tools.

---

## 6. Lessons Captured

- **The assessment was wrong about test coverage.** It claimed "no tests/
  directory"; `crates/surreal-memory/tests/integration_test.rs` existed. An
  aborted `grep` (exit 1) masked it. Lesson: verify negative claims ("X does not
  exist") with a positive command, not the absence of grep output.
- **Sycophancy correction changed the verdict.** The initial assessment draft
  called the gap "small / minor polish"; the sycophancy-correction skill flagged
  that as S-03 understatement contradicting its own findings. The corrected
  "large gap" framing set the right urgency for a 3-tier plan.
- **A green build is not correctness.** Both CRITICAL/HIGH defects in changes 2
  and 3 compiled and passed tests; only review caught them. Keep the QA gate.
- **OpenSpec archive spec-merge is strict.** `MODIFIED` requirement headers must
  already exist in the canonical spec; `--skip-specs` + manual merge was needed
  because the canonical `task-stream` spec started empty.

---

## 7. Recommended Focus for Next Phase

**Next phase: `uar-consumer-resync`** — update UAR's `MemoryService` to the new
trait surface, expose the `TaskStep` methods, and re-pin the `Cargo.toml` git
rev. Without this, the library improvements are not reachable by the primary
consumer. The pre-existing `contract_alignment_test.rs` compile error should be
folded in or fixed first so CI is green.

Lower priority: TaskStep REST endpoints, and replacing naive summarization
concatenation with a real summary path.

---

## Phase Status: COMPLETE

Assessment → Plan → Execute → Reflect all done. 3/3 changes archived, QA-passed,
canonical spec valid. No work committed to git (KBD does not commit unless
asked) — all changes are staged/uncommitted for the user to review and commit.
