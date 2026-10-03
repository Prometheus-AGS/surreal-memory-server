---
type: SessionRecord
id: karpathy-session-64285e3a00d327ca
title: Karpathy session 64285e3a00d3
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T14:48:36.069422+00:00
created_at: 2026-09-26T14:48:36.069422+00:00
updated_at: 2026-09-26T14:48:36.069422+00:00
revision: 0
---

## Delta

c2 has started on the new branch, but I don't have its answer yet. The traced load run that should show what's tearing down the connection is still going.

**Branch:**
- **New branch:** `feat/surrealdb-3x-connection-model`, off the new `main` (which includes merged PR #26). It's pushed, with the child phase commit (`476ae6d`) on it.
- **Committed right away:** I'm committing each step as soon as it builds, so the Codex sweep has nothing uncommitted to pick up. The latest commit is `5d39810`.

**What c2 has found so far,** from the SDK source (`surrealdb-3.3.0/src/engine/remote/ws/`):
- **Where the error comes from:** `Connection reset` is the SDK's own message. When its WebSocket router tears the connection down, it fails every request still in flight with that text, then reconnects. That fits the 23 extra WebSockets per run.
- **When the router tears down:** in four cases.
  - sending a request fails;
  - handling a response fails;
  - the socket stream ends or errors;
  - a ping fails. It pings every 5 s.
- **Why the logs showed nothing:** the specific reason is logged only at `trace` level, below what anything had been capturing.

**What's running:**
- **Harness change:** I added opt-in tracing to the load harness, test code only, off by default. It's switched on with `LOAD_REPRO_TRACE`.
- **The run:** one server-mode run on a fresh 3.3.0 scratch server, with the SDK's WebSocket router traced at `trace` and the server at `debug`.
- **The baseline stays clean:** the script restores `load_repro_baseline.md` afterward, so this diagnostic run doesn't get recorded as a baseline.

When it finishes, I'll look at the trace line just before each of the roughly 23 `Reconnecting...` entries to see which case fires. The fix then depends on the cause: a limit to raise, a stalled router to unblock, or something in our code.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T14:47:46.918424Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
