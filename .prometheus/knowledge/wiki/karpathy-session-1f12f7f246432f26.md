---
type: SessionRecord
id: karpathy-session-1f12f7f246432f26
title: Karpathy session 1f12f7f24643
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T13:35:21.891465+00:00
created_at: 2026-09-26T13:35:21.891465+00:00
updated_at: 2026-09-26T13:35:21.891465+00:00
revision: 0
---

## Delta

```
KBD STATUS — surreal-memory-server
phase: surrealdb-connection-architecture › change-5
worktree: /Users/gqadonis/Projects/prometheus/surreal-memory-server  ⚠ outside worktreeRoot (/Users/gqadonis/.claude/worktrees)
Last updated by: kbd-runtime (2026-09-26T09:55:51Z, revision 29, plan revision 2)
Lifecycle: ready

Implementation: 6/7 IN_PROGRESS
Evidence:       NOT_TRACKED
Certification:  NOT_TRACKED
Publication:    NOT_TRACKED
Project-wide:   14/17 implementation (all phases)

Changes:
  DONE:     change-1  load-repro-harness          (closed today, a50f6da)
  DONE:     change-2  arcswap-connection-cell     (≥3× p99 gate UNPROVEN, no before-fix baseline)
  DONE:     change-3  config-query-timeout
  DONE:     change-4  typed-error-retry
  PENDING:  change-5  workload-isolated-sessions  (CONDITIONAL: waiting on your answer to assessment §6.1)
  DONE:     change-6  embedded-mode-benchmark
  DONE:     change-7  docs-skill-references-and-discipline

OpenSpec:
  Active:   fix-surrealdb-connection-architecture (this phase),
            memory-code-graph-team, reconcile-library-pin-policy,
            resync-librefang-consumer
  Archived: 14 (latest: 2026-09-25-direct-operation-receipt-lookup,
            2026-09-25-bound-embedding-dimension-validation)

Decision log: none (no decision-log.md for this phase)

Next action (from waypoint): change-5, which needs your decision first:
  • Vertical scale (single binary): implement a SessionPool that sends reads
    and writes to separate session buckets.
  • Horizontal scale (behind a load balancer): skip the code and document
    the deployment shape in docs/DEPLOYMENT.md.
```

**Notes:**
- **Worktree warning:** this checkout is outside `~/.claude/worktrees`. That's only informational; this is the main repo checkout.
- **Baseline findings not yet planned:**
  - Server mode: about 68% of mixed read/write operations fail with `Connection reset`.
  - Embedded mode: 5–19% hit the 10 s per-query timeout.
  - Neither maps to a planned change. Change-5's session split might help with the server-mode failures, but those look like a reconnect problem, which is a separate fix. I'd add them as a new change (a `kbd revise`) rather than fold them into change-5.
- **Unpushed commits:** `a1b84ad`, `ad97c9c` and `a50f6da` are on `feat/closeout-scaling-rmcp3-surreal33`, which has no remote yet.

Completed kbd-status — surrealdb-connection-architecture

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T13:35:20.499240Z
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
