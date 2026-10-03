---
type: SessionRecord
id: karpathy-session-22efd7c4c0d46801
title: Karpathy session 22efd7c4c0d4
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T09:48:36.342260+00:00
created_at: 2026-09-26T09:48:36.342260+00:00
updated_at: 2026-09-26T09:48:36.342260+00:00
revision: 0
---

## Delta

The first round on 3.3.0 finished, and the current code still breaks under the mixed read/write load: 68% of operations failed in server mode, 19% in embedded mode. I'm re-running with the harness now recording one real error message per category, so each count can be traced to its cause. That takes about 20 minutes, in the background.

Round 1 results (3.3.0 client and server, machine load 120–250):

| Mode | Workload | p50 | p99 | Errors |
|---|---|---|---|---|
| server | hybrid_search × 64 | 3.0 s | 4.9 s | 0 |
| server | add_memory × 64 | 10.5 s | 18.5 s | 0 |
| server | mixed × 128 | 4.5 s | **158 s** | **2,172 / 3,200 connection** |
| embedded | hybrid_search × 64 | 3 ms | 53 ms | 0 |
| embedded | add_memory × 64 | 0.9 s | 7.2 s | 0 |
| embedded | mixed × 128 | 11.5 s | 40 s | **610 / 3,200 timeout** |

- **Neither mode hung.** Both finished within their limits, which the before-fix code couldn't do.
- **Embedded mode had zero lock or serialization errors,** which is change-2's embedded gate. Its errors are timeouts instead.
- **The latencies are unreliable** at this machine load. The error counts are the meaningful part.

**Likely cause of the server-mode errors,** to be confirmed by the samples (in `crates/surreal-memory/src/storage/surreal.rs`):
- The retry loop fetches the connection with `let db = self.live_db()?;` before trying the operation. While any task is reconnecting, the shared connection is marked `Reconnecting`. Every concurrent operation then fails immediately with "Connection is currently reconnecting" and never gets retried.
- Nothing stops several tasks from reconnecting at once. Each new connection drops the queries still waiting on the old one, which produces more connection errors and more reconnects.

**Likely cause of the embedded-mode timeouts:**
- Operations queue for one of 16 slots before their 30-second deadline even starts.
- The operation then has to finish within that 30 seconds while 128 tasks compete for the slots.

Neither is fixed in this change. Change-1's rules are measure and document only, so the fixes would be new work after it. The one file I edited is the harness, `load_repro.rs` (test-only):
- It now records a sample error message per category.
- Its section headings now say "3.3.0 client + server" instead of "pre-refactor baseline".

When the re-run finishes, I'll check the samples against these explanations, finalize `load_repro_baseline.md`, and commit the harness and results together. I'll then mark change-1 done and propose the fixes as the next change.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T09:40:25.384186Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/current-waypoint.json
- .kbd-orchestrator/position-reminder.txt
- .kbd-orchestrator/position.json
- .kbd-orchestrator/project.json
- crates/surreal-memory/tests/load_repro.rs
- crates/surreal-memory/tests/load_repro_baseline.md
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
