# Execution — surrealdb-connection-architecture

**Date**: 2026-05-24
**Backend**: `openspec` (per `.kbd-orchestrator/project.json` and the
presence of `openspec/changes/fix-surrealdb-connection-architecture/`)
**Change ID**: `fix-surrealdb-connection-architecture`
**Dispatch start**: Change 1 — `load-repro-harness`

## User answers captured (gates downstream changes)

| Question | Answer | Effect |
|---|---|---|
| Server test target for harness | Local docker-compose (`ws://localhost:28000`) | Change 1 reads `LOAD_REPRO_ENDPOINT` env (defaults to `ws://localhost:28000` per repo `docker-compose.yaml`); test is `#[ignore]` so CI is not coupled to a running container. No `testcontainers` dependency added. |
| Deployment shape (gates Change 5) | Decide later (after harness data) | Change 5 stays gated. Plan unchanged. Decision deferred to post-Change-2 harness re-run. |
| Embedded mode status | **Production target** | **Change 6 re-scoped**: not "dev-only framing" — now must include a benchmark proving embedded scales to a defined production load envelope. CLAUDE.md / AGENTS.md doc framing reversed (already done in this turn). |
| `MemoryStorage` trait surface (Change 4) | Decide after Change 4 design sketch | Change 4 executor writes the internal classifier first, then presents a design sketch; trait-surface decision made then, not now. |

## Change inventory and totals

`changes_total = 7`, `changes_completed = 1` (Change 7 docs were done during plan).

| # | Change | Status | Notes |
|---|---|---|---|
| 1 | `load-repro-harness` | **next** | Local docker-compose endpoint; `#[ignore]`. Baseline before any code change. |
| 2 | `arcswap-connection-cell` | pending | Critical fix. Blocked by Change 1's baseline. |
| 3 | `config-query-timeout` | pending | Parallel with 4 and 7 after 2. |
| 4 | `typed-error-retry` | pending | Internal classifier first; trait-surface decision after design sketch. |
| 5 | `workload-isolated-sessions` | pending-decision | Gated by user §6.1 answer + Change 1 harness data. |
| 6 | `embedded-mode-benchmark` *(re-scoped)* | pending | Now includes production-scale benchmark proving embedded + semaphore handles the target workload. |
| 7 | `docs-skill-references-and-discipline` | **done** | CLAUDE.md, AGENTS.md, docs/lessons.md, tasks.md updated during plan + execute prep. |

## Per-change QA gate

After each change reaches `DONE`:

1. Verify the cross-cutting discipline tasks (Karpathy + rust-skills invocations + lessons.md append if applicable).
2. Run the artifact-refiner QA gate (skip for Change 7 — docs-only, already done).
3. If PASS → mark COMPLETE in `progress.json`; if FAIL → mark BLOCKED and surface the refinement log.

## Skip conditions

- Change 7 is already done and is documentation-only — QA gate not re-run.
- Changes 1–6 all touch ≥3 files; **no QA skips authorized.**

## Dispatch contract for Change 1

**Goal**: produce a deterministic, repeatable load harness against
*both* server mode (docker-compose) and embedded mode that surfaces the
user-reported failure signatures (timeouts / sync errors under load) on
the *current* `Arc<std::sync::RwLock<ConnectionState>>` implementation.

**Acceptance criteria (verifiable)**:
1. `cargo test --test load_repro --features embedded,metal --release -- --ignored` runs to completion against a live docker-compose SurrealDB (`ws://localhost:28000`).
2. The harness emits p50 / p95 / p99 latency and error breakdown for:
   - `hybrid_search_memories` × 64 concurrent tasks
   - `add_memory` × 64 concurrent tasks
   - mixed workload (50/50) × 128 concurrent tasks
   - same three workloads against embedded mode (separate test run)
3. Baseline numbers are committed to `crates/surreal-memory/tests/load_repro_baseline.md`.
4. The harness either reproduces the symptoms (timeouts in server mode under mixed load OR sync errors in embedded under concurrent writes) OR the harness operator (Claude) documents the negative result and escalates to the user before Change 2 starts.

**Boundaries (Karpathy #3 — surgical)**:
- New files only: `crates/surreal-memory/tests/load_repro.rs`, `crates/surreal-memory/tests/load_repro_baseline.md`.
- May add a single `[dev-dependencies]` entry to `crates/surreal-memory/Cargo.toml` (likely none needed — use `tokio::join!`, `std::time::Instant`, and the existing crate API).
- **No other file may be modified** in Change 1. If the harness reveals an obvious bug, file it as a follow-up — do not fix it in this change.

**Skills to invoke before writing code**:
- `karpathy-guidelines` — discipline gate
- `rust-skills:m07-concurrency` — concurrent load generation
- `rust-skills:m10-performance` — measurement methodology
- `rust-skills:domain-cloud-native` — docker-compose interaction
- `prometheus-rust-auditor` — pre-merge review

**Lessons-loop expectation**: if the harness exposes any new failure mode
not predicted by the assessment, append a line to `docs/lessons.md` before
closing the change.

## Next step after Change 1 closes

`Change 2 — arcswap-connection-cell`. Re-run the same harness post-refactor;
require ≥3× p99 improvement on the mixed-workload server-mode test before
marking Change 2 COMPLETE.
