---
type: SessionRecord
id: karpathy-session-2076bf0b55c8b169
title: Karpathy session 2076bf0b55c8
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T15:44:28.586983+00:00
created_at: 2026-09-26T15:44:28.586983+00:00
updated_at: 2026-09-26T15:44:28.586983+00:00
revision: 0
---

## Delta

I have enough to lay out the plan. One gap remains, and I'll treat it as the first step rather than guess: the live server's companion `surreal-memory-mlx-executor` isn't built from this repo. So the exact build recipe for the installed pair still has to be confirmed.

# Plan: getting surreal-memory back online

## Current state (checked just now, read-only)

| Symptom | Evidence | Cause | Status |
|---|---|---|---|
| **Claude Code can't connect over MCP** | `mcp-remote` gets an HTTP 404 on `/mcp/messages?sessionId=…` (the path `/mcp/sse` advertises) | Legacy SSE message routing | Not diagnosed. `/mcp/http` (streamable HTTP) answers `initialize` with a 200 |
| **Live server: ledger timeouts, 500s** | Log: 34× `connection replacement timed out stage="receipt lookup"`, executor journal write failures, 22 s responses | Likely the per-call sessions and whole-table searches fixed by c1/c2, which the live binary lacks. Unconfirmed | Live binary is v1.8.0, built Sep 21, predating both fixes |
| **Mixed-load connection resets** | ~68% of mixed-load operations failed | Whole-table search responses over 64 MiB | **Fixed on the branch** (c2, gate met) |
| **Session per call** | ~2,700 sign-ins per run | `Surreal::clone()` opens a session in SDK 3.x | **Fixed on the branch** (c1, merged via PR #26) |
| **UAR** | Uses a vendored copy of this library | It still has both defects | Needs a handoff, not an edit here |

## Team (from `.agent-team/memory-core`, at most 4 agents at once)

| Role | Job here | Writes |
|---|---|---|
| **memory-lead** (me) | Order the work, own the deploy, docs and root instructions, integrate | `CLAUDE.md`, `AGENTS.md`, `docs/lessons.md`, KBD, PR, deploy |
| **memory-verifier** | Fresh-context review of c1 and c2 before merge; checks the deploy claims | Findings file only |
| **memory-transports** | Find the cause of the `/mcp/messages` 404; decide whether clients move to `/mcp/http` or SSE gets fixed | `src/mcp/*` if a fix is needed |
| **memory-runtime** | Only if ledger timeouts survive the deploy: diagnose the ledger connection path | `src/operations.rs` |
| **memory-product** | Write the UAR handoff (vendored library, one-way embedded migrations) | Handoff doc only |

Nothing else changes storage code. c1 and c2 are finished.

## Process (in order; each step has its own pass/fail check)

**0. Stop the Codex sweep from committing in this repo.** Its cross-repo integration session commits and merges uncommitted work here (it merged PR #26). While we deploy, I need you to pause it or exclude this repo. *This is the only step I can't do myself.*

**1. Independent review of c1 and c2.**
- **Who:** memory-verifier, using a review packet from `review-protocol.md`. That means the diff, intent and the load-test receipts, but not my reasoning.
- **Judge:** preferably a model from a different family through the liter-llm gateway; otherwise a fresh subagent, with that recorded as a limitation.
- **Pass:** no CRITICAL findings. Those would block the merge.

**2. One serialized quality run** on this branch:
- **Commands:** `cargo fmt --check`, `cargo clippy --all-targets --features embedded,metal -D warnings`, `cargo test --all-targets --features embedded,metal`.
- **Tests use** `TEST_SURREAL_ENDPOINT` pointed at a scratch 3.3.0 server, never `:28000`.
- **Known issue:** formatting drift in `storage/auth.rs` predates this work. I'll report it, not fold it in.

**3. c6 docs (me).** Correct the "clone `Surreal<Any>` per task" guidance in `CLAUDE.md` gotcha #6, `AGENTS.md` and `docs/lessons.md`, and record the lessons from today. **Pass:** no "clone per task" guidance remains.

**4. PR and merge.**
- A PR from `feat/surrealdb-3x-connection-model` to `main`, with the load-test evidence, the review findings and the quality-run output.
- **Your approval before merge.**

**5. Deploy to the launch agent.**
- **5a:** confirm how the installed `surreal-memory-server` plus `surreal-memory-mlx-executor` pair is built (features, source 

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T15:44:27.616232Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .claude/worktrees/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- crates/surreal-memory/.prometheus/
- history.txt
