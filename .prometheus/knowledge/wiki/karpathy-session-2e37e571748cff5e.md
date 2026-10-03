---
type: SessionRecord
id: karpathy-session-2e37e571748cff5e
title: Karpathy session 2e37e571748c
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T10:08:37.555545+00:00
created_at: 2026-09-26T10:08:37.555545+00:00
updated_at: 2026-09-26T10:08:37.555545+00:00
revision: 0
---

## Delta

Change-1 is done: the load test ran on SurrealDB 3.3.0 end to end, the results are committed, and KBD shows change-1 complete (6 of 7 changes done). But the results show the current code still fails badly under mixed read/write load, and change-2's "≥3× faster" goal can't be proven.

**Results** (two runs, 3.3.0 client against a fresh 3.3.0 scratch server, never the shared `:28000`):

| Mode | Mixed read/write, 128 tasks | Read-only or write-only, 64 tasks |
|---|---|---|
| Server | about 68% of operations fail (2,172 and 2,165 of 3,200), with `Connection reset` | 0 errors |
| Embedded | 19%, then 5%, time out (the SDK's 10 s per-query limit) | 0 errors |

- **No hangs:** every run finished well within its 20-minute cap.
- **Lock errors gone:** embedded mode had zero lock or serialization errors, which was change-2's embedded goal.
- **Change-2's speed goal is unproven, not passed.** There's no before-fix baseline under 3.3.0.
- **Latencies are unreliable:** the machine's load average was 110–250 during the runs. The error counts are the dependable part.

**What's committed on `feat/closeout-scaling-rmcp3-surreal33`** (not pushed; the branch has no remote yet):
- `a1b84ad`: the team setup.
- `ad97c9c`: the client pinned to 3.3.0. The whole workspace compiles cleanly against it.
- `a50f6da`: the harness now records one real error message per category. The results in `load_repro_baseline.md` are labelled with the version and machine load. There's also a new line in `docs/lessons.md`.

**Suggested next change** (not started):
1. **Find the cause of the server-mode `Connection reset`s.** Log the full error chain in one run, then fix the reconnect handling. I suspect two problems in `retry_operation_inner`: operations fail straight away without retrying whenever another task is mid-reconnect, and several tasks can reconnect at once, each dropping the others' in-flight queries. Neither is confirmed yet.
2. **Embedded timeouts:** the 10 s per-query limit is too short for 128 concurrent tasks sharing 16 slots. The fix should target the actual contention, not just raise the limit.

**Machine state:**
- **Launch agent:** on 3.3.0 and serving normally. The 3.2.4 snapshot is kept at `~/.prometheus/data/surrealdb/database.db.bak-3.2.4-20260926-004749`.
- **Scratch server:** stopped.

**Not committed:** KBD position files, wiki notes, the archived OpenSpec folders and `.claude/settings.local.json` are still modified in the working tree. Change-5 is the only change left in the phase, and it's waiting on your deployment-shape decision.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T09:57:39.947173Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/current-waypoint.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .kbd-orchestrator/position-reminder.txt
- .kbd-orchestrator/position.json
- .kbd-orchestrator/project.json
- openspec/changes/bound-embedding-dimension-validation/.openspec.yaml
- openspec/changes/bound-embedding-dimension-validation/deployment-evidence.md
- openspec/changes/bound-embedding-dimension-validation/design.md
- openspec/changes/bound-embedding-dimension-validation/proposal.md
- openspec/changes/bound-embedding-dimension-validation/specs/startup-embedding-validation/spec.md
- openspec/changes/bound-embedding-dimension-validation/tasks.md
- openspec/changes/direct-operation-receipt-lookup/.openspec.yaml
- openspec/changes/direct-operation-receipt-lookup/deployment-evidence.md
- openspec/changes/direct-operation-receipt-lookup/design.md
- openspec/changes/direct-operation-receipt-lookup/proposal.md
- openspec/changes/direct-operation-receipt-lookup/specs/operation-receipt-lookup/spec.md
- openspec/changes/direct-operation-receipt-lookup/tasks.md
- .claude/worktrees/
- .prometheus/project.json
- crates/surreal-memory/.prometheus/
- history.txt
- openspec/changes/archive/2026-09-25-bound-embedding-dimension-validation/
- openspec/changes/archive/2026-09-25-direct-operation-receipt-lookup/
- openspec/specs/operation-receipt-lookup/
- openspec/specs/startup-embedding-validation/
