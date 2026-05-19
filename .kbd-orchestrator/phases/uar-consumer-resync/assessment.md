# KBD Assessment — Phase: uar-consumer-resync

- **Project**: surreal-memory-server (library) — this phase touches its
  **two downstream consumers**:
  - `universal-agent-runtime` (UAR) — `/Users/gqadonis/Projects/prometheus/universal-agent-runtime`
  - `librefang` — `/Users/gqadonis/Projects/references/librefang`
- **Phase goal**: Re-synchronize both consumer repos to the `surreal-memory`
  library surface after two completed library phases
  (`task-stream-reliability`, `write-path-reliability`), so the library
  improvements are actually reachable by the consumers and both repos compile,
  test, and run against the current library.
- **Assessed**: 2026-05-19
- **Change backend**: OpenSpec
- **Specs consulted**: `openspec/specs/task-stream/spec.md`,
  `openspec/specs/write-path/spec.md`.

---

## 0. Headline Verdict

**MEDIUM GAP, BLOCKED ON AN EXTERNAL PREREQUISITE.**

This is **not** a light-touch mechanical phase. The single largest fact:
**every byte of both completed library phases is uncommitted.** `git log` on
surreal-memory-server ends at `edca63d` ("Configure local embedding Docker
setup"); `task_step.rs` has never been committed (`git log --all` returns
nothing for it); `git status` shows **31 modified/new files** — the entire
`task-stream-reliability` *and* `write-path-reliability` output sitting in the
working tree.

Both consumers pin the library by `branch = "main"`. `cargo update` resolves a
branch pin to the **latest pushed commit** — it cannot see a working tree.
Therefore the resync **cannot begin** until surreal-memory-server's two phases
are committed and pushed. This is a hard, blocking prerequisite, not a caveat.

Once unblocked, the *consumer-side* work is moderate: UAR has real
compile-breaking call-site drift; librefang is low-risk; and both pick up
schema migrations that run against their existing embedded databases.

> **Sycophancy correction applied.** The initial draft verdict called this
> "small-to-medium / mostly mechanical / light-touch" and led with the
> reassuring findings (no trait-impl break, `Default`-derived config). The
> sycophancy-correction skill flagged it S-03 (critical — substantive verdict
> with zero risks surfaced). The draft ignored the dominant fact (all library
> work uncommitted → phase blocked) and the migration-on-consumer-DB risk.
> Corrected here.

---

## 1. Phase Goals

| # | Goal | Rationale |
|---|------|-----------|
| G-1 | surreal-memory-server's two completed phases are committed and pushed to `main` so consumers can resolve them. | Hard prerequisite — see F-1. |
| G-2 | UAR compiles cleanly against the updated library (`cargo build`, `cargo test`). | F-2: real call-site arity drift. |
| G-3 | librefang compiles cleanly against the updated library. | F-3: low-risk, but `cargo update` must be verified. |
| G-4 | Both consumers' `Cargo.lock` files are refreshed and pin the new library commit. | F-4. |
| G-5 | Schema migrations v17→v19 are confirmed safe against each consumer's existing embedded database. | F-5: migrations run on consumer DBs. |
| G-6 | The `branch = "main"` pin hazard is acknowledged and a pinning decision recorded. | F-6. |

---

## 2. Evidence Gathered

### Library-side state
- `git log` HEAD = `edca63d`; `git status` = 31 uncommitted files.
- `task_step.rs` never committed (`git log --all` empty for it) — confirms the
  prior `task-stream-reliability` phase is also uncommitted, not just this one.
- New `MemoryStorage` trait methods (no default impls): `add_task_step`,
  `update_task_step_status`, `get_task_steps`, `get_current_step`,
  `complete_step` (`storage/mod.rs:166-209`).
- Prior-phase scoped signatures (already on `main`-uncommitted): `get_task_stream`,
  `add_to_task_stream`, `get_context_for_task`, `archive_task_stream`,
  `pause_task_stream`, `delete_task_stream` all gained `user_id`/`agent_id`.
- `write-path-reliability` delta: `RetryConfig` gained `pub operation_deadline_ms`;
  `EmbeddingService` gained `is_ready()` (defaulted); `SurrealStorage::new`
  signature unchanged (`config: &SurrealConfig, embedding_service` — 2 args).

### UAR (`universal-agent-runtime`)
- `Cargo.toml:140` — `surreal-memory = { git = "...surreal-memory-server.git",
  branch = "main", package = "surreal-memory", default-features = false }`.
- `Cargo.lock` pins `surreal-memory` at rev `01e917ac` — an **older** commit
  than current HEAD `edca63d`.
- Consumes the library via `SurrealStorage` directly (`MemoryService`,
  `src/uar/memory/service.rs:126` calls `SurrealStorage::new(&cfg, svc)`),
  building `SurrealConfig { ..Default::default() }`. **Does NOT implement
  `MemoryStorage`.**
- `surrealdb = "=3.0.5"` (`Cargo.toml:199`) — matches the library pin.
- WISC binary is **gone** — `src/bin/` contains only `test_db_setup.rs`. The
  CLAUDE.md `uar-wisc` references are stale.

### librefang
- `Cargo.toml:107` — `surreal-memory = { git = "...surreal-memory-server",
  branch = "main", default-features = false, features = ["embedded"] }`.
- `Cargo.lock` pins `surreal-memory` at rev `edca63d` — exactly current
  surreal-memory-server HEAD.
- `crates/librefang-memory/src/backends/surreal.rs` **wraps**
  `Option<Arc<surreal_memory::SurrealStorage>>` (`with_extended` / `extended()`).
  **Does NOT implement `MemoryStorage`.**
- Uses `RetryConfig::default()` (`backends/shared.rs:77,97`).
- Does **not** call the TaskStream/TaskStep surface at all.
- `surrealdb = "=3.0.5"` (`Cargo.toml:106`) — matches.

---

## 3. Findings (Gap Report)

### F-1 — BLOCKER — All library work is uncommitted; the resync cannot start

Both completed KBD phases (`task-stream-reliability` + `write-path-reliability`,
~31 files including the entire `task_step.rs` module, migrations v18/v19, the
`write-path` changes) are uncommitted on surreal-memory-server. Consumers pin by
`branch = "main"`; `cargo update` resolves that to the latest **pushed** commit.
Until surreal-memory-server commits and pushes, `cargo update` in either
consumer will resolve to `edca63d` or older — i.e. the resync target does not
exist remotely.

- **Severity**: BLOCKER — nothing downstream can proceed until this is done.
- **Action**: commit the two phases on surreal-memory-server (the user must
  authorize the commit — KBD does not commit unsolicited) and push to `main`.

### F-2 — HIGH — UAR `mcp_server.rs` has stale TaskStream call-site arity

`src/uar/memory/mcp_server.rs` calls the TaskStream API with the *pre-scoping*
signatures. Against the current library these are hard compile errors:

| Call site | Current UAR call | Required signature |
|---|---|---|
| `mcp_server.rs:718` | `get_task_stream(&p.name)` | `get_task_stream(name, user_id, agent_id)` |
| `mcp_server.rs:732` | `add_to_task_stream(&p.stream_name, mem)` | `add_to_task_stream(stream_name, user_id, agent_id, memory)` |
| `mcp_server.rs:747` | `get_context_for_task(&p.stream_name, &p.model_name, p.max_tokens)` | `get_context_for_task(stream_name, user_id, agent_id, model_name, max_tokens)` |
| `mcp_server.rs:773` | `archive_task_stream(&p.name)` | `archive_task_stream(name, user_id, agent_id)` |

`create_task_stream` (`:705`) and `list_task_streams` (`:760`) still match.
The UAR-side `*Params` structs may need `user_id`/`agent_id` fields added so the
scope values can be threaded through. Plan must inspect each param struct.

- **Severity**: HIGH — UAR will not compile against the updated library.
- **Files**: `src/uar/memory/mcp_server.rs` (call sites + the `*Params` structs
  around lines 280-296).

### F-3 — MEDIUM — librefang resync is low-risk but unverified

librefang wraps `SurrealStorage` and never calls the TaskStream/TaskStep
surface, uses `RetryConfig::default()`, and is already pinned at current HEAD
`edca63d`. The new trait methods (no defaults) do not break it because it does
not implement the trait; the new `RetryConfig` field does not break it because
it uses `::default()`. Expected resync: `cargo update -p surreal-memory` + a
build/test pass. "Expected" is not "verified" — the build must actually be run.

- **Severity**: MEDIUM — low blast radius, but must be confirmed, not assumed.
- **Files**: `crates/librefang-memory/src/backends/surreal.rs`, `shared.rs`.

### F-4 — MEDIUM — Both `Cargo.lock` files are stale and must be refreshed

UAR's lock pins `01e917ac` (older than HEAD); librefang's pins `edca63d`.
Neither includes the two uncommitted phases. After F-1, both need
`cargo update -p surreal-memory` so the lock pins the new pushed commit, and the
lock change must be committed in each consumer repo.

- **Severity**: MEDIUM — mechanical, but required for a reproducible build.

### F-5 — MEDIUM — Library migrations run against the consumers' existing DBs

Both consumers embed SurrealDB. On startup the library auto-runs pending
migrations. Resyncing pulls in **v17→v19** (dynamic embedding index metadata;
v18 `task_stream` scope unique index **with a NONE→'' backfill**; v19
`task_step` table). v18 issues `UPDATE task_stream SET ...` against existing
rows. For any consumer with an existing populated embedded database, this is a
data-touching schema change on first run after the resync — it must be verified
safe (the v18 migration was designed additive + backfilled, but confirm against
a real consumer DB, not just a fresh one).

- **Severity**: MEDIUM — not a regression, but a real first-run effect on
  consumer data that the resync must validate, not ignore.

### F-6 — LOW — `branch = "main"` pin is a latent supply-chain hazard

Both consumers pin by `branch = "main"`, not by `rev`. CLAUDE.md rule #7
describes a `rev` pin — the actual `Cargo.toml`s have drifted to a branch pin.
A branch pin means any future `cargo update` silently absorbs **whatever** is on
`main`, with no review gate. This is how an unreviewed library change reaches
production unnoticed. The phase should record an explicit decision: keep the
branch pin (convenient, risky) or move both consumers to `rev` pins (reviewable,
matches the documented rule). Either way, CLAUDE.md rule #7 should be reconciled
with reality.

- **Severity**: LOW — not blocking this resync, but a standing hazard worth a
  recorded decision.

---

## 4. Severity Roll-up

| Severity | Count | Findings |
|----------|-------|----------|
| BLOCKER  | 1 | F-1 |
| HIGH     | 1 | F-2 |
| MEDIUM   | 3 | F-3, F-4, F-5 |
| LOW      | 1 | F-6 |

**What is NOT broken (verified, so the Plan does not chase it):** neither
consumer implements `MemoryStorage`, so the 5 new no-default trait methods do
not break either compile. Both use `RetryConfig::default()` /
`SurrealConfig{..Default::default()}`, so the new `operation_deadline_ms` field
does not break either. `SurrealStorage::new`'s signature is unchanged.
`EmbeddingService::is_ready()` is defaulted. SurrealDB is `=3.0.5` in all three
repos. These are genuine de-risking facts — but they do not make the phase
"light-touch," because F-1 blocks it and F-2/F-5 are real work.

---

## 5. Recommended Direction for Plan Phase

Ordered, because F-1 gates everything:

1. **Commit + push the library** (F-1) — the user authorizes a commit of the
   two completed phases on surreal-memory-server; push to `main`. Fold in or
   first fix the pre-existing `tests/contract_alignment_test.rs:300/312`
   compile error so the library's own `cargo check --tests` is green before it
   becomes the resync target.
2. **UAR resync** (F-2, F-4) — `cargo update -p surreal-memory`; fix the 4
   stale TaskStream call sites in `mcp_server.rs` (and thread `user_id`/
   `agent_id` through the `*Params` structs); `cargo build` + `cargo test`;
   commit the lock + call-site changes.
3. **librefang resync** (F-3, F-4) — `cargo update -p surreal-memory`;
   `cargo build` + `cargo test`; commit the lock change. Expected to be a
   no-code-change update — confirm.
4. **Migration safety check** (F-5) — verify v17→v19 apply cleanly against a
   populated consumer database for each consumer, not only a fresh one.
5. **Pinning decision** (F-6) — record keep-branch vs move-to-rev; reconcile
   CLAUDE.md rule #7 with the actual pin style.

---

## 6. Out of Scope (note for Plan / follow-up)

- The new `TaskStep` methods give UAR a chance to expose durable step tracking
  in its MCP surface — but *adopting* TaskStep is a feature, not a resync.
  Resync = make it compile and run; TaskStep adoption is a separate phase.
- Standing prior follow-ups remain: `update_task_stream_status` bypassing the
  operation deadline; TaskStep REST endpoints; MCP progress behavioral test.

---

## Phase Status: ASSESSMENT COMPLETE

Gap report only — no code, no change structures produced (KBD strict phase
ordering). The phase is **blocked** until F-1 (commit + push the library) is
resolved. Next step: `/kbd-plan uar-consumer-resync`.
