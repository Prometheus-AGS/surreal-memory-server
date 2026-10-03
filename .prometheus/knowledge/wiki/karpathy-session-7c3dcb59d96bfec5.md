---
type: SessionRecord
id: karpathy-session-7c3dcb59d96bfec5
title: Karpathy session 7c3dcb59d96b
tags:
- karpathy
- session-learning
sources:
- session:2c0974aa-293e-4bc3-b424-a1541ffe6077
timestamp: 2026-09-25T21:56:02.018295+00:00
created_at: 2026-09-25T21:56:02.018295+00:00
updated_at: 2026-09-25T21:56:02.018295+00:00
revision: 0
---

## Delta

The plan is set. Here's where each part stands:

- **memory-storage** (worktree, running): SurrealDB 3.3.0, the change-2/3/4/6 fixes, server-mode scaling, harness fixes, then load runs on port 28110.
  - The scaling fix uses separate connections: an interactive-read connection and a heavy-write connection, instead of clones that share one WebSocket.
  - The change-6 fix applies the embedded concurrency cap to every storage call.
  - The load runs compare the heavy-write workload with and without the separate connections.
- **memory-transports** (worktree, running): rmcp 3.4.0, and explicit support for all five MCP protocol versions (2024-11-05 through 2026-07-28) over both stdio and streamable HTTP. It will find and fix the root cause of the initialization failures, and test every version on both transports.
- **Ledger** (done): activated the phase, recorded decisions D-001 to D-004 and plan revision 2, fixed `project.json`, and archived the two finished OpenSpec changes.

Once both implementers finish:
1. I merge their branches into `feat/closeout-scaling-rmcp3-surreal33` and run one serialized integration check.
2. A fresh memory-verifier reviews the result.
3. I open and merge the PR.
4. I resync UAR and librefang on fresh branches off their `main`, with surreal-memory updated and rmcp 3.4.0 and SurrealDB 3.3.0 pinned.
5. I close out the KBD phases with the recorded evidence.

I'll report back as each step completes.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 2c0974aa-293e-4bc3-b424-a1541ffe6077
- Captured: 2026-09-25T21:41:33.751534Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
