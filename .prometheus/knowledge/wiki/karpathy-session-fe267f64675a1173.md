---
type: SessionRecord
id: karpathy-session-fe267f64675a1173
title: Karpathy session fe267f64675a
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T09:17:26.643131+00:00
created_at: 2026-09-26T09:17:26.643131+00:00
updated_at: 2026-09-26T09:17:26.643131+00:00
revision: 0
---

## Delta

Everything now runs on SurrealDB 3.3.0, and the change-1 load test is running on `HEAD` (3.3.0 client against a 3.3.0 server). It has no results yet.

- **Launch agent:** confirmed on 3.3.0 and serving normally.
- **Client pin:** committed as `ad97c9c` (`surrealdb` and `surrealdb-types` pinned to `=3.3.0`). The full workspace compiled cleanly against it. A stale git lock file from about 01:22 was blocking the commit, and no git process was running, so I removed it.
- **3.0.5 builds:** I removed the before-fix and fix worktrees along with their comparison script. Nothing in this work uses anything older than 3.3.0 now.
- **Load test:** two rounds of server mode and embedded mode. Server mode uses a fresh scratch server started from the same 3.3.0 binary as the launch agent, not the shared `:28000` store your live services use. Each test is capped at 20 minutes, so a hang shows up as a result instead of stalling everything.

This drops the before-fix comparison, so change-2's "≥3× faster" gate can't be proven. I'll record it as unprovable rather than passed.

Load is still around 120, so the latency numbers will be noisy. The test will still show whether the fixed code hangs or throws lock or serialization errors under concurrent load. When the runs finish, I'll add the results to `load_repro_baseline.md` with the load at the time and mark change-1 done.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T09:11:31.037798Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
