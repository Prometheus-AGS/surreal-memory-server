# KBD Assessment — Phase: task-stream-reliability

- **Project**: surreal-memory-server
- **Phase goal**: Diagnose and close reliability gaps in TaskStream functionality so multi-step process tracking works end-to-end without clients falling back to other methods.
- **Assessed**: 2026-05-17
- **Phase status**: assessing
- **Cross-tool progress**: none (no `progress.json` — first KBD session)

---

## 1. Scope of Assessment

Inspected the full TaskStream surface:

- `crates/surreal-memory/src/task_stream.rs` — `TaskStream`, `TaskStreamStatus`, `ContextWindow`
- `crates/surreal-memory/src/storage/mod.rs` — `MemoryStorage` trait (TaskStream methods)
- `crates/surreal-memory/src/storage/surreal.rs` — `SurrealStorage` impl (lines 112–160, 836–840, 1640–1958, 2423–2525)
- `crates/surreal-memory/src/storage/migrations/mod.rs` — v3 + v7 task_stream schema
- `crates/surreal-memory/src/model_profiles.rs` — token budget / summarization threshold
- `src/mcp/handlers.rs` — TaskStream MCP tool handlers + param structs

---

## 2. Headline Verdict (sycophancy-corrected)

**The gap is large, not small.** An initial draft of this assessment described the
implementation as "fundamentally sound" with "minor polish items." The
sycophancy-correction skill (adversarial strictness) flagged that framing as an
**S-03 critical** pattern — a substantive verdict with no surfaced risk that
directly contradicts its own findings list. The corrected position:

> TaskStream **persists records** but does **not reliably track multi-step
> processes**. It carries one data-loss bug, one cross-tenant access bug, a
> race condition on its core counter, near-zero test coverage, and — most
> importantly — **has no concept of a step**. Clients fall back to other
> methods because the feature does not model the thing they need.

Severity tally: **2 CRITICAL, 3 HIGH, 2 MEDIUM.** This is not polish.

---

## 3. What Exists and Works

| Capability | State |
|---|---|
| `TaskStream` struct + `TaskStreamStatus` enum | Works; clean enum codec, round-trip tested |
| Schema ↔ struct sync (v3 + v7 migrations) | Correct; all 12 fields defined, `UNIQUE` index on `name` |
| `create_task_stream` / `get_task_stream` / `list_task_streams` | Functional CRUD |
| `archive_task_stream` / `pause_task_stream` / `delete_task_stream` | Functional; delete cascades to memories + nulls mindmap links |
| `get_context_for_task` | Token-budgeted view; importance-ordered, greedy fill |
| `model_profiles` token budgeting | Sound; `summarization_threshold` = 80% of `context_window − reserved` |
| MCP handler surface | All 8 tools wired with typed param structs |

The data model and trait abstraction are a usable foundation. The defects below
sit *on top* of that foundation — they are fixable without a rewrite.

---

## 4. Gap Findings

### CRITICAL

**C-1 — `auto_summarize_task_stream` deletes memories from OTHER streams.**
`surreal.rs:2447` builds the compaction query as:
```sql
SELECT * FROM memory WHERE task_stream_id != NONE
  [AND user_id = '..'] [AND agent_id = '..']
ORDER BY created_at ASC LIMIT 200
```
The loaded `stream.id` is **never bound into the WHERE clause**. When an agent
has more than one active stream, summarization selects the oldest half of
memories *across all of them* and `db.delete()`s the originals (`surreal.rs:2493`).
This is silent data loss. It also fires automatically from `add_to_task_stream`
(`surreal.rs:1733`) once `total_tokens` crosses the threshold, so a long-lived
stream can destroy a sibling stream's history with no user action.
*Best-practice contrast*: durable-execution engines treat the journal as the
**source of truth** and never destructively mutate it (Restate, Temporal).

**C-2 — `get_task_stream` / mutators have no scope enforcement.**
`get_task_stream(name)` (`surreal.rs:1662`) does a global `WHERE name = $name`
lookup with no `agent_id`/`user_id` filter. `archive_task_stream`,
`pause_task_stream`, `delete_task_stream`, `add_to_task_stream`, and
`get_context_for_task` all resolve streams the same way. Any caller can read,
mutate, archive, or delete **any other agent's or user's** stream by guessing
its name. This violates the scoping model documented in `CLAUDE.md`
("Who writes / Who reads" table) and the project's "know the scoping rules"
operational rule.

### HIGH

**H-1 — No step / checkpoint / idempotency model (the root cause).**
A `TaskStream` is a name plus an undifferentiated bag of `Memory` rows. There is
no `step`, no ordered sequence, no per-step status (pending/running/done/failed),
no idempotency key, no resumability. Clients tracking a *multi-step process*
cannot answer "which step am I on?", "did step 3 already run?", or "resume from
the last good checkpoint." Every researched best-practice system models exactly
this: Temporal/Restate journal each step with replay; LangGraph checkpoints
state per node; the durable-execution literature converges on
*idempotency keys + dedup table + checkpointing*. **This is why clients abandon
the feature** — it does not represent the domain object they need.

**H-2 — `total_tokens` update is a non-atomic read-modify-write race.**
`add_to_task_stream` (`surreal.rs:1689–1721`) stores the memory, then issues a
*separate* `UPDATE task_stream SET total_tokens += $tokens`. Two concurrent adds
interleave between the memory write and the counter update; `needs_summarization`
then reads a stale total. The `+=` is server-side so the counter itself won't
corrupt, but the *summarization trigger decision* and any client reading
`total_tokens` between the two statements sees inconsistent state. There is no
transaction wrapping memory-insert + counter-update.

**H-3 — Near-zero test coverage.**
`task_stream.rs` has 2 unit tests, both for `TaskStreamStatus` string codec.
There are **no integration tests** — no `tests/` directory in either crate.
Nothing exercises create→add→get_context→summarize→archive, the C-1 multi-stream
deletion path, the C-2 cross-scope access, or the H-2 race. The project's own
operational rule #5 ("test against the actual consumer") is unmet for this
feature. Defects of this severity surviving to clients is the predictable result.

### MEDIUM

**M-1 — Global `UNIQUE` index on `name` blocks legitimate reuse.**
Migration v3 (`migrations/mod.rs:174`) defines
`DEFINE INDEX task_stream_name ON task_stream FIELDS name UNIQUE`. `name` is
global, but `agent_id`/`user_id` are separate fields. Two different agents (or
users) cannot both have a stream called `"build"` — the second
`create_task_stream` fails at the unique constraint. The intended model is
per-agent/per-user uniqueness; the index should be a composite
`FIELDS agent_id, user_id, name`.

**M-2 — `auto_summarize` quality + accounting weaknesses.**
Compaction concatenates raw memory contents with `" | "` (`surreal.rs:2481`) —
no LLM summarization despite the name, so it can *grow* tokens for short inputs.
The deleted memories' tokens are **not subtracted** from `total_tokens`, so the
counter monotonically overcounts and re-triggers summarization indefinitely.
The summary memory is created without `task_stream_id`, so it is orphaned from
the stream it summarizes and excluded from future `get_context_for_task`.

---

## 5. Best-Practice Benchmark (Tavily research)

Sources: Conductor (durable execution best practices), Restate ("What is Durable
Execution"), Temporal ("Idempotency and Durable Execution"), Morling (SQLite DE
engine), Taskade (DE for AI workflows), JetBrains Research & Mem0 (context
management 2025), LangGraph state-management guides.

| Best practice | Industry standard | surreal-memory TaskStream |
|---|---|---|
| Explicit journaled **steps** | Every step recorded before result observed | Absent — flat memory bag (H-1) |
| **Idempotency keys** per step | `workflowId+taskId`, dedup table, claim-slot-first | Absent (H-1) |
| **Checkpointing / resumability** | Save state after each step; resume from last | Absent (H-1) |
| Journal is **immutable source of truth** | Never destructively mutate history | C-1 deletes history |
| **Scope isolation** of workflow state | Per-tenant namespacing | C-2 — no scope filter |
| **Atomic** state transitions | Transactional step writes | H-2 — non-atomic RMW |
| **Signals vs queries** separation | Mutations vs reads distinct | Partially OK (status mutators exist) |
| Summarization preserves stable facts | Compress mid-history, pin stable facts | M-2 — naive concat, orphaned summary |

The current design matches the *context-window-management* half of the
literature (token budgeting, `get_context_for_task` is genuinely close to
best practice) but is missing the *durable-execution* half entirely — and that
half is what "multi-step process tracking" means.

---

## 6. Gap Sizing & Path to Close

**Size: LARGE.** Two correctness bugs (C-1, C-2) make the feature unsafe for
multi-tenant or multi-stream use *today*. The strategic gap (H-1) is a missing
domain abstraction, not a bug — it requires new schema, not a patch.

Close in three ordered tiers (do not start any tier before the prior is planned):

1. **Stop the bleeding (C-1, C-2)** — scope the `auto_summarize` query to
   `stream.id`; add `agent_id`/`user_id` scope filtering to `get_task_stream`
   and all stream mutators. Smallest, highest-urgency change set.
2. **Correctness & confidence (H-2, H-3, M-1, M-2)** — wrap memory-insert +
   counter-update in a transaction; fix the unique index to a composite; fix
   summary token accounting + `task_stream_id` linkage; build the integration
   test suite that should have caught C-1/C-2.
3. **Close the strategic gap (H-1)** — introduce a first-class `task_step`
   concept: ordered steps, per-step status, idempotency key, checkpoint/resume
   API. New migration, new struct, new trait methods, new MCP tools. This is the
   change that lets clients stop falling back to other methods.

---

## 7. Next KBD Step

This is the **Assess** phase output only. Per KBD ordering, do **not** write code
or specs yet. Proceed to **`/kbd-plan task-stream-reliability`** (or
`/opsx:new` for an OpenSpec change) to turn the three tiers above into an
ordered, prioritized change list. Recommend each tier be its own change so Tier 1
can ship independently and fast.
